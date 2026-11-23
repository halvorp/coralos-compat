//! Binary wire codec for shipping parsed bindings from a host (which owns the s-expression reader) into an OS library over
//! `heirloom_ui_abi::wire::Control::Input`. Zero text parsing lives here (ONE-PARSER-PER-FORMAT).
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_ui_abi::{GuestRect, IconRole, WindowRole};

use crate::cdef::{ControlDefBinding, ControlDefIdentity, ControlDefKind};
use crate::plane_chrome::{BandRule, ButtonRule, PlaneChromeBinding, PlaneChromeIdentity};
use crate::spec::{
    AnchorRule, BindingSpec, BoxField, CStrField, CountRule, FileForm, FlagTest, GroupLayout,
    IntWidth, ItemLayout, PlacementLayout, StrListPick,
};
use crate::wdef::{WindowDefBinding, WindowDefIdentity, WindowMeaning};

/// `Control::Input` name for a [`BindingSpec`].
pub const BINDING_INPUT: &str = "binding";
/// `Control::Input` name for a [`WindowDefBinding`].
pub const WDEF_BINDING_INPUT: &str = "wdef-binding";
/// `Control::Input` name for a [`ControlDefBinding`].
pub const CDEF_BINDING_INPUT: &str = "cdef-binding";
/// `Control::Input` name for a [`PlaneChromeBinding`].
pub const PLANE_CHROME_BINDING_INPUT: &str = "plane-chrome-binding";

const SPEC_MAGIC: [u8; 5] = *b"HBND\x01";
const WDEF_MAGIC: [u8; 5] = *b"HWDF\x01";
const CDEF_MAGIC: [u8; 5] = *b"HCDF\x01";
const PCHR_MAGIC: [u8; 5] = *b"HPCM\x01";

/// Encode a [`BindingSpec`] into its deterministic little-endian wire representation.
pub fn encode_spec(spec: &BindingSpec) -> Result<Vec<u8>, String> {
    let mut b = Vec::new();
    b.extend_from_slice(&SPEC_MAGIC);
    put_str(&mut b, &spec.id)?;
    b.extend_from_slice(&spec.file.form_type);
    if spec.file.legacy_form_types.len() > u8::MAX as usize {
        return Err(String::from("binding wire: too many legacy form types"));
    }
    b.push(spec.file.legacy_form_types.len() as u8);
    for l in &spec.file.legacy_form_types {
        b.extend_from_slice(l);
    }
    // group
    b.extend_from_slice(&spec.group.chunk);
    b.extend_from_slice(&spec.group.size.to_le_bytes());
    match &spec.group.title {
        None => b.push(0),
        Some(c) => {
            b.push(1);
            b.extend_from_slice(&c.at.to_le_bytes());
            b.extend_from_slice(&c.max.to_le_bytes());
        }
    }
    match &spec.group.window_box {
        None => b.push(0),
        Some(bx) => {
            b.push(1);
            b.extend_from_slice(&bx.at.to_le_bytes());
        }
    }
    b.extend_from_slice(&spec.group.inset.to_le_bytes());
    put_opt_flag(&mut b, spec.group.no_inset_if.as_ref());
    // item
    b.extend_from_slice(&spec.item.chunk);
    b.extend_from_slice(&spec.item.min_size.to_le_bytes());
    put_opt_flag(&mut b, spec.item.skip_if.as_ref());
    b.extend_from_slice(&spec.item.label.strings_at.to_le_bytes());
    match &spec.item.label.count {
        CountRule::Field { at, width, unless_flag } => {
            b.push(0);
            b.extend_from_slice(&at.to_le_bytes());
            b.push(encode_width(*width));
            match unless_flag {
                None => b.push(0),
                Some((flag, fallback)) => {
                    b.push(1);
                    put_flag(&mut b, flag);
                    b.extend_from_slice(&fallback.to_le_bytes());
                }
            }
        }
    }
    b.extend_from_slice(&spec.item.label.per_record.to_le_bytes());
    b.extend_from_slice(&spec.item.label.pick.to_le_bytes());
    // placement
    match &spec.placement {
        None => b.push(0),
        Some(pl) => {
            b.push(1);
            b.extend_from_slice(&pl.chunk);
            b.extend_from_slice(&pl.min_size.to_le_bytes());
            b.extend_from_slice(&pl.window_box.at.to_le_bytes());
            b.extend_from_slice(&pl.path.at.to_le_bytes());
            b.extend_from_slice(&pl.path.max.to_le_bytes());
        }
    }
    // anchor
    match spec.anchor {
        AnchorRule::LabelInFill { prefix_cut, strip_lines_inside, accent_from_swatch } => {
            b.push(0);
            let mut flags = 0u8;
            if prefix_cut { flags |= 1; }
            if strip_lines_inside { flags |= 2; }
            if accent_from_swatch { flags |= 4; }
            b.push(flags);
        }
    }
    Ok(b)
}

