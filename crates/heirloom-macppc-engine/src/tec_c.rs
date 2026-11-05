//! The Text Encoding Converter as NATIVE libraries: `TextCommon` (TextEncoding words, `UpgradeScriptInfoToTextEncoding`), `TextEncodingConverter`
//! (`TECCreateConverter` / `TECConvertText` / sniffers) and `UnicodeConverter` (`ConvertFromTextToUnicode` / `ConvertFromUnicodeToText` ...), the calls
//! Photoshop 7, its ACELib and its MSL import (measured from their import lists), over the shared model `heirloom_mac68k_engine::textenc` and the one Mac
//! Roman table `heirloom_mac68k_engine::macroman` (Unicode Consortium ROMAN.TXT). Signatures and constants: Apple's public headers (TextCommon.h,
//! TextEncodingConverter.h, UnicodeConverter.h, MacErrors.h).
//! * What converts: Mac Roman (euro-sign and currency-sign variants), Unicode UTF-16 (big-endian, the guest's own order) and UTF-8, ISO Latin-1,
//!   US-ASCII. A converter, Unicode info or sniffer for any other encoding is kTECNoConversionPathErr (-8749), returned to the program and logged by
//!   name (`[tec] ...`): the program learns the truth, nothing is converted through a guessed table.
//! * Converter objects (`TECObjectRef`, `TextToUnicodeInfo`, `UnicodeToTextInfo`, `TECSnifferObjectRef`) are OPAQUE ids in a growable host table; a
//!   disposed or foreign id is kTECCorruptConverterErr (-8748) / paramErr.
//! * Control flags honoured: kUnicodeUseFallbacksMask (Unicode -> text: a character the encoding lacks becomes `?` and the status is
//!   kTECUsedFallbacksStatus; without it the call stops there with kTECUnmappableElementErr), kUnicodeStringUnterminatedMask (input ending inside a
//!   character is not kTECPartialCharErr: the bytes are left unread). Named, not applied: Apple's own fallback table (its exact substitutes are not
//!   checked against 9.2.1: `?` is ours), loose mappings (kUnicodeLooseMappingsMask), directionality / vertical forms (no bidi or CJK text here),
//!   kUnicodeKeepInfoMask (no state is kept between calls: every Mac Roman character is one byte, so nothing is ever pending for Mac Roman input).
//! * `UpgradeScriptInfoToTextEncoding`: a script code 0..=32 is the encoding base of the same number (TextCommon.h); smSystemScript / smCurrentScript and
//!   kTextScriptDontCare resolve to this system's script, Roman; under Roman the languages with their own Mac encodings give them (Icelandic 15 and
//!   Faroese 30 -> MacIcelandic, Turkish 17 -> MacTurkish, Croatian 18 -> MacCroatian, Romanian 37 -> MacRomanian; the Faroese alias is read from 9.2.1's
//!   TextCommon); the font names Symbol and Zapf Dingbats give MacSymbol / MacDingbats. A script above 32 is paramErr. Named limits: the region
//!   argument is not consulted (9.2.1 reads it from a table not reproduced here), a language given with kTextScriptDontCare is refused by name (no
//!   verified language -> script table).
use crate::clib::arg;
use crate::cpu::Cpu;
use crate::machine::MacPpc;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_mac68k_engine::textenc::{self, Charset, TextEncoding};
use heirloom_ui_abi::GuestUi;

pub const TEXT_COMMON: &str = "TextCommon";
pub const TEC: &str = "TextEncodingConverter";
pub const UNICODE_CONVERTER: &str = "UnicodeConverter";
/// The libraries this module provides.
pub fn provided(l: &str) -> bool { l == TEXT_COMMON || l == TEC || l == UNICODE_CONVERTER }

