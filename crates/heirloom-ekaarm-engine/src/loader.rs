use alloc::vec::Vec;
use heirloom_cpu_arm::{Config, Cpu, Memory, Unaligned};
use heirloom_fmt_e32::{E32Error, E32Image, OrdinalTable};
use heirloom_mem_paged32::{ChunkId, Prot, VmError};
use crate::layout::{self, CODE_BASE, DATA_BASE, EXIT_STUB, HEAP_BASE, PAGE, STACK_TOP, STUB_BASE};
use crate::mem::GuestMem;
use crate::stubs::{self, Stub};

pub struct Process {
    pub cpu: Cpu,
    pub mem: GuestMem,
    pub stubs: Vec<Stub>,
    pub insns: u64,
    pub heap: Option<crate::heap::Heap>,
    pub heap_chunk_id: ChunkId,
    /// Census mode (`Some`): an unimplemented import returns 0 to the guest and is recorded here in call order, so one run
    /// lists the whole sequence a program reaches. `None` (the default) stops at the first one.
    pub census: Option<Vec<alloc::string::String>>,
    /// The image's UID3 (E32 header): the application start checks `AppDllUid()` against it.
    pub uid3: u32,
    /// Length of the loaded code chunk from `CODE_BASE`: a virtual-table slot inside it is a guest override.
    pub code_len: u32,
    /// Guest calls in flight (the callback machine, `callback.rs`).
    pub frames: crate::callback::Frames,
    /// The virtual-table slot numbers the application start uses (header-derived defaults; a test may change one).
    pub slots: crate::appstart::Slots,
    /// Host state of the natives.
    pub nat: crate::natives::NativeState,
    /// A host extension asked before the natives (tests); `None` in the engine.
    pub native_hook: Option<crate::natives::NativeHook>,
    /// What the application start did, in order (one line per step).
    pub log: Vec<alloc::string::String>,
    /// The host's objects: the guest's file view and its UI (`ui.rs`).
    pub host: crate::ui::Host,
}