/// Decode a [`BindingSpec`] from its wire representation; malformed or truncated bytes are refused by name.
pub fn decode_spec(bytes: &[u8]) -> Result<BindingSpec, String> {
    let mut r = Reader::new(bytes);
    if r.take(5)? != SPEC_MAGIC {
        return Err(String::from("binding wire: bad magic or version"));
    }
    let id = r.str()?;
    let form_type = r.four()?;
    let legacy_n = r.u8()? as usize;
    let mut legacy_form_types = Vec::with_capacity(legacy_n);
    for _ in 0..legacy_n {
        legacy_form_types.push(r.four()?);
    }
    let group_chunk = r.four()?;
    let group_size = r.u32()?;
    let group_title = match r.u8()? {
        0 => None,
        1 => Some(CStrField { at: r.u32()?, max: r.u32()? }),
        t => return Err(format!("binding wire: bad group title tag {t}")),
    };
    let group_box = match r.u8()? {
        0 => None,
        1 => Some(BoxField { at: r.u32()? }),
        t => return Err(format!("binding wire: bad group window-box tag {t}")),
    };
    let group_inset = r.u32()?;
    let no_inset_if = r.opt_flag()?;
    let item_chunk = r.four()?;
    let item_min_size = r.u32()?;
    let skip_if = r.opt_flag()?;
    let strings_at = r.u32()?;
    let count = match r.u8()? {
        0 => {
            let at = r.u32()?;
            let width = decode_width(r.u8()?)?;
            let unless_flag = match r.u8()? {
                0 => None,
                1 => Some((r.flag()?, r.u32()?)),
                t => return Err(format!("binding wire: bad unless-flag tag {t}")),
            };
            CountRule::Field { at, width, unless_flag }
        }
        t => return Err(format!("binding wire: bad count rule tag {t}")),
    };
    let per_record = r.u32()?;
    let pick = r.u32()?;
    let placement = match r.u8()? {
        0 => None,
        1 => Some(PlacementLayout {
            chunk: r.four()?,
            min_size: r.u32()?,
            window_box: BoxField { at: r.u32()? },
            path: CStrField { at: r.u32()?, max: r.u32()? },
        }),
        t => return Err(format!("binding wire: bad placement tag {t}")),
    };
    let anchor = match r.u8()? {
        0 => {
            let f = r.u8()?;
            if f & !0x07 != 0 {
                return Err(format!("binding wire: unknown anchor flag bits {f:#x}"));
            }
            AnchorRule::LabelInFill {
                prefix_cut: f & 1 != 0,
                strip_lines_inside: f & 2 != 0,
                accent_from_swatch: f & 4 != 0,
            }
        }
        t => return Err(format!("binding wire: bad anchor tag {t}")),
    };
    r.finish("binding wire")?;
    Ok(BindingSpec {
        id,
        file: FileForm { form_type, legacy_form_types },
        group: GroupLayout {
            chunk: group_chunk,
            size: group_size,
            title: group_title,
            window_box: group_box,
            inset: group_inset,
            no_inset_if,
        },
        item: ItemLayout {
            chunk: item_chunk,
            min_size: item_min_size,
            skip_if,
            label: StrListPick { strings_at, count, per_record, pick },
        },
        placement,
        anchor,
    })
}

