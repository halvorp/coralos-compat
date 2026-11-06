//! Rung 1 proofs: real execution of hand-assembled and toolchain-built Mach-O x86_64 images on the host, and every refusal.
//! One guest runs per process, so every test that calls `run` holds `GUEST`.
extern crate std;

use crate::hostmem::HostMem;
use crate::libsystem::{self, Session};
use crate::{darwin_class, run, CodeMem, DarwinClass, Host, Outcome, Prot, Refusal, TrapKind, DARWIN_CLASS_MIN};
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use heirloom_fmt_macho::writer::{BuiltImage, ImageBuilder, Patch};
use heirloom_fmt_macho::{MachO, MachoError};
use std::sync::Mutex;

static GUEST: Mutex<()> = Mutex::new(());

#[derive(Default)]
struct RecordingHost {
    markers: Vec<String>,
    writes: Vec<(i32, Vec<u8>)>,
}

impl Host for RecordingHost {
    fn marker(&mut self, line: &str) {
        self.markers.push(line.to_string());
    }
    fn write(&mut self, fd: i32, bytes: &[u8]) -> isize {
        self.writes.push((fd, bytes.to_vec()));
        bytes.len() as isize
    }
}

/// A tiny x86_64 assembler: just the encodings these programs need, recording RIP-relative patches.
#[derive(Default)]
struct Asm {
    code: Vec<u8>,
    patches: Vec<Patch>,
}

impl Asm {
    fn raw(mut self, bytes: &[u8]) -> Self {
        self.code.extend_from_slice(bytes);
        self
    }
    fn prologue(self) -> Self {
        self.raw(&[0x55, 0x48, 0x89, 0xE5]) // push rbp; mov rbp, rsp
    }
    fn epilogue(self) -> Self {
        self.raw(&[0x5D, 0xC3]) // pop rbp; ret
    }
    fn imm(self, opcode: u8, v: u32) -> Self {
        let mut s = self.raw(&[opcode]);
        s.code.extend_from_slice(&v.to_le_bytes());
        s
    }
    fn mov_edi(self, v: u32) -> Self {
        self.imm(0xBF, v)
    }
    fn mov_esi(self, v: u32) -> Self {
        self.imm(0xBE, v)
    }
    fn mov_edx(self, v: u32) -> Self {
        self.imm(0xBA, v)
    }
    fn mov_eax(self, v: u32) -> Self {
        self.imm(0xB8, v)
    }
    fn lea_rsi_data(mut self, data_offset: usize) -> Self {
        self.code.extend_from_slice(&[0x48, 0x8D, 0x35]);
        self.patches.push(Patch::DataRel32 { at: self.code.len(), data_offset });
        self.raw(&[0, 0, 0, 0])
    }
    fn mov_rsi_from_data(mut self, data_offset: usize) -> Self {
        self.code.extend_from_slice(&[0x48, 0x8B, 0x35]);
        self.patches.push(Patch::DataRel32 { at: self.code.len(), data_offset });
        self.raw(&[0, 0, 0, 0])
    }
    fn call_got(mut self, import: usize) -> Self {
        self.code.extend_from_slice(&[0xFF, 0x15]);
        self.patches.push(Patch::GotRel32 { at: self.code.len(), import });
        self.raw(&[0, 0, 0, 0])
    }
}

/// Builds an image whose imports are registered in the given order (`true` = through the lazy-bind stream).
fn image(asm: Asm, imports: &[(&str, bool)], data: &[u8], rebase_at: &[usize]) -> BuiltImage {
    let mut b = ImageBuilder::new();
    for &(name, lazy) in imports {
        if lazy {
            b.lazy_import(name);
        } else {
            b.import(name);
        }
    }
    b = b.code(&asm.code).data(data);
    for &r in rebase_at {
        b = b.rebase_at_data(r);
    }
    for p in asm.patches {
        b = b.patch(p);
    }
    b.build()
}

const MSG: &[u8] = b"hello from macho\n";

fn hello_program(lazy: bool) -> BuiltImage {
    let asm = Asm::default()
        .prologue()
        .mov_edi(1)
        .lea_rsi_data(0)
        .mov_edx(MSG.len() as u32)
        .call_got(0)
        .mov_eax(7)
        .epilogue();
    image(asm, &[("_write", lazy)], MSG, &[])
}

