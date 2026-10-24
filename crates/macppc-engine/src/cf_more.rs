//! CoreFoundation breadth beyond [`crate::cf`]'s minimum, measured from what Warcraft III, its World Editors and BNUpdate import: string creation from UniChars, mutable
//! strings, substrings, search, splitting/joining, `CFStringGetBytes`/`GetCharacters`; `CFArray`, `CFDictionary`, `CFNumber`, file-system-path `CFURL`s; `CFGetRetainCount`, `CFShow`,
//! `CFStringGetSystemEncoding`; and the allocator constants (`kCFAllocatorDefault` ... are data: NULL means "the default allocator", which is all there is).
//! Same model as `cf.rs`: every object is an OPAQUE id in a host table with a retain count; strings are bytes kept as given (MacRoman = ASCII-exact; a UniChar above 0x7F is REFUSED
//! BY NAME rather than guessed into MacRoman -- no mapping table is in the tree). Named limits: mutable and immutable strings are not told apart (a program that mutates an
//! immutable string is not caught); `CFArray`/`CFDictionary` callbacks are ignored (values are not retained, dictionary keys are equal when both are strings with the same bytes, else
//! when the pointers are); a `CFURL` is its path string plus style, nothing else. Stay unbound (stop by name): `CFStringCreateWithFormat` (variadic, `%@`),
//! `CFPropertyListCreateFromXMLData`, `CFURLCreateDataAndPropertiesFromResource`, `CFURLCreateFromFSRef` (no FSRef); `CFBundleCreate` answers NULL (there are no bundles).
use crate::clib::arg;
use crate::machine::MacPpc;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use guestui_abi::GuestUi;
use retro_ppc::Cpu;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CfxFn {
    StringCreateWithCharacters, StringCreateWithCharactersNoCopy, StringCreateMutable, StringCreateMutableCopy, StringAppendCharacters, StringReplace,
    StringCreateWithSubstring, StringFind, StringFindWithOptions, StringGetBytes, StringGetCharacters, StringCreateArrayBySeparatingStrings, StringCreateByCombiningStrings,
    StringGetSystemEncoding, GetRetainCount, Show, ArrayCreate, ArrayGetCount, ArrayGetValueAtIndex, DictionaryCreate, DictionaryGetValue, DictionaryContainsKey, DictionaryGetCount,
    NumberCreate, NumberGetValue, UrlCreateWithFileSystemPath, UrlCreateCopyAppendingPathComponent, BundleCreate,
}

impl CfxFn {
    pub fn by_name(name: &str) -> Option<CfxFn> {
        use CfxFn::*;
        Some(match name {
            "CFStringCreateWithCharacters" => StringCreateWithCharacters, "CFStringCreateWithCharactersNoCopy" => StringCreateWithCharactersNoCopy,
            "CFStringCreateMutable" => StringCreateMutable, "CFStringCreateMutableCopy" => StringCreateMutableCopy, "CFStringAppendCharacters" => StringAppendCharacters,
            "CFStringReplace" => StringReplace, "CFStringCreateWithSubstring" => StringCreateWithSubstring, "CFStringFind" => StringFind, "CFStringFindWithOptions" => StringFindWithOptions,
            "CFStringGetBytes" => StringGetBytes, "CFStringGetCharacters" => StringGetCharacters, "CFStringCreateArrayBySeparatingStrings" => StringCreateArrayBySeparatingStrings,
            "CFStringCreateByCombiningStrings" => StringCreateByCombiningStrings, "CFStringGetSystemEncoding" => StringGetSystemEncoding, "CFGetRetainCount" => GetRetainCount, "CFShow" => Show,
            "CFArrayCreate" => ArrayCreate, "CFArrayGetCount" => ArrayGetCount, "CFArrayGetValueAtIndex" => ArrayGetValueAtIndex,
            "CFDictionaryCreate" => DictionaryCreate, "CFDictionaryGetValue" => DictionaryGetValue, "CFDictionaryContainsKey" => DictionaryContainsKey, "CFDictionaryGetCount" => DictionaryGetCount,
            "CFNumberCreate" => NumberCreate, "CFNumberGetValue" => NumberGetValue,
            "CFURLCreateWithFileSystemPath" => UrlCreateWithFileSystemPath, "CFURLCreateCopyAppendingPathComponent" => UrlCreateCopyAppendingPathComponent,
            "CFBundleCreate" => BundleCreate,
            _ => return None,
        })
    }
}