/// Encode a [`WindowDefBinding`] into its deterministic little-endian wire representation.
pub fn encode_wdef(wdef: &WindowDefBinding) -> Result<Vec<u8>, String> {
    let mut b = Vec::new();
    b.extend_from_slice(&WDEF_MAGIC);
    put_str(&mut b, &wdef.id)?;
    match wdef.identity {
        WindowDefIdentity::FragmentCode { code_length, code_fnv1a64, entry_offset } => {
            b.push(0);
            b.extend_from_slice(&code_length.to_le_bytes());
            b.extend_from_slice(&code_fnv1a64.to_le_bytes());
            b.extend_from_slice(&entry_offset.to_le_bytes());
        }
    }
    b.push(match wdef.meaning.role {
        WindowRole::Document => 0,
        WindowRole::Dialog => 1,
        WindowRole::Utility => 2,
    });
    let mut flags = 0u8;
    if wdef.meaning.titled { flags |= 1; }
    if wdef.meaning.movable { flags |= 2; }
    if wdef.meaning.closable { flags |= 4; }
    if wdef.meaning.resizable { flags |= 8; }
    b.push(flags);
    Ok(b)
}

/// Decode a [`WindowDefBinding`] from its wire representation; malformed or truncated bytes are refused by name.
pub fn decode_wdef(bytes: &[u8]) -> Result<WindowDefBinding, String> {
    let mut r = Reader::new(bytes);
    if r.take(5)? != WDEF_MAGIC {
        return Err(String::from("wdef-binding wire: bad magic or version"));
    }
    let id = r.str()?;
    let identity = match r.u8()? {
        0 => WindowDefIdentity::FragmentCode {
            code_length: r.u32()?,
            code_fnv1a64: r.u64()?,
            entry_offset: r.u32()?,
        },
        t => return Err(format!("wdef-binding wire: bad identity tag {t}")),
    };
    let role = match r.u8()? {
        0 => WindowRole::Document,
        1 => WindowRole::Dialog,
        2 => WindowRole::Utility,
        t => return Err(format!("wdef-binding wire: bad window role {t}")),
    };
    let f = r.u8()?;
    if f & !0x0F != 0 {
        return Err(format!("wdef-binding wire: unknown meaning flag bits {f:#x}"));
    }
    r.finish("wdef-binding wire")?;
    Ok(WindowDefBinding {
        id,
        identity,
        meaning: WindowMeaning {
            role,
            titled: f & 1 != 0,
            movable: f & 2 != 0,
            closable: f & 4 != 0,
            resizable: f & 8 != 0,
        },
    })
}

/// Encode a [`ControlDefBinding`] into its deterministic little-endian wire representation.
pub fn encode_cdef(cdef: &ControlDefBinding) -> Result<Vec<u8>, String> {
    let mut b = Vec::new();
    b.extend_from_slice(&CDEF_MAGIC);
    put_str(&mut b, &cdef.id)?;
    match cdef.identity {
        ControlDefIdentity::Resource { res_id, length, fnv1a64 } => {
            b.push(0);
            b.extend_from_slice(&res_id.to_le_bytes());
            b.extend_from_slice(&length.to_le_bytes());
            b.extend_from_slice(&fnv1a64.to_le_bytes());
        }
    }
    b.push(match cdef.kind {
        ControlDefKind::PushButton => 0,
        ControlDefKind::CheckBox => 1,
        ControlDefKind::RadioButton => 2,
        ControlDefKind::ScrollBar => 3,
    });
    Ok(b)
}

