use alloc::string::String;
use alloc::vec::Vec;

use crate::header::{MachO, MachoError, Segment};

const RECORD_CAP: u64 = 4_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rebase {
    pub segment: usize,
    pub offset: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bind {
    pub segment: usize,
    pub offset: u64,
    pub symbol: String,
    pub dylib_ordinal: i32,
    pub addend: i64,
    pub weak_import: bool,
    pub lazy: bool,
}

impl MachO<'_> {
    pub fn rebases(&self) -> Result<Vec<Rebase>, MachoError> {
        let Some(info) = self.dyld_info else {
            return Ok(Vec::new());
        };
        let Some(stream) = self.info_stream(info.rebase, "rebase info")? else {
            return Ok(Vec::new());
        };
        decode_rebase_raw(&self.segments, stream)
    }

    pub fn binds(&self) -> Result<Vec<Bind>, MachoError> {
        let Some(info) = self.dyld_info else {
            return Ok(Vec::new());
        };
        let mut result = Vec::new();
        if let Some(stream) = self.info_stream(info.bind, "bind info")? {
            result.extend(decode_bind_raw(&self.segments, stream, false)?);
        }
        if let Some(stream) = self.info_stream(info.lazy_bind, "lazy-bind info")? {
            result.extend(decode_bind_raw(&self.segments, stream, true)?);
        }
        Ok(result)
    }

    fn info_stream(
        &self,
        range: (u32, u32),
        what: &'static str,
    ) -> Result<Option<&[u8]>, MachoError> {
        if range.1 == 0 {
            return Ok(None);
        }
        let start = range.0 as usize;
        let end = start
            .checked_add(range.1 as usize)
            .ok_or(MachoError::Truncated(what))?;
        let stream = self
            .slice
            .get(start..end)
            .ok_or(MachoError::Truncated(what))?;
        Ok(Some(stream))
    }
}

pub(crate) fn decode_rebase_raw(
    segments: &[Segment],
    stream: &[u8],
) -> Result<Vec<Rebase>, MachoError> {
    let mut result = Vec::new();
    let mut pos = 0usize;
    let mut segment = None;
    let mut offset = 0u64;
    let mut count = 0u64;

    while pos < stream.len() {
        let at = pos;
        let opcode = stream[pos];
        pos += 1;
        let immediate = opcode & 0x0f;
        match opcode & 0xf0 {
            0x00 => break,
            0x10 => {
                if immediate != 1 {
                    return Err(stream_error("rebase", at, "unsupported rebase type"));
                }
            }
            0x20 => {
                let index = immediate as usize;
                if index >= segments.len() {
                    return Err(stream_error("rebase", at, "segment index out of range"));
                }
                segment = Some(index);
                offset = read_uleb(stream, &mut pos, "rebase")?;
            }
            0x30 => {
                offset = offset.wrapping_add(read_uleb(stream, &mut pos, "rebase")?);
            }
            0x40 => offset = offset.wrapping_add(immediate as u64 * 8),
            0x50 => {
                ensure_capacity(count, immediate as u64, "rebase", at)?;
                for _ in 0..immediate {
                    emit_rebase(&mut result, segments, segment, offset, at)?;
                    offset = offset.wrapping_add(8);
                    count += 1;
                }
            }
            0x60 => {
                let times = read_uleb(stream, &mut pos, "rebase")?;
                ensure_capacity(count, times, "rebase", at)?;
                for _ in 0..times {
                    emit_rebase(&mut result, segments, segment, offset, at)?;
                    offset = offset.wrapping_add(8);
                    count += 1;
                }
            }
            0x70 => {
                ensure_capacity(count, 1, "rebase", at)?;
                emit_rebase(&mut result, segments, segment, offset, at)?;
                offset = offset.wrapping_add(8);
                offset = offset.wrapping_add(read_uleb(stream, &mut pos, "rebase")?);
                count += 1;
            }
            0x80 => {
                let times = read_uleb(stream, &mut pos, "rebase")?;
                let skip = read_uleb(stream, &mut pos, "rebase")?;
                ensure_capacity(count, times, "rebase", at)?;
                for _ in 0..times {
                    emit_rebase(&mut result, segments, segment, offset, at)?;
                    offset = offset.wrapping_add(8).wrapping_add(skip);
                    count += 1;
                }
            }
            _ => return Err(stream_error("rebase", at, "unknown opcode")),
        }
    }
    Ok(result)
}

