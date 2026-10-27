//! Entries whose behaviour is entirely the engine's are graded by the engine's tests (`heirloom_gl_abi::coverage::ENGINE_GRADED`): each
//! listed test file must call the entry, and every entry listed must be switched on in the contract.
use heirloom_gl_abi::coverage::ENGINE_GRADED;
use heirloom_gl_abi::{find, ENTRIES};

fn source(file: &str) -> String {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    std::fs::read_to_string(dir.join(file)).unwrap_or_else(|e| panic!("{file}: {e}"))
}

#[test]
fn every_engine_graded_entry_is_switched_on_and_called_by_the_test_file_that_claims_it() {
    for (name, file) in ENGINE_GRADED {
        let i = find(ENTRIES, name).unwrap_or_else(|| panic!("{name} is not a contract entry"));
        assert!(ENTRIES[i].implemented, "{name} is listed as graded but is not switched on");
        assert!(source(file).contains(&format!("\"{name}\"")), "{file} does not call {name}");
    }
}

#[test]
fn the_engine_graded_list_has_no_duplicates() {
    let mut names: Vec<&str> = ENGINE_GRADED.iter().map(|(n, _)| *n).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), ENGINE_GRADED.len());
}
