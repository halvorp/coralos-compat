//! Prints the core tally per version and what stands between each version and a claim: `cargo run -q -p heirloom-gl-abi --example tally`.
use heirloom_gl_abi::contract::{core_tally, ENTRIES};
use heirloom_gl_abi::Home;

fn main() {
    for t in core_tally(ENTRIES) {
        println!("{:?}: {} listed, {} callable, {} complete, {} refused by name", t.version, t.listed, t.implemented, t.complete, t.refused());
    }
    println!("-- callable with a gap (not claimable):");
    for e in ENTRIES.iter().filter(|e| matches!(e.home, Home::Core(_)) && e.implemented && e.gap.is_some()) {
        println!("  {}: {}", e.name, e.gap.unwrap_or(""));
    }
    println!("-- refused by name:");
    let refused: Vec<&str> = ENTRIES.iter().filter(|e| matches!(e.home, Home::Core(_)) && !e.implemented).map(|e| e.name).collect();
    println!("  {}", refused.join(" "));
}
