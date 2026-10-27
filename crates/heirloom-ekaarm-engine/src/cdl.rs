//! The CDL engine (`cdlengine.dll`) and the layout instances it serves, native. S60 layout (pane rectangles, list metrics) is
//! "customisation data": interfaces (an interface UID and API ids) implemented by instances stacked in a customisation stack; the
//! client asks `CdlEngine::GetFunction(uid, apiId)` for the implementation from the top-most instance that has one and CALLS it.
//! Facts from the Symbian Foundation UI-resources source (EPL-1.0, reference): layouts/cdl/CdlEngine/src/CdlEngine.cpp (GetFunction),
//! EABI/CdlEngineU.DEF (ordinal 55), layouts/aknlayout2 (the instances, the packs, LayoutMetaData).
//!
//! The instances' DATA is not in this repo: the host extracts it from the owner's local source tree with `examples/gen_cdl_layout`
//! into one file and hands it in ([`CdlState::load`]); until it does, every CDL call is a named refusal.
//!
//! A function the guest gets back is an engine stub named `cdl@<instance>:<api>`; calling it runs [`call_function`].
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use crate::layoutdecode::{self, Instance as DecodeInstance};
use crate::loader::Process;
use crate::natives::NativeOutcome;

/// What an instance does for one API id.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// Not implemented here: the layer below answers.
    None,
    /// The function returns this constant (a `TBool`, an int or a `TReal32`'s bits).
    Const(u32),
    /// A decode operation over the instance's byte-coded data (`AknLayoutScalableDecode::<name>`).
    Decode(String),
    /// Real code the extractor does not read (an adaptation layer, a font table).
    Opaque(String),
}

#[derive(Debug, Clone)]
pub struct Instance {
    pub name: String,
    pub uid: u32,
    /// (blob name, offset into it) and the lookup table, for a decode instance.
    pub data: Option<(String, usize)>,
    pub lookup: Vec<u16>,
    pub ops: Vec<Op>,
}

#[derive(Debug, Clone)]
pub struct Pack {
    pub name: String,
    pub width: i32,
    pub height: i32,
    pub layout_id: String,
    pub zoom: String,
    pub priority: i32,
    pub contents: Vec<String>,
}

#[derive(Debug, Default)]
pub struct CdlData {
    pub blobs: BTreeMap<String, Vec<u8>>,
    pub instances: Vec<Instance>,
    pub packs: Vec<Pack>,
    /// Interface UID -> (name, api names in id order).
    pub interfaces: BTreeMap<u32, (String, Vec<String>)>,
    /// avkon resource ids by name (`R_AVKON_*PANE*`), from the SDK's own avkon.rsg.
    pub rsg: BTreeMap<String, u32>,
}

/// The CDL engine's state in a process: the data file, and per interface UID the customisation stack (instance indices, bottom first).
#[derive(Debug, Default)]
pub struct CdlState {
    pub data: Option<CdlData>,
    pub stacks: BTreeMap<u32, Vec<usize>>,
    /// The pack the stacks were built from.
    pub pack: Option<String>,
    /// `CdlEngine::LastApiId()`: the api of the last `GetFunction`.
    pub last_api: i32,
}

fn parse_const(lit: &str) -> Option<u32> {
    match lit {
        "ETrue" | "true" => Some(1),
        "EFalse" | "false" | "NULL" => Some(0),
        _ => {
            if let Some(f) = lit.strip_suffix('f') {
                return parse_f32(f).map(f32::to_bits);
            }
            if let Some(h) = lit.strip_prefix("0x") {
                return u32::from_str_radix(h, 16).ok();
            }
            lit.parse::<i64>().ok().map(|v| v as u32)
        }
    }
}

/// A decimal float literal ("6.75") without std.
fn parse_f32(s: &str) -> Option<f32> {
    let (neg, s) = match s.strip_prefix('-') { Some(r) => (true, r), None => (false, s) };
    let (int, frac) = s.split_once('.').unwrap_or((s, ""));
    let mut v = int.parse::<u32>().ok()? as f64;
    let mut scale = 0.1f64;
    for c in frac.chars() {
        v += c.to_digit(10)? as f64 * scale;
        scale /= 10.0;
    }
    Some(if neg { -(v as f32) } else { v as f32 })
}

