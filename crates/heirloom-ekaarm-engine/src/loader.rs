use alloc::vec::Vec;
use heirloom_cpu_arm::{Config, Cpu, Unaligned};
use heirloom_fmt_e32::{E32Error, E32Image, OrdinalTable};
use heirloom_mem_paged32::{Prot, VmError};
use crate::layout::{self, CODE_BASE, DATA_BASE, EXIT_STUB, HEAP_BASE, PAGE, STACK_TOP, STUB_BASE};
use crate::mem::GuestMem;
use crate::stubs::{self, Stub};

pub struct Process {
    pub cpu: Cpu,
    pub mem: GuestMem,
    pub stubs: Vec<Stub>,
    pub insns: u64,
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
    let mut cpu = Cpu::new(Config { unaligned: Unaligned::Armv6, vfp: true });
    cpu.regs[13] = STACK_TOP & !7;
    cpu.regs[14] = EXIT_STUB;
    cpu.regs[0..4].copy_from_slice(&[0; 4]);
    cpu.set_pc_interworking(entry);

    Ok(Process { cpu, mem, stubs: stub_list, insns: 0 })
}
