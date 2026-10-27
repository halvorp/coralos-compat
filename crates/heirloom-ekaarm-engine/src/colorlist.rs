//! `CColorList` / `CColorArray` (gulcolor.h:183-262, the EGUL library): the logical-to-physical colour table `CEikonEnv::iColorList`
//! holds. Built the way `CCoeDataStorage::GetSystemColorListL` builds it (CoeDataStorage.cpp:150-205): the `EIKCOLORLIST` resource
//! `r_eik_color_color16` of the ROM's `eikpriv` resource file names (two LONG links) an array of `CTRL_COLOR`s (a WORD count, then per
//! colour a WORD logical colour and R, G, B bytes: `ResourceUtils::PopulateColorArrayL`, GULUTIL.CPP:~531) and the extras array,
//! which goes in under the custom-colours UID. The guest sees opaque objects; the table lives host-side (`objects.rs`).
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_fmt_rsc::Reader;
use crate::loader::Process;
use crate::natives::NativeOutcome;
use crate::objects::HostObj;

/// `r_eik_color_color16` (EIKPRIV.rsg:9): the colour list of a device with 16 or more colours, the S60 screen modes.
pub const R_EIK_COLOR_COLOR16: u32 = 0x19bd_6003;
/// `KEikCustomColorsArrayValue` (EIKDEF.H:181): the UID the extras array is filed under.
pub const KEIK_CUSTOM_COLORS_ARRAY: u32 = 0x1000_57c2;
/// `EColorLabelHighlightFullEmphasis` + 1: the logical colours `CColorList` holds (CoeDataStorage.cpp:~174 asserts the array has this many).
const LOGICAL_COLOURS: usize = 59;
/// `CEikonEnv::iColorList` (eikenv.h:385), the offset the app's inline `CEikonEnv::ColorList()` reads (`ldr r?,[env,#0xE4]`).
pub const EIKENV_I_COLOR_LIST: u32 = 0xE4;

/// The host state of a `CColorList`: the logical-colour table and the per-application colour arrays (guest `CColorArray` objects).
#[derive(Debug, Clone, Default)]
pub struct ColorListState {
    pub eik: Vec<u32>,
    pub arrays: BTreeMap<u32, u32>,
}

/// A `CColorArray`: (logical colour, TRgb) in the resource's order.
#[derive(Debug, Clone, Default)]
pub struct ColorArrayState(pub Vec<(i32, u32)>);

/// `TRgb(r, g, b)` (gdi.inl:53): opaque, `0xAARRGGBB`.
fn trgb(r: u8, g: u8, b: u8) -> u32 {
    0xff00_0000 | (r as u32) << 16 | (g as u32) << 8 | b as u32
}

/// `ResourceUtils::PopulateColorArrayL` over the bytes of an ARRAY of CTRL_COLOR.
fn parse_colors(bytes: &[u8]) -> Result<Vec<(i32, u32)>, String> {
    let e = |e| format!("CTRL_COLOR array: {e:?}");
    let mut r = Reader::new(bytes);
    let n = r.word().map_err(e)?;
    let mut out = Vec::with_capacity(n as usize);
    for _ in 0..n {
        let logical = r.word().map_err(e)? as i32;
        let (red, green, blue) = (r.byte().map_err(e)?, r.byte().map_err(e)?, r.byte().map_err(e)?);
        out.push((logical, trgb(red, green, blue)));
    }
    Ok(out)
}

