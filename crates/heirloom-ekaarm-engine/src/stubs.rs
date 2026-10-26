use alloc::string::String;
use alloc::vec::Vec;
use heirloom_fmt_e32::{ImportedDll, OrdinalTable};
use crate::layout::{STUB_BASE, STUB_SVC_BASE};

pub struct Stub {
    pub dll_base: String,
    pub dll: String,
    pub ordinal: u16,
    pub name: Option<String>,
}

pub fn dll_base(full: &str) -> String {
    match full.find('{') {
        Some(n) => String::from(&full[..n]),
        None => String::from(full),
    }
}

pub fn build(dlls: &[ImportedDll], table: &OrdinalTable) -> Vec<Stub> {
    let mut out = Vec::new();
    for d in dlls {
        for imp in &d.imports {
            out.push(Stub {
                dll_base: dll_base(&d.name),
                dll: d.name.clone(),
                ordinal: imp.ordinal,
                name: table.name_of(&d.name, imp.ordinal).map(String::from),
            });
        }
    }
    out
}

pub fn encode_stub(index: usize) -> [u8; 8] {
    let svc = 0xEF00_0000 | ((STUB_SVC_BASE.wrapping_add(index as u32)) & 0x00FF_FFFF);
    let bx_lr = 0xE12F_FF1Eu32;
    let mut bytes = [0u8; 8];
    bytes[..4].copy_from_slice(&svc.to_le_bytes());
    bytes[4..].copy_from_slice(&bx_lr.to_le_bytes());
    bytes
}

pub fn stub_addr(index: usize) -> u32 {
    STUB_BASE.wrapping_add((index as u32).wrapping_mul(8))
}