impl CdlData {
    /// Parse the extractor's file (`examples/gen_cdl_layout.rs`).
    pub fn parse(bytes: &[u8]) -> Result<CdlData, String> {
        let text = core::str::from_utf8(bytes).map_err(|_| String::from("cdl data: not UTF-8"))?;
        let mut lines = text.lines();
        if lines.next() != Some("HEIRLOOM-CDL-LAYOUT 1") {
            return Err(String::from("cdl data: not a HEIRLOOM-CDL-LAYOUT 1 file"));
        }
        let mut d = CdlData::default();
        while let Some(line) = lines.next() {
            let mut t = line.split_whitespace();
            match t.next() {
                Some("blob") => {
                    let name = t.next().ok_or("cdl data: blob without a name")?;
                    let hex = lines.next().ok_or("cdl data: blob without data")?.trim();
                    let mut v = Vec::with_capacity(hex.len() / 2);
                    for i in (0..hex.len().saturating_sub(1)).step_by(2) {
                        v.push(u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| String::from("cdl data: bad blob hex"))?);
                    }
                    d.blobs.insert(String::from(name), v);
                }
                Some("instance") => {
                    let name = String::from(t.next().ok_or("cdl data: instance without a name")?);
                    let uid = u32::from_str_radix(t.next().ok_or("cdl data: instance without a uid")?, 16).map_err(|_| String::from("cdl data: bad uid"))?;
                    d.instances.push(Instance { name, uid, data: None, lookup: Vec::new(), ops: Vec::new() });
                }
                Some(k @ ("data" | "lookup" | "impl")) => {
                    let inst = d.instances.last_mut().ok_or("cdl data: a field before any instance")?;
                    match k {
                        "data" => {
                            let blob = String::from(t.next().ok_or("cdl data: data without a blob")?);
                            let off = t.next().and_then(|o| o.parse().ok()).ok_or("cdl data: bad data offset")?;
                            inst.data = Some((blob, off));
                        }
                        "lookup" => {
                            for v in t {
                                inst.lookup.push(u16::from_str_radix(v, 16).map_err(|_| String::from("cdl data: bad lookup"))?);
                            }
                        }
                        _ => {
                            for tok in t {
                                inst.ops.push(match tok {
                                    "-" => Op::None,
                                    _ if tok.starts_with("c:") => Op::Const(parse_const(&tok[2..]).ok_or_else(|| format!("cdl data: bad constant {tok}"))?),
                                    _ if tok.starts_with("d:") => Op::Decode(String::from(&tok[2..])),
                                    _ => Op::Opaque(String::from(tok.trim_start_matches("o:"))),
                                });
                            }
                        }
                    }
                }
                Some("interface") => {
                    let uid = u32::from_str_radix(t.next().ok_or("cdl data: interface without a uid")?, 16).map_err(|_| String::from("cdl data: bad interface uid"))?;
                    let name = String::from(t.next().ok_or("cdl data: interface without a name")?);
                    d.interfaces.insert(uid, (name, t.map(String::from).collect()));
                }
                Some("rsg") => {
                    let name = String::from(t.next().ok_or("cdl data: rsg without a name")?);
                    let v = u32::from_str_radix(t.next().ok_or("cdl data: rsg without a value")?, 16).map_err(|_| String::from("cdl data: bad rsg value"))?;
                    d.rsg.insert(name, v);
                }
                Some("pack") => {
                    let name = String::from(t.next().ok_or("cdl data: pack without a name")?);
                    let mut num = |what: &str| t.next().and_then(|v| v.parse::<i32>().ok()).ok_or_else(|| format!("cdl data: bad pack {what}"));
                    let (width, height) = (num("width")?, num("height")?);
                    let layout_id = String::from(t.next().unwrap_or(""));
                    let zoom = String::from(t.next().unwrap_or(""));
                    let _hash = t.next();
                    let priority = t.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                    let contents_line = lines.next().ok_or("cdl data: pack without contents")?;
                    let contents = contents_line.split_whitespace().skip(1).map(String::from).collect();
                    d.packs.push(Pack { name, width, height, layout_id, zoom, priority, contents });
                }
                _ => {}
            }
        }
        Ok(d)
    }

    fn instance_named(&self, name: &str) -> Option<usize> {
        self.instances.iter().position(|i| i.name == name)
    }
}

