//! The guest address map. Windows are sized for real S60 images: code and data up to 60 MiB each, a stack up to 64 MiB,
//! a heap up to 1.5 GiB (an image's heap_max is a reservation; only heap_min is committed).
pub const PAGE: u32 = 0x1000;
pub const CODE_BASE: u32 = 0x0040_0000;
pub const DATA_BASE: u32 = 0x0400_0000;
pub const STACK_TOP: u32 = 0x0C00_0000;
pub const HEAP_BASE: u32 = 0x1000_0000;
pub const STUB_BASE: u32 = 0x7000_0000;
pub const EXIT_STUB: u32 = STUB_BASE - PAGE;
pub const STUB_SVC_BASE: u32 = 0x00F0_0000;
/// The SVC of the stub the entry point returns into (LR at start): the process returns from its entry with r0.
pub const EXIT_SVC: u32 = 0x00FF_0000;

pub fn pru(n: u32) -> u32 {
    n.saturating_add(PAGE - 1) & !(PAGE - 1)
}

pub fn check_overlap(
    code_sz: u32,
    data_sz: u32,
    heap_max: u32,
    stack_sz: u32,
    stub_sz: u32,
) -> Result<(), &'static str> {
    let stack_top = STACK_TOP as u64;
    let stack_sz = stack_sz as u64;
    if stack_sz > stack_top {
        return Err("stack extends below address zero");
    }

    let regions = [
        ("code", CODE_BASE as u64, CODE_BASE as u64 + code_sz as u64),
        ("data", DATA_BASE as u64, DATA_BASE as u64 + data_sz as u64),
        ("heap", HEAP_BASE as u64, HEAP_BASE as u64 + heap_max as u64),
        ("stack", stack_top - stack_sz, stack_top),
        ("stubs", STUB_BASE as u64, STUB_BASE as u64 + stub_sz as u64),
    ];

    for i in 0..regions.len() {
        for j in i + 1..regions.len() {
            let (an, a0, a1) = regions[i];
            let (bn, b0, b1) = regions[j];
            if a0 == a1 || b0 == b1 {
                continue;
            }
            if a0 < b1 && b0 < a1 {
                return Err(match (an, bn) {
                    ("code", "data") => "code/data overlap",
                    ("code", "heap") => "code/heap overlap",
                    ("code", "stack") => "code/stack overlap",
                    ("code", "stubs") => "code/stubs overlap",
                    ("data", "heap") => "data/heap overlap",
                    ("data", "stack") => "data/stack overlap",
                    ("data", "stubs") => "data/stubs overlap",
                    ("heap", "stack") => "heap/stack overlap",
                    ("heap", "stubs") => "heap/stubs overlap",
                    _ => "stack/stubs overlap",
                });
            }
        }
    }
    Ok(())
}