/// The allocator constants are 4-byte variables holding a CFAllocatorRef; NULL is the default allocator.
pub fn is_allocator_constant(name: &str) -> bool { matches!(name, "kCFAllocatorDefault" | "kCFAllocatorNull" | "kCFAllocatorSystemDefault" | "kCFAllocatorMalloc" | "kCFAllocatorMallocZone") }

#[derive(Clone, Copy)]
pub(crate) enum Num { Int(i64), Float(f64) }

#[derive(Default)]
pub(crate) struct CfxState {
    arrays: BTreeMap<u32, (Vec<u32>, u32)>,
    dicts: BTreeMap<u32, (Vec<(u32, u32)>, u32)>,
    numbers: BTreeMap<u32, (Num, u32)>,
    /// url -> (path bytes, path style, retain)
    urls: BTreeMap<u32, (Vec<u8>, u32, u32)>,
}

impl<U: GuestUi> MacPpc<U> {
    fn cfx_id(&mut self) -> u32 { self.rt.cf.next_id() }
    fn cfx_str(&self, r: u32, who: &str) -> Result<Vec<u8>, String> { self.rt.cf.string(r).map(<[u8]>::to_vec).ok_or_else(|| format!("{who}: {r:#x} is not a CFString this engine made")) }
    fn cfx_new_str(&mut self, b: Vec<u8>) -> u32 { self.cf_make(b) }
    /// UniChars at `p` as bytes: ASCII only (see the module doc).
    fn cfx_unichars(&self, p: u32, n: u32, who: &str) -> Result<Vec<u8>, String> {
        let mut out = Vec::with_capacity(n as usize);
        for i in 0..n {
            let c = self.mac.mem.read_u16(p.wrapping_add(2 * i)).map_err(|_| format!("{who}: the UniChars at {p:#x} leave guest RAM"))?;
            if c > 0x7F { return Err(format!("{who}: UniChar {c:#06x} is not ASCII and no MacRoman mapping table is available (refused rather than guessed)")); }
            out.push(c as u8);
        }
        Ok(out)
    }
    fn cfx_range(len: usize, loc: u32, n: u32, who: &str) -> Result<core::ops::Range<usize>, String> {
        let (l, e) = (loc as usize, loc as usize + n as usize);
        if e > len { return Err(format!("{who}: range ({loc}, {n}) is outside the {len}-character string")); }
        Ok(l..e)
    }
    fn cfx_find(hay: &[u8], needle: &[u8], fold: bool, backwards: bool) -> Option<usize> {
        if needle.is_empty() || needle.len() > hay.len() { return None; }
        let eq = |a: &[u8], b: &[u8]| if fold { a.eq_ignore_ascii_case(b) } else { a == b };
        let mut at = (0..=hay.len() - needle.len()).filter(|&i| eq(&hay[i..i + needle.len()], needle));
        if backwards { at.last() } else { at.next() }
    }