#[derive(Debug)]
pub enum LoadError {
    Parse(E32Error),
    Relocate(E32Error),
    Vm { chunk: &'static str, err: VmError },
    EntryOutsideCode,
    Overlap(&'static str),
}

pub fn load(file: &[u8], table: &OrdinalTable) -> Result<Process, LoadError> {
    let mut img = E32Image::parse(file).map_err(LoadError::Parse)?;
    let header = &img.header;
    let code_len = img.code.len() as u32;
    let data_len = img.data.len() as u32;
    let bss_len = header.bss_size;
    let stack_sz = header.stack_size;
    let heap_max = header.heap_max;
    let heap_min = header.heap_min;
    let entry_point = header.entry_point;
    let uid3 = header.uid3;

    let stub_list = stubs::build(&img.imports, table);
    let stub_total = stub_list.len() as u32 * 8;
    layout::check_overlap(code_len, data_len + bss_len, heap_max, stack_sz, stub_total)
        .map_err(LoadError::Overlap)?;

    img.relocate(CODE_BASE, DATA_BASE).map_err(LoadError::Relocate)?;

    let mut mem = GuestMem::new();
    mem.vm.set_budget(u64::MAX);

    let code_id = mem.vm.create_chunk(CODE_BASE, layout::pru(code_len), Prot::RW)
        .map_err(|err| LoadError::Vm { chunk: "code create", err })?;
    mem.vm.commit(code_id, 0, layout::pru(code_len))
        .map_err(|err| LoadError::Vm { chunk: "code commit", err })?;
    mem.vm.write_bytes(CODE_BASE, &img.code)
        .map_err(|err| LoadError::Vm { chunk: "code write", err })?;
    mem.vm.set_prot(code_id, Prot::RX)
        .map_err(|err| LoadError::Vm { chunk: "code protect", err })?;

    let data_total = data_len + bss_len;
    let data_id = mem.vm.create_chunk(DATA_BASE, layout::pru(data_total), Prot::RW)
        .map_err(|err| LoadError::Vm { chunk: "data create", err })?;
    mem.vm.commit(data_id, 0, layout::pru(data_total))
        .map_err(|err| LoadError::Vm { chunk: "data commit", err })?;
    if data_len > 0 {
        mem.vm.write_bytes(DATA_BASE, &img.data)
            .map_err(|err| LoadError::Vm { chunk: "data write", err })?;
    }
    if bss_len > 0 {
        mem.vm.write_bytes(DATA_BASE + data_len, &alloc::vec![0u8; bss_len as usize])
            .map_err(|err| LoadError::Vm { chunk: "bss zero", err })?;
    }

    let heap_id = mem.vm.create_chunk(HEAP_BASE, layout::pru(heap_max), Prot::RW)
        .map_err(|err| LoadError::Vm { chunk: "heap create", err })?;
    mem.vm.commit(heap_id, 0, layout::pru(heap_min))
        .map_err(|err| LoadError::Vm { chunk: "heap commit", err })?;

    let stack_id = mem.vm.create_chunk(STACK_TOP - layout::pru(stack_sz), layout::pru(stack_sz), Prot::RW)
        .map_err(|err| LoadError::Vm { chunk: "stack create", err })?;
    mem.vm.commit(stack_id, 0, layout::pru(stack_sz))
        .map_err(|err| LoadError::Vm { chunk: "stack commit", err })?;

    let stub_id = mem.vm.create_chunk(STUB_BASE, layout::pru(stub_total), Prot::RW)
        .map_err(|err| LoadError::Vm { chunk: "stubs create", err })?;
    mem.vm.commit(stub_id, 0, layout::pru(stub_total))
        .map_err(|err| LoadError::Vm { chunk: "stubs commit", err })?;
    for i in 0..stub_list.len() {
        mem.vm.write_bytes(stubs::stub_addr(i), &stubs::encode_stub(i))
            .map_err(|err| LoadError::Vm { chunk: "stubs write", err })?;
    }
    mem.vm.set_prot(stub_id, Prot::RX)
        .map_err(|err| LoadError::Vm { chunk: "stubs protect", err })?;

    let exit_id = mem.vm.create_chunk(EXIT_STUB, PAGE, Prot::RW)
        .map_err(|err| LoadError::Vm { chunk: "exit create", err })?;
    mem.vm.commit(exit_id, 0, PAGE)
        .map_err(|err| LoadError::Vm { chunk: "exit commit", err })?;
    let mut exit_code = [0u8; 8];
    exit_code[..4].copy_from_slice(&(0xEF00_0000 | layout::EXIT_SVC).to_le_bytes());
    exit_code[4..].copy_from_slice(&0xE12F_FF1Eu32.to_le_bytes());
    mem.vm.write_bytes(EXIT_STUB, &exit_code)
        .map_err(|err| LoadError::Vm { chunk: "exit write", err })?;
    mem.vm.set_prot(exit_id, Prot::RX)
        .map_err(|err| LoadError::Vm { chunk: "exit protect", err })?;
    map_callback_page(&mut mem)?;

    mem.vm.set_prot(code_id, Prot::RW)
        .map_err(|err| LoadError::Vm { chunk: "imports unprotect", err })?;
    let mut idx = 0usize;
    for dll in &img.imports {
        for imp in &dll.imports {
            mem.vm.write_u32(
                CODE_BASE + imp.site,
                stubs::stub_addr(idx).wrapping_add(imp.addend as u32),
            ).map_err(|err| LoadError::Vm { chunk: "imports patch", err })?;
            idx += 1;
        }
    }
    mem.vm.set_prot(code_id, Prot::RX)
        .map_err(|err| LoadError::Vm { chunk: "imports protect", err })?;

    // entry_point is an offset into the code section (header.rs)
    let entry = CODE_BASE.wrapping_add(entry_point);
    if entry < CODE_BASE || entry >= CODE_BASE + code_len {
        return Err(LoadError::EntryOutsideCode);
    }

    // EKA2 runs user code with SCTLR.U=1 A=0 and enables VFP for every thread on ARM11 (S60 3rd/5th edition hardware)
    let cpu = Cpu::new(Config { unaligned: Unaligned::Armv6, vfp: true });
    let mut proc = Process {
        cpu,
        mem,
        stubs: stub_list,
        insns: 0,
        heap: None,
        heap_chunk_id: heap_id,
        census: None,
        uid3,
        code_len,
        frames: Vec::new(),
        slots: crate::appstart::Slots::default(),
        nat: Default::default(),
        native_hook: None,
        log: Vec::new(),
        host: Default::default(),
    };

    // EKA2 entry state: kernel places SStdEpocThreadCreateInfo (0x40 bytes)
    // at the top of the user stack; SP and r1 point at it.
    let info_addr = STACK_TOP - 0x40; // 0x0BFF_FFC0, 8-aligned

    // Zero every register before setting entry arguments so no later partial clear clobbers r1 or r4.
    for r in proc.cpu.regs.iter_mut() {
        *r = 0;
    }
    proc.cpu.regs[1] = info_addr; // r1 = &SStdEpocThreadCreateInfo
    proc.cpu.regs[4] = 0;         // r4 = 0 (first thread of process)
    proc.cpu.regs[13] = info_addr; // SP = top of the 0x40 block (stack grows down)
    proc.cpu.regs[14] = EXIT_STUB; // LR = exit stub
    // r0, r2, r3, r5..r12 stay 0.
    // NOTE: tpidrurw not set — field absent from Cpu.
    // CPSR.T = entry bit 0 (set_pc_interworking handles this).

    // Write SStdEpocThreadCreateInfo (16 LE words, 0x40 bytes).
    let info: [u32; 16] = [
        0,         // 0x00 handle
        0,         // 0x04 type
        entry,     // 0x08 func_ptr
        0,         // 0x0C ptr
        0,         // 0x10 supervisor_stack
        0,         // 0x14 supervisor_stack_size
        info_addr, // 0x18 user_stack (this struct's own address)
        stack_sz,  // 0x1C user_stack_size
        0,         // 0x20 init_thread_priority
        0,         // 0x24 name_len
        0,         // 0x28 name_ptr
        0x40,      // 0x2C total_size
        0,         // 0x30 allocator (0 = "create one")
        heap_min,  // 0x34 heap_min
        heap_max,  // 0x38 heap_max
        0,         // 0x3C padding
    ];
    for (i, word) in info.iter().enumerate() {
        proc.mem.write_u32(info_addr + (i as u32) * 4, *word)
            .map_err(|_| LoadError::Overlap("failed to write SStdEpocThreadCreateInfo"))?;
    }

    proc.cpu.set_pc_interworking(entry);
    Ok(proc)
}
/// Map the callback return stub's page (`callback::CALLBACK_STUB`): `SVC #CALLBACK_SVC`, read-execute.
fn map_callback_page(mem: &mut GuestMem) -> Result<(), LoadError> {
    let id = mem.vm.create_chunk(crate::callback::CALLBACK_STUB, PAGE, Prot::RW)
        .map_err(|err| LoadError::Vm { chunk: "callback create", err })?;
    mem.vm.commit(id, 0, PAGE).map_err(|err| LoadError::Vm { chunk: "callback commit", err })?;
    mem.vm.write_bytes(crate::callback::CALLBACK_STUB, &crate::callback::return_stub_code())
        .map_err(|err| LoadError::Vm { chunk: "callback write", err })?;
    mem.vm.set_prot(id, Prot::RX).map_err(|err| LoadError::Vm { chunk: "callback protect", err })
}

/// TEST HOOK: a process with no image: a 64 KiB stack, a 1 MiB heap (set up), the callback page, and one import stub per
/// name in `stub_names` (stub `i` is `SVC #(STUB_SVC_BASE + i)`, named as given, DLL `test`). Code is added with
/// [`Process::map_code`]. Lets the callback machine and the application start run without the owner's corpus.
#[cfg(feature = "test-hooks")]
pub fn bare(stub_names: &[&str]) -> Result<Process, LoadError> {
    let mut mem = GuestMem::new();
    mem.vm.set_budget(u64::MAX);
    let stack_sz = 0x1_0000u32;
    let stack_id = mem.vm.create_chunk(STACK_TOP - stack_sz, stack_sz, Prot::RW)
        .map_err(|err| LoadError::Vm { chunk: "stack create", err })?;
    mem.vm.commit(stack_id, 0, stack_sz).map_err(|err| LoadError::Vm { chunk: "stack commit", err })?;
    let heap_sz = 0x10_0000u32;
    let heap_id = mem.vm.create_chunk(HEAP_BASE, heap_sz, Prot::RW)
        .map_err(|err| LoadError::Vm { chunk: "heap create", err })?;
    mem.vm.commit(heap_id, 0, heap_sz).map_err(|err| LoadError::Vm { chunk: "heap commit", err })?;
    let stubs: Vec<Stub> = stub_names.iter().enumerate().map(|(i, n)| Stub {
        dll_base: "test".into(), dll: "test".into(), ordinal: i as u16, name: Some((*n).into()),
    }).collect();
    let stub_total = (stubs.len() as u32 * 8).max(8);
    let stub_id = mem.vm.create_chunk(STUB_BASE, layout::pru(stub_total), Prot::RW)
        .map_err(|err| LoadError::Vm { chunk: "stubs create", err })?;
    mem.vm.commit(stub_id, 0, layout::pru(stub_total)).map_err(|err| LoadError::Vm { chunk: "stubs commit", err })?;
    for i in 0..stubs.len() {
        mem.vm.write_bytes(stubs::stub_addr(i), &stubs::encode_stub(i)).map_err(|err| LoadError::Vm { chunk: "stubs write", err })?;
    }
    mem.vm.set_prot(stub_id, Prot::RX).map_err(|err| LoadError::Vm { chunk: "stubs protect", err })?;
    map_callback_page(&mut mem)?;
    let mut cpu = Cpu::new(Config { unaligned: Unaligned::Armv6, vfp: true });
    cpu.regs[13] = STACK_TOP - 0x40;
    cpu.regs[14] = EXIT_STUB;
    Ok(Process {
        cpu, mem, stubs, insns: 0,
        heap: Some(crate::heap::Heap::new(heap_id, HEAP_BASE, heap_sz, heap_sz)),
        heap_chunk_id: heap_id, census: None, uid3: 0, code_len: 0,
        frames: Vec::new(), slots: crate::appstart::Slots::default(), nat: Default::default(), native_hook: None, log: Vec::new(), host: Default::default(),
    })
}

impl Process {
    /// TEST HOOK: map `bytes` as read-execute code at the page-aligned `base` (anything at or above `CODE_BASE` extends the
    /// loaded-code range the vtable-anchor check accepts) and return the chunk.
    #[cfg(feature = "test-hooks")]
    pub fn map_code(&mut self, base: u32, bytes: &[u8]) -> Result<(), LoadError> {
        let len = layout::pru(bytes.len() as u32).max(PAGE);
        let id = self.mem.vm.create_chunk(base, len, Prot::RW).map_err(|err| LoadError::Vm { chunk: "test code create", err })?;
        self.mem.vm.commit(id, 0, len).map_err(|err| LoadError::Vm { chunk: "test code commit", err })?;
        self.mem.vm.write_bytes(base, bytes).map_err(|err| LoadError::Vm { chunk: "test code write", err })?;
        self.mem.vm.set_prot(id, Prot::RX).map_err(|err| LoadError::Vm { chunk: "test code protect", err })?;
        if base >= CODE_BASE {
            self.code_len = self.code_len.max(base - CODE_BASE + len);
        }
        Ok(())
    }

    /// TEST HOOK: map zeroed read-write data at the page-aligned `base` holding `words`, return nothing.
    #[cfg(feature = "test-hooks")]
    pub fn map_data(&mut self, base: u32, words: &[u32]) -> Result<(), LoadError> {
        let len = layout::pru(words.len() as u32 * 4).max(PAGE);
        let id = self.mem.vm.create_chunk(base, len, Prot::RW).map_err(|err| LoadError::Vm { chunk: "test data create", err })?;
        self.mem.vm.commit(id, 0, len).map_err(|err| LoadError::Vm { chunk: "test data commit", err })?;
        for (i, w) in words.iter().enumerate() {
            self.mem.vm.write_u32(base + i as u32 * 4, *w).map_err(|err| LoadError::Vm { chunk: "test data write", err })?;
        }
        Ok(())
    }
}