pub(crate) fn decode_bind_raw(
    segments: &[Segment],
    stream: &[u8],
    lazy: bool,
) -> Result<Vec<Bind>, MachoError> {
    let kind = if lazy { "lazy-bind" } else { "bind" };
    let mut result = Vec::new();
    let mut pos = 0usize;
    let mut ordinal = 0i32;
    let mut symbol: Option<String> = None;
    let mut addend = 0i64;
    let mut weak = false;
    let mut segment = None;
    let mut offset = 0u64;
    let mut count = 0u64;

    while pos < stream.len() {
        let at = pos;
        let opcode = stream[pos];
        pos += 1;
        let immediate = opcode & 0x0f;
        match opcode & 0xf0 {
            0x00 => {
                if !lazy {
                    break;
                }
                ordinal = 0;
                symbol = None;
                addend = 0;
                weak = false;
                segment = None;
                offset = 0;
            }
            0x10 => ordinal = immediate as i32,
            0x20 => ordinal = read_uleb(stream, &mut pos, kind)? as i32,
            0x30 => {
                ordinal = if immediate == 0 {
                    0
                } else {
                    (0xf0 | immediate) as i8 as i32
                };
            }
            0x40 => {
                weak = immediate & 1 != 0;
                let bytes = read_cstr(stream, &mut pos, kind)?;
                symbol = Some(String::from_utf8_lossy(bytes).into_owned());
            }
            0x50 => {
                if immediate != 1 {
                    return Err(stream_error(kind, at, "unsupported bind type"));
                }
            }
            0x60 => addend = read_sleb(stream, &mut pos, kind)?,
            0x70 => {
                let index = immediate as usize;
                if index >= segments.len() {
                    return Err(stream_error(kind, at, "segment index out of range"));
                }
                segment = Some(index);
                offset = read_uleb(stream, &mut pos, kind)?;
            }
            0x80 => offset = offset.wrapping_add(read_uleb(stream, &mut pos, kind)?),
            0x90 => {
                ensure_capacity(count, 1, kind, at)?;
                emit_bind(&mut result, &BindCursor { segments, segment, symbol: &symbol, offset, ordinal, addend, weak, lazy, kind }, at)?;
                offset = offset.wrapping_add(8);
                count += 1;
            }
            0xa0 => {
                ensure_capacity(count, 1, kind, at)?;
                emit_bind(&mut result, &BindCursor { segments, segment, symbol: &symbol, offset, ordinal, addend, weak, lazy, kind }, at)?;
                offset = offset.wrapping_add(8);
                offset = offset.wrapping_add(read_uleb(stream, &mut pos, kind)?);
                count += 1;
            }
            0xb0 => {
                ensure_capacity(count, 1, kind, at)?;
                emit_bind(&mut result, &BindCursor { segments, segment, symbol: &symbol, offset, ordinal, addend, weak, lazy, kind }, at)?;
                offset = offset
                    .wrapping_add(8)
                    .wrapping_add(immediate as u64 * 8);
                count += 1;
            }
            0xc0 => {
                let times = read_uleb(stream, &mut pos, kind)?;
                let skip = read_uleb(stream, &mut pos, kind)?;
                ensure_capacity(count, times, kind, at)?;
                for _ in 0..times {
                    emit_bind(&mut result, &BindCursor { segments, segment, symbol: &symbol, offset, ordinal, addend, weak, lazy, kind }, at)?;
                    offset = offset.wrapping_add(8).wrapping_add(skip);
                    count += 1;
                }
            }
            0xd0 => return Err(stream_error(kind, at, "threaded binds are unsupported")),
            _ => return Err(stream_error(kind, at, "unknown opcode")),
        }
    }
    Ok(result)
}

