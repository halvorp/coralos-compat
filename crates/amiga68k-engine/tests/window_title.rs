//! A window the guest opens without a title is named after the PROGRAM (its file name, `Amiga::start_hunk_as`), never the runtime's
//! default; an explicit title and a later SetWindowTitles still win. Negative controls: a program started anonymously leaves it empty.
mod gfxcommon;
use amiga68k_engine::Step;
use gfxcommon::*;
use guestui_abi::*;

const LVO_OPEN_WINDOW_TAGS: i32 = -606;
const LVO_SET_WINDOW_TITLES: i32 = -276;

fn titles(name: Option<&str>, explicit: Option<&str>, later: Option<&str>) -> (Vec<String>, Vec<String>) {
    let mut g = G::new_plane();
    let mut t = vec![(WA + 3, 100), (WA + 4, 60)];
    if let Some(e) = explicit { let p = g.s(e); t.push((WA + 11, p)); }
    let tags = g.tags(&t);
    let mut calls = vec![intu(LVO_OPEN_WINDOW_TAGS, &[a(0, 0), a(1, tags)])];
    if let Some(l) = later { let p = g.s(l); calls.push(intu(LVO_SET_WINDOW_TITLES, &[a_res(0, 0), a(1, p), a(2, u32::MAX)])); }
    let code = script(&calls);
    match name { Some(n) => g.a.start_hunk_as(&code, n).unwrap(), None => g.a.start_hunk(&code).unwrap() };
    assert_eq!(g.a.step(1_000_000).0, Step::Exited, "{:?}", g.a.refused);
    let opened = g.a.ui.calls.iter().filter_map(|c| if let UiCall::OpenWindow(m) = c { Some(m.title.clone()) } else { None }).collect();
    let set = g.a.ui.calls.iter().filter_map(|c| if let UiCall::SetWindowTitle(_, t) = c { Some(t.clone()) } else { None }).collect();
    (opened, set)
}

#[test]
fn an_untitled_window_is_named_after_the_program() {
    assert_eq!(titles(Some("DirectoryOpus"), None, None).0, ["DirectoryOpus"]);
    // negative control: an anonymous start leaves the title empty (the host's own fallback, not the engine's business)
    assert_eq!(titles(None, None, None).0, [""]);
}

#[test]
fn the_guests_own_title_and_a_later_set_window_titles_win() {
    assert_eq!(titles(Some("prog"), Some("Mine"), None).0, ["Mine"], "an explicit WA_Title is never replaced");
    let (opened, set) = titles(Some("prog"), None, Some("Later"));
    assert_eq!((opened, set), (vec!["prog".to_string()], vec!["Later".to_string()]));
}
