//! intuition requesters become native alerts (one generic `GuestUi::request`): EasyRequestArgs, AutoRequest,
//! BuildEasyRequestArgs + SysReqHandler + FreeSysRequest, with Intuition's return values, the calling task parked alone, and the
//! IDCMP early close. Driven by compiled 68k programs (tests/guest/c/requester*.c).
mod common;
use amiga68k_engine::{requester, Amiga, Step};
use common::*;
use guestui_abi::{GuestUi, GuestUiEvent, RecordingGuestUi, RequestModel, UiCall};
use retro_hunk::writer::{build, Hunk};

const PROG: &[u8] = include_bytes!("guest/requester.bin");
const TASKS: &[u8] = include_bytes!("guest/requester_tasks.bin");
const IDCMP: &[u8] = include_bytes!("guest/requester_idcmp.bin");

type M = Amiga<RecordingGuestUi>;

fn start(a: &mut M, blob: &[u8]) { a.start_hunk(&build(&[Hunk::code(blob.to_vec(), vec![])], None)).unwrap(); }

/// Step until the program ends (true) or every task is parked with the host having nothing to say (false).
fn go(a: &mut M) -> bool {
    for _ in 0..200 {
        match a.step(2_000_000).0 {
            Step::Exited => return true,
            Step::Blocked => { if !requester::pump(a).unwrap() { return false; } }
            Step::BudgetSpent => {}
            Step::Fault(e) => panic!("{e}\n{:#?}", a.log),
        }
    }
    panic!("no end in sight\n{:#?}", a.log)
}

fn res(a: &M, i: u32) -> u32 { a.mem.read_u32(RES + 4 * i).unwrap() }
fn requests(a: &M) -> Vec<(Option<guestui_abi::UiObjectRef>, RequestModel)> {
    a.ui.calls.iter().filter_map(|c| if let UiCall::Request(w, m) = c { Some((*w, m.clone())) } else { None }).collect()
}

#[test]
fn easy_and_auto_requests_return_intuitions_numbers_and_the_host_sees_native_alerts() {
    let mut a = machine();
    // Easy: leftmost, middle, rightmost of 3; the lone button; then Auto positive, negative, and the one-button Auto.
    a.ui.request_answers.extend([0, 1, 2, 0, 0, 1, 0]);
    start(&mut a, PROG);
    assert!(!go(&mut a), "the program parks on the first requester nobody has answered");
    let r = |i| res(&a, i);
    assert_eq!((r(0), r(1), r(2)), (1, 2, 0), "buttons number 1..n-1 left to right and the rightmost returns 0");
    assert_eq!((r(3), r(4)), (0, 0), "a lone button returns 0; a NULL EasyStruct returns 0 without asking");
    assert_eq!((r(5), r(6), r(7)), (1, 0, 0), "AutoRequest: TRUE for the positive text, FALSE for the negative, FALSE for a lone negative");
    let q = requests(&a);
    assert_eq!(q.len(), 8, "one native request per call that asked (the NULL EasyStruct asked nothing), plus the first BuildEasyRequestArgs");
    let m = &q[0].1;
    assert_eq!((m.title.as_str(), m.text.as_str(), m.default, m.cancel), ("Save", "Save notes (1234 bytes)?", Some(0), Some(2)), "es_TextFormat formatted with RawDoFmt arguments");
    assert_eq!(m.buttons, ["Yes", "No", "Cancel"], "es_GadgetFormat split at '|', left to right");
    assert_eq!((q[3].1.title.as_str(), q[3].1.text.as_str(), &q[3].1.buttons[..], q[3].1.default, q[3].1.cancel), ("System Request", "Done", &["OK".to_string()][..], Some(0), Some(0)), "no es_Title: Intuition's own");
    assert_eq!((q[4].1.text.as_str(), &q[4].1.buttons[..]), ("Delete the file\nreally?", &["Delete".to_string(), "Cancel".to_string()][..]), "an IntuiText chain: a lower TopEdge is a new line; positive then negative");
    assert_eq!(&q[6].1.buttons[..], &["Cancel".to_string()][..], "negative control: with no positive text there is one button");
    assert!(q.iter().all(|(w, _)| w.is_none()), "no reference window was passed");

    // BuildEasyRequestArgs: a real requester window; nothing answered yet.
    assert_eq!((r(10), r(11)), (1, -2i32 as u32), "SysReqHandler(no wait) on an unanswered requester is -2");
    assert_eq!(r(12), 0, "the program is parked in Wait on the requester's UserPort signal");
    assert!(a.requests_pending());
    let live_ref = a.ui.open_requests()[0];
    // Negative controls first: a wrong or stale answer changes nothing.
    let bogus = guestui_abi::UiObjectRef { id: 999, generation: 1 };
    let e = a.deliver(GuestUiEvent::RequestAnswered { req: bogus, button: 0 }).unwrap_err();
    assert!(e.0.contains("not an open request"), "{e:?}");
    assert!(a.requests_pending() && res(&a, 12) == 0);
    let e = a.deliver(GuestUiEvent::RequestAnswered { req: live_ref, button: 5 }).unwrap_err();
    assert!(e.0.contains("button 5 of a request with 3 buttons"), "{e:?}");
    assert!(a.requests_pending(), "a refused answer leaves the requester open");
    // The user clicks the middle button: IDCMP_GADGETUP wakes the program; SysReqHandler returns 2.
    a.ui.answer_request(live_ref, 1).unwrap();
    assert!(!go(&mut a), "it parks again on the next requester (WaitInput = TRUE)");
    assert_eq!((res(&a, 12), res(&a, 13)), (2, -2i32 as u32), "the clicked button's number, then -2: the answer was consumed");
    assert_eq!(res(&a, 14), 0, "SysReqHandler(WaitInput) is parked inside the call");
    let waiting = a.ui.open_requests()[0];
    a.ui.answer_request(waiting, 0).unwrap();
    assert!(go(&mut a), "the program ran to its end");
    assert_eq!(res(&a, 14), 1, "WaitInput = TRUE returns the answer (button 0 of 3 = 1)");
    assert_eq!((res(&a, 15), res(&a, 16), res(&a, 17)), (1, 0, 1), "NULL and 1 answer 0 and 1");
    assert!(a.ui.calls.iter().any(|c| matches!(c, UiCall::CloseRequest(_))), "FreeSysRequest withdrew the unanswered requester from the host");
    assert_eq!(a.open_requesters(), 0, "every requester was freed");
    assert!(a.ui.requests.is_empty(), "and the host holds none");
    assert!(a.refused.is_empty(), "{:?}", a.refused);
}

