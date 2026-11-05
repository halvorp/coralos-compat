//! A SECOND PROCESS in one machine (process_c.rs, ae_c.rs, heirloom_mac68k_engine::processes / aedesc): a client application launches a
//! server application from the volume with LaunchApplication (launchContinue | launchDontSwitch), builds an Apple event with a parameter,
//! AESends it with kAEWaitReply and reads the reply parameter the server's PowerPC handler put; then it counts the processes, sends the
//! server a "quit" (kAENoReply) whose handler calls ExitToShell, idles, and counts again. Both programs are assembled here; nothing is
//! special-cased by name.
use heirloom_common::{GuestMachine, Step};
use heirloom_fmt_pef::loader::SymbolClass;
use heirloom_fmt_pef::writer::PefBuilder;
use heirloom_fmt_pef::Pef;
use heirloom_fmt_resfork::writer::ForkBuilder;
use heirloom_macppc_engine::asm::Asm;
use heirloom_macppc_engine::{cfrg, MacPpc};
use heirloom_ui_abi::RecordingGuestUi;

type M = MacPpc<RecordingGuestUi>;
fn fc(b: &[u8; 4]) -> u32 { u32::from_be_bytes(*b) }

/// A program: `tvs` routines (the first is main; the others are TVectors at 8*i, for handlers), `imports` from InterfaceLib in TOC words
/// after them, then the data at `d` (returned) holding `consts` (4-byte words, then raw bytes).
struct Prog { tvs: usize, imports: &'static [&'static str] }
impl Prog {
    fn toc(&self, i: usize) -> i32 { (8 * self.tvs + 4 * i) as i32 }
    fn d(&self) -> i32 { (8 * self.tvs + 4 * self.imports.len()).next_multiple_of(16) as i32 }
    fn build(&self, routines: &[&Asm], consts: &[u8]) -> Vec<u8> {
        let mut code = Vec::new();
        let mut at = Vec::new();
        for r in routines { at.push(code.len() as u32); code.extend(r.finish()); }
        let mut b = PefBuilder::new();
        b.code(&code);
        let d = self.d() as usize;
        let mut init = vec![0u8; d + 512];
        for (i, a) in at.iter().enumerate() { init[8 * i..8 * i + 4].copy_from_slice(&a.to_be_bytes()); }
        init[d..d + consts.len()].copy_from_slice(consts);
        let ds = b.data(&init, init.len() as u32);
        let imports: Vec<(&str, SymbolClass, bool)> = self.imports.iter().map(|n| (*n, SymbolClass::TVector, false)).collect();
        b.import_library("InterfaceLib", false, &imports);
        b.reloc(ds as u16, &[0x4600 | (self.tvs as u16 - 1), 0x4A00 | (self.imports.len() as u16 - 1)]);
        b.main(ds as i32, 0);
        b.build()
    }
}

fn words(w: &[u32]) -> Vec<u8> { w.iter().flat_map(|v| v.to_be_bytes()).collect() }

const SERVER: Prog = Prog { tvs: 3, imports: &["AEInstallEventHandler", "WaitNextEvent", "AEProcessAppleEvent", "AEGetParamPtr", "AEPutParamPtr", "ExitToShell"] };

fn server() -> Vec<u8> {
    let p = &SERVER;
    let d = p.d();
    // D: 'Test' 'ping' 'quit' 'data' 'long' 'rply'; REC D+32, BUF D+48, TYPE D+52, ACT D+56
    let (rec, buf, ty, act) = (d + 32, d + 48, d + 52, d + 56);
    let mut main = Asm::new();
    main.mflr(0).stw(0, 8, 1).stwu(1, -96, 1);
    main.lwz(3, d, 2).lwz(4, d + 4, 2).addi(5, 2, 8).li(6, 0).li(7, 0).call_toc(p.toc(0));   // AEInstallEventHandler('Test','ping',h,0,false)
    main.lwz(3, d, 2).lwz(4, d + 8, 2).addi(5, 2, 16).li(6, 0).li(7, 0).call_toc(p.toc(0));  // ... 'quit'
    main.label("loop").li(3, -1).addi(4, 2, rec).li(5, 10).li(6, 0).call_toc(p.toc(1));       // WaitNextEvent(everyEvent, &rec, 10, nil)
    main.lhz(9, rec, 2).cmpwi(9, 23).bne("loop");
    main.addi(3, 2, rec).call_toc(p.toc(2)).b("loop");                                        // AEProcessAppleEvent(&rec)
    // ping(event, reply, refcon): reply 'rply' = event 'data' + 1
    let mut ping = Asm::new();
    ping.mflr(0).stw(0, 8, 1).stwu(1, -80, 1).stw(4, 56, 1);
    ping.lwz(4, d + 12, 2).lwz(5, d + 16, 2).addi(6, 2, ty).addi(7, 2, buf).li(8, 4).addi(9, 2, act).call_toc(p.toc(3));
    ping.lwz(9, buf, 2).addi(9, 9, 1).stw(9, buf, 2);
    ping.lwz(3, 56, 1).lwz(4, d + 20, 2).lwz(5, d + 16, 2).addi(6, 2, buf).li(7, 4).call_toc(p.toc(4));
    ping.li(3, 0).addi(1, 1, 80).lwz(0, 8, 1).mtlr(0).blr();
    let mut quit = Asm::new();
    quit.mflr(0).stw(0, 8, 1).stwu(1, -80, 1).call_toc(p.toc(5));
    p.build(&[&main, &ping, &quit], &words(&[fc(b"Test"), fc(b"ping"), fc(b"quit"), fc(b"data"), fc(b"long"), fc(b"rply")]))
}