impl CdlState {
    /// The host hands in the data file; the customisation stacks are built from the pack for a `width` x `height` screen.
    /// The pack is the ELAF (Latin) one at normal zoom with the lowest priority: what a device starts with.
    pub fn load(&mut self, bytes: &[u8], width: i32, height: i32) -> Result<String, String> {
        let data = CdlData::parse(bytes)?;
        let pack = data
            .packs
            .iter()
            .filter(|p| p.width == width && p.height == height && p.layout_id == "EAknLayoutIdELAF" && p.zoom == "EAknUiZoomNormal")
            .min_by_key(|p| p.priority)
            .ok_or_else(|| format!("cdl data: no ELAF normal-zoom pack for {width}x{height}"))?
            .clone();
        let mut stacks: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
        for name in &pack.contents {
            let i = data.instance_named(name).ok_or_else(|| format!("cdl data: pack {} names an instance {name} the file lacks", pack.name))?;
            stacks.entry(data.instances[i].uid).or_default().push(i);
        }
        self.stacks = stacks;
        self.pack = Some(pack.name.clone());
        self.data = Some(data);
        Ok(pack.name)
    }

    /// `CdlEngine::GetFunction`'s walk: the top-most instance of `uid`'s stack with an implementation of `api`.
    pub fn implementor(&self, uid: u32, api: usize) -> Result<usize, String> {
        let data = self.data.as_ref().ok_or_else(|| String::from("no CDL layout data was handed to the engine (the host passes the file gen_cdl_layout wrote)"))?;
        let stack = self.stacks.get(&uid).ok_or_else(|| format!("CdlEngine panic ECdlEngPanic_CustomisationNotPresent: no customisation for interface {uid:#x}"))?;
        for &i in stack.iter().rev() {
            if data.instances[i].ops.get(api).is_some_and(|o| *o != Op::None) {
                return Ok(i);
            }
        }
        Err(format!("CdlEngine panic ECdlEngPanic_NoImplementation: interface {uid:#x} api {api}"))
    }

    /// `Layout_Meta_Data::IsMirrored()` (api 1 of 0x10204fc6), for the decoder.
    pub fn is_mirrored(&self) -> bool {
        self.implementor(META_DATA_UID, META_IS_MIRRORED).ok().and_then(|i| self.data.as_ref().map(|d| d.instances[i].ops[META_IS_MIRRORED] == Op::Const(1))).unwrap_or(false)
    }
}

/// The interface UID of "Layout Meta Data" and its `IsMirrored` api id (layoutmetadata.cdl.common.h).
pub const META_DATA_UID: u32 = 0x1020_4fc6;
pub const META_IS_MIRRORED: usize = 1;

/// The name of the stub standing for `instance`'s function `api`.
fn stub_name(instance: usize, api: usize) -> String {
    format!("cdl@{instance}:{api}")
}

pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    if let Some(rest) = name.strip_prefix("cdl@") {
        let (i, a) = rest.split_once(':')?;
        return Some(call_function(p, i.parse().ok()?, a.parse().ok()?));
    }
    Some(match name {
        // static TCdlEngineFunc* CdlEngine::GetFunction(TUid aCdlUid, TInt aApiId) (CdlEngine.cpp): the implementation from the
        // top of the stack. The guest calls what comes back, so it is an engine stub bound to the instance and api.
        "_ZN9CdlEngine11GetFunctionE4TUidi" => {
            let (uid, api) = (p.cpu.regs[0], p.cpu.regs[1] as i32);
            if api < 0 {
                return Some(NativeOutcome::Refused(format!("NEXT-NEEDED CdlEngine/55 {name}: negative api id {api}")));
            }
            let found = p.nat.cdl.implementor(uid, api as usize);
            match found.and_then(|i| p.engine_stub(&stub_name(i, api as usize)).map(|a| (i, a))) {
                Ok((_, addr)) => {
                    p.nat.cdl.last_api = api;
                    p.cpu.regs[0] = addr;
                    NativeOutcome::Return
                }
                Err(e) => NativeOutcome::Refused(format!("NEXT-NEEDED CdlEngine/55 {name}({uid:#x}, {api}): {e}")),
            }
        }
        _ => return None,
    })
}

