//! The CoreFoundation minimum a Carbon program needs to name a window: `CFString`s. A CFStringRef is an OPAQUE id (never dereferenced by a
//! program) into a host table of byte strings with a retain count. Bytes are kept as given -- MacRoman for `kCFStringEncodingMacRoman` (0) -- with no
//! transcoding, which is exact for ASCII and for MacRoman text; a UTF-8 source keeps its UTF-8 bytes (so a non-ASCII title would show as MacRoman bytes:
//! stated, not hidden).
//! `CFSTR("...")` compiles to a call of `__CFStringMakeConstantString`; equal constants share one object and are never freed (as in CoreFoundation).
use crate::clib::arg;
use crate::machine::MacPpc;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use guestui_abi::GuestUi;
use retro_ppc::Cpu;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CfFn { MakeConstantString, CreateWithCString, CreateWithPascalString, Retain, Release, GetLength, GetCString, GetPascalString, GetCStringPtr, GetCharacterAtIndex, NoBundle, CreateWithFormat }

impl CfFn {
    pub fn by_name(name: &str) -> Option<CfFn> {
        use CfFn::*;
        Some(match name {
            "__CFStringMakeConstantString" => MakeConstantString, "CFStringCreateWithCString" => CreateWithCString, "CFStringCreateWithPascalString" => CreateWithPascalString,
            "CFRetain" => Retain, "CFRelease" => Release, "CFStringGetLength" => GetLength, "CFStringGetCString" => GetCString, "CFStringGetPascalString" => GetPascalString,
            // There are no bundles on this machine (no Mac OS X, no frameworks on disk): the lookups answer NULL, which is their documented "not found" answer
            "CFBundleGetBundleWithIdentifier" | "CFBundleGetMainBundle" | "CFBundleGetIdentifier" | "CFBundleGetFunctionPointerForName" | "CFBundleGetFunction" => NoBundle,
            "CFStringCreateWithFormat" => CreateWithFormat, "CFStringGetCStringPtr" => GetCStringPtr, "CFStringGetCharacterAtIndex" => GetCharacterAtIndex,
            _ => return None,
        })
    }
}

#[derive(Default)]
pub(crate) struct CfState { strings: BTreeMap<u32, (Vec<u8>, u32, bool)>, constants: BTreeMap<Vec<u8>, u32>, next: u32 }

impl CfState {
    /// A fresh object id (shared by strings, arrays, dictionaries, numbers and URLs: one id space).
    pub(crate) fn next_id(&mut self) -> u32 { self.next += 1; CF_BASE + 8 * self.next }
    pub(crate) fn string(&self, r: u32) -> Option<&[u8]> { self.strings.get(&r).map(|s| s.0.as_slice()) }
    pub(crate) fn string_mut(&mut self, r: u32) -> Option<&mut Vec<u8>> { self.strings.get_mut(&r).filter(|s| !s.2).map(|s| &mut s.0) }
    pub(crate) fn retain_count(&self, r: u32) -> Option<u32> { self.strings.get(&r).map(|s| s.1) }
}

const CF_BASE: u32 = 0x00CF_0000;

impl<U: GuestUi> MacPpc<U> {
    pub(crate) fn cf_make(&mut self, bytes: Vec<u8>) -> u32 { self.cf_new(bytes, false) }
    fn cf_new(&mut self, bytes: Vec<u8>, constant: bool) -> u32 {
        let id = self.rt.cf.next_id();
        self.rt.cf.strings.insert(id, (bytes, 1, constant));
        id
    }
    /// The bytes of a CFString (for the engine's own use: window titles).
    pub fn cf_bytes(&self, r: u32) -> Option<&[u8]> { self.rt.cf.strings.get(&r).map(|s| s.0.as_slice()) }

