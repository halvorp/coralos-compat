//! Binary wire codec for shipping parsed bindings from a host (which owns the s-expression reader) into an OS library over
//! `heirloom_ui_abi::wire::Control::Input`. Zero text parsing lives here (ONE-PARSER-PER-FORMAT).
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_ui_abi::{GuestPoint, GuestRect, IconRole, PenRole, WindowRole};

use crate::cdef::{ControlDefBinding, ControlDefIdentity, ControlDefKind};
use crate::plane_chrome::{
    BandRule, ButtonRule, FooterKeyRule, MiniToggleRule, PaletteRule, PaletteRuleKind,
    PixelMaskGuard, PlaneChromeBinding, PlaneChromeIdentity, TabProbeRule, ToolCellRule,
    WindowRange,
};
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
        IconRole::BoxSelect => 14,
        IconRole::Move => 15,
        IconRole::Lasso => 16,
        IconRole::MagicWand => 17,
        IconRole::Crop => 18,
        IconRole::Paintbrush => 19,
        IconRole::Stamp => 20,
        IconRole::Eraser => 21,
        IconRole::PaintBucket => 22,
        IconRole::Droplet => 23,
        IconRole::MousePointer => 24,
        IconRole::Type => 25,
        IconRole::PenTool => 26,
        IconRole::Shapes => 27,
        IconRole::Pipette => 28,
        IconRole::Hand => 29,
        IconRole::ZoomIn => 30,
        IconRole::ZoomOut => 31,
        IconRole::Eye => 32,
        IconRole::Link => 33,
        IconRole::Lock => 34,
        IconRole::FilePlus => 35,
        IconRole::FolderPlus => 36,
        IconRole::Layers => 37,
        IconRole::Sliders => 38,
        IconRole::Square => 39,
        IconRole::SquaresUnite => 40,
        IconRole::SquaresSubtract => 41,
        IconRole::SquaresIntersect => 42,
        IconRole::SquareDashed => 43,
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
        14 => Ok(IconRole::BoxSelect),
        15 => Ok(IconRole::Move),
        16 => Ok(IconRole::Lasso),
        17 => Ok(IconRole::MagicWand),
        18 => Ok(IconRole::Crop),
        19 => Ok(IconRole::Paintbrush),
        20 => Ok(IconRole::Stamp),
        21 => Ok(IconRole::Eraser),
        22 => Ok(IconRole::PaintBucket),
        23 => Ok(IconRole::Droplet),
        24 => Ok(IconRole::MousePointer),
        25 => Ok(IconRole::Type),
        26 => Ok(IconRole::PenTool),
        27 => Ok(IconRole::Shapes),
        28 => Ok(IconRole::Pipette),
        29 => Ok(IconRole::Hand),
        30 => Ok(IconRole::ZoomIn),
        31 => Ok(IconRole::ZoomOut),
        32 => Ok(IconRole::Eye),
        33 => Ok(IconRole::Link),
        34 => Ok(IconRole::Lock),
        35 => Ok(IconRole::FilePlus),
        36 => Ok(IconRole::FolderPlus),
        37 => Ok(IconRole::Layers),
        38 => Ok(IconRole::Sliders),
        39 => Ok(IconRole::Square),
        40 => Ok(IconRole::SquaresUnite),
        41 => Ok(IconRole::SquaresSubtract),
        42 => Ok(IconRole::SquaresIntersect),
        43 => Ok(IconRole::SquareDashed),
        _ => Err(format!("plane-chrome wire: bad icon role {t}")),
    }
}

fn encode_pen_role(p: PenRole) -> u8 {
    match p {
        PenRole::Surface => 0,
        PenRole::Text => 1,
        PenRole::TextMuted => 2,
        PenRole::Shine => 3,
        PenRole::Shadow => 4,
        PenRole::Fill => 5,
        PenRole::FillText => 6,
        PenRole::Accent => 7,
        PenRole::Highlight => 8,
        PenRole::Background => 9,
        PenRole::Selection => 10,
    }
}

