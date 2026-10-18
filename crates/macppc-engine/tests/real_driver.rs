//! Instantiating a REAL CodeWarrior-built PEF: SheepShaver's `ethernet.ndrv` (a Mac OS network driver, a PowerPC code
//! fragment with 6 InterfaceLib imports and 5 exports). Not in this repository; `PEF_CORPUS` (default
//! `/data/refs/retro/macemu/SheepShaver/src/EthernetDriver`) names the directory, a missing file FAILS unless
//! `RETRO_ALLOW_MISSING_REFS` is set (a skip must be chosen, never silent).
use guestui_abi::RecordingGuestUi;
use macppc_engine::census::Census;
use macppc_engine::cfm::{Fragment, ImportStatus};
use macppc_engine::{GlueOutcome, MacPpc, PpcHost};
use retro_pef::Pef;

fn ndrv() -> Option<Vec<u8>> {
    let root = std::env::var("PEF_CORPUS").unwrap_or_else(|_| "/data/refs/retro/macemu/SheepShaver/src/EthernetDriver".into());
    match std::fs::read(format!("{root}/ethernet.ndrv")) {
        Ok(b) => Some(b),
        Err(e) if std::env::var("RETRO_ALLOW_MISSING_REFS").is_ok() => { eprintln!("SKIPPED (RETRO_ALLOW_MISSING_REFS set): real PEF {root}/ethernet.ndrv not available ({e})"); None }
        Err(e) => panic!("the real PEF {root}/ethernet.ndrv is not available ({e}); set PEF_CORPUS, or RETRO_ALLOW_MISSING_REFS=1 to skip these tests knowingly"),
    }
}

fn load(bytes: &[u8]) -> (MacPpc<RecordingGuestUi>, Fragment) {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let frag = m.instantiate("ethernet.ndrv", &Pef::parse(bytes).unwrap()).unwrap();
    (m, frag)
}

/// Call the glue behind import `name` as the driver's own code would: TOC word is the import's TVector, its first word the stub.
fn call(m: &mut MacPpc<RecordingGuestUi>, frag: &Fragment, name: &str, args: &[u32]) -> u32 {
    let tv = frag.imports.iter().find(|i| i.symbol == name).unwrap().value;
    let stub = m.mac.mem.read_u32(tv).unwrap();
    let sym = m.glue_symbol_at(stub).unwrap();
    let mut regs = [0u32; 32];
    regs[3..3 + args.len()].copy_from_slice(args);
    assert_eq!(m.glue_call(&sym, &mut regs), GlueOutcome::Return, "{name}");
    regs[3]
}

#[test]
fn every_import_of_the_real_driver_is_bound_and_its_exports_are_relocated_into_its_sections() {
    let Some(bytes) = ndrv() else { return };
    let (m, frag) = load(&bytes);
    let c = Census::of(&frag);
    eprintln!("{}\n{}", c.lines.join("\n"), c.summary());
    assert_eq!((c.bound, c.next_needed.len(), c.weak_absent), (6, 0, 0));
    assert!(frag.imports.iter().all(|i| matches!(i.status, ImportStatus::Bound(_))));
    // exports landed inside the placed sections: the data section holds TheDriverDescription
    let pef = Pef::parse(&bytes).unwrap();
    for (name, addr) in &frag.exports {
        let inside = pef.sections.iter().enumerate().any(|(i, s)| frag.section_bases[i].is_some_and(|b| *addr >= b && *addr < b + s.total_size));
        assert!(inside, "{name} = {addr:#x} is outside every instantiated section");
    }
    let desc = frag.exports.iter().find(|(n, _)| n == "TheDriverDescription").unwrap().1;
    assert_eq!(&m.mac.mem.read_u32(desc).unwrap().to_be_bytes(), b"mtej", "a DriverDescription record starts with kTheDescriptionSignature 'mtej'");
    // every relocated TVector the driver exports points its code into the code section and its TOC into the data section
    let (code, data) = (frag.section_bases[0].unwrap(), frag.section_bases[1].unwrap());
    let init = frag.exports.iter().find(|(n, _)| n == "InitStreamModule").unwrap().1;
    let (c0, t0) = (m.mac.mem.read_u32(init).unwrap(), m.mac.mem.read_u32(init + 4).unwrap());
    assert!(c0 >= code && c0 < code + pef.sections[0].total_size, "code word {c0:#x} not in section 0 at {code:#x}");
    assert!(t0 >= data && t0 <= data + pef.sections[1].total_size, "toc word {t0:#x} not in section 1 at {data:#x}");
}

#[test]
fn the_drivers_memory_imports_run_the_shared_memory_manager() {
    let Some(bytes) = ndrv() else { return };
    let (mut m, frag) = load(&bytes);
    let p = call(&mut m, &frag, "NewPtrSys", &[64]);
    assert_ne!(p, 0);
    assert_eq!(m.mac.mm.live_ptrs() >= 1, true);
    m.mac.mem.write_bytes(p, b"0123456789abcdef").unwrap();
    call(&mut m, &frag, "BlockMoveData", &[p, p + 16, 16]);
    assert_eq!(m.mac.mem.bytes(m.mac.mem.span(p + 16, 16).unwrap()).unwrap(), b"0123456789abcdef");
    call(&mut m, &frag, "BlockZero", &[p, 16]);
    assert_eq!(m.mac.mem.bytes(m.mac.mem.span(p, 16).unwrap()).unwrap(), [0u8; 16]);
    let live = m.mac.mm.live_ptrs();
    call(&mut m, &frag, "DisposePtr", &[p]);
    assert_eq!(m.mac.mm.live_ptrs(), live - 1);
    // negative control: disposing it again is the Memory Manager's named memWZErr, not silence
    call(&mut m, &frag, "DisposePtr", &[p]);
    assert!(m.mac.refused.iter().any(|r| r.starts_with("DisposePtr:") && r.contains("memWZErr")), "{:?}", m.mac.refused);
}

#[test]
fn the_driver_finds_its_own_imports_dynamically_the_way_open_transport_does() {
    let Some(bytes) = ndrv() else { return };
    let (mut m, frag) = load(&bytes);
    let w = |m: &mut MacPpc<RecordingGuestUi>, at: u32, s: &str| { m.mac.mem.write_u8(at, s.len() as u8).unwrap(); m.mac.mem.write_bytes(at + 1, s.as_bytes()).unwrap(); };
    w(&mut m, 0x6000, "InterfaceLib");
    assert_eq!(call(&mut m, &frag, "GetSharedLibrary", &[0x6000, u32::from_be_bytes(*b"pwpc"), 0, 0x6100, 0x6104, 0x6200]), 0);
    let conn = m.mac.mem.read_u32(0x6100).unwrap();
    w(&mut m, 0x6300, "BlockMoveData");
    assert_eq!(call(&mut m, &frag, "FindSymbol", &[conn, 0x6300, 0x6400, 0x6404]), 0);
    // calling the TVector FindSymbol returned works like the statically imported one
    let tv = m.mac.mem.read_u32(0x6400).unwrap();
    assert_eq!(tv, frag.imports.iter().find(|i| i.symbol == "BlockMoveData").unwrap().value);
    // negative control: a symbol InterfaceLib does not have here
    w(&mut m, 0x6500, "ThisIsNotInInterfaceLib");
    assert_eq!(call(&mut m, &frag, "FindSymbol", &[conn, 0x6500, 0x6400, 0x6404]) as i32, -2802);
}