/// Decode a [`ControlDefBinding`] from its wire representation; malformed or truncated bytes are refused by name.
pub fn decode_cdef(bytes: &[u8]) -> Result<ControlDefBinding, String> {
    let mut r = Reader::new(bytes);
    if r.take(5)? != CDEF_MAGIC {
        return Err(String::from("cdef-binding wire: bad magic or version"));
    }
    let id = r.str()?;
    let identity = match r.u8()? {
        0 => {
            let id_bytes = r.take(2)?;
            let res_id = i16::from_le_bytes([id_bytes[0], id_bytes[1]]);
            ControlDefIdentity::Resource { res_id, length: r.u32()?, fnv1a64: r.u64()? }
        }
        t => return Err(format!("cdef-binding wire: bad identity tag {t}")),
    };
    let kind = match r.u8()? {
        0 => ControlDefKind::PushButton,
        1 => ControlDefKind::CheckBox,
        2 => ControlDefKind::RadioButton,
        3 => ControlDefKind::ScrollBar,
        t => return Err(format!("cdef-binding wire: bad control kind {t}")),
    };
    r.finish("cdef-binding wire")?;
    Ok(ControlDefBinding { id, identity, kind })
}

fn put_rect(b: &mut Vec<u8>, r: GuestRect) {
    b.extend_from_slice(&r.left.to_le_bytes());
    b.extend_from_slice(&r.top.to_le_bytes());
    b.extend_from_slice(&r.width.to_le_bytes());
    b.extend_from_slice(&r.height.to_le_bytes());
}

fn encode_icon_role(r: IconRole) -> u8 {
    match r {
        IconRole::Disk => 0,
        IconRole::Volume => 1,
        IconRole::Drawer => 2,
        IconRole::Tool => 3,
        IconRole::Project => 4,
        IconRole::Trash => 5,
        IconRole::Device => 6,
        IconRole::Kick => 7,
        IconRole::AlignLeft => 8,
        IconRole::AlignCenter => 9,
        IconRole::AlignRight => 10,
        IconRole::AlignJustify => 11,
        IconRole::TabStop => 12,
        IconRole::TabDecimal => 13,
    }
}

fn decode_icon_role(t: u8) -> Result<IconRole, String> {
    match t {
        0 => Ok(IconRole::Disk),
        1 => Ok(IconRole::Volume),
        2 => Ok(IconRole::Drawer),
        3 => Ok(IconRole::Tool),
        4 => Ok(IconRole::Project),
        5 => Ok(IconRole::Trash),
        6 => Ok(IconRole::Device),
        7 => Ok(IconRole::Kick),
        8 => Ok(IconRole::AlignLeft),
        9 => Ok(IconRole::AlignCenter),
        10 => Ok(IconRole::AlignRight),
        11 => Ok(IconRole::AlignJustify),
        12 => Ok(IconRole::TabStop),
        13 => Ok(IconRole::TabDecimal),
        _ => Err(format!("plane-chrome wire: bad icon role {t}")),
    }
}

/// Encode a [`PlaneChromeBinding`] into its deterministic little-endian wire representation.
pub fn encode_plane_chrome(pc: &PlaneChromeBinding) -> Result<Vec<u8>, String> {
    let mut b = Vec::new();
    b.extend_from_slice(&PCHR_MAGIC);
    put_str(&mut b, &pc.id)?;
    match pc.identity {
        PlaneChromeIdentity::Resource { res_type, res_id, length, fnv1a64 } => {
            b.push(0);
            b.extend_from_slice(&res_type);
            b.extend_from_slice(&res_id.to_le_bytes());
            b.extend_from_slice(&length.to_le_bytes());
            b.extend_from_slice(&fnv1a64.to_le_bytes());
        }
    }
    match &pc.present_if_text {
        None => b.push(0),
        Some(s) => {
            b.push(1);
            put_str(&mut b, s)?;
        }
    }
    let bn: u8 = pc.bands.len().try_into().map_err(|_| String::from("plane-chrome wire: too many bands"))?;
    b.push(bn);
    for band in &pc.bands {
        put_rect(&mut b, band.rect);
        b.push(u8::from(band.remap_1bit_pixels));
    }
    let btn_n: u8 = pc.buttons.len().try_into().map_err(|_| String::from("plane-chrome wire: too many buttons"))?;
    b.push(btn_n);
    for btn in &pc.buttons {
        put_rect(&mut b, btn.rect);
        put_rect(&mut b, btn.clear_rect);
        match btn.active_rect {
            None => b.push(0),
            Some(ar) => {
                b.push(1);
                put_rect(&mut b, ar);
            }
        }
        put_str(&mut b, &btn.label)?;
        match btn.icon {
            None => b.push(0),
            Some(role) => {
                b.push(1);
                b.push(encode_icon_role(role));
            }
        }
    }
    match pc.suppress_bottom_right {
        None => b.push(0),
        Some((cw, ch)) => {
            b.push(1);
            b.extend_from_slice(&cw.to_le_bytes());
            b.extend_from_slice(&ch.to_le_bytes());
        }
    }
    Ok(b)
}