    pub(crate) fn cfx_call(&mut self, f: CfxFn, cpu: &mut Cpu) -> Result<(), String> {
        use CfxFn::*;
        let name = format!("CoreFoundation {f:?}");
        let n = name.as_str();
        let (a0, a1, a2, a3) = (arg(self, cpu, 0)?, arg(self, cpu, 1).unwrap_or(0), arg(self, cpu, 2).unwrap_or(0), arg(self, cpu, 3).unwrap_or(0));
        let mut r2: Option<u32> = None; // r4, for CFRange results
        let r: u32 = match f {
            StringCreateWithCharacters | StringCreateWithCharactersNoCopy => { let b = self.cfx_unichars(a1, a2, n)?; self.cfx_new_str(b) }
            StringCreateMutable => self.cfx_new_str(Vec::new()),
            StringCreateMutableCopy => { let b = self.cfx_str(a2, n)?; self.cfx_new_str(b) }
            StringAppendCharacters => { let add = self.cfx_unichars(a1, a2, n)?; self.rt.cf.string_mut(a0).ok_or_else(|| format!("{n}: {a0:#x} is not a CFString this engine made"))?.extend(add); 0 }
            // CFStringReplace(str, CFRange range, replacement)
            StringReplace => {
                let rep = self.cfx_str(a3, n)?;
                let s = self.rt.cf.string_mut(a0).ok_or_else(|| format!("{n}: {a0:#x} is not a CFString this engine made"))?;
                let rg = Self::cfx_range(s.len(), a1, a2, n)?;
                s.splice(rg, rep);
                0
            }
            StringCreateWithSubstring => { let s = self.cfx_str(a1, n)?; let rg = Self::cfx_range(s.len(), a2, a3, n)?; self.cfx_new_str(s[rg].to_vec()) }
            // CFStringFind(str, stringToFind, compareOptions) -> CFRange (r3, r4); {-1... } no: kCFNotFound = -1 with length 0
            StringFind => {
                let (h, nd) = (self.cfx_str(a0, n)?, self.cfx_str(a1, n)?);
                match Self::cfx_find(&h, &nd, a2 & 1 != 0, a2 & 4 != 0) { Some(i) => { r2 = Some(nd.len() as u32); i as u32 } None => { r2 = Some(0); (-1i32) as u32 } }
            }
            // CFStringFindWithOptions(str, find, CFRange search, options, CFRange *result) -> Boolean
            StringFindWithOptions => {
                let (opts, out) = (arg(self, cpu, 4)?, arg(self, cpu, 5)?);
                let (h, nd) = (self.cfx_str(a0, n)?, self.cfx_str(a1, n)?);
                let rg = Self::cfx_range(h.len(), a2, a3, n)?;
                match Self::cfx_find(&h[rg.clone()], &nd, opts & 1 != 0, opts & 4 != 0) {
                    Some(i) => { if out != 0 { self.mac.mem.write_u32(out, (rg.start + i) as u32).and_then(|_| self.mac.mem.write_u32(out + 4, nd.len() as u32)).map_err(|_| format!("{n}: the result range leaves guest RAM"))?; } 1 }
                    None => 0,
                }
            }
            // CFStringGetBytes(str, range, encoding, lossByte, isExternalRepresentation, buffer, maxBufLen, CFIndex *usedBufLen) -> characters converted
            // (spec: macos9-specs/CoreFoundationLib/CFStringGetBytes). The strings here are 8-bit MacRoman, the system default encoding, so the fast path applies: the bytes are copied directly,
            // count = min(range length, maxBufLen); a NULL buffer copies nothing and reports the count that WOULD convert (maxBufLen ignored). ASCII, ISO Latin-1 and UTF-8 requests are
            // the same bytes for ASCII text; non-ASCII text in those encodings (a real conversion) and every other encoding are refused by name. isExternalRepresentation (a BOM/header) is
            // only meaningful for the Unicode encodings, which are refused.
            StringGetBytes => {
                let (loc, len, enc, buf, max, used) = (a1, a2, a3, arg(self, cpu, 6)?, arg(self, cpu, 7)?, arg(self, cpu, 8)?);
                let s = self.cfx_str(a0, n)?;
                let rg = Self::cfx_range(s.len(), loc, len, n)?;
                let ascii_only = s[rg.clone()].iter().all(|&b| b < 0x80);
                let ok = match enc { 0 => true, 0x0600 | 0x0201 | 0x0800_0100 => ascii_only, _ => false };
                if !ok { return Err(format!("{n}: encoding {enc:#x} for this text is not implemented (MacRoman 0, and ASCII 0x600 / ISO Latin-1 0x201 / UTF-8 0x8000100 for ASCII text only)")); }
                let take = if buf == 0 { rg.len() } else { rg.len().min(max as usize) };
                if buf != 0 { let b = s[rg.start..rg.start + take].to_vec(); self.put_bytes(buf, &b, n)?; }
                if used != 0 { self.mac.mem.write_u32(used, take as u32).map_err(|_| format!("{n}: usedBufLen leaves guest RAM"))?; }
                take as u32
            }
            // CFStringGetCharacters(str, CFRange, UniChar *buffer)
            StringGetCharacters => {
                let s = self.cfx_str(a0, n)?;
                let rg = Self::cfx_range(s.len(), a1, a2, n)?;
                for (i, b) in s[rg].iter().enumerate() {
                    if *b > 0x7F { return Err(format!("{n}: byte {b:#04x} is MacRoman text this engine cannot widen to UTF-16 (no mapping table)")); }
                    self.mac.mem.write_u16(a3.wrapping_add(2 * i as u32), u16::from(*b)).map_err(|_| format!("{n}: the buffer at {a3:#x} leaves guest RAM"))?;
                }
                0
            }
            StringCreateArrayBySeparatingStrings => {
                let (s, sep) = (self.cfx_str(a1, n)?, self.cfx_str(a2, n)?);
                let mut parts = Vec::new();
                if sep.is_empty() { parts.push(s); } else {
                    let (mut start, mut i) = (0, 0);
                    while i + sep.len() <= s.len() { if s[i..i + sep.len()] == sep[..] { parts.push(s[start..i].to_vec()); i += sep.len(); start = i; } else { i += 1; } }
                    parts.push(s[start..].to_vec());
                }
                let items: Vec<u32> = parts.into_iter().map(|p| self.cfx_new_str(p)).collect();
                let id = self.cfx_id();
                self.rt.cfx.arrays.insert(id, (items, 1));
                id
            }
            StringCreateByCombiningStrings => {
                let items = self.rt.cfx.arrays.get(&a1).ok_or_else(|| format!("{n}: {a1:#x} is not a CFArray this engine made"))?.0.clone();
                let sep = self.cfx_str(a2, n)?;
                let mut out = Vec::new();
                for (i, it) in items.iter().enumerate() { if i > 0 { out.extend_from_slice(&sep); } out.extend(self.cfx_str(*it, n)?); }
                self.cfx_new_str(out)
            }
            // spec: macos9-specs/CoreFoundationLib/CFStringGetSystemEncoding: the Text Encoding base of the system script/region (GetScriptManagerVariable(smRegionCode = 40) = 2, Britain: a
            // Roman region) is kCFStringEncodingMacRoman, 0; cached for the process, so a later script change is not noticed
            StringGetSystemEncoding => 0,
            GetRetainCount => self.cfx_retain_count(a0).ok_or_else(|| format!("{n}: {a0:#x} is not a CoreFoundation object this engine made"))?,
            Show => {
                let text = match self.rt.cf.string(a0) { Some(s) => String::from_utf8_lossy(s).into_owned(), None => format!("<CF object {a0:#x}>") };
                self.mac.log.push(format!("[cf] CFShow: {text}"));
                self.console.extend_from_slice(text.as_bytes()); self.console.push(b'\n');
                0
            }
            // CFArrayCreate(alloc, const void **values, CFIndex n, callbacks)
            ArrayCreate => {
                let mut v = Vec::new();
                for i in 0..a2 { v.push(self.mac.mem.read_u32(a1.wrapping_add(4 * i)).map_err(|_| format!("{n}: the values at {a1:#x} leave guest RAM"))?); }
                let id = self.cfx_id();
                self.rt.cfx.arrays.insert(id, (v, 1));
                id
            }
            ArrayGetCount => self.rt.cfx.arrays.get(&a0).ok_or_else(|| format!("{n}: {a0:#x} is not a CFArray this engine made"))?.0.len() as u32,
            ArrayGetValueAtIndex => {
                let a = &self.rt.cfx.arrays.get(&a0).ok_or_else(|| format!("{n}: {a0:#x} is not a CFArray this engine made"))?.0;
                *a.get(a1 as usize).ok_or_else(|| format!("{n}: index {a1} is past the end of the {}-element array (undefined in CoreFoundation, refused here)", a.len()))?
            }
            // CFDictionaryCreate(alloc, keys, values, n, keyCallbacks, valueCallbacks)
            DictionaryCreate => {
                let mut v = Vec::new();
                for i in 0..a3 {
                    let rd = |p: u32| self.mac.mem.read_u32(p.wrapping_add(4 * i)).map_err(|_| format!("{n}: the keys/values leave guest RAM"));
                    v.push((rd(a1)?, rd(a2)?));
                }
                let id = self.cfx_id();
                self.rt.cfx.dicts.insert(id, (v, 1));
                id
            }
            DictionaryGetValue | DictionaryContainsKey => {
                let d = self.rt.cfx.dicts.get(&a0).ok_or_else(|| format!("{n}: {a0:#x} is not a CFDictionary this engine made"))?;
                let hit = d.0.iter().find(|(k, _)| *k == a1 || matches!((self.rt.cf.string(*k), self.rt.cf.string(a1)), (Some(x), Some(y)) if x == y));
                if f == DictionaryGetValue { hit.map_or(0, |(_, v)| *v) } else { u32::from(hit.is_some()) }
            }
            DictionaryGetCount => self.rt.cfx.dicts.get(&a0).ok_or_else(|| format!("{n}: {a0:#x} is not a CFDictionary this engine made"))?.0.len() as u32,
            // CFNumberCreate(alloc, CFNumberType, const void *valuePtr); types: 1..4 SInt8/16/32/64, 5/6 Float32/64, 7 Char, 8 Short, 9 Int, 10 Long, 11 LongLong, 12 Float, 13 Double
            NumberCreate => {
                let num = self.cfx_read_number(a2, a1, n)?;
                let id = self.cfx_id();
                self.rt.cfx.numbers.insert(id, (num, 1));
                id
            }
            // CFNumberGetValue(num, type, valuePtr) -> Boolean (TRUE when the value was stored without loss)
            NumberGetValue => {
                let (num, _) = *self.rt.cfx.numbers.get(&a0).ok_or_else(|| format!("{n}: {a0:#x} is not a CFNumber this engine made"))?;
                self.cfx_write_number(num, a1, a2, n)?
            }
            // CFURLCreateWithFileSystemPath(alloc, filePath, CFURLPathStyle, isDirectory)
            UrlCreateWithFileSystemPath => {
                let p = self.cfx_str(a1, n)?;
                let id = self.cfx_id();
                self.rt.cfx.urls.insert(id, (p, a2, 1));
                id
            }
            // CFURLCreateCopyAppendingPathComponent(alloc, url, pathComponent, isDirectory)
            UrlCreateCopyAppendingPathComponent => {
                let (mut p, style, _) = self.rt.cfx.urls.get(&a1).cloned().ok_or_else(|| format!("{n}: {a1:#x} is not a CFURL this engine made"))?;
                let comp = self.cfx_str(a2, n)?;
                let sep = if style == 1 { b':' } else if style == 2 { b'\\' } else { b'/' };
                if p.last() != Some(&sep) { p.push(sep); }
                p.extend(comp);
                let id = self.cfx_id();
                self.rt.cfx.urls.insert(id, (p, style, 1));
                id
            }
            // there are no bundles: CFBundleCreate's documented failure is NULL
            BundleCreate => 0,
        };
        cpu.gpr[3] = r;
        if let Some(v) = r2 { cpu.gpr[4] = v; }
        let _ = (a2, a3);
        Ok(())
    }