fn run_image(bytes: &[u8]) -> (Outcome, RecordingHost) {
    let _g = GUEST.lock().unwrap_or_else(|e| e.into_inner());
    let mut host = RecordingHost::default();
    let mut mem = HostMem;
    let out = run(bytes, &["a.out"], &mut host, &mut mem);
    (out, host)
}

// ---- the happy path -------------------------------------------------------------------------------------------------

#[test]
fn hello_writes_and_returns_seven() {
    let (out, host) = run_image(&hello_program(false).bytes);
    assert_eq!(out, Outcome::Exited(7));
    assert_eq!(host.writes, vec![(1, MSG.to_vec())]);
    assert_eq!(
        host.markers,
        vec![
            "[OSXX86 LOAD OK] segments=3 imports=1".to_string(),
            "[OSXX86 WRITE fd=1 n=17]".to_string(),
            "[OSXX86 EXIT code=7]".to_string(),
        ]
    );
}

#[test]
fn lazy_import_is_bound_eagerly() {
    let (out, host) = run_image(&hello_program(true).bytes);
    assert_eq!(out, Outcome::Exited(7));
    assert_eq!(host.writes, vec![(1, MSG.to_vec())]);
}

#[test]
fn rebased_data_pointer_reaches_write() {
    // data = [8-byte absolute pointer to the message][message]; with one GOT slot the data starts at base + 0x1000 + 8.
    let msg_vmaddr = 0x1_0000_0000u64 + 0x1000 + 8 + 8;
    let mut data = msg_vmaddr.to_le_bytes().to_vec();
    data.extend_from_slice(MSG);
    let asm = Asm::default()
        .prologue()
        .mov_edi(1)
        .mov_rsi_from_data(0)
        .mov_edx(MSG.len() as u32)
        .call_got(0)
        .mov_eax(0)
        .epilogue();
    let built = image(asm, &[("_write", false)], &data, &[0]);
    assert_eq!(built.data_vmaddr + 8, msg_vmaddr, "the test's layout assumption");
    let (out, host) = run_image(&built.bytes);
    assert_eq!(out, Outcome::Exited(0));
    assert_eq!(host.writes, vec![(1, MSG.to_vec())], "fails if the slide is not applied to the pointer");
}

#[test]
fn exit_deep_in_the_stack_ends_the_guest() {
    // main: push rbp; mov rbp,rsp; call next; next: push rbp; mov edi,3; call [_exit]; mov eax,99; ... (never reached)
    let asm = Asm::default()
        .prologue()
        .raw(&[0xE8, 0, 0, 0, 0, 0x55])
        .mov_edi(3)
        .call_got(0)
        .mov_eax(99)
        .epilogue();
    let built = image(asm, &[("_exit", false)], &[], &[]);
    let (out, host) = run_image(&built.bytes);
    assert_eq!(out, Outcome::Exited(3));
    assert_eq!(host.markers.last().map(String::as_str), Some("[OSXX86 EXIT code=3]"));
    // the engine survives: a second guest in the same process
    let (again, _) = run_image(&built.bytes);
    assert_eq!(again, Outcome::Exited(3));
    let (hello, _) = run_image(&hello_program(false).bytes);
    assert_eq!(hello, Outcome::Exited(7));
}

#[test]
fn exit_alias_double_underscore() {
    let asm = Asm::default().prologue().mov_edi(5).call_got(0).epilogue();
    let (out, _) = run_image(&image(asm, &[("__exit", false)], &[], &[]).bytes);
    assert_eq!(out, Outcome::Exited(5));
}

// ---- write(2) pointer and fd validation --------------------------------------------------------------------------------

fn write_call(fd: u32, ptr_is_data: bool, ptr: u32, len: u32) -> BuiltImage {
    let mut asm = Asm::default().prologue().mov_edi(fd);
    asm = if ptr_is_data { asm.lea_rsi_data(0) } else { asm.mov_esi(ptr) };
    let asm = asm.mov_edx(len).call_got(0).epilogue(); // the return value of write stays in eax
    image(asm, &[("_write", false)], MSG, &[])
}