    pub(crate) fn cf_call(&mut self, f: CfFn, cpu: &mut Cpu) -> Result<(), String> {
        use CfFn::*;
        let (a0, a1, a2, a3) = (arg(self, cpu, 0)?, arg(self, cpu, 1).unwrap_or(0), arg(self, cpu, 2).unwrap_or(0), arg(self, cpu, 3).unwrap_or(0));
        let bad = |who: &str, r: u32| format!("{who}: {r:#x} is not a CFString this engine made");
        let r: u32 = match f {
            MakeConstantString => {
                let s = self.cstr(a0, "__CFStringMakeConstantString")?;
                match self.rt.cf.constants.get(&s) { Some(&id) => id, None => { let id = self.cf_new(s.clone(), true); self.rt.cf.constants.insert(s, id); id } }
            }
            CreateWithCString => { let s = self.cstr(a1, "CFStringCreateWithCString")?; self.cf_new(s, false) }
            CreateWithPascalString => {
                let n = self.mac.mem.read_u8(a1).map_err(|_| String::from("CFStringCreateWithPascalString: the string leaves guest RAM"))?;
                let s = self.get_bytes(a1 + 1, u32::from(n), "CFStringCreateWithPascalString")?;
                self.cf_new(s, false)
            }
            // CFStringCreateWithFormat(alloc, formatOptions, CFStringRef format, ...) (spec: macos9-specs/CoreFoundationLib/CFStringCreateWithFormat): the printf engine over the variadic
            // arguments (integers from r6.., doubles from the FPRs and the parameter area), plus %@ for CFStrings; formatOptions is ignored. A NULL or non-string format gives NULL.
            CreateWithFormat => match self.rt.cf.string(a2).map(<[u8]>::to_vec) {
                None => 0,
                Some(fmt) => {
                    let tmp = self.mac.mm.new_ptr(&mut self.mac.mem, &mut self.mac.heap, fmt.len() as u32 + 1, true).map_err(|_| String::from("CFStringCreateWithFormat: out of memory"))?;
                    let mut b = fmt; b.push(0);
                    self.mac.mem.write_bytes(tmp, &b).map_err(|_| String::from("CFStringCreateWithFormat: format buffer"))?;
                    let out = self.format_c(cpu, 2, tmp);
                    let _ = self.mac.mm.dispose_ptr(&mut self.mac.heap, tmp);
                    let text = out?;
                    self.cf_new(text, false)
                }
            },
            Retain if self.cfx_adjust(a0, 1).is_some() => a0,
            Release if self.cfx_adjust(a0, -1).is_some() => 0,
            Retain => { let e = self.rt.cf.strings.get_mut(&a0).ok_or_else(|| bad("CFRetain", a0))?; e.1 += 1; a0 }
            Release => {
                let e = self.rt.cf.strings.get_mut(&a0).ok_or_else(|| bad("CFRelease", a0))?;
                if !e.2 { e.1 -= 1; if e.1 == 0 { self.rt.cf.strings.remove(&a0); } }
                0
            }
            GetLength => self.rt.cf.strings.get(&a0).ok_or_else(|| bad("CFStringGetLength", a0))?.0.len() as u32,
            GetCString => {
                // CFStringGetCString(str, buffer, bufferSize, encoding) -> Boolean: false when the text and its NUL do not fit
                let s = self.rt.cf.strings.get(&a0).ok_or_else(|| bad("CFStringGetCString", a0))?.0.clone();
                if (s.len() as u64) + 1 > u64::from(a2) { 0 } else { let mut b = s; b.push(0); self.put_bytes(a1, &b, "CFStringGetCString")?; 1 }
            }
            GetPascalString => {
                let s = self.rt.cf.strings.get(&a0).ok_or_else(|| bad("CFStringGetPascalString", a0))?.0.clone();
                if s.len() > 255 || (s.len() as u64) + 1 > u64::from(a2) { 0 } else { let mut b = alloc::vec![s.len() as u8]; b.extend(s); self.put_bytes(a1, &b, "CFStringGetPascalString")?; 1 }
            }
            // "may return NULL": the internal storage is not exposed
            GetCStringPtr => { self.rt.cf.strings.get(&a0).ok_or_else(|| bad("CFStringGetCStringPtr", a0))?; 0 }
            NoBundle => 0,
            GetCharacterAtIndex => {
                let s = &self.rt.cf.strings.get(&a0).ok_or_else(|| bad("CFStringGetCharacterAtIndex", a0))?.0;
                u32::from(*s.get(a1 as usize).ok_or_else(|| format!("CFStringGetCharacterAtIndex: index {a1} is past the end ({} characters)", s.len()))?)
            }
        };
        let _ = (a2, a3);
        cpu.gpr[3] = r;
        Ok(())
    }
}