fn decode_pen_role(t: u8) -> Result<PenRole, String> {
    match t {
        0 => Ok(PenRole::Surface),
        1 => Ok(PenRole::Text),
        2 => Ok(PenRole::TextMuted),
        3 => Ok(PenRole::Shine),
        4 => Ok(PenRole::Shadow),
        5 => Ok(PenRole::Fill),
        6 => Ok(PenRole::FillText),
        7 => Ok(PenRole::Accent),
        8 => Ok(PenRole::Highlight),
        9 => Ok(PenRole::Background),
        10 => Ok(PenRole::Selection),
        _ => Err(format!("plane-chrome wire: bad pen role {t}")),
    }
}

fn put_point(b: &mut Vec<u8>, p: GuestPoint) {
    b.extend_from_slice(&p.x.to_le_bytes());
    b.extend_from_slice(&p.y.to_le_bytes());
}

fn put_opt_rect(b: &mut Vec<u8>, r: Option<GuestRect>) {
    match r {
        None => b.push(0),
        Some(rect) => {
            b.push(1);
            put_rect(b, rect);
        }
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
    if pc.window_range.is_some() || !pc.pixel_guards.is_empty() || !pc.palette_rules.is_empty() {
        b.push(1); // v1 extension tag
        match pc.window_range {
            None => b.push(0),
            Some(wr) => {
                b.push(1);
                b.extend_from_slice(&wr.min_w.to_le_bytes());
                b.extend_from_slice(&wr.max_w.to_le_bytes());
                b.extend_from_slice(&wr.min_h.to_le_bytes());
                b.extend_from_slice(&wr.max_h.to_le_bytes());
            }
        }
        let gn: u8 = pc.pixel_guards.len().try_into().map_err(|_| String::from("plane-chrome wire: too many pixel guards"))?;
        b.push(gn);
        for g in &pc.pixel_guards {
            put_rect(&mut b, g.rect);
            b.push(g.dark_luma);
            b.extend_from_slice(&g.fnv1a64.to_le_bytes());
        }
        let pn: u8 = pc.palette_rules.len().try_into().map_err(|_| String::from("plane-chrome wire: too many palette rules"))?;
        b.push(pn);
        for pr in &pc.palette_rules {
            put_rect(&mut b, pr.rect);
            put_opt_rect(&mut b, pr.clear_rect);
            match &pr.kind {
                PaletteRuleKind::SurfaceFill { pen_role } => {
                    b.push(0);
                    b.push(encode_pen_role(*pen_role));
                }
                PaletteRuleKind::KeepPixels => {
                    b.push(1);
                }
                PaletteRuleKind::Tabs { tabs, flyout } => {
                    b.push(2);
                    b.push(u8::from(*flyout));
                    let tn: u8 = tabs.len().try_into().map_err(|_| String::from("plane-chrome wire: too many tabs"))?;
                    b.push(tn);
                    for t in tabs {
                        put_str(&mut b, &t.label)?;
                        put_rect(&mut b, t.probe_rect);
                    }
                }
                PaletteRuleKind::ToolGrid { cols, tools } => {
                    b.push(3);
                    b.push(*cols);
                    let tn: u8 = tools.len().try_into().map_err(|_| String::from("plane-chrome wire: too many tools"))?;
                    b.push(tn);
                    for t in tools {
                        b.push(encode_icon_role(t.icon));
                        b.push(u8::from(t.has_flyout) | (u8::from(t.break_after) << 1));
                        put_rect(&mut b, t.probe_rect);
                    }
                }
                PaletteRuleKind::ColorWell { fore_pt, back_pt } => {
                    b.push(4);
                    put_point(&mut b, *fore_pt);
                    put_point(&mut b, *back_pt);
                }
                PaletteRuleKind::Footer { keys } => {
                    b.push(5);
                    let kn: u8 = keys.len().try_into().map_err(|_| String::from("plane-chrome wire: too many footer keys"))?;
                    b.push(kn);
                    for k in keys {
                        b.push(encode_icon_role(k.icon));
                        put_opt_rect(&mut b, k.probe_rect);
                    }
                }
                PaletteRuleKind::MiniToggles { keys } => {
                    b.push(6);
                    let kn: u8 = keys.len().try_into().map_err(|_| String::from("plane-chrome wire: too many mini toggles"))?;
                    b.push(kn);
                    for k in keys {
                        b.push(encode_icon_role(k.icon));
                        put_rect(&mut b, k.probe_rect);
                        b.push(k.threshold_luma);
                    }
                }
                PaletteRuleKind::GradientSlider { label, stops, track_rect, default_permille } => {
                    b.push(7);
                    put_str(&mut b, label)?;
                    let sn: u8 = stops.len().try_into().map_err(|_| String::from("plane-chrome wire: too many stops"))?;
                    b.push(sn);
                    for &s in stops {
                        b.extend_from_slice(&s.to_le_bytes());
                    }
                    put_opt_rect(&mut b, *track_rect);
                    b.extend_from_slice(&default_permille.to_le_bytes());
                }
                PaletteRuleKind::Label { text, muted } => {
                    b.push(8);
                    put_str(&mut b, text)?;
                    b.push(u8::from(*muted));
                }
                PaletteRuleKind::Popup { label, icon, enabled } => {
                    b.push(9);
                    put_str(&mut b, label)?;
                    match icon {
                        None => b.push(0),
                        Some(role) => {
                            b.push(1);
                            b.push(encode_icon_role(*role));
                        }
                    }
                    b.push(u8::from(*enabled));
                }
                PaletteRuleKind::Check { label, probe_rect, enabled } => {
                    b.push(10);
                    put_str(&mut b, label)?;
                    put_opt_rect(&mut b, *probe_rect);
                    b.push(u8::from(*enabled));
                }
                PaletteRuleKind::FieldXs { value, unit, numeric_cap, enabled } => {
                    b.push(11);
                    put_str(&mut b, value)?;
                    put_str(&mut b, unit)?;
                    b.push(u8::from(*numeric_cap) | (u8::from(*enabled) << 1));
                }
                PaletteRuleKind::MiniSlider { track_rect, default_permille, enabled } => {
                    b.push(12);
                    put_opt_rect(&mut b, *track_rect);
                    b.extend_from_slice(&default_permille.to_le_bytes());
                    b.push(u8::from(*enabled));
                }
                PaletteRuleKind::EditField { value, enabled } => {
                    b.push(13);
                    put_str(&mut b, value)?;
                    b.push(u8::from(*enabled));
                }
            }
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
    let (window_range, pixel_guards, palette_rules) = if r.remaining() > 0 {
        match r.u8()? {
            1 => {
                let window_range = match r.u8()? {
                    0 => None,
                    1 => Some(WindowRange { min_w: r.i32()?, max_w: r.i32()?, min_h: r.i32()?, max_h: r.i32()? }),
                    t => return Err(format!("plane-chrome wire: bad window_range tag {t}")),
                };
                let gn = r.u8()? as usize;
                let mut pixel_guards = Vec::with_capacity(gn);
                for _ in 0..gn {
                    pixel_guards.push(PixelMaskGuard { rect: r.rect()?, dark_luma: r.u8()?, fnv1a64: r.u64()? });
                }
                let pn = r.u8()? as usize;
                let mut palette_rules = Vec::with_capacity(pn);
                for _ in 0..pn {
                    let rect = r.rect()?;
                    let clear_rect = r.opt_rect()?;
                    let kind = match r.u8()? {
                        0 => PaletteRuleKind::SurfaceFill { pen_role: decode_pen_role(r.u8()?)? },
                        1 => PaletteRuleKind::KeepPixels,
                        2 => {
                            let flyout = match r.u8()? {
                                0 => false,
                                1 => true,
                                t => return Err(format!("plane-chrome wire: bad flyout bool {t}")),
                            };
                            let tn = r.u8()? as usize;
                            let mut tabs = Vec::with_capacity(tn);
                            for _ in 0..tn {
                                tabs.push(TabProbeRule { label: r.str()?, probe_rect: r.rect()? });
                            }
                            PaletteRuleKind::Tabs { tabs, flyout }
                        }
                        3 => {
                            let cols = r.u8()?;
                            let tn = r.u8()? as usize;
                            let mut tools = Vec::with_capacity(tn);
                            for _ in 0..tn {
                                let icon = decode_icon_role(r.u8()?)?;
                                let flags = r.u8()?;
                                if flags & !0x03 != 0 {
                                    return Err(format!("plane-chrome wire: bad tool flags {flags:#x}"));
                                }
                                tools.push(ToolCellRule {
                                    icon,
                                    has_flyout: flags & 1 != 0,
                                    break_after: flags & 2 != 0,
                                    probe_rect: r.rect()?,
                                });
                            }
                            PaletteRuleKind::ToolGrid { cols, tools }
                        }
                        4 => PaletteRuleKind::ColorWell {
                            fore_pt: GuestPoint { x: r.i32()?, y: r.i32()? },
                            back_pt: GuestPoint { x: r.i32()?, y: r.i32()? },
                        },
                        5 => {
                            let kn = r.u8()? as usize;
                            let mut keys = Vec::with_capacity(kn);
                            for _ in 0..kn {
                                keys.push(FooterKeyRule { icon: decode_icon_role(r.u8()?)?, probe_rect: r.opt_rect()? });
                            }
                            PaletteRuleKind::Footer { keys }
                        }
                        6 => {
                            let kn = r.u8()? as usize;
                            let mut keys = Vec::with_capacity(kn);
                            for _ in 0..kn {
                                keys.push(MiniToggleRule {
                                    icon: decode_icon_role(r.u8()?)?,
                                    probe_rect: r.rect()?,
                                    threshold_luma: r.u8()?,
                                });
                            }
                            PaletteRuleKind::MiniToggles { keys }
                        }
                        7 => {
                            let label = r.str()?;
                            let sn = r.u8()? as usize;
                            let mut stops = Vec::with_capacity(sn);
                            for _ in 0..sn {
                                stops.push(r.u32()?);
                            }
                            let track_rect = r.opt_rect()?;
                            let default_permille = r.u16()?;
                            PaletteRuleKind::GradientSlider { label, stops, track_rect, default_permille }
                        }
                        8 => {
                            let text = r.str()?;
                            let muted = r.u8()? != 0;
                            PaletteRuleKind::Label { text, muted }
                        }
                        9 => {
                            let label = r.str()?;
                            let icon = match r.u8()? {
                                0 => None,
                                1 => Some(decode_icon_role(r.u8()?)?),
                                t => return Err(format!("plane-chrome wire: bad popup icon tag {t}")),
                            };
                            let enabled = r.u8()? != 0;
                            PaletteRuleKind::Popup { label, icon, enabled }
                        }
                        10 => {
                            let label = r.str()?;
                            let probe_rect = r.opt_rect()?;
                            let enabled = r.u8()? != 0;
                            PaletteRuleKind::Check { label, probe_rect, enabled }
                        }
                        11 => {
                            let value = r.str()?;
                            let unit = r.str()?;
                            let fl = r.u8()?;
                            PaletteRuleKind::FieldXs { value, unit, numeric_cap: fl & 1 != 0, enabled: fl & 2 != 0 }
                        }
                        12 => {
                            let track_rect = r.opt_rect()?;
                            let default_permille = r.u16()?;
                            let enabled = r.u8()? != 0;
                            PaletteRuleKind::MiniSlider { track_rect, default_permille, enabled }
                        }
                        13 => {
                            let value = r.str()?;
                            let enabled = r.u8()? != 0;
                            PaletteRuleKind::EditField { value, enabled }
                        }
                        t => return Err(format!("plane-chrome wire: bad palette rule tag {t}")),
                    };
                    palette_rules.push(PaletteRule { rect, clear_rect, kind });
                }
                (window_range, pixel_guards, palette_rules)
            }
            t => return Err(format!("plane-chrome wire: bad extension tag {t}")),
        }
    } else {
        (None, Vec::new(), Vec::new())
    };
    r.finish("plane-chrome wire")?;
    Ok(PlaneChromeBinding {
        id,
        identity,
        present_if_text,
        bands,
        buttons,
        suppress_bottom_right,
        window_range,
        pixel_guards,
        palette_rules,
    })
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
    fn opt_rect(&mut self) -> Result<Option<GuestRect>, String> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.rect()?)),
            t => Err(format!("binding wire: bad rect option tag {t}")),
        }
    }
    fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
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
            window_range: None,
            pixel_guards: vec![],
            palette_rules: vec![],
        };
        let bytes = encode_plane_chrome(&pc).unwrap();
        assert_eq!(decode_plane_chrome(&bytes).unwrap(), pc);
        let mut bad_magic = bytes.clone();
        bad_magic[0] = b'Z';
        assert!(decode_plane_chrome(&bad_magic).unwrap_err().contains("bad magic"));
        assert!(decode_plane_chrome(&bytes[..bytes.len() - 1]).unwrap_err().contains("truncated"));

        // Extended palette-rule binding roundtrip + bad extension tag rejection.
        let ext = PlaneChromeBinding {
            id: String::from("test-palette-wire"),
            identity: PlaneChromeIdentity::Resource {
                res_type: *b"WDEF",
                res_id: 128,
                length: 15012,
                fnv1a64: 0xccfe_72ef_63c8_a2ab,
            },
            present_if_text: None,
            bands: vec![],
            buttons: vec![],
            suppress_bottom_right: None,
            window_range: Some(WindowRange { min_w: 200, max_w: 214, min_h: 100, max_h: 120 }),
            pixel_guards: vec![PixelMaskGuard { rect: gr(4, 2, 48, 12), dark_luma: 80, fnv1a64: 0x1234_5678 }],
            palette_rules: vec![
                PaletteRule {
                    rect: gr(0, 0, 0, 0),
                    clear_rect: Some(gr(0, 0, 0, 0)),
                    kind: PaletteRuleKind::SurfaceFill { pen_role: PenRole::Surface },
                },
                PaletteRule {
                    rect: gr(0, 0, 0, 22),
                    clear_rect: None,
                    kind: PaletteRuleKind::Tabs {
                        tabs: vec![TabProbeRule { label: String::from("Color"), probe_rect: gr(4, 2, 36, 14) }],
                        flyout: true,
                    },
                },
                PaletteRule {
                    rect: gr(2, 24, 52, 240),
                    clear_rect: None,
                    kind: PaletteRuleKind::ToolGrid {
                        cols: 2,
                        tools: vec![ToolCellRule {
                            icon: IconRole::BoxSelect,
                            has_flyout: true,
                            break_after: true,
                            probe_rect: gr(4, 26, 20, 20),
                        }],
                    },
                },
                PaletteRule {
                    rect: gr(6, 270, 44, 40),
                    clear_rect: None,
                    kind: PaletteRuleKind::ColorWell {
                        fore_pt: GuestPoint { x: 16, y: 280 },
                        back_pt: GuestPoint { x: 32, y: 294 },
                    },
                },
                PaletteRule {
                    rect: gr(0, -22, 0, 22),
                    clear_rect: None,
                    kind: PaletteRuleKind::Footer {
                        keys: vec![FooterKeyRule { icon: IconRole::Trash, probe_rect: Some(gr(180, -18, 16, 14)) }],
                    },
                },
                PaletteRule {
                    rect: gr(8, 50, 80, 20),
                    clear_rect: None,
                    kind: PaletteRuleKind::MiniToggles {
                        keys: vec![MiniToggleRule { icon: IconRole::Lock, probe_rect: gr(10, 52, 14, 14), threshold_luma: 180 }],
                    },
                },
                PaletteRule {
                    rect: gr(8, 76, 190, 14),
                    clear_rect: None,
                    kind: PaletteRuleKind::GradientSlider {
                        label: String::from("R"),
                        stops: vec![0x0000_0000, 0x00FF_0000],
                        track_rect: Some(gr(26, 78, 120, 8)),
                        default_permille: 500,
                    },
                },
                PaletteRule {
                    rect: gr(4, 22, 198, 66),
                    clear_rect: None,
                    kind: PaletteRuleKind::KeepPixels,
                },
            ],
        };
        let ext_bytes = encode_plane_chrome(&ext).unwrap();
        assert_eq!(decode_plane_chrome(&ext_bytes).unwrap(), ext);
        assert!(decode_plane_chrome(&ext_bytes[..ext_bytes.len() - 1]).unwrap_err().contains("truncated"));
        let mut bad_ext = bytes;
        bad_ext.push(99);
        assert!(decode_plane_chrome(&bad_ext).unwrap_err().contains("extension tag"));
    }
}