    /// The retain count of any CF object this engine made.
    pub(crate) fn cfx_retain_count(&self, id: u32) -> Option<u32> {
        self.rt.cf.retain_count(id)
            .or_else(|| self.rt.cfx.arrays.get(&id).map(|a| a.1)).or_else(|| self.rt.cfx.dicts.get(&id).map(|a| a.1))
            .or_else(|| self.rt.cfx.numbers.get(&id).map(|a| a.1)).or_else(|| self.rt.cfx.urls.get(&id).map(|a| a.2))
    }
    /// `CFRetain` / `CFRelease` for the non-string objects; `None` when `id` is none of them.
    pub(crate) fn cfx_adjust(&mut self, id: u32, delta: i32) -> Option<()> {
        let c = if let Some(a) = self.rt.cfx.arrays.get_mut(&id) { &mut a.1 } else if let Some(a) = self.rt.cfx.dicts.get_mut(&id) { &mut a.1 }
            else if let Some(a) = self.rt.cfx.numbers.get_mut(&id) { &mut a.1 } else if let Some(a) = self.rt.cfx.urls.get_mut(&id) { &mut a.2 } else { return None };
        *c = (*c as i32 + delta).max(0) as u32;
        if *c == 0 { self.rt.cfx.arrays.remove(&id); self.rt.cfx.dicts.remove(&id); self.rt.cfx.numbers.remove(&id); self.rt.cfx.urls.remove(&id); }
        Some(())
    }

