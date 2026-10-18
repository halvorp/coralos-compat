//! The on-disk `.info` file (Workbench icon) as plain Rust data: parse and write, no guest memory involved.
//!
//! Layout (facts from AROS workbench/libs/icon/diskobjio.c's stream descriptors, cross-checked against a real
//! iGame.info; offsets by m68k gcc -mno-align-int): the file starts with the 78-byte `struct DiskObject` image
//! (do_Magic 0xE310, do_Version 1, the 44-byte Gadget at 4, do_Type 48, do_DefaultTool 50, do_ToolTypes 54,
//! do_CurrentX 58, do_CurrentY 62, do_DrawerData 66, do_ToolWindow 70, do_StackSize 74). Every pointer field holds a
//! nonzero FLAG when the thing follows in the file. Then, in this order: the 56-byte old DrawerData (if do_DrawerData),
//! the gadget image and the select image (each a 20-byte `struct Image` header + `((w+15)>>4)*h*depth*2` bytes of
//! planar data), the default tool string, the tool-types table, the tool window string (a string is a longword length
//! including the NUL, then the bytes; the table is a longword byte count `(n+1)*4` then n strings), and for a drawer
//! the 6-byte "new" DrawerData (dd_Flags, dd_ViewModes). Anything after that (GlowIcons' `FORM ICON`, NewIcons) is kept
//! as an opaque trailer and not interpreted.
use alloc::vec::Vec;

pub const MAGIC: u16 = 0xE310;
pub const HEAD: usize = 78;
pub const OLD_DRAWER: usize = 56;
pub const NEW_DRAWER: usize = 6;
pub const IMAGE_HDR: usize = 20;
/// Offsets inside the 78-byte head.
pub const O_GADGET_RENDER: usize = 4 + 18;
pub const O_SELECT_RENDER: usize = 4 + 22;
pub const O_DO_TYPE: usize = 48;
pub const O_DEFAULT_TOOL: usize = 50;
pub const O_TOOL_TYPES: usize = 54;
pub const O_DRAWER_DATA: usize = 66;
pub const O_TOOL_WINDOW: usize = 70;
/// Inside a 20-byte Image header: Width 4, Height 6, Depth 8.
const I_WIDTH: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IconError { NotAnIcon, Truncated(&'static str), TooBig(&'static str) }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image { pub hdr: [u8; IMAGE_HDR], pub data: Vec<u8> }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Icon {
    /// The DiskObject image with its pointer fields zeroed: presence is the `Option`s below.
    pub head: [u8; HEAD],
    pub drawer: Option<[u8; OLD_DRAWER]>,
    pub drawer_new: Option<[u8; NEW_DRAWER]>,
    pub images: [Option<Image>; 2],
    pub default_tool: Option<Vec<u8>>,
    pub tool_types: Option<Vec<Vec<u8>>>,
    pub tool_window: Option<Vec<u8>>,
    pub trailer: Vec<u8>,
}

fn be16(b: &[u8], at: usize) -> usize { u16::from_be_bytes([b[at], b[at + 1]]) as usize }
fn be32(b: &[u8], at: usize) -> usize { u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]) as usize }
fn flagged(h: &[u8; HEAD], at: usize) -> bool { h[at..at + 4] != [0; 4] }

/// Bytes of planar data for an Image header.
pub fn image_bytes(hdr: &[u8; IMAGE_HDR]) -> usize {
    ((be16(hdr, I_WIDTH) + 15) >> 4) * be16(hdr, I_WIDTH + 2) * be16(hdr, I_WIDTH + 4) * 2
}

struct Cur<'a> { b: &'a [u8], at: usize }
impl<'a> Cur<'a> {
    fn take(&mut self, n: usize, what: &'static str) -> Result<&'a [u8], IconError> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.b.len()).ok_or(IconError::Truncated(what))?;
        let s = &self.b[self.at..end];
        self.at = end;
        Ok(s)
    }
    fn string(&mut self, what: &'static str) -> Result<Vec<u8>, IconError> {
        let n = be32(self.take(4, what)?, 0);
        let s = self.take(n, what)?;
        Ok(s.strip_suffix(&[0]).unwrap_or(s).to_vec())
    }
    fn image(&mut self) -> Result<Image, IconError> {
        let hdr: [u8; IMAGE_HDR] = self.take(IMAGE_HDR, "image header")?.try_into().expect("20 bytes");
        let n = image_bytes(&hdr);
        let data = self.take(n, "image data")?.to_vec();
        Ok(Image { hdr: [&hdr[..10], &[0; 4], &hdr[14..16], &[0; 4]].concat().try_into().expect("20 bytes"), data })
    }
}

impl Icon {
    pub fn do_type(&self) -> u8 { self.head[O_DO_TYPE] }