#[test]
fn write_to_bad_pointer_is_refused_by_the_table() {
    let (out, host) = run_image(&write_call(1, false, 0x10, 5).bytes);
    assert_eq!(out, Outcome::Exited(-1));
    assert!(host.writes.is_empty());
    assert!(!host.markers.iter().any(|m| m.contains("WRITE")));
}

#[test]
fn write_to_fd_7_returns_minus_one() {
    let (out, host) = run_image(&write_call(7, true, 0, 17).bytes);
    assert_eq!(out, Outcome::Exited(-1));
    assert!(host.writes.is_empty());
}

#[test]
fn write_with_length_past_the_end_of_a_range_is_refused() {
    let (out, host) = run_image(&write_call(1, true, 0, 0x10_0000).bytes);
    assert_eq!(out, Outcome::Exited(-1));
    assert!(host.writes.is_empty());
}

#[test]
fn write_of_zero_bytes_is_zero_without_a_host_call() {
    let (out, host) = run_image(&write_call(1, true, 0, 0).bytes);
    assert_eq!(out, Outcome::Exited(0));
    assert!(host.writes.is_empty());
}

#[test]
fn guest_runs_on_its_own_stack() {
    // main: push rbp; mov rbp,rsp; write(1, rsp, 8): the table accepts a buffer only inside the image or the guest stack,
    // so 8 comes back only if rsp is inside the guest stack reservation, not on the engine's own stack.
    let asm = Asm::default().prologue().mov_edi(1).raw(&[0x48, 0x89, 0xE6]).mov_edx(8).call_got(0).epilogue();
    let built = image(asm, &[("_write", false)], &[], &[]);
    let (out, host) = run_image(&built.bytes);
    assert_eq!(out, Outcome::Exited(8));
    assert_eq!(host.writes.len(), 1);
    assert_eq!(host.writes[0].1.len(), 8);
}

// ---- admission: raw traps ---------------------------------------------------------------------------------------------------

fn raw_code(code: &[u8]) -> Vec<u8> {
    image(Asm::default().raw(code), &[], &[], &[]).bytes
}

#[test]
fn raw_syscall_is_refused_before_it_runs() {
    let (out, host) = run_image(&raw_code(&[0xB8, 0x04, 0x00, 0x00, 0x02, 0x0F, 0x05, 0xC3]));
    assert_eq!(
        out,
        Outcome::Refused(Refusal::RawTrap { section: "__TEXT,__text".to_string(), offset: 5, kind: TrapKind::Syscall })
    );
    assert!(host.writes.is_empty());
    assert_eq!(host.markers, vec!["[OSXX86 REFUSE RAW-TRAP __TEXT,__text+0x5 kind=syscall]".to_string()]);
}

#[test]
fn raw_int80_and_sysenter_are_refused() {
    let (out, _) = run_image(&raw_code(&[0xB8, 0x01, 0x00, 0x00, 0x00, 0xCD, 0x80, 0xC3]));
    assert_eq!(
        out,
        Outcome::Refused(Refusal::RawTrap { section: "__TEXT,__text".to_string(), offset: 5, kind: TrapKind::Int80 })
    );
    let (out, _) = run_image(&raw_code(&[0x31, 0xC0, 0x0F, 0x34, 0xC3]));
    assert_eq!(
        out,
        Outcome::Refused(Refusal::RawTrap { section: "__TEXT,__text".to_string(), offset: 2, kind: TrapKind::Sysenter })
    );
}

#[test]
fn trap_bytes_inside_an_immediate_are_not_a_trap() {
    // mov eax, 0x0000050f; ret: the bytes 0F 05 are an operand, not an instruction. Real code has these by chance
    // (about one per 100 KiB of /usr/bin/bash and git text), so a byte scan would refuse nearly every real app.
    let (out, _) = run_image(&raw_code(&[0xB8, 0x0F, 0x05, 0x00, 0x00, 0xC3]));
    assert_eq!(out, Outcome::Exited(0x50F));
}

#[test]
fn trap_bytes_in_a_data_segment_are_not_scanned() {
    let asm = Asm::default().prologue().mov_eax(0).epilogue();
    let built = image(asm, &[], &[0x0F, 0x05, 0xCD, 0x80], &[]);
    let (out, _) = run_image(&built.bytes);
    assert_eq!(out, Outcome::Exited(0));
}