const PARAM_ERR: i32 = -50;
const SCRIPT_DONT_CARE: i16 = -128;
const USE_FALLBACKS: u32 = 1 << 0;
const STRING_UNTERMINATED: u32 = 1 << 6;
/// The first opaque object id (ids never repeat: a stale reference cannot name a newer object).
const FIRST_ID: u32 = 0x5445_4301;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TecFn {
    CreateTextEncoding, GetTextEncodingBase, GetTextEncodingVariant, GetTextEncodingFormat, ResolveDefaultTextEncoding, UpgradeScriptInfoToTextEncoding,
    TECCountAvailableTextEncodings, TECGetAvailableTextEncodings, TECCreateConverter, TECDisposeConverter, TECClearConverterContextInfo, TECConvertText, TECFlushText,
    TECCountAvailableSniffers, TECGetAvailableSniffers, TECCreateSniffer, TECDisposeSniffer, TECClearSnifferContextInfo, TECSniffTextEncoding,
    CreateTextToUnicodeInfo, CreateTextToUnicodeInfoByEncoding, CreateUnicodeToTextInfo, CreateUnicodeToTextInfoByEncoding, DisposeTextToUnicodeInfo,
    DisposeUnicodeToTextInfo, ConvertFromTextToUnicode, ConvertFromUnicodeToText, ConvertFromUnicodeToPString,
}

