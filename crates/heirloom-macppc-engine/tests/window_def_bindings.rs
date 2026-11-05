//! The window-definition binding DATA (`bindings/macppc/*.sexp`) reads into the values the engine takes; a malformed file is refused by name.
mod wdef_bindings;
use heirloom_bind::WindowDefIdentity;
use heirloom_ui_abi::WindowRole;

#[test]
fn the_shipped_bindings_read_and_name_one_exact_definition() {
    let all = wdef_bindings::load("macppc");
    let b = all.iter().find(|b| b.id == "privilege-drmpa-4.7.1.44").expect("the dialog library's binding");
    assert_eq!(b.identity, WindowDefIdentity::FragmentCode { code_length: 439244, code_fnv1a64: 0xcc2b_69a9_d552_7ccc, entry_offset: 388208 });
    assert_eq!((b.meaning.role, b.meaning.titled, b.meaning.movable, b.meaning.closable, b.meaning.resizable), (WindowRole::Dialog, false, false, false, false));
    // ids are unique: two bindings never claim one label
    let mut ids: Vec<_> = all.iter().map(|b| b.id.clone()).collect();
    ids.sort(); ids.dedup();
    assert_eq!(ids.len(), all.len());
}

#[test]
fn a_malformed_binding_is_refused_by_name() {
    assert!(wdef_bindings::read("(window-definition (id \"x\") (fragment-code (length 1) (fnv1a64 \"0x1\")) (meaning (role dialog) (titled false) (movable false) (closable false) (resizable false)))").unwrap_err().contains("entry-offset"));
    assert!(wdef_bindings::read("(window-definition (id \"x\")").unwrap_err().contains("unbalanced"));
    assert!(wdef_bindings::read("(window-definition (id \"x\") (fragment-code (length 1) (fnv1a64 \"0x1\") (entry-offset 0)) (meaning (role palette) (titled false) (movable false) (closable false) (resizable false)))").unwrap_err().contains("role"));
}