    fn cfx_read_number(&self, p: u32, ty: u32, who: &str) -> Result<Num, String> {
        let m = &self.mac.mem;
        let e = |_| format!("{who}: the value at {p:#x} leaves guest RAM");
        Ok(match ty {
            1 | 7 => Num::Int(i64::from(m.read_u8(p).map_err(e)? as i8)),
            2 | 8 => Num::Int(i64::from(m.read_u16(p).map_err(e)? as i16)),
            3 | 9 | 10 => Num::Int(i64::from(m.read_u32(p).map_err(e)? as i32)),
            4 | 11 => Num::Int((u64::from(m.read_u32(p).map_err(e)?) << 32 | u64::from(m.read_u32(p + 4).map_err(e)?)) as i64),
            5 | 12 => Num::Float(f64::from(f32::from_bits(m.read_u32(p).map_err(e)?))),
            6 | 13 => Num::Float(f64::from_bits(u64::from(m.read_u32(p).map_err(e)?) << 32 | u64::from(m.read_u32(p + 4).map_err(e)?))),
            t => return Err(format!("{who}: CFNumberType {t} is not implemented")),
        })
    }
    fn cfx_write_number(&mut self, num: Num, ty: u32, p: u32, who: &str) -> Result<u32, String> {
        let e = |_| format!("{who}: the value at {p:#x} leaves guest RAM");
        let m = &mut self.mac.mem;
        let (lossless, ()) = match (ty, num) {
            (1 | 7, Num::Int(v)) => (i8::try_from(v).is_ok(), m.write_u8(p, v as u8).map_err(e)?),
            (2 | 8, Num::Int(v)) => (i16::try_from(v).is_ok(), m.write_u16(p, v as u16).map_err(e)?),
            (3 | 9 | 10, Num::Int(v)) => (i32::try_from(v).is_ok(), m.write_u32(p, v as u32).map_err(e)?),
            (4 | 11, Num::Int(v)) => (true, { m.write_u32(p, (v >> 32) as u32).map_err(e)?; m.write_u32(p + 4, v as u32).map_err(e)? }),
            (5 | 12, Num::Float(v)) => (f64::from(v as f32) == v, m.write_u32(p, (v as f32).to_bits()).map_err(e)?),
            (6 | 13, Num::Float(v)) => (true, { let b = v.to_bits(); m.write_u32(p, (b >> 32) as u32).map_err(e)?; m.write_u32(p + 4, b as u32).map_err(e)? }),
            (3 | 9 | 10, Num::Float(v)) => (f64::from(v as i32) == v, m.write_u32(p, v as i32 as u32).map_err(e)?),
            (5 | 6 | 12 | 13, Num::Int(v)) => { let f = v as f64; return self.cfx_write_number(Num::Float(f), ty, p, who).map(|ok| ok & u32::from(f as i64 == v)); }
            (t, _) => return Err(format!("{who}: converting to CFNumberType {t} is not implemented")),
        };
        Ok(u32::from(lossless))
    }
}