const CLIENT: Prog = Prog { tvs: 1, imports: &["FSMakeFSSpec", "LaunchApplication", "AECreateDesc", "AECreateAppleEvent", "AEPutParamPtr", "AESend", "AEGetParamPtr", "GetNextProcess", "WaitNextEvent"] };
// client data offsets (from D)
const SPEC: i32 = 48; const PB: i32 = 128; const TARGET: i32 = 176; const EV: i32 = 184; const REPLY: i32 = 192; const TY: i32 = 200; const OUT: i32 = 204;
const ACT: i32 = 208; const OK: i32 = 224; const PSN: i32 = 300; const EV2: i32 = 312; const REC: i32 = 320;

fn client() -> Vec<u8> {
    let p = &CLIENT;
    let d = p.d();
    let mut a = Asm::new();
    a.mflr(0).stw(0, 8, 1).stwu(1, -96, 1);
    a.li(3, 0).li(4, 0).addi(5, 2, d + 36).addi(6, 2, d + SPEC).call_toc(p.toc(0)).stw(3, d + OK, 2);   // FSMakeFSSpec(0, 0, "\pServer", &spec)
    // LaunchParamBlockRec: launchBlockID 'LC', launchEPBLength 32, launchControlFlags launchContinue | launchDontSwitch, launchAppSpec
    a.li(9, 0x4C43).sth(9, d + PB + 6, 2).li(9, 32).stw(9, d + PB + 8, 2).li(9, 0x4200).sth(9, d + PB + 14, 2).addi(9, 2, d + SPEC).stw(9, d + PB + 16, 2);
    a.addi(3, 2, d + PB).call_toc(p.toc(1)).stw(3, d + OK + 4, 2);                                      // LaunchApplication
    a.lwz(3, d, 2).addi(4, 2, d + 4).li(5, 4).addi(6, 2, d + TARGET).call_toc(p.toc(2)).stw(3, d + OK + 8, 2); // AECreateDesc('sign', 'SRVR')
    a.lwz(3, d + 8, 2).lwz(4, d + 12, 2).addi(5, 2, d + TARGET).li(6, -1).li(7, 0).addi(8, 2, d + EV).call_toc(p.toc(3)).stw(3, d + OK + 12, 2);
    a.addi(3, 2, d + EV).lwz(4, d + 16, 2).lwz(5, d + 20, 2).addi(6, 2, d + 32).li(7, 4).call_toc(p.toc(4)).stw(3, d + OK + 16, 2); // 'data' = 41
    a.addi(3, 2, d + EV).addi(4, 2, d + REPLY).li(5, 3).li(6, 0).li(7, -1).li(8, 0).li(9, 0).call_toc(p.toc(5)).stw(3, d + OK + 20, 2); // AESend kAEWaitReply
    a.addi(3, 2, d + REPLY).lwz(4, d + 24, 2).lwz(5, d + 20, 2).addi(6, 2, d + TY).addi(7, 2, d + OUT).li(8, 4).addi(9, 2, d + ACT).call_toc(p.toc(6)).stw(3, d + OK + 24, 2);
    // count the processes: GetNextProcess from kNoProcess until procNotFound
    a.label("count").addi(3, 2, d + PSN).call_toc(p.toc(7)).cmpwi(3, 0).bne("counted");
    a.lwz(9, d + OK + 32, 2).addi(9, 9, 1).stw(9, d + OK + 32, 2).b("count");
    a.label("counted");
    // 'quit' with kAENoReply; then idle twice so the server runs it and ends
    a.lwz(3, d + 8, 2).lwz(4, d + 28, 2).addi(5, 2, d + TARGET).li(6, -1).li(7, 0).addi(8, 2, d + EV2).call_toc(p.toc(3));
    a.addi(3, 2, d + EV2).li(4, 0).li(5, 1).li(6, 0).li(7, -1).li(8, 0).li(9, 0).call_toc(p.toc(5)).stw(3, d + OK + 36, 2);
    a.li(3, -1).addi(4, 2, d + REC).li(5, 5).li(6, 0).call_toc(p.toc(8));
    a.li(3, -1).addi(4, 2, d + REC).li(5, 5).li(6, 0).call_toc(p.toc(8));
    a.li(9, 0).stw(9, d + PSN, 2).stw(9, d + PSN + 4, 2);
    a.label("count2").addi(3, 2, d + PSN).call_toc(p.toc(7)).cmpwi(3, 0).bne("done");
    a.lwz(9, d + OK + 40, 2).addi(9, 9, 1).stw(9, d + OK + 40, 2).b("count2");
    a.label("done").addi(1, 1, 96).lwz(0, 8, 1).mtlr(0).blr();
    let mut consts = words(&[fc(b"sign"), fc(b"SRVR"), fc(b"Test"), fc(b"ping"), fc(b"data"), fc(b"long"), fc(b"rply"), fc(b"quit"), 41]);
    consts.extend_from_slice(b"\x06Server");
    p.build(&[&a], &consts)
}