impl TecFn {
    /// `name` as library `lib` exports it (each call belongs to exactly one of the three libraries).
    pub fn by_name(lib: &str, name: &str) -> Option<TecFn> {
        use TecFn::*;
        let f = match name {
            "CreateTextEncoding" => CreateTextEncoding, "GetTextEncodingBase" => GetTextEncodingBase, "GetTextEncodingVariant" => GetTextEncodingVariant,
            "GetTextEncodingFormat" => GetTextEncodingFormat, "ResolveDefaultTextEncoding" => ResolveDefaultTextEncoding,
            "UpgradeScriptInfoToTextEncoding" => UpgradeScriptInfoToTextEncoding,
            "TECCountAvailableTextEncodings" => TECCountAvailableTextEncodings, "TECGetAvailableTextEncodings" => TECGetAvailableTextEncodings,
            "TECCreateConverter" => TECCreateConverter, "TECDisposeConverter" => TECDisposeConverter, "TECClearConverterContextInfo" => TECClearConverterContextInfo,
            "TECConvertText" => TECConvertText, "TECFlushText" => TECFlushText, "TECCountAvailableSniffers" => TECCountAvailableSniffers,
            "TECGetAvailableSniffers" => TECGetAvailableSniffers, "TECCreateSniffer" => TECCreateSniffer, "TECDisposeSniffer" => TECDisposeSniffer,
            "TECClearSnifferContextInfo" => TECClearSnifferContextInfo, "TECSniffTextEncoding" => TECSniffTextEncoding,
            "CreateTextToUnicodeInfo" => CreateTextToUnicodeInfo, "CreateTextToUnicodeInfoByEncoding" => CreateTextToUnicodeInfoByEncoding,
            "CreateUnicodeToTextInfo" => CreateUnicodeToTextInfo, "CreateUnicodeToTextInfoByEncoding" => CreateUnicodeToTextInfoByEncoding,
            "DisposeTextToUnicodeInfo" => DisposeTextToUnicodeInfo, "DisposeUnicodeToTextInfo" => DisposeUnicodeToTextInfo,
            "ConvertFromTextToUnicode" => ConvertFromTextToUnicode, "ConvertFromUnicodeToText" => ConvertFromUnicodeToText,
            "ConvertFromUnicodeToPString" => ConvertFromUnicodeToPString,
            _ => return None,
        };
        let home = match f {
            CreateTextEncoding | GetTextEncodingBase | GetTextEncodingVariant | GetTextEncodingFormat | ResolveDefaultTextEncoding | UpgradeScriptInfoToTextEncoding => TEXT_COMMON,
            CreateTextToUnicodeInfo | CreateTextToUnicodeInfoByEncoding | CreateUnicodeToTextInfo | CreateUnicodeToTextInfoByEncoding | DisposeTextToUnicodeInfo
            | DisposeUnicodeToTextInfo | ConvertFromTextToUnicode | ConvertFromUnicodeToText | ConvertFromUnicodeToPString => UNICODE_CONVERTER,
            _ => TEC,
        };
        (home == lib).then_some(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Obj {
    Converter { from: Charset, to: Charset },
    Sniffer { encodings: Vec<TextEncoding> },
    /// A Unicode Converter info: `text` is the non-Unicode side, `unicode` the Unicode form; `to_unicode` its direction.
    Info { text: Charset, unicode: Charset, to_unicode: bool },
}

#[derive(Default)]
pub struct TecState { next: u32, objs: BTreeMap<u32, Obj> }

impl TecState {
    fn add(&mut self, o: Obj) -> u32 {
        if self.next == 0 { self.next = FIRST_ID; }
        let id = self.next;
        self.next += 1;
        self.objs.insert(id, o);
        id
    }
}

impl<U: GuestUi> MacPpc<U> {
    fn tec_put(&mut self, at: u32, v: u32, who: &str) -> Result<(), String> {
        if at == 0 { return Ok(()); }
        self.mac.mem.write_u32(at, v).map_err(|_| format!("{who}: the result pointer {at:#x} leaves guest RAM"))
    }

    fn tec_log(&mut self, s: String) { self.mac.log.push(format!("[tec] {s}")); }

    /// A new object for the two encodings, written to `*out`; kTECNoConversionPathErr (named in the log) when either has no table here.
    fn tec_create(&mut self, who: &str, out: u32, a: TextEncoding, b: TextEncoding, make: impl Fn(Charset, Charset) -> Obj) -> Result<i32, String> {
        match (textenc::charset(a), textenc::charset(b)) {
            (Some(x), Some(y)) => { let id = self.rt_tec().add(make(x, y)); self.tec_put(out, id, who)?; self.tec_log(format!("{who}({a:#x}, {b:#x}) -> {id:#x}")); Ok(0) }
            _ => {
                self.tec_log(format!("{who}({a:#x}, {b:#x}) -> kTECNoConversionPathErr: this machine converts Mac Roman, Unicode (UTF-16/UTF-8), ISO Latin-1 and ASCII only"));
                self.tec_put(out, 0, who)?;
                Ok(i32::from(textenc::NO_CONVERSION_PATH))
            }
        }
    }

    fn rt_tec(&mut self) -> &mut TecState { &mut self.tec }

    /// The `UnicodeMapping` at `p`: (unicodeEncoding, otherEncoding).
    fn tec_mapping(&self, p: u32, who: &str) -> Result<(TextEncoding, TextEncoding), String> {
        let m = |e| format!("{who}: the UnicodeMapping at {p:#x} leaves guest RAM ({e})");
        Ok((self.mac.mem.read_u32(p).map_err(m)?, self.mac.mem.read_u32(p + 4).map_err(m)?))
    }

    /// Write `list` to the array at `arr` (room for `max`), the count written to `*actual`; kTECArrayFullErr when it did not all fit.
    fn tec_list(&mut self, list: &[u32], arr: u32, max: u32, actual: u32, who: &str) -> Result<i32, String> {
        let n = list.len().min(max as usize);
        for (i, &e) in list[..n].iter().enumerate() { self.tec_put(arr + 4 * i as u32, e, who)?; }
        self.tec_put(actual, n as u32, who)?;
        Ok(if n < list.len() { i32::from(textenc::ARRAY_FULL) } else { 0 })
    }

    pub(crate) fn tec_call(&mut self, f: TecFn, cpu: &mut Cpu) -> Result<(), String> {
        use TecFn::*;
        let a: Vec<u32> = (0..12).map(|i| arg(self, cpu, i).unwrap_or(0)).collect();
        let who = format!("{f:?}");
        let r: i32 = match f {
            CreateTextEncoding => textenc::create(a[0], a[1], a[2]) as i32,
            GetTextEncodingBase => textenc::base(a[0]) as i32,
            GetTextEncodingVariant => textenc::variant(a[0]) as i32,
            GetTextEncodingFormat => textenc::format(a[0]) as i32,
            ResolveDefaultTextEncoding => textenc::resolve_default(a[0], textenc::SYSTEM_VERSION) as i32,
            UpgradeScriptInfoToTextEncoding => {
                let (script, lang, font, out) = (a[0] as u16 as i16, a[1] as u16 as i16, a[3], a[4]);
                let font_name = if font == 0 { None } else { Some(heirloom_mac68k_engine::pascal::read_pstring_bytes(&self.mac.mem, font).map_err(|e| format!("{who}: {}", e.0))?) };
                let r = upgrade_script_info(script, lang, font_name.as_deref());
                self.tec_log(format!("UpgradeScriptInfoToTextEncoding(script {script}, language {lang}, region {}, font {:?}) -> {r:?}", a[2] as u16 as i16,
                    font_name.as_deref().map(heirloom_mac68k_engine::macroman::decode)));
                match r { Ok(e) => { self.tec_put(out, e, &who)?; 0 } Err(Upgrade::Param) => PARAM_ERR, Err(Upgrade::Unverified(why)) => return Err(format!("{who}: {why}")) }
            }
            TECCountAvailableTextEncodings | TECCountAvailableSniffers => { self.tec_put(a[0], textenc::AVAILABLE.len() as u32, &who)?; self.tec_log(format!("{who} -> {}", textenc::AVAILABLE.len())); 0 }
            TECGetAvailableTextEncodings | TECGetAvailableSniffers => self.tec_list(&textenc::AVAILABLE, a[0], a[1], a[2], &who)?,
            // TECCreateConverter(TECObjectRef *new, TextEncoding in, TextEncoding out)
            TECCreateConverter => self.tec_create(&who, a[0], a[1], a[2], |from, to| Obj::Converter { from, to })?,
            TECDisposeConverter | TECDisposeSniffer | DisposeTextToUnicodeInfo | DisposeUnicodeToTextInfo => {
                // the Unicode Converter's Dispose calls take a POINTER to the info (and clear it); the TEC ones take the reference
                let (id, at) = if matches!(f, DisposeTextToUnicodeInfo | DisposeUnicodeToTextInfo) {
                    (self.mac.mem.read_u32(a[0]).map_err(|_| format!("{who}: the info pointer {:#x} leaves guest RAM", a[0]))?, a[0])
                } else { (a[0], 0) };
                if self.tec.objs.remove(&id).is_some() { self.tec_put(at, 0, &who)?; 0 } else { i32::from(textenc::CORRUPT_CONVERTER) }
            }
            TECClearConverterContextInfo | TECClearSnifferContextInfo => if self.tec.objs.contains_key(&a[0]) { 0 } else { i32::from(textenc::CORRUPT_CONVERTER) },
            // TECConvertText(conv, in, inLen, *actualIn, out, outLen, *actualOut)
            TECConvertText => {
                let Some(Obj::Converter { from, to }) = self.tec.objs.get(&a[0]).cloned() else { return self.tec_ret(cpu, i32::from(textenc::CORRUPT_CONVERTER)) };
                let input = self.get_bytes(a[1], a[2], &who)?;
                let c = textenc::convert(from, to, &input, a[5] as usize, None);
                self.put_bytes(a[4], &c.out, &who)?;
                self.tec_put(a[3], c.read as u32, &who)?;
                self.tec_put(a[6], c.out.len() as u32, &who)?;
                i32::from(c.status)
            }
            // TECFlushText(conv, out, outLen, *actualOut): nothing is ever held back (whole characters are converted or left unread)
            TECFlushText => {
                if !matches!(self.tec.objs.get(&a[0]), Some(Obj::Converter { .. })) { return self.tec_ret(cpu, i32::from(textenc::CORRUPT_CONVERTER)); }
                self.tec_put(a[3], 0, &who)?;
                0
            }
            // TECCreateSniffer(TECSnifferObjectRef *new, const TextEncoding encodings[], ItemCount n)
            TECCreateSniffer => {
                let encodings: Vec<u32> = (0..a[2]).map(|i| self.mac.mem.read_u32(a[1] + 4 * i)).collect::<Result<_, _>>().map_err(|_| format!("{who}: the encoding list leaves guest RAM"))?;
                if let Some(bad) = encodings.iter().find(|&&e| textenc::charset(e).is_none()) {
                    self.tec_log(format!("TECCreateSniffer: no sniffer for encoding {bad:#x} -> kTECNoConversionPathErr"));
                    self.tec_put(a[0], 0, &who)?;
                    i32::from(textenc::NO_CONVERSION_PATH)
                } else { let id = self.tec.add(Obj::Sniffer { encodings }); self.tec_put(a[0], id, &who)?; 0 }
            }
            // TECSniffTextEncoding(sniffer, in, inLen, TextEncoding encs[], n, ItemCount errs[], maxErrs, ItemCount features[], maxFeatures):
            // each encoding's error and feature counts (capped), and all three arrays reordered most likely first (fewest errors, then most features)
            TECSniffTextEncoding => {
                if !matches!(self.tec.objs.get(&a[0]), Some(Obj::Sniffer { .. })) { return self.tec_ret(cpu, i32::from(textenc::CORRUPT_CONVERTER)); }
                let input = self.get_bytes(a[1], a[2], &who)?;
                let mut rows = Vec::new();
                for i in 0..a[4] {
                    let e = self.mac.mem.read_u32(a[3] + 4 * i).map_err(|_| format!("{who}: the encoding array leaves guest RAM"))?;
                    let (errs, feats) = textenc::charset(e).map_or((a[6], 0), |cs| textenc::sniff(cs, &input));
                    rows.push((e, errs.min(a[6]), feats.min(a[8])));
                }
                rows.sort_by(|x, y| x.1.cmp(&y.1).then(y.2.cmp(&x.2)));
                for (i, (e, errs, feats)) in rows.into_iter().enumerate() {
                    let i = i as u32;
                    self.tec_put(a[3] + 4 * i, e, &who)?;
                    self.tec_put(a[5] + 4 * i, errs, &who)?;
                    self.tec_put(a[7] + 4 * i, feats, &who)?;
                }
                0
            }
            // CreateTextToUnicodeInfo(ConstUnicodeMappingPtr, TextToUnicodeInfo *) / CreateUnicodeToTextInfo(ConstUnicodeMappingPtr, UnicodeToTextInfo *)
            CreateTextToUnicodeInfo | CreateUnicodeToTextInfo => {
                let (uni, other) = self.tec_mapping(a[0], &who)?;
                let to_unicode = f == CreateTextToUnicodeInfo;
                if !(0x100..0x200).contains(&textenc::base(uni)) { self.tec_put(a[1], 0, &who)?; i32::from(textenc::UNSUPPORTED_ENCODING) }
                else { self.tec_create(&who, a[1], other, uni, |text, unicode| Obj::Info { text, unicode, to_unicode })? }
            }
            // Create...InfoByEncoding(TextEncoding, info *): the Unicode side is the default Unicode in UTF-16
            CreateTextToUnicodeInfoByEncoding | CreateUnicodeToTextInfoByEncoding => {
                let to_unicode = f == CreateTextToUnicodeInfoByEncoding;
                self.tec_create(&who, a[1], a[0], textenc::UNICODE_DEFAULT, |text, unicode| Obj::Info { text, unicode, to_unicode })?
            }
            // ConvertFromTextToUnicode(info, srcLen, src, flags, iOffsetCount, iOffsets[], *oOffsetCount, oOffsets[], outBufLen, *srcRead, *uniLen, out[])
            // ConvertFromUnicodeToText(info, uniLen, src, flags, iOffsetCount, iOffsets[], *oOffsetCount, oOffsets[], outBufLen, *inputRead, *outLen, out)
            ConvertFromTextToUnicode | ConvertFromUnicodeToText => {
                let to_unicode = f == ConvertFromTextToUnicode;
                let Some(Obj::Info { text, unicode, to_unicode: dir }) = self.tec.objs.get(&a[0]).cloned() else { return self.tec_ret(cpu, PARAM_ERR) };
                if dir != to_unicode { return self.tec_ret(cpu, PARAM_ERR); }
                let (from, to) = if to_unicode { (text, unicode) } else { (unicode, text) };
                let input = self.get_bytes(a[2], a[1], &who)?;
                let flags = a[3];
                let fallback = (!to_unicode && flags & USE_FALLBACKS != 0).then_some(&b"?"[..]);
                let mut c = textenc::convert(from, to, &input, a[8] as usize, fallback);
                if c.status == textenc::PARTIAL_CHAR && flags & STRING_UNTERMINATED != 0 { c.status = 0; }
                self.put_bytes(a[11], &c.out, &who)?;
                self.tec_put(a[9], c.read as u32, &who)?;
                self.tec_put(a[10], c.out.len() as u32, &who)?;
                // the offset arrays: each input offset that was read maps to the output offset of the same character (the end maps to the end)
                let mut outs = Vec::new();
                for i in 0..a[4] {
                    let o = self.mac.mem.read_u32(a[5] + 4 * i).map_err(|_| format!("{who}: the offset array leaves guest RAM"))? as usize;
                    if o == c.read { outs.push(c.out.len() as u32); continue; }
                    match c.offsets.iter().find(|&&(inp, _)| inp == o) { Some(&(_, out)) => outs.push(out as u32), None => break }
                }
                for (i, &o) in outs.iter().enumerate() { self.tec_put(a[7] + 4 * i as u32, o, &who)?; }
                self.tec_put(a[6], outs.len() as u32, &who)?;
                i32::from(c.status)
            }
            // ConvertFromUnicodeToPString(info, uniLen, src, Str255 out): the whole string or an error; no fallbacks (none can be asked for)
            ConvertFromUnicodeToPString => {
                let Some(Obj::Info { text, unicode, to_unicode: false }) = self.tec.objs.get(&a[0]).cloned() else { return self.tec_ret(cpu, PARAM_ERR) };
                let input = self.get_bytes(a[2], a[1], &who)?;
                let c = textenc::convert(unicode, text, &input, 255, None);
                if c.status == 0 { let mut p = alloc::vec![c.out.len() as u8]; p.extend(c.out); self.put_bytes(a[3], &p, &who)?; }
                i32::from(c.status)
            }
        };
        cpu.gpr[3] = r as u32;
        Ok(())
    }

    fn tec_ret(&mut self, cpu: &mut Cpu, r: i32) -> Result<(), String> { cpu.gpr[3] = r as u32; Ok(()) }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Upgrade { Param, Unverified(String) }

/// `UpgradeScriptInfoToTextEncoding`'s answer (see the module doc), without the guest.
pub fn upgrade_script_info(script: i16, lang: i16, font: Option<&[u8]>) -> Result<TextEncoding, Upgrade> {
    let font_base = match font {
        Some(b"Symbol") => Some(textenc::MAC_SYMBOL),
        Some(b"Zapf Dingbats") => Some(textenc::MAC_DINGBATS),
        _ => None,
    };
    let script = match script {
        SCRIPT_DONT_CARE if lang != SCRIPT_DONT_CARE && font_base.is_none() => {
            return Err(Upgrade::Unverified(format!("language {lang} without a script: no verified language -> script table here")));
        }
        SCRIPT_DONT_CARE | -1 | -2 => 0, // the system script / the current script: Roman on this machine
        s if (0..=32).contains(&s) => s,
        _ => return Err(Upgrade::Param),
    };
    if let Some(b) = font_base { return Ok(textenc::create(b, 0, 0)); }
    let base = match (script, lang) {
        (0, 15 | 30) => textenc::MAC_ICELANDIC,
        (0, 17) => textenc::MAC_TURKISH,
        (0, 18) => textenc::MAC_CROATIAN,
        (0, 37) => textenc::MAC_ROMANIAN,
        (s, _) => s as u32,
    };
    Ok(textenc::create(base, 0, 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_membership_is_exact() {
        assert_eq!(TecFn::by_name(TEC, "TECCountAvailableTextEncodings"), Some(TecFn::TECCountAvailableTextEncodings));
        assert_eq!(TecFn::by_name(TEXT_COMMON, "UpgradeScriptInfoToTextEncoding"), Some(TecFn::UpgradeScriptInfoToTextEncoding));
        assert_eq!(TecFn::by_name(UNICODE_CONVERTER, "ConvertFromTextToUnicode"), Some(TecFn::ConvertFromTextToUnicode));
        // NEGATIVE: a call named under the wrong library is not bound there
        assert_eq!(TecFn::by_name(TEC, "ConvertFromTextToUnicode"), None);
        assert_eq!(TecFn::by_name(UNICODE_CONVERTER, "TECConvertText"), None);
        assert_eq!(TecFn::by_name(TEXT_COMMON, "TECGetInfo"), None);
    }

    #[test]
    fn upgrade_script_info_follows_the_script_codes() {
        assert_eq!(upgrade_script_info(0, -128, None), Ok(0));
        assert_eq!(upgrade_script_info(-128, -128, None), Ok(0));
        assert_eq!(upgrade_script_info(-1, -128, None), Ok(0));
        assert_eq!(upgrade_script_info(1, -128, None), Ok(1)); // smJapanese -> kTextEncodingMacJapanese
        assert_eq!(upgrade_script_info(0, 15, None), Ok(textenc::MAC_ICELANDIC));
        assert_eq!(upgrade_script_info(0, 30, None), Ok(textenc::MAC_ICELANDIC));
        assert_eq!(upgrade_script_info(0, 0, Some(b"Symbol")), Ok(textenc::MAC_SYMBOL));
        assert_eq!(upgrade_script_info(0, 0, Some(b"Geneva")), Ok(0));
        // NEGATIVE: an out-of-range script is paramErr; a bare language is refused by name, never guessed
        assert_eq!(upgrade_script_info(33, -128, None), Err(Upgrade::Param));
        assert!(matches!(upgrade_script_info(-128, 11, None), Err(Upgrade::Unverified(_))));
    }
}
