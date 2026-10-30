//! heirloom-cpu-abi::ppc::WHAT is the closed vocabulary a Trap's reason crosses cpu-ppc.library's table in (PpcExit.detail). Every
//! reason string the core can name must be listed there, or the engine would see WHAT_UNLISTED instead of the family.
use std::path::Path;

fn reasons_in(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    // the argument of unimpl / invalid / privileged / aligned4, a `what:` / `why:` field, or a `=> "..."` arm of a name table
    for pat in ["unimpl(w, \"", "invalid(w, \"", "privileged(w, \"", "what: \"", "why: \"", "=> \"", ", \""] {
        let mut rest = src;
        while let Some(i) = rest.find(pat) {
            let after = &rest[i + pat.len()..];
            if let Some(j) = after.find('"') { out.push(after[..j].to_string()); }
            rest = &rest[i + pat.len()..];
        }
    }
    out
}

#[test]
fn what_table_lists_every_reason_the_core_names() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut missing = Vec::new();
    let mut seen = 0;
    for e in std::fs::read_dir(&dir).unwrap() {
        let p = e.unwrap().path();
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        // the files that make Traps (the `, "` pattern is only meaningful in them: aligned4(cpu, ea, "lmw"))
        if !["exec.rs", "mem.rs", "fpmem.rs", "fpu.rs", "int.rs", "branch.rs", "sys.rs", "x31.rs", "names.rs", "fpscr.rs", "t1op.rs"].contains(&name.as_str()) { continue; }
        // code only: a doc comment may quote the manual
        let src: String = std::fs::read_to_string(&p).unwrap().lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
        for r in reasons_in(&src) {
            seen += 1;
            if r.is_empty() || r.contains('{') || r.contains('\\') { continue; } // format strings / escapes are not reasons
            if !heirloom_cpu_abi::ppc::WHAT.contains(&r.as_str()) && (src.contains(&format!("unimpl(w, \"{r}\"")) || src.contains(&format!("invalid(w, \"{r}\"")) || src.contains(&format!("privileged(w, \"{r}\"")) || src.contains(&format!("=> \"{r}\"")) || src.contains(&format!("\"{r}\")")) ) {
                missing.push(format!("{name}: {r:?}"));
            }
        }
    }
    assert!(seen > 30, "the scan found the core's reasons ({seen})");
    assert!(missing.is_empty(), "reasons missing from heirloom_cpu_abi::ppc::WHAT (append them):\n{}", missing.join("\n"));
}

#[test]
fn a_trap_crosses_the_exit_record_unchanged_and_an_unknown_reason_is_named() {
    use heirloom_cpu_abi::ppc::{PpcExit, Trap, WHAT_UNLISTED};
    for t in [
        Trap::Unimplemented { pc: 0x100, word: 0, what: "primary opcode 0 (reserved/zero word)" },
        Trap::InvalidForm { pc: 4, word: 7, why: "stwcx. with Rc=0" },
        Trap::Privileged { pc: 8, word: 9, what: "rfi" },
        Trap::BusFault { pc: 12, addr: 0xDEAD, write: true },
        Trap::Alignment { pc: 16, addr: 0x13, what: "lwarx" },
        Trap::Syscall { pc: 20 },
        Trap::TrapWord { pc: 24, word: 0x7FE00008 },
        Trap::NativeUnhandled { addr: 0x9000 },
    ] {
        assert_eq!(PpcExit::trap(&t, 5).decode_trap(), Some(t));
    }
    let odd = Trap::Unimplemented { pc: 0, word: 0, what: "a reason nobody listed" };
    assert_eq!(PpcExit::trap(&odd, 0).decode_trap(), Some(Trap::Unimplemented { pc: 0, word: 0, what: WHAT_UNLISTED }), "NEGATIVE CONTROL: named, not lost");
    let mut bad = PpcExit::trap(&odd, 0);
    bad.detail = 0xFF;
    assert_eq!(bad.decode_trap(), None, "an unknown kind is refused");
}
