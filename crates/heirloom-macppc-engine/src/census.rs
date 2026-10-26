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
    /// One row per imported library, in first-import order: how its symbols were served.
    pub libraries: Vec<LibraryRow>,
}

/// How one library's imports fared: bound, refused by name at call (`named`), or weak and absent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibraryRow { pub library: String, pub bound: usize, pub named: usize, pub weak_missing: usize }

impl Census {
    pub fn of(frag: &Fragment) -> Census {
        let mut c = Census::default();
        for i in &frag.imports {
            let kind = i.class.name();
            let at = match c.libraries.iter().position(|r| r.library == i.library) {
                Some(at) => at,
                None => { c.libraries.push(LibraryRow { library: i.library.clone(), ..Default::default() }); c.libraries.len() - 1 }
            };
            let row = &mut c.libraries[at];
            match &i.status { ImportStatus::Bound(_) => row.bound += 1, ImportStatus::Unbound(_) => row.named += 1, ImportStatus::WeakAbsent(_) => row.weak_missing += 1 }
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

    /// The per-library table a big application's gap is read from: `LIBRARY bound named weak-missing`, widest column first.
    pub fn library_table(&self) -> Vec<String> {
        let w = self.libraries.iter().map(|r| r.library.chars().count()).max().unwrap_or(0).max(7);
        let mut out = alloc::vec![format!("{:<w$}  {:>6}  {:>6}  {:>12}", "LIBRARY", "bound", "named", "weak-missing")];
        for r in &self.libraries { out.push(format!("{:<w$}  {:>6}  {:>6}  {:>12}", r.library, r.bound, r.named, r.weak_missing)); }
        out
    }
}