/// Decode a [`PlaneChromeBinding`] from its wire representation; malformed or truncated bytes are refused by name.
pub fn decode_plane_chrome(bytes: &[u8]) -> Result<PlaneChromeBinding, String> {
    let mut r = Reader::new(bytes);
    if r.take(5)? != PCHR_MAGIC {
        return Err(String::from("plane-chrome wire: bad magic or version"));
    }
    let id = r.str()?;
    let identity = match r.u8()? {
        0 => {
            let res_type = r.four()?;
            let id_b = r.take(2)?;
            let res_id = i16::from_le_bytes([id_b[0], id_b[1]]);
            PlaneChromeIdentity::Resource { res_type, res_id, length: r.u32()?, fnv1a64: r.u64()? }
        }
        t => return Err(format!("plane-chrome wire: bad identity tag {t}")),
    };
    let present_if_text = match r.u8()? {
        0 => None,
        1 => Some(r.str()?),
        t => return Err(format!("plane-chrome wire: bad present_if_text tag {t}")),
    };
    let bn = r.u8()? as usize;
    let mut bands = Vec::with_capacity(bn);
    for _ in 0..bn {
        let rect = r.rect()?;
        let remap_1bit_pixels = match r.u8()? {
            0 => false,
            1 => true,
            t => return Err(format!("plane-chrome wire: bad remap bool {t}")),
        };
        bands.push(BandRule { rect, remap_1bit_pixels });
    }
    let btn_n = r.u8()? as usize;
    let mut buttons = Vec::with_capacity(btn_n);
    for _ in 0..btn_n {
        let rect = r.rect()?;
        let clear_rect = r.rect()?;
        let active_rect = match r.u8()? {
            0 => None,
            1 => Some(r.rect()?),
            t => return Err(format!("plane-chrome wire: bad active_rect tag {t}")),
        };
        let label = r.str()?;
        let icon = match r.u8()? {
            0 => None,
            1 => Some(decode_icon_role(r.u8()?)?),
            t => return Err(format!("plane-chrome wire: bad icon tag {t}")),
        };
        buttons.push(ButtonRule { rect, clear_rect, active_rect, label, icon });
    }
    let suppress_bottom_right = match r.u8()? {
        0 => None,
        1 => Some((r.i32()?, r.i32()?)),
        t => return Err(format!("plane-chrome wire: bad suppress tag {t}")),
    };
    r.finish("plane-chrome wire")?;
    Ok(PlaneChromeBinding { id, identity, present_if_text, bands, buttons, suppress_bottom_right })
}

fn put_str(b: &mut Vec<u8>, s: &str) -> Result<(), String> {
    let n: u16 = s.len().try_into().map_err(|_| String::from("binding wire: id too long"))?;
    b.extend_from_slice(&n.to_le_bytes());
    b.extend_from_slice(s.as_bytes());
    Ok(())
}

fn encode_width(w: IntWidth) -> u8 {
    match w {
        IntWidth::U8 => 0,
        IntWidth::I16 => 1,
        IntWidth::U16 => 2,
        IntWidth::U32 => 3,
    }
}

fn decode_width(b: u8) -> Result<IntWidth, String> {
    match b {
        0 => Ok(IntWidth::U8),
        1 => Ok(IntWidth::I16),
        2 => Ok(IntWidth::U16),
        3 => Ok(IntWidth::U32),
        t => Err(format!("binding wire: bad int width {t}")),
    }
}