fn machine(file_type: [u8; 4]) -> (M, u32) {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let root = m.mac.fm.def_vol;
    let id = m.mac.fm.tree.put_file(root, "Server", server()).unwrap();
    let node = m.mac.fm.tree.get_mut(id);
    node.finfo.file_type = file_type;
    node.finfo.creator = *b"SRVR";
    node.rsrc = ForkBuilder::new().add(b"cfrg", 0, None, &cfrg::build(b"pwpc", cfrg::USAGE_APPLICATION, cfrg::WHERE_DATA_FORK, 0, 0, "Server")).build();
    let frag = m.instantiate("client", &Pef::parse(&client()).unwrap()).unwrap();
    m.start(&frag).unwrap();
    let toc = m.cpu().unwrap().gpr[2];
    (m, toc)
}

fn run(m: &mut M) -> Step {
    let mut t = 0u64;
    for _ in 0..200 {
        let (st, _) = GuestMachine::step(m, 2_000_000);
        if matches!(st, Step::Exited | Step::Fault(_)) { return st }
        t += 20_000;
        GuestMachine::advance_clock_to(m, t).unwrap();
    }
    Step::BudgetSpent
}

fn rd(m: &M, toc: u32, off: i32) -> i32 { m.mac.mem.read_u32(toc + (CLIENT.d() + off) as u32).unwrap() as i32 }

#[test]
fn a_launched_server_process_answers_an_apple_event_with_a_reply_and_quits_alone() {
    let (mut m, toc) = machine(*b"APPL");
    let st = run(&mut m);
    let log = m.mac.log.join("\n");
    assert_eq!(st, Step::Exited, "{:?}\n{log}", m.fault());
    assert_eq!(rd(&m, toc, OK), 0, "FSMakeFSSpec");
    assert_eq!(rd(&m, toc, OK + 4), 0, "LaunchApplication admitted the server\n{log}");
    let psn = m.mac.mem.read_u32(toc + (CLIENT.d() + PB + 24) as u32).unwrap();
    assert_eq!(psn, heirloom_mac68k_engine::processes::FIRST_PSN + 1, "launchProcessSN is the second process's serial number");
    assert_eq!((rd(&m, toc, OK + 8), rd(&m, toc, OK + 12), rd(&m, toc, OK + 16)), (0, 0, 0), "AECreateDesc, AECreateAppleEvent, AEPutParamPtr");
    assert_eq!(rd(&m, toc, OK + 20), 0, "AESend kAEWaitReply returned once the server's handler replied\n{log}");
    assert_eq!(rd(&m, toc, OK + 24), 0, "AEGetParamPtr of the reply's 'rply'");
    assert_eq!(rd(&m, toc, OUT), 42, "the server's PowerPC handler read 'data' 41 and replied 42");
    assert_eq!(rd(&m, toc, TY) as u32, fc(b"long"));
    assert_eq!(rd(&m, toc, OK + 32), 2, "GetNextProcess sees two processes while the server runs");
    assert_eq!(rd(&m, toc, OK + 36), 0, "AESend kAENoReply");
    assert_eq!(rd(&m, toc, OK + 40), 1, "after its quit handler's ExitToShell only the client is left (the machine went on)\n{log}");
    assert!(log.contains("LaunchApplication admitted"), "{log}");
    assert!(log.contains("\"Server\" ended (ExitToShell)"), "{log}");
}

#[test]
fn an_apple_event_to_a_process_that_is_not_running_is_proc_not_found() {
    // the same client with the server's file typed 'TEXT': LaunchApplication fails (paramErr) and AESend to 'SRVR' answers procNotFound
    let (mut m, toc) = machine(*b"TEXT"); // NEGATIVE CONTROL: not an application, so it is never launched
    let st = run(&mut m);
    assert_eq!(st, Step::Exited, "{:?}", m.fault());
    assert_eq!(rd(&m, toc, OK + 4), -50, "a file that is not an 'APPL' is not launched");
    assert_eq!(rd(&m, toc, OK + 20), -600, "AESend to a signature no process has: procNotFound");
    assert_eq!(rd(&m, toc, OK + 32), 1, "one process");
}
