//! Loading more than one fragment: a private library the program carries or has beside it (PEF bytes supplied by name), its exports bound into the importer's TOC, its init
//! routine run before main; guest RAM large enough for a big application; Universal Procedure Pointer constructors that exist in every library; and naming the import behind a call to 0.
use heirloom_ui_abi::RecordingGuestUi;
use heirloom_macppc_engine::asm::Asm;
use heirloom_macppc_engine::cfm::ImportStatus;
use heirloom_macppc_engine::census::Census;
use heirloom_macppc_engine::symbols::{Binding, Native};
use heirloom_macppc_engine::{GlueOutcome, MacPpc, PpcHost, Step};
use heirloom_fmt_pef::loader::SymbolClass;
use heirloom_fmt_pef::writer::PefBuilder;
use heirloom_fmt_pef::Pef;

type M = MacPpc<RecordingGuestUi>;
const F: SymbolClass = SymbolClass::TVector;

/// A library exporting `get42()` (a function returning 42), `counter` (data) and an init routine that stores 7 into `counter`.
fn test_lib() -> Vec<u8> {
    let mut data = vec![0u8; 32];
    data[8..12].copy_from_slice(&8u32.to_be_bytes());       // init's code offset
    let mut b = PefBuilder::new();
    b.code(&{ let mut c = Asm::new(); c.li(3, 42).blr(); c.li(9, 7).stw(9, 16, 2).li(3, 0).blr(); c.finish() });
    let d = b.data(&data, 32);
    b.reloc(d as u16, &[0x4601]);
    b.export("get42", F, d as i16, 0).export("counter", SymbolClass::Data, d as i16, 16).init(d as i32, 8);
    b.build()
}

/// An application importing TestLib's get42, counter, and a symbol TestLib does not export; main stores get42() and counter into its data.
fn app() -> Vec<u8> {
    let mut a = Asm::new();
    a.mflr(0).stw(0, 8, 1).stwu(1, -64, 1);
    a.call_toc(8).stw(3, 24, 2);                           // get42() -> data[24]
    a.lwz(9, 12, 2).lwz(10, 0, 9).stw(10, 28, 2);          // *counter -> data[28]
    a.addi(1, 1, 64).lwz(0, 8, 1).mtlr(0).blr();
    let mut b = PefBuilder::new();
    b.code(&a.finish());
    let d = b.data(&[0u8; 40], 40);
    b.import_library("TestLib", false, &[("get42", F, false), ("counter", SymbolClass::Data, false), ("absent", F, false)]);
    b.reloc(d as u16, &[0x4600, 0x4A00 | 2]);
    b.main(d as i32, 0);
    b.build()
}

fn boot(lib: Option<&[u8]>) -> (M, heirloom_macppc_engine::cfm::Fragment) {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    if let Some(l) = lib { m.provide_library("TestLib", l); }
    let frag = m.instantiate("app", &Pef::parse(&app()).unwrap()).unwrap();
    m.start(&frag).unwrap();
    (m, frag)
}

#[test]
fn a_supplied_library_is_loaded_on_first_import_its_exports_bind_and_its_init_runs_before_main() {
    let (mut m, frag) = boot(Some(&test_lib()));
    assert!(matches!(&frag.imports[0].status, ImportStatus::Bound(d) if d.contains("export of a loaded fragment")), "{:?}", frag.imports[0].status);
    assert!(matches!(&frag.imports[1].status, ImportStatus::Bound(_)));
    assert!(matches!(&frag.imports[2].status, ImportStatus::Unbound(w) if w.contains("is not exported by the PEF library TestLib")), "{:?}", frag.imports[2].status);
    let c = Census::of(&frag);
    assert_eq!((c.libraries[0].library.as_str(), c.libraries[0].bound, c.libraries[0].named), ("TestLib", 2, 1));
    let data = frag.section_bases[1].unwrap();
    assert_eq!(m.run_loop(100_000).0, Step::Exited, "{:?} {:?}", m.fault(), m.mac.log);
    assert_eq!(m.mac.mem.read_u32(data + 24).unwrap(), 42, "the call went to the library's own code, not to glue");
    assert_eq!(m.mac.mem.read_u32(data + 28).unwrap(), 7, "the library's init routine had run (it stored 7 into its data) before main read it");
    assert_eq!(m.lib_inits.len(), 1);
}

#[test]
fn without_the_library_or_with_a_broken_one_every_import_names_why() {
    let (_, frag) = boot(None);
    assert!(frag.imports.iter().all(|i| matches!(&i.status, ImportStatus::Unbound(w) if w.contains("library TestLib is not provided"))), "{:?}", frag.imports);
    let (_, frag) = boot(Some(b"Joy!peffpwpc not a real fragment"));
    assert!(frag.imports.iter().all(|i| matches!(&i.status, ImportStatus::Unbound(w) if w.contains("could not be loaded"))), "{:?}", frag.imports);
}