fn put_flag(b: &mut Vec<u8>, f: &FlagTest) {
    b.extend_from_slice(&f.at.to_le_bytes());
    b.push(encode_width(f.width));
    b.extend_from_slice(&f.mask.to_le_bytes());
}

fn put_opt_flag(b: &mut Vec<u8>, f: Option<&FlagTest>) {
    match f {
        None => b.push(0),
        Some(f) => {
            b.push(1);
            put_flag(b, f);
        }
    }
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self { Self { buf, pos: 0 } }
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.pos.checked_add(n).ok_or_else(|| String::from("binding wire: length overflow"))?;
        if end > self.buf.len() {
            return Err(format!("binding wire: truncated at offset {} (need {n}, have {})", self.pos, self.buf.len() - self.pos));
        }
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> { Ok(self.take(1)?[0]) }
    fn u16(&mut self) -> Result<u16, String> { Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap())) }
    fn u32(&mut self) -> Result<u32, String> { Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap())) }
    fn i32(&mut self) -> Result<i32, String> { Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap())) }
    fn u64(&mut self) -> Result<u64, String> { Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap())) }
    fn rect(&mut self) -> Result<GuestRect, String> {
        Ok(GuestRect { left: self.i32()?, top: self.i32()?, width: self.i32()?, height: self.i32()? })
    }
    fn four(&mut self) -> Result<[u8; 4], String> { Ok(self.take(4)?.try_into().unwrap()) }
    fn str(&mut self) -> Result<String, String> {
        let n = self.u16()? as usize;
        let b = self.take(n)?;
        core::str::from_utf8(b).map(String::from).map_err(|_| String::from("binding wire: non-UTF-8 id"))
    }
    fn flag(&mut self) -> Result<FlagTest, String> {
        Ok(FlagTest { at: self.u32()?, width: decode_width(self.u8()?)?, mask: self.u32()? })
    }
    fn opt_flag(&mut self) -> Result<Option<FlagTest>, String> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.flag()?)),
            t => Err(format!("binding wire: bad flag option tag {t}")),
        }
    }
    fn finish(&self, kind: &str) -> Result<(), String> {
        if self.pos != self.buf.len() {
            return Err(format!("{kind}: {} trailing byte(s)", self.buf.len() - self.pos));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn sample_spec() -> BindingSpec {
        BindingSpec {
            id: String::from("sample-1.0"),
            file: FileForm { form_type: *b"SAMP", legacy_form_types: vec![] },
            group: GroupLayout {
                chunk: *b"BTNW",
                size: 132,
                title: Some(CStrField { at: 0, max: 32 }),
                window_box: Some(BoxField { at: 32 }),
                inset: 1,
                no_inset_if: Some(FlagTest { at: 128, width: IntWidth::U32, mask: 16 }),
            },
            item: ItemLayout {
                chunk: *b"BUTN",
                min_size: 14,
                skip_if: Some(FlagTest { at: 2, width: IntWidth::U32, mask: 4 }),
                label: StrListPick {
                    strings_at: 14,
                    count: CountRule::Field {
                        at: 6,
                        width: IntWidth::I16,
                        unless_flag: Some((FlagTest { at: 2, width: IntWidth::U32, mask: 16 }, 3)),
                    },
                    per_record: 2,
                    pick: 0,
                },
            },
            placement: Some(PlacementLayout {
                chunk: *b"BANK",
                min_size: 29,
                window_box: BoxField { at: 20 },
                path: CStrField { at: 28, max: 256 },
            }),
            anchor: AnchorRule::LabelInFill {
                prefix_cut: true,
                strip_lines_inside: true,
                accent_from_swatch: true,
            },
        }
    }

    #[test]
    fn binding_spec_roundtrips_and_rejects_malformed_wire_bytes() {
        let spec = sample_spec();
        let bytes = encode_spec(&spec).unwrap();
        assert_eq!(decode_spec(&bytes).unwrap(), spec);
        // NEGATIVE CONTROLS: bad magic, truncated, trailing bytes
        let mut bad_magic = bytes.clone();
        bad_magic[0] = b'X';
        assert!(decode_spec(&bad_magic).unwrap_err().contains("bad magic"));
        assert!(decode_spec(&bytes[..bytes.len() - 1]).unwrap_err().contains("truncated"));
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(decode_spec(&trailing).unwrap_err().contains("trailing"));
    }

    #[test]
    fn wdef_binding_roundtrips_and_rejects_malformed_wire_bytes() {
        let w = WindowDefBinding {
            id: String::from("photoshop-7.0-emptywdef"),
            identity: WindowDefIdentity::FragmentCode {
                code_length: 1312,
                code_fnv1a64: 0xb194c5872806ba7f,
                entry_offset: 400,
            },
            meaning: WindowMeaning {
                role: WindowRole::Utility,
                titled: false,
                movable: false,
                closable: false,
                resizable: false,
            },
        };
        let bytes = encode_wdef(&w).unwrap();
        assert_eq!(decode_wdef(&bytes).unwrap(), w);
        // NEGATIVE CONTROLS: bad magic, bad role, unknown flags, trailing bytes
        let mut bad_role = bytes.clone();
        let n = bad_role.len();
        bad_role[n - 2] = 9;
        assert!(decode_wdef(&bad_role).unwrap_err().contains("window role"));
        let mut bad_flags = bytes.clone();
        bad_flags[n - 1] = 0x80;
        assert!(decode_wdef(&bad_flags).unwrap_err().contains("flag bits"));
        assert!(decode_wdef(&bytes[..4]).unwrap_err().contains("truncated"));
    }

    #[test]
    fn cdef_binding_roundtrips_and_rejects_malformed_wire_bytes() {
        let c = ControlDefBinding {
            id: String::from("test-cdef2"),
            identity: ControlDefIdentity::Resource {
                res_id: 2,
                length: 1824,
                fnv1a64: 0x6cc07d31998bde0d,
            },
            kind: ControlDefKind::ScrollBar,
        };
        let bytes = encode_cdef(&c).unwrap();
        assert_eq!(decode_cdef(&bytes).unwrap(), c);
        let mut bad_kind = bytes.clone();
        let n = bad_kind.len();
        bad_kind[n - 1] = 9;
        assert!(decode_cdef(&bad_kind).unwrap_err().contains("control kind"));
        assert!(decode_cdef(&bytes[..4]).unwrap_err().contains("truncated"));
    }

    #[test]
    fn plane_chrome_binding_roundtrips_and_rejects_malformed_wire_bytes() {
        let gr = |left, top, width, height| GuestRect { left, top, width, height };
        let pc = PlaneChromeBinding {
            id: String::from("test-ruler"),
            identity: PlaneChromeIdentity::Resource {
                res_type: *b"CODE",
                res_id: 1,
                length: 2530,
                fnv1a64: 0x73421e483f3e48ef,
            },
            present_if_text: Some(String::from("6 lines/inch")),
            bands: vec![BandRule { rect: gr(0, 0, 487, 46), remap_1bit_pixels: true }],
            buttons: vec![ButtonRule {
                rect: gr(327, 26, 33, 18),
                clear_rect: gr(326, 25, 35, 20),
                active_rect: Some(gr(329, 29, 28, 12)),
                label: String::from(" "),
                icon: Some(IconRole::AlignLeft),
            }],
            suppress_bottom_right: Some((22, 22)),
        };
        let bytes = encode_plane_chrome(&pc).unwrap();
        assert_eq!(decode_plane_chrome(&bytes).unwrap(), pc);
        let mut bad_magic = bytes.clone();
        bad_magic[0] = b'Z';
        assert!(decode_plane_chrome(&bad_magic).unwrap_err().contains("bad magic"));
        assert!(decode_plane_chrome(&bytes[..bytes.len() - 1]).unwrap_err().contains("truncated"));
    }
}
