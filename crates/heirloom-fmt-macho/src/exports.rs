//! The export trie (`LC_DYLD_INFO` export range): the symbols a dylib or executable offers, with the address of each as an offset
//! from the start of the image (the mach header).
use crate::bind::read_uleb;
use crate::header::{MachO, MachoError};
use alloc::string::String;
use alloc::vec::Vec;

/// `EXPORT_SYMBOL_FLAGS_REEXPORT`: the entry names a symbol of another library.
pub const FLAG_REEXPORT: u64 = 0x08;
/// `EXPORT_SYMBOL_FLAGS_STUB_AND_RESOLVER`.
pub const FLAG_STUB_AND_RESOLVER: u64 = 0x10;

/// One exported symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Export {
    /// The symbol name, with the Mach-O leading underscore.
    pub name: String,
    pub flags: u64,
    /// The address as an offset from the image's mach header (a regular export), or the stub address of a stub-and-resolver entry.
    pub address: u64,
    /// For a re-export: the 1-based dylib ordinal and the name it has there (empty = the same name).
    pub reexport: Option<(u64, String)>,
}

const MAX_NODES: usize = 1_000_000;

impl MachO<'_> {
    /// Every symbol in the export trie.
    pub fn exports(&self) -> Result<Vec<Export>, MachoError> {
        let Some(info) = self.dyld_info else { return Ok(Vec::new()) };
        if info.export.1 == 0 {
            return Ok(Vec::new());
        }
        let start = info.export.0 as usize;
        let end = start.checked_add(info.export.1 as usize).ok_or(MachoError::Truncated("export info"))?;
        let trie = self.slice.get(start..end).ok_or(MachoError::Truncated("export info"))?;
        parse_trie(trie)
    }

    /// The export named `name`, if any.
    pub fn export(&self, name: &str) -> Result<Option<Export>, MachoError> {
        Ok(self.exports()?.into_iter().find(|e| e.name == name))
    }
}

fn err(at: usize, why: &'static str) -> MachoError {
    MachoError::BadOpcodeStream { stream: "export", at, why }
}

fn read_cstr(trie: &[u8], pos: &mut usize) -> Result<String, MachoError> {
    let start = *pos;
    let len = trie.get(start..).and_then(|t| t.iter().position(|b| *b == 0)).ok_or_else(|| err(start, "unterminated string"))?;
    *pos = start + len + 1;
    Ok(String::from_utf8_lossy(&trie[start..start + len]).into_owned())
}

/// Walks a trie given as bytes; a node reached twice, a node past the end or a trie of more than a million nodes is an error.
pub fn parse_trie(trie: &[u8]) -> Result<Vec<Export>, MachoError> {
    let mut out = Vec::new();
    let mut stack: Vec<(usize, String)> = Vec::new();
    stack.push((0, String::new()));
    let mut seen = alloc::collections::BTreeSet::new();
    while let Some((node, prefix)) = stack.pop() {
        if node >= trie.len() {
            return Err(err(node, "node offset outside the trie"));
        }
        if !seen.insert(node) || seen.len() > MAX_NODES {
            return Err(err(node, "node revisited or trie too large"));
        }
        let mut pos = node;
        let terminal_size = read_uleb(trie, &mut pos, "export")? as usize;
        let children_at = pos.checked_add(terminal_size).ok_or_else(|| err(pos, "terminal size overflows"))?;
        if terminal_size > 0 {
            let flags = read_uleb(trie, &mut pos, "export")?;
            if flags & FLAG_REEXPORT != 0 {
                let ordinal = read_uleb(trie, &mut pos, "export")?;
                let name = read_cstr(trie, &mut pos)?;
                out.push(Export { name: prefix.clone(), flags, address: 0, reexport: Some((ordinal, name)) });
            } else {
                let address = read_uleb(trie, &mut pos, "export")?;
                if flags & FLAG_STUB_AND_RESOLVER != 0 {
                    read_uleb(trie, &mut pos, "export")?;
                }
                out.push(Export { name: prefix.clone(), flags, address, reexport: None });
            }
        }
        let mut pos = children_at;
        let count = *trie.get(pos).ok_or_else(|| err(pos, "missing child count"))?;
        pos += 1;
        for _ in 0..count {
            let label = read_cstr(trie, &mut pos)?;
            let child = read_uleb(trie, &mut pos, "export")? as usize;
            let mut name = prefix.clone();
            name.push_str(&label);
            stack.push((child, name));
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    // root (offset 0): no terminal, two children; "_a" -> node at 10 (flags 0, address 0x10); "_b" -> node at 14 (re-export of ordinal 2, "_c")
    fn sample() -> Vec<u8> {
        let mut t = vec![0u8, 2];
        t.extend_from_slice(b"_a\0");
        t.push(10);
        t.extend_from_slice(b"_b\0");
        t.push(14);
        assert_eq!(t.len(), 10);
        t.extend_from_slice(&[2, 0x00, 0x10, 0]); // node at 10: terminal size 2, no children
        t.extend_from_slice(&[5, 0x08, 2, b'_', b'c', 0, 0]); // node at 14: terminal size 5, no children
        t
    }

    #[test]
    fn walks_a_trie_with_a_regular_export_and_a_reexport() {
        let got = parse_trie(&sample()).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], Export { name: "_a".into(), flags: 0, address: 0x10, reexport: None });
        assert_eq!(got[1].name, "_b");
        assert_eq!(got[1].reexport, Some((2, "_c".into())));
    }

    #[test]
    fn a_child_offset_outside_the_trie_is_an_error() {
        let mut t = sample();
        t[5] = 200;
        assert!(parse_trie(&t).is_err());
    }

    #[test]
    fn a_cycle_is_an_error_not_a_hang() {
        // root has one child "x" whose node is the root again
        let t = vec![0u8, 1, b'x', 0, 0];
        assert!(parse_trie(&t).is_err());
    }

    #[test]
    fn an_empty_trie_has_no_exports() {
        assert_eq!(parse_trie(&[0, 0]).unwrap(), Vec::<Export>::new());
    }
}