/// A decode operation's shape: what it returns and how.
enum Kind {
    Line { table: bool, text: bool },
    ParamLimits { table: bool },
    TableLimits,
    ComponentType,
    ParamLimitsById,
    Component { text_only: bool },
}

/// `<Kind><letters>`: the parameter letters are L (table line), V (variety), C (column), R (row), I (component id), in argument order.
fn parse_op(op: &str) -> Option<(Kind, &str)> {
    for (prefix, kind) in [
        ("WindowLine", Kind::Line { table: false, text: false }),
        ("TextLine", Kind::Line { table: false, text: true }),
        ("WindowTable", Kind::Line { table: true, text: false }),
        ("TextTable", Kind::Line { table: true, text: true }),
        ("ParameterLimitsTable", Kind::ParamLimits { table: true }),
        ("ParameterLimits", Kind::ParamLimits { table: false }),
    ] {
        if let Some(letters) = op.strip_prefix(prefix) {
            return Some((kind, letters));
        }
    }
    match op {
        "TableLimits" => Some((Kind::TableLimits, "")),
        "GetComponentTypeById" => Some((Kind::ComponentType, "I")),
        "GetParamLimitsById" => Some((Kind::ParamLimitsById, "IV")),
        "GetWindowComponentById" => Some((Kind::Component { text_only: false }, "IVCR")),
        "GetTextComponentById" => Some((Kind::Component { text_only: true }, "IVCR")),
        _ => None,
    }
}

/// The parameters of a layout call, by the letters of the operation's name.
#[derive(Default, Clone, Copy)]
pub struct Args {
    pub line: i32,
    pub var: i32,
    pub col: i32,
    pub row: i32,
    pub id: i32,
}

/// What a decode operation produced: a scalar (r0) or a struct (the guest's hidden-pointer result).
pub enum Out {
    Scalar(u32),
    Struct(Vec<u8>),
}

fn ints(v: &[i32]) -> Out {
    Out::Struct(v.iter().flat_map(|x| x.to_le_bytes()).collect())
}

impl CdlState {
    /// Evaluate `instance`'s decode operation for `api` (the host-side core of every layout function).
    pub fn eval(&self, instance: usize, api: usize, a: &Args) -> Result<Out, String> {
        let data = self.data.as_ref().ok_or_else(|| String::from("no CDL layout data"))?;
        let inst = data.instances.get(instance).ok_or_else(|| String::from("no such instance"))?;
        let op = match inst.ops.get(api).unwrap_or(&Op::None) {
            Op::Const(v) => return Ok(Out::Scalar(*v)),
            Op::Decode(op) => op,
            Op::Opaque(n) => return Err(format!("{}: {n} (code the extractor does not read, or ids it cannot name)", inst.name)),
            Op::None => return Err(String::from("the instance has no implementation of this api")),
        };
        let (kind, _) = parse_op(op).ok_or_else(|| format!("decode operation {op} is not implemented"))?;
        let (blob_name, base) = inst.data.as_ref().ok_or_else(|| format!("{} has no byte-coded data", inst.name))?;
        let blob = data.blobs.get(blob_name).ok_or_else(|| format!("the file lacks blob {blob_name}"))?;
        let di = DecodeInstance { blob, base: *base, lookup: &inst.lookup, mirrored: self.is_mirrored() };
        match kind {
            Kind::Line { table, text } => di.line(api, table.then_some(a.line), text, a.var, a.col, a.row).map(|w| Out::Struct(layoutdecode::words_to_bytes(&w).to_vec())),
            Kind::ParamLimits { table } => di.limits(api, table.then_some(a.line), a.var).map(|l| ints(&l)),
            Kind::TableLimits => di.table_limits(api).map(|l| ints(&l)),
            Kind::ComponentType => di.component_type(a.id as usize).map(|t| Out::Scalar(t as u32)),
            Kind::ParamLimitsById => di.limits_by_id(a.id as usize, a.var).map(|l| ints(&l)),
            Kind::Component { text_only } => di.component(a.id as usize, text_only, a.var, a.col, a.row).map(|w| Out::Struct(layoutdecode::words_to_bytes(&w).to_vec())),
        }
    }

