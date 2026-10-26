//! An Application's commodity (Zune application.c): every Application with commodities enabled owns a CxBroker named after its title; the
//! commands BrokerCommand sends are handled inside MUIM_Application_NewInput. Driven by tests/guest/c/mui_broker.c.
use heirloom_ami68k_engine::{Amiga, Step};
use heirloom_ui_abi::RecordingGuestUi;
use heirloom_fmt_hunk::writer::{build, Hunk};

const BIN: &[u8] = include_bytes!("guest/mui_broker.bin");
const RES: u32 = 0x1F000;

fn r(a: &Amiga<RecordingGuestUi>, i: u32) -> u32 { a.mem.read_u32(RES + 4 * i).unwrap() }

fn run() -> Amiga<RecordingGuestUi> {
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    a.start_hunk(&build(&[Hunk::code(BIN.to_vec(), vec![])], None)).unwrap();
    let (s, _) = a.step(20_000_000);
    assert_eq!(s, Step::Exited, "{s:?} {:?}\n{:#?}", a.refused, a.log.iter().rev().take(6).collect::<Vec<_>>());
    assert!(a.refused.is_empty(), "{:?}", a.refused);
    a
}

#[test]
fn an_application_owns_a_broker_and_its_attributes_read_back() {
    let a = run();
    assert_ne!(r(&a, 1), 0, "MUIA_Application_Broker is the CxBroker object");
    assert_ne!(r(&a, 2), 0, "MUIA_Application_BrokerPort is its port");
    assert_eq!(r(&a, 3) as i32, -5, "MUIA_Application_BrokerPri is a BYTE: -5 reads back sign-extended");
    assert_eq!(r(&a, 24), 1, "the Application is active at birth (application.c: app_Active = 1)");
    assert_eq!(r(&a, 18), 1, "the port's signal is part of the mask NewInput asks the program to wait on");
    // NEGATIVE CONTROLS: UseCommodities = FALSE makes no broker; and a broker disappears with its Application.
    assert_eq!(r(&a, 20), 0, "no commodity for UseCommodities FALSE");
    assert_eq!(r(&a, 22) as i32, -1, "after MUI_DisposeObject the broker is gone: BrokerCommand finds nobody");
    assert_eq!(r(&a, 8) as i32, -1, "an unknown broker name: -1");
    assert_eq!(r(&a, 21), 0, "an Application without a title is the broker \"Unnamed\"");
}

#[test]
fn exchange_sees_the_applications_broker_in_copy_broker_list() {
    let a = run();
    assert_eq!(r(&a, 26), 1, "CopyBrokerList counts the one broker");
    assert_eq!((r(&a, 27), r(&a, 28), r(&a, 29)), (1, 1, 1), "BrokerCopy name, title without \"$VER: \", description \"?\"");
    assert_eq!(r(&a, 30), 2, "COF_ACTIVE: the Application is active");
    assert_eq!(r(&a, 33), 1, "bc_Port is the BrokerPort");
    assert_eq!(r(&a, 31), 1, "FreeBrokerList emptied the list");
}

#[test]
fn commodity_commands_are_handled_in_new_input_as_zune_does() {
    let a = run();
    assert_eq!(r(&a, 23), 0, "NEGATIVE CONTROL: with nothing sent, NewInput calls no hook");
    assert_eq!((r(&a, 4), r(&a, 5), r(&a, 6), r(&a, 7)), (0, 0, 0, 0), "BrokerCommand delivered all four");
    assert_eq!(r(&a, 11), 4, "the broker hook saw every command (even those Zune handles itself)");
    assert_eq!([r(&a, 12), r(&a, 13), r(&a, 14), r(&a, 15)], [19, 15, 17, 23], "CXCMD_APPEAR, DISABLE, ENABLE, KILL in order");
    assert_eq!(r(&a, 16), 1, "the hook's object is the Application");
    assert_eq!(r(&a, 9), 0, "CXCMD_DISABLE sets MUIA_Application_Active FALSE");
    assert_eq!(r(&a, 10), 1, "CXCMD_ENABLE sets it TRUE again");
    assert_eq!(r(&a, 17), 1, "CXCMD_KILL is SetSignal(SIGBREAKF_CTRL_C) on the running task");
}
