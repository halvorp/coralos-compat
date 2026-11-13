//! `DrawDialog` from PowerPC code calls the dialog's user items as PowerPC procedures (Mixed Mode callbacks) and then RETURNS to its caller: the
//! Warcraft III Installer's licence dialog draws its scrolling text from a user item this way. Real interpreted code on both sides.
use heirloom_ui_abi::RecordingGuestUi;
use heirloom_macppc_engine::asm::Asm;
use heirloom_macppc_engine::{MacPpc, Step};
use heirloom_fmt_pef::loader::SymbolClass;
use heirloom_fmt_pef::writer::PefBuilder;
use heirloom_fmt_pef::Pef;

const IMPORTS: [&str; 6] = ["GetNewDialog", "GetDialogItem", "SetDialogItem", "NewUserItemUPP", "DrawDialog", "DialogSelect"];
const TOC: i32 = 16; // data: main TVector [0,8), the user item's [8,16), then one word per import
const D: i32 = TOC + 4 * IMPORTS.len() as i32; // the data area: type, handle, box[8] (D+8..D+16), dialog, calls, item seen, after

fn words(w: &[u32]) -> Vec<u8> { w.iter().flat_map(|x| x.to_be_bytes()).collect() }

fn program() -> Vec<u8> {
    let mut a = Asm::new();
    a.mflr(0).stw(0, 8, 1).stwu(1, -64, 1);
    a.li(3, 128).li(4, 0).li(5, -1).call_toc(TOC); // GetNewDialog(128, NULL, (WindowPtr)-1)
    a.stw(3, D + 16, 2);
    a.lwz(3, D + 16, 2).li(4, 6).addi(5, 2, D).addi(6, 2, D + 4).addi(7, 2, D + 8).call_toc(TOC + 4); // GetDialogItem(d, 6, &type, &h, &box)
    a.addi(3, 2, 8).call_toc(TOC + 12); // NewUserItemUPP(proc)
    a.mr(6, 3).lwz(3, D + 16, 2).li(4, 6).li(5, 0).addi(7, 2, D + 8).call_toc(TOC + 8); // SetDialogItem(d, 6, userItem, upp, &box)
    a.lwz(3, D + 16, 2).call_toc(TOC + 16); // DrawDialog(d)
    a.li(3, 77).stw(3, D + 28, 2).lwz(3, D + 20, 2).stw(3, D + 44, 2); // the caller continues after DrawDialog; the calls so far
    // DialogSelect(&{updateEvt, d}, &dlg, &item): the fresh dialog's update draws the user item again; the answer is FALSE
    a.li(3, 6).sth(3, D + 32, 2).lwz(3, D + 20 - 4, 2).stw(3, D + 34, 2);
    a.addi(3, 2, D + 32).addi(4, 2, D + 52).addi(5, 2, D + 56).call_toc(TOC + 20);
    a.stw(3, D + 60, 2);
    a.addi(1, 1, 64).lwz(0, 8, 1).mtlr(0).blr();
    let mut code = a.finish();
    let proc_at = code.len() as u32;
    // the user item: calls += 1; item seen = r4
    let mut p = Asm::new();
    p.lwz(5, D + 20, 2).addi(5, 5, 1).stw(5, D + 20, 2).stw(4, D + 24, 2).blr();
    code.extend(p.finish());
    let mut b = PefBuilder::new();
    b.code(&code);
    let mut init = vec![0u8; D as usize];
    init[8..12].copy_from_slice(&proc_at.to_be_bytes());
    init.extend(words(&[0; 20]));
    init[(D + 60) as usize..(D + 64) as usize].copy_from_slice(&0xFFFFu32.to_be_bytes()); // DialogSelect's answer slot, not 0 until written
    let d = b.data(&init, init.len() as u32);
    let imports: Vec<(&str, SymbolClass, bool)> = IMPORTS.iter().map(|n| (*n, SymbolClass::TVector, false)).collect();
    b.import_library("InterfaceLib", false, &imports);
    b.reloc(d as u16, &[0x4601, 0x4A00 | (IMPORTS.len() as u16 - 1)]);
    b.main(d as i32, 0);
    b.build()
}

#[test]
fn draw_dialog_calls_the_user_items_powerpc_procedure_and_returns_to_its_caller() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    m.mac.rm.open_fork(heirloom_mac68k_engine::resmgr::APP_REF_NUM, heirloom_fmt_resfork::ResFork::parse(heirloom_mac68k_engine::fixture::dialog_fork()).unwrap());
    let frag = m.instantiate("test", &Pef::parse(&program()).unwrap()).unwrap();
    m.start(&frag).unwrap();
    let base = m.cpu().unwrap().gpr[2];
    let (step, _) = m.run_loop(2_000_000);
    assert_eq!(step, Step::Exited, "{:?} {:?}", m.fault(), &m.mac.log[m.mac.log.len().saturating_sub(12)..]);
    let r = |o: i32| m.mac.mem.read_u32(base + o as u32).unwrap();
    assert_ne!(r(D + 16), 0, "the dialog");
    assert_eq!(r(D + 44), 1, "DrawDialog ran the user item's procedure once");
    assert_eq!(r(D + 24) as u16, 6, "with its item number");
    assert_eq!(r(D + 28), 77, "NEGATIVE CONTROL against re-entry: DrawDialog returned to the caller, which went on");
    assert_eq!(r(D + 20), 2, "DialogSelect's update event drew the user item again");
    assert_eq!(r(D + 60), 0, "and answered FALSE");
    assert!(m.mac.log.iter().any(|l| l.contains("DialogSelect") && l.contains("update event")), "{:?}", m.mac.log);
}