    /// The api id of `name` in interface `uid` (the interface's `enum TApiId`).
    pub fn api_id(&self, uid: u32, name: &str) -> Result<usize, String> {
        let data = self.data.as_ref().ok_or_else(|| String::from("no CDL layout data"))?;
        let (_, names) = data.interfaces.get(&uid).ok_or_else(|| format!("no interface {uid:#x} in the layout data"))?;
        names.iter().position(|n| n == name).ok_or_else(|| format!("interface {uid:#x} has no api {name}"))
    }

    /// A window component of an interface by api name, e.g. ("main_pane", variety 6): what `AknLayoutScalable_Avkon::main_pane(6)`
    /// returns. The 16 words of `TAknWindowComponentLayout`.
    pub fn window_layout(&self, uid: u32, name: &str, variety: i32) -> Result<layoutdecode::Words, String> {
        let api = self.api_id(uid, name)?;
        let inst = self.implementor(uid, api)?;
        match self.eval(inst, api, &Args { var: variety, ..Args::default() })? {
            Out::Struct(b) if b.len() == 32 => {
                let mut w = [0u16; 16];
                for (i, c) in b.chunks(2).enumerate() {
                    w[i] = u16::from_le_bytes([c[0], c[1]]);
                }
                Ok(w)
            }
            _ => Err(format!("{name} is not a window component")),
        }
    }
}

/// Run the function `instance`'s `api` for the guest: arguments as the C++ ABI passes them (a struct result through the hidden
/// pointer in r0, then the arguments in r1..r3 and on the stack; a scalar result in r0 and the arguments from r0).
fn call_function(p: &mut Process, instance: usize, api: usize) -> NativeOutcome {
    let fail = |m: String| NativeOutcome::Refused(format!("NEXT-NEEDED cdl@{instance}:{api}: {m}"));
    let Some(op) = p.nat.cdl.data.as_ref().and_then(|d| d.instances.get(instance)).and_then(|i| i.ops.get(api)).cloned() else { return fail(String::from("no such instance or api")) };
    let letters = match &op {
        Op::Decode(name) => match parse_op(name) {
            Some((_, l)) => String::from(l),
            None => return fail(format!("decode operation {name} is not implemented")),
        },
        _ => String::new(),
    };
    let first = if letters == "I" { 0 } else { 1 };
    let sp = p.cpu.regs[13];
    let mut a = Args::default();
    for (i, ch) in letters.chars().enumerate() {
        let slot = first + i;
        let v = if slot < 4 { Ok(p.cpu.regs[slot]) } else { p.mem.vm.read_u32(sp.wrapping_add(((slot - 4) * 4) as u32)) };
        let Ok(v) = v else { return fail(String::from("an argument is unreadable")) };
        match ch {
            'L' => a.line = v as i32,
            'V' => a.var = v as i32,
            'C' => a.col = v as i32,
            'R' => a.row = v as i32,
            'I' => a.id = v as i32,
            _ => return fail(format!("decode operation: unknown parameter letter {ch}")),
        }
    }
    match p.nat.cdl.eval(instance, api, &a) {
        Ok(Out::Scalar(v)) => {
            p.cpu.regs[0] = v;
            NativeOutcome::Return
        }
        Ok(Out::Struct(bytes)) => {
            let dst = p.cpu.regs[0];
            if p.mem.vm.write_bytes(dst, &bytes).is_err() {
                return fail(format!("the result at {dst:#x} is unwritable"));
            }
            NativeOutcome::Return
        }
        Err(e) => fail(e),
    }
}

impl Process {
    /// The host hands in the layout data file (`examples/gen_cdl_layout`); the stacks are built for the device's screen
    /// (`cone::S60_5TH_APP_RECT`). Returns the pack name.
    pub fn set_cdl_data(&mut self, bytes: &[u8]) -> Result<String, String> {
        let r = crate::cone::S60_5TH_APP_RECT;
        self.nat.cdl.load(bytes, r.width, r.height)
    }
}