fn emit_rebase(
    result: &mut Vec<Rebase>,
    segments: &[Segment],
    segment: Option<usize>,
    offset: u64,
    at: usize,
) -> Result<(), MachoError> {
    let segment = segment.ok_or_else(|| stream_error("rebase", at, "rebase before segment"))?;
    check_target(segments, segment, offset, "rebase")?;
    result.push(Rebase { segment, offset });
    Ok(())
}

/// The decoder state at the moment a bind is emitted.
struct BindCursor<'a> {
    segments: &'a [Segment],
    segment: Option<usize>,
    symbol: &'a Option<String>,
    offset: u64,
    ordinal: i32,
    addend: i64,
    weak: bool,
    lazy: bool,
    kind: &'static str,
}

fn emit_bind(result: &mut Vec<Bind>, c: &BindCursor<'_>, at: usize) -> Result<(), MachoError> {
    let segment = c.segment.ok_or_else(|| stream_error(c.kind, at, "bind before segment"))?;
    let symbol = c.symbol.as_ref().ok_or_else(|| stream_error(c.kind, at, "bind before symbol"))?;
    check_target(c.segments, segment, c.offset, c.kind)?;
    result.push(Bind {
        segment,
        offset: c.offset,
        symbol: symbol.clone(),
        dylib_ordinal: c.ordinal,
        addend: c.addend,
        weak_import: c.weak,
        lazy: c.lazy,
    });
    Ok(())
}

fn check_target(
    segments: &[Segment],
    segment: usize,
    offset: u64,
    kind: &'static str,
) -> Result<(), MachoError> {
    let end = offset
        .checked_add(8)
        .ok_or(MachoError::TargetOutsideSegment { stream: kind })?;
    if end > segments[segment].vmsize {
        return Err(MachoError::TargetOutsideSegment { stream: kind });
    }
    Ok(())
}

fn ensure_capacity(
    count: u64,
    additional: u64,
    kind: &'static str,
    at: usize,
) -> Result<(), MachoError> {
    if additional > RECORD_CAP || count.saturating_add(additional) > RECORD_CAP {
        return Err(stream_error(kind, at, "too many records"));
    }
    Ok(())
}

fn stream_error(kind: &'static str, at: usize, why: &'static str) -> MachoError {
    MachoError::BadOpcodeStream {
        stream: kind,
        at,
        why,
    }
}