    pub fn parse(file: &[u8]) -> Result<Icon, IconError> {
        if file.len() < HEAD || be16(file, 0) != MAGIC as usize { return Err(IconError::NotAnIcon); }
        let mut head: [u8; HEAD] = file[..HEAD].try_into().expect("78 bytes");
        let mut c = Cur { b: file, at: HEAD };
        let has = |h: &[u8; HEAD], o| flagged(h, o);
        let (g, s, d, tt, tw, dr) = (has(&head, O_GADGET_RENDER), has(&head, O_SELECT_RENDER), has(&head, O_DEFAULT_TOOL), has(&head, O_TOOL_TYPES), has(&head, O_TOOL_WINDOW), has(&head, O_DRAWER_DATA));
        for o in [O_GADGET_RENDER, O_SELECT_RENDER, 4 + 26, 4 + 34, O_DEFAULT_TOOL, O_TOOL_TYPES, O_DRAWER_DATA, O_TOOL_WINDOW] { head[o..o + 4].fill(0); }
        let drawer = if dr { Some(c.take(OLD_DRAWER, "drawer data")?.try_into().expect("56 bytes")) } else { None };
        let images = [if g { Some(c.image()?) } else { None }, if s { Some(c.image()?) } else { None }];
        let default_tool = if d { Some(c.string("default tool")?) } else { None };
        let tool_types = if tt {
            let table = be32(c.take(4, "tool types")?, 0);
            let n = (table / 4).saturating_sub(1);
            if n > c.b.len() { return Err(IconError::TooBig("tool types")); }
            Some((0..n).map(|_| c.string("tool type")).collect::<Result<Vec<_>, _>>()?)
        } else { None };
        let tool_window = if tw { Some(c.string("tool window")?) } else { None };
        let drawer_new = if dr && file.len() - c.at >= NEW_DRAWER { Some(c.take(NEW_DRAWER, "new drawer data")?.try_into().expect("6 bytes")) } else { None };
        Ok(Icon { head, drawer, drawer_new, images, default_tool, tool_types, tool_window, trailer: file[c.at..].to_vec() })
    }

    /// The file image. Pointer fields get the flag value 1 where the thing is present.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut head = self.head;
        let mut set = |o: usize, on: bool| head[o..o + 4].copy_from_slice(&u32::from(on).to_be_bytes());
        set(O_GADGET_RENDER, self.images[0].is_some());
        set(O_SELECT_RENDER, self.images[1].is_some());
        set(O_DEFAULT_TOOL, self.default_tool.is_some());
        set(O_TOOL_TYPES, self.tool_types.is_some());
        set(O_DRAWER_DATA, self.drawer.is_some());
        set(O_TOOL_WINDOW, self.tool_window.is_some());
        let mut out = head.to_vec();
        if let Some(d) = &self.drawer { out.extend_from_slice(d); }
        for i in self.images.iter().flatten() { out.extend_from_slice(&i.hdr); out.extend_from_slice(&i.data); }
        let put = |out: &mut Vec<u8>, s: &[u8]| { out.extend_from_slice(&(s.len() as u32 + 1).to_be_bytes()); out.extend_from_slice(s); out.push(0); };
        if let Some(s) = &self.default_tool { put(&mut out, s); }
        if let Some(t) = &self.tool_types {
            out.extend_from_slice(&((t.len() as u32 + 1) * 4).to_be_bytes());
            for s in t { put(&mut out, s); }
        }
        if let Some(s) = &self.tool_window { put(&mut out, s); }
        if let Some(d) = &self.drawer_new { out.extend_from_slice(d); }
        out.extend_from_slice(&self.trailer);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn sample() -> Icon {
        let mut head = [0u8; HEAD];
        head[0..2].copy_from_slice(&MAGIC.to_be_bytes());
        head[3] = 1;
        head[O_DO_TYPE] = 4;
        let mut hdr = [0u8; IMAGE_HDR];
        hdr[4..6].copy_from_slice(&20u16.to_be_bytes()); // 20 wide -> 2 words per row
        hdr[6..8].copy_from_slice(&3u16.to_be_bytes());
        hdr[8..10].copy_from_slice(&2u16.to_be_bytes()); // 2 planes
        let img = Image { hdr, data: (0..2 * 3 * 2 * 2).map(|i| i as u8).collect() };
        Icon { head, drawer: None, drawer_new: None, images: [Some(img), None], default_tool: Some(b"SYS:C/More".to_vec()),
               tool_types: Some(vec![b"ICON=1".to_vec(), b"(OFF=2)".to_vec()]), tool_window: None, trailer: vec![] }
    }

    #[test]
    fn round_trips_and_sizes_images_by_the_word_rounded_formula() {
        let i = sample();
        assert_eq!(image_bytes(&i.images[0].as_ref().unwrap().hdr), 24);
        let back = Icon::parse(&i.to_bytes()).unwrap();
        assert_eq!(back, i);
        assert_eq!(back.do_type(), 4);
    }

    #[test]
    fn bad_magic_and_every_truncation_are_errors_not_panics() {
        let b = sample().to_bytes();
        let mut bad = b.clone();
        bad[0] = 0;
        assert_eq!(Icon::parse(&bad), Err(IconError::NotAnIcon));
        for n in 0..b.len() { assert!(Icon::parse(&b[..n]).is_err(), "a {n}-byte prefix of {} must not parse", b.len()); }
        assert!(Icon::parse(&b).is_ok(), "negative control: the whole file parses");
    }
}
