//! The census: which of a fragment's imports are bound, which is NEXT-NEEDED. The PowerPC counterpart of
//! `mac68k-run --census` (which runs a program to its first missing trap); with no CPU yet this reads the import table.
use crate::cfm::{Fragment, ImportStatus};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Census {
    pub lines: Vec<String>,
    pub bound: usize,
    pub next_needed: Vec<String>,
    pub weak_absent: usize,
}

impl Census {
    pub fn of(frag: &Fragment) -> Census {
        let mut c = Census::default();
        for i in &frag.imports {
            let kind = i.class.name();
            match &i.status {
                ImportStatus::Bound(how) => { c.bound += 1; c.lines.push(format!("BOUND        {}/{} ({kind}) <- {how}", i.library, i.symbol)); }
                ImportStatus::Unbound(why) => { c.next_needed.push(format!("{}/{}", i.library, i.symbol)); c.lines.push(format!("NEXT-NEEDED  {}/{} ({kind}): {why}", i.library, i.symbol)); }
                ImportStatus::WeakAbsent(why) => { c.weak_absent += 1; c.lines.push(format!("WEAK-ABSENT  {}/{} ({kind}): {why}", i.library, i.symbol)); }
            }
        }
        c
    }
    pub fn summary(&self) -> String {
        format!("{} imports: {} bound, {} NEXT-NEEDED, {} weak-absent", self.lines.len(), self.bound, self.next_needed.len(), self.weak_absent)
    }
}