#[test]
fn a_host_without_alerts_refuses_by_name_and_the_guest_gets_zero() {
    let mut a = machine();
    a.ui.refuse_requests = true;
    start(&mut a, PROG);
    go(&mut a); // (the program's later BuildEasyRequestArgs gives it a NULL requester, which it then misuses; only the first calls matter)
    assert_eq!((res(&a, 0), res(&a, 1), res(&a, 5)), (0, 0, 0), "EasyRequestArgs and AutoRequest answer 0 (an error)");
    assert!(a.refused.iter().any(|r| r.starts_with("intuition.library/EasyRequestArgs:") && r.contains("no alert part in this host")), "{:?}", a.refused);
    assert!(a.refused.iter().any(|r| r.starts_with("intuition.library/AutoRequest:")), "{:?}", a.refused);
    assert!(!a.requests_pending());
}

#[test]
fn a_synchronous_requester_parks_only_its_own_task() {
    let mut a = machine();
    start(&mut a, TASKS);
    for _ in 0..4 { let (s, _) = a.step(200_000); assert!(matches!(s, Step::BudgetSpent), "the counter task keeps the engine busy: {s:?}"); }
    let first = res(&a, 60);
    assert!(first > 100, "the other task ran while the main task waited for the answer: {first}");
    assert_eq!(res(&a, 62), 0, "negative control: the asking task did NOT continue without an answer");
    assert!(a.requests_pending(), "{:?}\n{:#?}", a.refused, a.log);
    a.step(200_000);
    assert!(res(&a, 60) > first && res(&a, 62) == 0, "still counting, still parked");
    let r = a.ui.open_requests()[0];
    a.ui.answer_request(r, 0).unwrap();
    let ev = a.ui.poll_event().unwrap(); // the driver's loop hands host events to the engine while other tasks keep it busy
    assert_eq!(a.deliver(ev).unwrap(), amiga68k_engine::run::Delivered::Handled);
    assert!(go(&mut a));
    assert_eq!(res(&a, 62), 101, "the answer (button 0 of 2 = 1) returned to the asking task after the host answered");
    assert!(!a.requests_pending() && a.open_requesters() == 0);
}

#[test]
fn idcmp_classes_close_a_requester_early() {
    let mut a = machine();
    start(&mut a, IDCMP);
    assert!(!go(&mut a));
    assert_eq!(a.notify_requesters(0x4000).unwrap(), 0, "negative control: a class the requester did not list closes nothing");
    assert!(a.requests_pending());
    assert_eq!(a.notify_requesters(0x8000).unwrap(), 1, "DISKINSERTED was listed");
    assert!(a.ui.calls.iter().any(|c| matches!(c, UiCall::CloseRequest(_))), "the host closes the alert");
    assert!(!go(&mut a), "parks on the second requester");
    assert_eq!((res(&a, 0) as i32, res(&a, 1)), (-1, 0x8000), "EasyRequestArgs returns -1 and stores the class through the pointer");
    assert_eq!(a.notify_requesters(0x4000).unwrap(), 0, "another class: ignored");
    let r = a.ui.open_requests()[0];
    a.ui.answer_request(r, 0).unwrap();
    assert!(!go(&mut a), "parks on the first AutoRequest");
    assert_eq!((res(&a, 2), res(&a, 3)), (1, 0x8000), "an ordinary click returns 1 and leaves the flags pointer alone");
    assert_eq!(a.notify_requesters(0x4000).unwrap(), 1, "nFlag");
    assert!(!go(&mut a));
    assert_eq!(res(&a, 4), 0, "AutoRequest: the negative class is FALSE");
    assert_eq!(a.notify_requesters(0x8000).unwrap(), 1, "pFlag");
    assert!(go(&mut a));
    assert_eq!(res(&a, 5), 1, "AutoRequest: the positive class is TRUE");
}