#[test]
fn a_warcraft_sized_partition_holds_a_big_block_and_gestalt_reports_real_and_logical_ram() {
    // Warcraft III's SIZE asks for 156 MiB (preferred = minimum, measured under real Mac OS 9.2.2, 2026-10-26).
    let ram = heirloom_macppc_engine::machine::ram_for_program(16 << 20, &[]).max((156 + 16 + 8) << 20);
    let mut m = MacPpc::with_ram(RecordingGuestUi::new(), ram).unwrap();
    let big = m.mac.mm.new_ptr(&mut m.mac.mem, &mut m.mac.heap, 100 << 20, false);
    assert!(big.is_ok(), "a 100 MB block: {big:?}");
    m.bind("Gestalt");
    for (sel, want) in [(*b"ram ", m.mac.mem.len()), (*b"lram", heirloom_macppc_engine::osroutines::LOGICAL_RAM.max(m.mac.mem.len()))] {
        let mut regs = [0u32; 32];
        regs[3] = u32::from_be_bytes(sel); regs[4] = 0x3000;
        assert_eq!(m.glue_call("Gestalt", &mut regs), GlueOutcome::Return);
        assert_eq!(m.mac.mem.read_u32(0x3000).unwrap() as usize, want, "{}", core::str::from_utf8(&sel).unwrap());
    }
    // NEGATIVE CONTROL: a 16 MB machine cannot hold it
    let mut small = MacPpc::with_ram(RecordingGuestUi::new(), 16 << 20).unwrap();
    assert!(small.mac.mm.new_ptr(&mut small.mac.mem, &mut small.mac.heap, 100 << 20, false).is_err());
}

#[test]
fn upp_constructors_are_bound_in_every_library_even_an_absent_one_and_build_a_powerpc_descriptor() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    for (lib, name) in [("CarbonLib", "NewThreadTerminationUPP"), ("Apple;Carbon;Networking", "NewOTNotifyUPP"), ("OpenTransportLib", "NewNMUPP")] {
        assert_eq!(m.bind_from(lib, name), Binding::Native(Native::NewUpp), "{lib}/{name}");
    }
    assert_eq!(m.bind_from("Apple;Carbon;Networking", "DisposeOTNotifyUPP"), Binding::Native(Native::DisposeUpp));
    // NEGATIVE CONTROL: only the UPP constructors: another symbol of the absent library stays unbound by name
    assert!(matches!(m.bind_from("Apple;Carbon;Networking", "OTOpenEndpoint"), Binding::Unbound(w) if w.contains("not provided")));
    // calling it builds a routine descriptor around the TVector (what NewRoutineDescriptor would build for a PowerPC routine)
    m.bind_from("CarbonLib", "NewThreadTerminationUPP");
    let tv = 0x5000u32;
    m.mac.mem.write_u32(tv, 0x6000).unwrap(); m.mac.mem.write_u32(tv + 4, 0x7000).unwrap();
    let mut cpu = heirloom_macppc_engine::cpu::Cpu::new(0, 0x8000);
    let (app_frag, stub) = {
        let frag = m.instantiate("x", &Pef::parse(&{ let mut b = PefBuilder::new(); b.code(&[0; 4]); let d = b.data(&[0; 8], 8); b.import_library("CarbonLib", false, &[("NewThreadTerminationUPP", F, false)]); b.reloc(d as u16, &[0x4A00]); b.main(d as i32, 0); b.build() }).unwrap()).unwrap();
        let data = frag.section_bases[1].unwrap();
        let slot = m.mac.mem.read_u32(data).unwrap();
        (frag, m.mac.mem.read_u32(slot).unwrap())
    };
    let _ = app_frag;
    cpu.gpr[3] = tv;
    assert!(heirloom_macppc_engine::cpu::Host::native(&mut m, &mut cpu, stub));
    let upp = cpu.gpr[3];
    let parsed = heirloom_mac68k_engine::mixed_mode::parse(&m.mac.mem, upp).and_then(|d| heirloom_mac68k_engine::mixed_mode::ppc_record(&d)).unwrap();
    assert_eq!(parsed.proc_descriptor, tv);
}

#[test]
fn a_call_through_an_absent_weak_import_is_traced_to_its_name_instead_of_a_bare_fault_at_zero() {
    // main: bl glue ; blr -- glue (the pointer-glue sequence, TOC slot 8 = the import): lwz r12,8(r2) ; stw r2,20(r1) ; lwz r0,0(r12) ; mtctr r0 ; lwz r2,4(r12) ; bctr
    let glue: [u32; 6] = [0x8182_0008, 0x9041_0014, 0x800C_0000, 0x7C09_03A6, 0x804C_0004, 0x4E80_0420];
    let mut code: Vec<u32> = vec![0x7C08_02A6, 0x9001_0008, 0x9421_FFC0, 0x4800_0011 /* bl +16: the glue below */, 0x4E80_0020, 0, 0];
    code.extend(glue);
    let bytes: Vec<u8> = code.iter().flat_map(|w| w.to_be_bytes()).collect();
    let mut b = PefBuilder::new();
    b.code(&bytes);
    let d = b.data(&[0u8; 16], 16);
    b.import_library("Apple;Carbon;Networking", true, &[("OTOpenEndpoint", F, false)]);
    b.reloc(d as u16, &[0x4600, 0x4A00]);
    b.main(d as i32, 0);
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let frag = m.instantiate("x", &Pef::parse(&b.build()).unwrap()).unwrap();
    assert!(matches!(frag.imports[0].status, ImportStatus::WeakAbsent(_)));
    m.start(&frag).unwrap();
    let (step, _) = m.run_loop(1000);
    assert!(matches!(&step, Step::Fault(w) if w.contains("Apple;Carbon;Networking/OTOpenEndpoint") && w.contains("WEAK and absent")), "{step:?}");
}