#[test]
fn residual_gap_a_jump_into_an_immediate_passes_the_scan() {
    // jmp +1; mov eax, 0xC3050F90 (B8 90 0F 05 C3): the sweep decodes the mov and sees no syscall, yet a jump to the byte after
    // B8 would land on 0F 05. The admission scan is hygiene for honest apps; the boundary against a hostile guest is the
    // engine's capability confinement and the kernel's refusal of Darwin numbers (docs/heirloom/25-OSXX86-ENGINE.md section 2).
    // This test pins the gap so nobody mistakes the scan for the boundary. It never runs the code.
    let code = [0xEB, 0x01, 0xB8, 0x90, 0x0F, 0x05, 0xC3, 0xC3];
    assert_eq!(crate::trap::scan_code(&code), None);
    // ...and the same bytes entered at the jump target are a syscall, found when the sweep starts there:
    assert_eq!(crate::trap::scan_code(&code[4..]), Some((0, TrapKind::Syscall)));
}

#[test]
fn trap_scan_unit() {
    use crate::trap::{scan_all, scan_code};
    assert_eq!(scan_all(&[0x90, 0x0F, 0x05, 0xCD, 0x80, 0x0F, 0x34]), vec![(1, TrapKind::Syscall), (3, TrapKind::Int80), (5, TrapKind::Sysenter)]);
    assert_eq!(scan_code(&[]), None);
    assert_eq!(scan_code(&[0x0F]), None);
    assert_eq!(scan_code(&[0xCD, 0x2E]), None, "int 0x2e is not a Darwin gate");
    assert_eq!(scan_code(&[0xCD, 0x81]), Some((0, TrapKind::Int80)), "the Mach gate 0x81 is refused with the BSD one");
}

// ---- admission: imports and container -----------------------------------------------------------------------------

#[test]
fn unbound_import_is_named() {
    let asm = Asm::default().prologue().call_got(0).epilogue();
    let built = image(asm, &[("_fork", false), ("_write", false)], &[], &[]);
    let (out, host) = run_image(&built.bytes);
    assert_eq!(out, Outcome::Refused(Refusal::Unbound { symbols: vec!["_fork".to_string()] }));
    assert_eq!(host.markers, vec!["[OSXX86 REFUSE UNBOUND-SYMBOL _fork]".to_string()]);
    assert!(host.writes.is_empty());
}

#[test]
fn stub_binder_hit_is_refused() {
    let asm = Asm::default().prologue().call_got(0).epilogue();
    let built = image(asm, &[("dyld_stub_binder", false)], &[], &[]);
    let (out, host) = run_image(&built.bytes);
    assert_eq!(out, Outcome::Refused(Refusal::StubBinder));
    assert_eq!(host.markers.last().map(String::as_str), Some("[OSXX86 REFUSE STUB-BINDER]"));
    let (hello, _) = run_image(&hello_program(false).bytes);
    assert_eq!(hello, Outcome::Exited(7), "the engine survives a refused guest");
}