impl Process {
    /// The guest `CColorList` of `CEikonEnv`, read from the ROM resource exactly as the framework does.
    pub(crate) fn make_color_list(&mut self) -> Result<u32, String> {
        let list = self.resource(R_EIK_COLOR_COLOR16)?;
        let mut r = Reader::new(&list);
        let (array_id, ext_id) = (r.long().map_err(|e| format!("EIKCOLORLIST: {e:?}"))?, r.long().map_err(|e| format!("EIKCOLORLIST: {e:?}"))?);
        let colours = parse_colors(&self.resource(array_id)?)?;
        let mut eik = Vec::with_capacity(LOGICAL_COLOURS);
        for logical in 0..LOGICAL_COLOURS as i32 {
            let (_, rgb) = colours.iter().find(|(l, _)| *l == logical).ok_or_else(|| format!("EIKCOLORLIST {R_EIK_COLOR_COLOR16:#x}: logical colour {logical} missing from {array_id:#x} (the framework panics ECoePanicLogicalColorNotFound)"))?;
            eik.push(*rgb);
        }
        let extras = parse_colors(&self.resource(ext_id)?)?;
        let array = self.plain_object("CColorArray", 0x20)?;
        self.obj_set(array, HostObj::ColorArray(ColorArrayState(extras)))?;
        let obj = self.plain_object("CColorList", 12)?;
        let mut arrays = BTreeMap::new();
        arrays.insert(KEIK_CUSTOM_COLORS_ARRAY, array);
        self.obj_set(obj, HostObj::ColorList(ColorListState { eik, arrays }))?;
        Ok(obj)
    }
}

pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    if !name.contains("CColorList") && !name.contains("CColorArray") {
        return None;
    }
    let (this, a1, a2) = (p.cpu.regs[0], p.cpu.regs[1], p.cpu.regs[2]);
    let refuse = |m: String| Some(NativeOutcome::Refused(format!("NEXT-NEEDED egul {name}: {m}")));
    let ret = |p: &mut Process, v: u32| { p.cpu.regs[0] = v; Some(NativeOutcome::Return) };
    let list = match p.obj_get(this) {
        Some(HostObj::ColorList(s)) => Some(s.clone()),
        _ => None,
    };
    match (name, list) {
        // TRgb CColorList::Color(TLogicalColor) const (GULCOLOR.CPP:~80): out of range panics EGUL 3.
        ("_ZNK10CColorList5ColorE13TLogicalColor", Some(s)) => match s.eik.get(a1 as usize) {
            Some(&v) if (a1 as i32) >= 0 => ret(p, v),
            _ => refuse(format!("logical colour {} is not in the list of {} (the framework panics EGUL 3)", a1 as i32, s.eik.len())),
        },
        ("_ZNK10CColorList5CountEv", Some(s)) => ret(p, s.eik.len() as u32),
        // TRgb CColorList::Color(TUid aApp, TInt aColor) const: TUid is a one-word class passed in a register.
        ("_ZNK10CColorList5ColorE4TUidi", Some(s)) => match s.arrays.get(&a1).copied().and_then(|a| match p.obj_get(a) { Some(HostObj::ColorArray(c)) => c.0.iter().find(|(l, _)| *l == a2 as i32).map(|x| x.1), _ => None }) {
            Some(v) => ret(p, v),
            None => refuse(format!("application {a1:#x} has no colour array with colour {} (the framework panics EGUL 6 / 4)", a2 as i32)),
        },
        ("_ZNK10CColorList10ColorArrayE4TUid", Some(s)) => match s.arrays.get(&a1) {
            Some(&a) => ret(p, a),
            None => refuse(format!("application {a1:#x} has no entry in the colour list (the framework panics EGUL 6)")),
        },
        ("_ZNK10CColorList18ContainsColorArrayE4TUid", Some(s)) => ret(p, s.arrays.contains_key(&a1) as u32),
        ("_ZNK11CColorArray5CountEv", None) => match p.obj_get(this) { Some(HostObj::ColorArray(c)) => { let n = c.0.len() as u32; ret(p, n) } _ => refuse(format!("{this:#x} is not a colour array this engine made")) },
        ("_ZNK11CColorArray8ContainsEi", None) => match p.obj_get(this) { Some(HostObj::ColorArray(c)) => { let v = c.0.iter().any(|(l, _)| *l == a1 as i32) as u32; ret(p, v) } _ => refuse(format!("{this:#x} is not a colour array this engine made")) },
        ("_ZNK11CColorArray5ColorEi", None) => match p.obj_get(this) {
            Some(HostObj::ColorArray(c)) => match c.0.iter().find(|(l, _)| *l == a1 as i32) { Some(&(_, v)) => ret(p, v), None => refuse(format!("logical colour {} is not in the array", a1 as i32)) },
            _ => refuse(format!("{this:#x} is not a colour array this engine made")),
        },
        _ => None,
    }
}