fn read_uleb(
    stream: &[u8],
    pos: &mut usize,
    kind: &'static str,
) -> Result<u64, MachoError> {
    let start = *pos;
    let mut value = 0u64;
    for index in 0..10 {
        let Some(&byte) = stream.get(*pos) else {
            return Err(stream_error(kind, start, "unterminated ULEB128"));
        };
        *pos += 1;
        let shift = index * 7;
        let payload = (byte & 0x7f) as u64;
        if shift == 63 && payload > 1 {
            return Err(stream_error(kind, start, "ULEB128 overflows u64"));
        }
        value |= payload << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(stream_error(kind, start, "ULEB128 overflows u64"))
}

fn read_sleb(
    stream: &[u8],
    pos: &mut usize,
    kind: &'static str,
) -> Result<i64, MachoError> {
    let start = *pos;
    let mut value = 0u64;
    let mut shift = 0u32;
    for _ in 0..10 {
        let Some(&byte) = stream.get(*pos) else {
            return Err(stream_error(kind, start, "unterminated SLEB128"));
        };
        *pos += 1;
        value |= ((byte & 0x7f) as u64) << shift;
        shift += 7;
        if byte & 0x80 == 0 {
            if shift < 64 && byte & 0x40 != 0 {
                value |= u64::MAX << shift;
            }
            return Ok(value as i64);
        }
    }
    Err(stream_error(kind, start, "SLEB128 overflows i64"))
}

fn read_cstr<'a>(
    stream: &'a [u8],
    pos: &mut usize,
    kind: &'static str,
) -> Result<&'a [u8], MachoError> {
    let start = *pos;
    let tail = stream
        .get(start..)
        .ok_or_else(|| stream_error(kind, start, "unterminated symbol"))?;
    let length = tail
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| stream_error(kind, start, "unterminated symbol"))?;
    *pos = start + length + 1;
    Ok(&tail[..length])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writer::ImageBuilder;
    use alloc::string::ToString;
    use alloc::vec;

    fn seg(name: &str, vmsize: u64) -> Segment {
        Segment {
            name: name.to_string(),
            vmaddr: 0,
            vmsize,
            fileoff: 0,
            filesize: 0,
            maxprot: 3,
            initprot: 3,
            sections: Vec::new(),
        }
    }

    /// Segment 0 plays __PAGEZERO, segment 1 a 4 KiB data segment.
    fn segs() -> Vec<Segment> {
        vec![seg("__PAGEZERO", 0x1000), seg("__DATA", 0x1000)]
    }

    fn rebases_of(stream: &[u8]) -> Result<Vec<u64>, MachoError> {
        decode_rebase_raw(&segs(), stream).map(|v| v.iter().map(|r| r.offset).collect())
    }

    fn binds_of(stream: &[u8], lazy: bool) -> Result<Vec<Bind>, MachoError> {
        decode_bind_raw(&segs(), stream, lazy)
    }

    fn is_stream_error<T: core::fmt::Debug>(r: Result<T, MachoError>) -> bool {
        matches!(r, Err(MachoError::BadOpcodeStream { .. }))
    }

    #[test]
    fn decodes_non_lazy_bind() {
        let mut builder = ImageBuilder::new();
        builder.import("_write");
        let image = builder.build();
        let macho = MachO::parse(&image.bytes).unwrap();
        let binds = macho.binds().unwrap();
        assert_eq!(binds.len(), 1);
        assert_eq!(binds[0].symbol, "_write");
        assert_eq!(binds[0].dylib_ordinal, 1);
        assert_eq!(binds[0].segment, 2);
        assert_eq!(binds[0].offset, 0);
        assert_eq!(binds[0].addend, 0);
        assert!(!binds[0].lazy);
    }

    #[test]
    fn decodes_lazy_bind() {
        let mut builder = ImageBuilder::new();
        builder.lazy_import("_write");
        let image = builder.build();
        let binds = MachO::parse(&image.bytes).unwrap().binds().unwrap();
        assert_eq!(binds.len(), 1);
        assert_eq!((binds[0].symbol.as_str(), binds[0].offset, binds[0].lazy), ("_write", 0, true));
    }

    #[test]
    fn decodes_two_imports_in_order() {
        let mut builder = ImageBuilder::new();
        builder.import("_write");
        builder.lazy_import("_exit");
        let image = builder.build();
        let binds = MachO::parse(&image.bytes).unwrap().binds().unwrap();
        let got: Vec<_> = binds.iter().map(|b| (b.symbol.as_str(), b.offset, b.lazy)).collect();
        assert_eq!(got, vec![("_write", 0, false), ("_exit", 8, true)]);
    }

    #[test]
    fn rebase_decodes_data_pointer() {
        let mut builder = ImageBuilder::new().data(&[0u8; 16]).rebase_at_data(8);
        builder.import("_write");
        let image = builder.build();
        let rebases = MachO::parse(&image.bytes).unwrap().rebases().unwrap();
        // one GOT slot (8 bytes) precedes the data bytes
        assert_eq!(rebases, vec![Rebase { segment: 2, offset: 16 }]);
    }

    #[test]
    fn no_dyld_info_decodes_to_nothing() {
        let image = ImageBuilder::new().build();
        let macho = MachO::parse(&image.bytes).unwrap();
        assert_eq!(macho.binds().unwrap(), Vec::new());
    }

    #[test]
    fn hand_assembled_rebase_streams() {
        // SET_SEGMENT 1 @ 8, DO_REBASE_ULEB_TIMES 3
        assert_eq!(rebases_of(&[0x11, 0x21, 0x08, 0x60, 0x03, 0x00]).unwrap(), vec![8, 16, 24]);
        // DO_REBASE_ULEB_TIMES_SKIPPING_ULEB: 2 times, skip 8 -> stride 16
        assert_eq!(rebases_of(&[0x11, 0x21, 0x00, 0x80, 0x02, 0x08, 0x00]).unwrap(), vec![0, 16]);
        // DO_REBASE_IMM_TIMES 3, then ADD_ADDR_IMM_SCALED 2 (+16), then one more
        assert_eq!(rebases_of(&[0x11, 0x21, 0x00, 0x53, 0x42, 0x51, 0x00]).unwrap(), vec![0, 8, 16, 40]);
        // DO_REBASE_ADD_ADDR_ULEB: emit, then advance 8 + 0x20
        assert_eq!(rebases_of(&[0x11, 0x21, 0x00, 0x70, 0x20, 0x51, 0x00]).unwrap(), vec![0, 40]);
        // ADD_ADDR_ULEB with a multi-byte number (0x81 0x01 = 129)
        assert_eq!(rebases_of(&[0x11, 0x21, 0x00, 0x30, 0x81, 0x01, 0x51, 0x00]).unwrap(), vec![129]);
    }

    #[test]
    fn hand_assembled_bind_streams() {
        let sym = |flags: u8, name: &[u8]| {
            let mut v = vec![0x40 | flags];
            v.extend_from_slice(name);
            v.push(0);
            v
        };
        let one = |special: u8| {
            let mut s = vec![special];
            s.extend(sym(0, b"a"));
            s.extend_from_slice(&[0x71, 0x00, 0x90]);
            binds_of(&s, false).unwrap().remove(0).dylib_ordinal
        };
        assert_eq!(one(0x30), 0, "BIND_SPECIAL_DYLIB_SELF");
        assert_eq!(one(0x3F), -1, "main executable");
        assert_eq!(one(0x3E), -2, "flat lookup");
        assert_eq!(one(0x13), 3, "ordinal immediate");

        // addend -1 (SLEB 0x7F)
        let mut s = vec![0x11];
        s.extend(sym(0, b"x"));
        s.extend_from_slice(&[0x60, 0x7F, 0x71, 0x00, 0x90]);
        assert_eq!(binds_of(&s, false).unwrap()[0].addend, -1);

        // weak import flag
        let mut s = vec![0x11];
        s.extend(sym(1, b"w"));
        s.extend_from_slice(&[0x71, 0x00, 0x90]);
        let b = binds_of(&s, false).unwrap();
        assert!(b[0].weak_import);
        assert_eq!(b[0].symbol, "w");

        // DO_BIND_ADD_ADDR_IMM_SCALED 1: emit at 0, next at 8 + 8
        let mut s = vec![0x11];
        s.extend(sym(0, b"q"));
        s.extend_from_slice(&[0x71, 0x00, 0xB1, 0x90]);
        let offs: Vec<_> = binds_of(&s, false).unwrap().iter().map(|b| b.offset).collect();
        assert_eq!(offs, vec![0, 16]);

        // DO_BIND_ULEB_TIMES_SKIPPING_ULEB: 2 times, skip 0
        let mut s = vec![0x11];
        s.extend(sym(0, b"q"));
        s.extend_from_slice(&[0x71, 0x00, 0xC0, 0x02, 0x00]);
        let offs: Vec<_> = binds_of(&s, false).unwrap().iter().map(|b| b.offset).collect();
        assert_eq!(offs, vec![0, 8]);

        // DO_BIND_ADD_ADDR_ULEB 0x10: emit at 0, next at 8 + 16
        let mut s = vec![0x11];
        s.extend(sym(0, b"q"));
        s.extend_from_slice(&[0x71, 0x00, 0xA0, 0x10, 0x90]);
        let offs: Vec<_> = binds_of(&s, false).unwrap().iter().map(|b| b.offset).collect();
        assert_eq!(offs, vec![0, 24]);
    }

    #[test]
    fn done_ends_a_bind_stream_but_only_an_entry_of_a_lazy_stream() {
        let stream = [
            0x71, 0x00, 0x11, 0x40, b'a', 0, 0x90, 0x00, //
            0x71, 0x08, 0x11, 0x40, b'b', 0, 0x90, 0x00,
        ];
        let bind = binds_of(&stream, false).unwrap();
        assert_eq!(bind.len(), 1, "DONE ends the non-lazy stream");
        let lazy = binds_of(&stream, true).unwrap();
        let got: Vec<_> = lazy.iter().map(|b| (b.symbol.as_str(), b.offset, b.lazy)).collect();
        assert_eq!(got, vec![("a", 0, true), ("b", 8, true)]);
    }

    #[test]
    fn rejects_bind_before_symbol() {
        assert!(is_stream_error(binds_of(&[0x71, 0x00, 0x90], false)));
    }

    #[test]
    fn rejects_bind_before_segment() {
        assert!(is_stream_error(binds_of(&[0x11, 0x40, b'a', 0, 0x90], false)));
    }

    #[test]
    fn rejects_unterminated_uleb() {
        assert!(is_stream_error(rebases_of(&[0x21, 0x80])));
        assert!(is_stream_error(binds_of(&[0x11, 0x40, b'a', 0, 0x71, 0x80], false)));
    }

    #[test]
    fn rejects_uleb_wider_than_64_bits() {
        let mut s = vec![0x21];
        s.extend_from_slice(&[0xFF; 10]);
        s.push(0x01);
        assert!(is_stream_error(rebases_of(&s)));
    }

    #[test]
    fn rejects_unterminated_symbol_name() {
        assert!(is_stream_error(binds_of(&[0x11, 0x40, b'a', b'b'], false)));
    }

    #[test]
    fn rejects_threaded_bind() {
        assert!(is_stream_error(binds_of(&[0xD0, 0x00], false)));
    }

    #[test]
    fn rejects_target_outside_segment() {
        // offset 0xFFC + 8 > 0x1000
        assert_eq!(
            rebases_of(&[0x11, 0x21, 0xFC, 0x1F, 0x51]).unwrap_err(),
            MachoError::TargetOutsideSegment { stream: "rebase" }
        );
        // offset 0xFF8 is the last whole word
        assert_eq!(rebases_of(&[0x11, 0x21, 0xF8, 0x1F, 0x51]).unwrap(), vec![0xFF8]);
    }

    #[test]
    fn rejects_unknown_rebase_type_and_bad_segment_index() {
        assert!(is_stream_error(rebases_of(&[0x12])));
        assert!(is_stream_error(rebases_of(&[0x25, 0x00, 0x51])), "segment 5 does not exist");
        assert!(is_stream_error(rebases_of(&[0x11, 0x51])), "rebase before any segment");
        assert!(is_stream_error(rebases_of(&[0xF0])), "unknown opcode");
    }

    #[test]
    fn huge_count_is_capped() {
        // DO_REBASE_ULEB_TIMES with 2^40
        let s = [0x11, 0x21, 0x00, 0x60, 0x80, 0x80, 0x80, 0x80, 0x80, 0x20];
        assert!(is_stream_error(rebases_of(&s)));
        let mut b = vec![0x11, 0x40, b'a', 0, 0x71, 0x00, 0xC0];
        b.extend_from_slice(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x20, 0x00]);
        assert!(is_stream_error(binds_of(&b, false)));
    }
}
