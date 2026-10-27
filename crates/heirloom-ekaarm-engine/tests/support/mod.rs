//! A synthetic device ROM for corpus-free tests: one in-memory `\resource\uiklaf\eikpriv.rsc` holding the `EIKCOLORLIST` the environment reads
//! when it is made (resource 0x19bd6003 -> array 0x19bd6006 of 59 `CTRL_COLOR`s, extras 0x19bd6007 empty). The colours are test data
//! (logical colour n is RGB (n, n, n)); the real file is the owner's corpus.
use heirloom_ekaarm_engine::drive::{Drive, RomSource};

const ID_OFFSET: u32 = 0x19bd_6000;
const UID1_PLAIN: u32 = 0x101f_4a6b;

/// A plain resource file: resource 1 is the signature record, then `res` (raw bytes) as resources 2...
fn rsc(res: &[Vec<u8>]) -> Vec<u8> {
    let mut f = Vec::new();
    f.extend_from_slice(&UID1_PLAIN.to_le_bytes());
    f.extend_from_slice(&0u32.to_le_bytes());
    f.extend_from_slice(&0x0019_bd60u32.to_le_bytes());
    f.extend_from_slice(&[0; 4]); // checksum
    f.push(0); // flags
    f.extend_from_slice(&[0, 0]); // largest
    let mut sig = 4u32.to_le_bytes().to_vec();
    sig.extend_from_slice(&(ID_OFFSET | 1).to_le_bytes());
    let mut all = vec![sig];
    all.extend_from_slice(res);
    f.extend(core::iter::repeat_n(0u8, all.len().div_ceil(8))); // the unicode bit array: no runs
    let mut offsets = Vec::new();
    for b in &all {
        offsets.push(f.len() as u16);
        f.extend_from_slice(b);
    }
    let index_at = f.len() as u16;
    for o in offsets {
        f.extend_from_slice(&o.to_le_bytes());
    }
    f.extend_from_slice(&index_at.to_le_bytes());
    f
}

fn color_array(n: u16) -> Vec<u8> {
    let mut a = n.to_le_bytes().to_vec();
    for i in 0..n {
        a.extend_from_slice(&i.to_le_bytes());
        a.extend_from_slice(&[i as u8, i as u8, i as u8]);
    }
    a
}

fn eikpriv() -> Vec<u8> {
    let mut list = (ID_OFFSET | 6).to_le_bytes().to_vec();
    list.extend_from_slice(&(ID_OFFSET | 7).to_le_bytes());
    // resources 2..7: gray4 list, color16 list (3), items (4, 5 unused fillers), items color16 (6), extras (7)
    rsc(&[vec![0; 8], list, vec![0; 2], vec![0; 2], color_array(59), color_array(0)])
}

struct Rom(Vec<u8>);

impl RomSource for Rom {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        path.eq_ignore_ascii_case("\\resource\\uiklaf\\eikpriv.rsc").then(|| self.0.clone())
    }
    fn exists(&self, path: &str) -> bool {
        self.read(path).is_some()
    }
    fn list(&self, path: &str) -> Option<Vec<String>> {
        match path.to_ascii_lowercase().as_str() {
            "\\resource" => Some(vec!["uiklaf".into()]),
            "\\resource\\uiklaf" => Some(vec!["eikpriv.rsc".into()]),
            _ => None,
        }
    }
}

/// A drive with only the synthetic ROM behind it.
pub fn drive_with_rom() -> Drive {
    let mut d = Drive::new();
    d.set_rom(Box::new(Rom(eikpriv())));
    d
}