fn refused(bytes: &[u8]) -> Refusal {
    match run_image(bytes).0 {
        Outcome::Refused(r) => r,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn non_pie_is_refused_fixed_base() {
    let mut b = ImageBuilder::new().pie(false).code(&[0xC3]);
    b.import("_write");
    assert_eq!(refused(&b.build().bytes), Refusal::FixedBase { vmaddr: 0x1_0000_0000 });
}

#[test]
fn arm64_is_refused() {
    let bytes = ImageBuilder::new().arch(0x0100_000c).code(&[0xC3]).build().bytes;
    assert_eq!(refused(&bytes), Refusal::Image(MachoError::NotX86_64 { cputype: 0x0100_000c }));
}

#[test]
fn chained_fixups_are_refused() {
    let bytes = ImageBuilder::new().code(&[0xC3]).chained_fixups_marker().build().bytes;
    assert_eq!(refused(&bytes), Refusal::UnsupportedFixups);
}

#[test]
fn unixthread_is_refused_no_entry() {
    let bytes = ImageBuilder::new().code(&[0xC3]).entry_unixthread().build().bytes;
    assert_eq!(refused(&bytes), Refusal::NoEntry);
}

#[test]
fn truncated_image_is_refused() {
    let built = hello_program(false);
    let cut = &built.bytes[..0x1800]; // inside __DATA's file range
    assert!(matches!(refused(cut), Refusal::Image(MachoError::SegmentOutOfFile { .. })));
}

#[test]
fn not_macho_is_refused() {
    assert!(matches!(refused(b"#!/bin/sh\necho hi\n#########################"), Refusal::Image(MachoError::BadMagic(_))));
}

#[test]
fn foreign_dylib_import_is_refused() {
    let mut bytes = hello_program(false).bytes;
    // rename the libSystem path to a framework path of the same length so the image still parses
    let at = bytes.windows(26).position(|w| w == b"/usr/lib/libSystem.B.dylib").unwrap();
    bytes[at..at + 26].copy_from_slice(b"/usr/lib/libOtherOne.dylib");
    assert_eq!(refused(&bytes), Refusal::ForeignDylib { path: "/usr/lib/libOtherOne.dylib".to_string() });
}

// ---- memory discipline -------------------------------------------------------------------------------------------------

/// A CodeMem that wraps HostMem and notes what the loader asked for and what the pages looked like at publish time.
#[derive(Default)]
struct Recording {
    inner: HostMem,
    reserves: Vec<usize>,
    published: Vec<Vec<(usize, usize, Prot)>>,
    got_slot_at_publish: Vec<u64>,
}

impl CodeMem for Recording {
    fn reserve(&mut self, len: usize) -> Option<*mut u8> {
        self.reserves.push(len);
        self.inner.reserve(len)
    }
    fn publish(&mut self, base: *mut u8, ranges: &[(usize, usize, Prot)]) -> Result<(), &'static str> {
        if self.published.is_empty() {
            // __DATA starts at +0x1000; its first word is the GOT slot of the first import
            // SAFETY: the reservation covers __TEXT, __DATA and __LINKEDIT (0x3000 bytes) and is still read-write here.
            let slot = unsafe { core::ptr::read_unaligned(base.add(0x1000) as *const u64) };
            self.got_slot_at_publish.push(slot);
        }
        self.published.push(ranges.to_vec());
        self.inner.publish(base, ranges)
    }
    fn release(&mut self, base: *mut u8, len: usize) {
        self.inner.release(base, len)
    }
}

#[test]
fn w_xor_x_is_published() {
    let _g = GUEST.lock().unwrap_or_else(|e| e.into_inner());
    let mut host = RecordingHost::default();
    let mut mem = Recording::default();
    let out = run(&hello_program(false).bytes, &["a.out"], &mut host, &mut mem);
    assert_eq!(out, Outcome::Exited(7));
    assert_eq!(mem.got_slot_at_publish.len(), 1);
    assert_ne!(mem.got_slot_at_publish[0], 0, "binds are applied before the pages are published");
    // image publish first, then the guest stack
    assert_eq!(mem.published[0], vec![(0, 0x1000, Prot::RX), (0x1000, 0x1000, Prot::RW), (0x2000, 0x1000, Prot::R)]);
    assert_eq!(mem.published[1][0], (0, 0x1000, Prot::None), "the stack has a guard page at the bottom");
    assert_eq!(mem.published[1][1].2, Prot::RW);
    assert_eq!(mem.reserves.len(), 2, "one reservation for the image, one for the stack");
}

#[test]
fn busy_is_refused_when_a_guest_is_active() {
    let _g = GUEST.lock().unwrap_or_else(|e| e.into_inner());
    let mut host = RecordingHost::default();
    let mut save = crate::enter::SavedCtx::default();
    let mut dummy = Session::new(&mut host, &mut save, Vec::new());
    assert!(libsystem::begin(&mut dummy));
    let mut host2 = RecordingHost::default();
    let out = run(&hello_program(false).bytes, &["a.out"], &mut host2, &mut HostMem);
    libsystem::end();
    assert_eq!(out, Outcome::Refused(Refusal::Busy));
    assert_eq!(host2.markers, vec!["[OSXX86 REFUSE BUSY]".to_string()]);
}

// ---- the Darwin trap classes --------------------------------------------------------------------------------------------

#[test]
fn darwin_class_of_stray_traps() {
    assert_eq!(darwin_class(0x2000004), DarwinClass::Bsd);
    assert_eq!(darwin_class(0x1000000 + 31), DarwinClass::Mach);
    assert_eq!(darwin_class(0x3000003), DarwinClass::MachineDependent);
    assert_eq!(darwin_class(4), DarwinClass::Invalid);
    assert_eq!(darwin_class(0), DarwinClass::Invalid);
    assert_eq!(darwin_class(0x4000000), DarwinClass::Invalid);
    for n in 0..=1000u64 {
        assert_eq!(darwin_class(n), DarwinClass::Invalid, "{n} is below the class range");
    }
    assert_eq!(darwin_class(DARWIN_CLASS_MIN), DarwinClass::Mach);
}

// ---- real linker output -------------------------------------------------------------------------------------------------

fn tool(name: &str) -> bool {
    std::process::Command::new(name).arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

/// Compiles and links a fixture with clang + ld64.lld for macOS 10.13 (or `min`), returning the executable's bytes, or None (loudly) without the tools.
fn toolchain_image(test: &str, source: &str, min: &str, extra: &[&str]) -> Option<Vec<u8>> {
    toolchain_image_with(test, source, min, extra, &["libSystem.tbd"], &[])
}

/// `toolchain_image` with its own link inputs (stub .tbd files under fixtures/) and compiler flags.
fn toolchain_image_with(test: &str, source: &str, min: &str, extra: &[&str], tbds: &[&str], cflags: &[&str]) -> Option<Vec<u8>> {
    if !tool("clang") || !tool("ld64.lld") {
        std::eprintln!("SKIP {test}: clang / ld64.lld not found");
        return None;
    }
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let dir = std::env::temp_dir().join(std::format!("osxx86-{}-{test}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let obj = dir.join("x.o");
    let exe = dir.join("x");
    let c = std::process::Command::new("clang")
        .args(["-target", &std::format!("x86_64-apple-macos{min}"), "-O1", "-fno-stack-protector", "-c"])
        .args(cflags)
        .arg(fixtures.join(source))
        .arg("-o")
        .arg(&obj)
        .output()
        .unwrap();
    assert!(c.status.success(), "clang: {}", String::from_utf8_lossy(&c.stderr));
    let l = std::process::Command::new("ld64.lld")
        .args(["-arch", "x86_64", "-platform_version", "macos", min, min])
        .args(extra)
        .arg("-o")
        .arg(&exe)
        .arg(&obj)
        .args(tbds.iter().map(|t| fixtures.join(t)))
        .args(["-e", "_main"])
        .output()
        .unwrap();
    assert!(l.status.success(), "ld64.lld: {}", String::from_utf8_lossy(&l.stderr));
    let bytes = std::fs::read(&exe).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    Some(bytes)
}

#[test]
fn toolchain_hello_runs() {
    let Some(bytes) = toolchain_image("hello", "hello.c", "10.13", &[]) else { return };
    let (out, host) = run_image(&bytes);
    assert_eq!(out, Outcome::Exited(7));
    assert_eq!(host.writes, vec![(1, MSG.to_vec())]);
    assert_eq!(host.markers[0], "[OSXX86 LOAD OK] segments=3 imports=2", "the GOT binder slot and the lazy _write");
}

#[test]
fn toolchain_decoders_agree_with_llvm_objdump() {
    let Some(bytes) = toolchain_image("objdump", "hello.c", "10.13", &[]) else { return };
    if !tool("llvm-objdump") {
        std::eprintln!("SKIP toolchain_decoders_agree_with_llvm_objdump: llvm-objdump not found");
        return;
    }
    let m = MachO::parse(&bytes).unwrap();
    let binds = m.binds().unwrap();
    let got: Vec<(String, u64, bool)> = binds
        .iter()
        .map(|b| (b.symbol.clone(), m.segments[b.segment].vmaddr + b.offset, b.lazy))
        .collect();
    let path = std::env::temp_dir().join(std::format!("osxx86-{}-objdump.bin", std::process::id()));
    std::fs::write(&path, &bytes).unwrap();
    let dump = std::process::Command::new("llvm-objdump").args(["--macho", "--bind", "--lazy-bind", "--rebase"]).arg(&path).output().unwrap();
    std::fs::remove_file(&path).ok();
    let text = String::from_utf8_lossy(&dump.stdout);
    for (sym, addr, lazy) in &got {
        let want = std::format!("{addr:#x}");
        assert!(
            text.lines().any(|l| l.contains(&want) && l.contains(sym.as_str())),
            "llvm-objdump has no line for {sym} at {want} (lazy={lazy}):\n{text}"
        );
    }
    assert!(got.iter().any(|(s, _, lazy)| s == "_write" && *lazy), "the lazy bind of _write was decoded");
    let rebases = m.rebases().unwrap();
    assert!(!rebases.is_empty());
    for r in &rebases {
        let want = std::format!("{:#x}", m.segments[r.segment].vmaddr + r.offset);
        assert!(text.contains(&want), "llvm-objdump has no rebase at {want}:\n{text}");
    }
}

#[test]
fn toolchain_exit_deep_in_the_stack() {
    let Some(bytes) = toolchain_image("exits", "exits.c", "10.13", &[]) else { return };
    let (out, host) = run_image(&bytes);
    assert_eq!(out, Outcome::Exited(3));
    assert_eq!(host.writes, vec![(2, b"before exit\n".to_vec())], "nothing after exit() ran");
}

#[test]
fn toolchain_raw_syscall_app_is_refused() {
    let Some(bytes) = toolchain_image("rawtrap", "rawtrap.c", "10.13", &[]) else { return };
    let (out, host) = run_image(&bytes);
    assert!(matches!(out, Outcome::Refused(Refusal::RawTrap { kind: TrapKind::Syscall, .. })), "{out:?}");
    assert!(host.writes.is_empty());
    assert_eq!(host.markers.len(), 1, "no LOAD OK: nothing ran");
}

#[test]
fn toolchain_chained_fixups_are_refused() {
    let Some(bytes) = toolchain_image("chained", "hello.c", "12.0", &["-fixup_chains"]) else { return };
    if !MachO::parse(&bytes).unwrap().has_chained_fixups {
        std::eprintln!("SKIP toolchain_chained_fixups_are_refused: this linker emitted classic fixups for 12.0");
        return;
    }
    assert_eq!(refused(&bytes), Refusal::UnsupportedFixups);
}

#[test]
fn toolchain_no_pie_is_refused() {
    let Some(bytes) = toolchain_image("nopie", "hello.c", "10.13", &["-no_pie"]) else { return };
    assert!(matches!(refused(&bytes), Refusal::FixedBase { .. }), "{:?}", MachO::parse(&bytes).unwrap().flags);
}

/// The first GUI fixture (a Cocoa app in our own source, built against our own declarations) is a real Mach-O with Apple ObjC2
/// metadata whose class references bind to AppKit and whose `objc_msgSend` binds to libobjc. Rung 1 serves libSystem only, so
/// it is refused by name: the negative control that keeps rung 2 honest (it must turn green only when the runtime exists).
#[test]
fn toolchain_gui_fixture_is_refused_until_the_objc_runtime_exists() {
    let tbds = ["libSystem.tbd", "gui/appkit.tbd", "gui/libobjc.tbd"];
    let Some(bytes) = toolchain_image_with("gui", "gui/OsxGuiHello.m", "10.13", &[], &tbds, &["-x", "objective-c", "-fobjc-runtime=macosx", "-fno-objc-arc"]) else { return };
    let m = MachO::parse(&bytes).unwrap();
    assert!(m.dylibs.iter().any(|d| d.path.ends_with("AppKit")), "{:?}", m.dylibs);
    let binds = m.binds().unwrap();
    assert!(binds.iter().any(|b| b.symbol == "_OBJC_CLASS_$_NSWindow"), "the class references bind by name");
    assert!(binds.iter().any(|b| b.symbol == "_objc_msgSend"));
    let (out, host) = run_image(&bytes);
    assert!(matches!(out, Outcome::Refused(Refusal::ForeignDylib { .. })), "{out:?}");
    assert!(host.writes.is_empty());
}
