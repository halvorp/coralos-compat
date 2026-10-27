//! The S60 scalable-layout byte-code decoder: a native Rust implementation of `AknLayoutScalableDecodeSupport`
//! (layouts/aknlayout2/src/AknLayout2Scalable.cpp:1267-1780 in the Symbian Foundation UI-resources source, EPL-1.0; consulted as
//! reference, not copied). It turns one line of an instance's byte-coded data (read at run time from the local layout data file,
//! `cdl.rs`) into the 16-word `TAknWindowComponentLayout` / `TAknTextComponentLayout` structs the guest receives.
//!
//! Byte codes (AknLayoutByteCodes.h is not in the source tree; the values are read off the data itself and checked by the tests):
//! 0xff = 16-bit word follows, 0xfe = "empty", 0xfd = parent-relative, 1 signed byte, 0xfc = parent-relative, 2 bytes,
//! 0xfb = 32-bit value, anything lower = the literal byte.
use alloc::string::String;
use alloc::vec::Vec;

const BYTE_WORD: u8 = 0xff;
const BYTE_EMPTY: u8 = 0xfe;
const BYTE_P1: u8 = 0xfd;
const BYTE_P2: u8 = 0xfc;
const BYTE_LONG: u8 = 0xfb;
/// avkon.hrh:890 `ELayoutEmpty`; AknLayout2Def.h:42 `ELayoutP`.
pub const LAYOUT_EMPTY: u16 = (-16380i16) as u16;
const P: u16 = 0x4000;
/// In the packed struct: five words of ids, then the type, then the cells.
const WORDS_BEFORE_TYPE: usize = 5;
pub const CELLS_WINDOW: usize = 7;
pub const CELLS_TEXT: usize = 9;
/// numCols and numRows come first when a line has a header block.
const CELLS_NOT_SIMPLE: usize = 2;
/// {numCols; numRows; iC; il; it; ir; ib; iW; iH; iJ; iFont}: which cells count along the column index (the rest, along the row index).
const IS_COL: [bool; 11] = [false, false, false, true, false, true, false, true, false, false, false];

type R<T> = Result<T, String>;

fn err<T>(what: &str) -> R<T> {
    Err(alloc::format!("layout data: {what}"))
}

/// A read position in the blob.
#[derive(Clone, Copy)]
struct Cur<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Cur<'a> {
    fn byte(&mut self) -> R<u8> {
        let v = *self.b.get(self.pos).ok_or_else(|| String::from("layout data: read past the blob"))?;
        self.pos += 1;
        Ok(v)
    }

    /// `DecodeBytes(aPtr, aDecodedPtr)`: one coded value as 1 or 2 output words (low first).
    fn value(&mut self, out: &mut Vec<u16>) -> R<()> {
        let ch = self.byte()?;
        match ch {
            BYTE_WORD => {
                let hi = self.byte()? as u16;
                let lo = self.byte()? as u16;
                out.push((hi << 8) | lo);
            }
            BYTE_EMPTY => out.push(LAYOUT_EMPTY),
            BYTE_P1 => {
                let off = self.byte()? as i8;
                out.push(P.wrapping_add(off as i16 as u16));
            }
            BYTE_P2 => {
                let hi = self.byte()? as u16;
                let lo = self.byte()? as u16;
                out.push(P.wrapping_add((hi << 8) | lo));
            }
            BYTE_LONG => {
                let b3 = self.byte()? as u16;
                let b2 = self.byte()? as u16;
                let b1 = self.byte()? as u16;
                let b0 = self.byte()? as u16;
                out.push((b1 << 8) | b0);
                out.push((b3 << 8) | b2);
            }
            lit => out.push(lit as u16),
        }
        Ok(())
    }

    /// `DecodeBytes` returning a TInt32 (the low word, or both for a long).
    fn int(&mut self) -> R<i32> {
        let mut w = Vec::with_capacity(2);
        self.value(&mut w)?;
        Ok(w[0] as i32 | ((w.get(1).copied().unwrap_or(0) as i32) << 16))
    }

    fn skip(&mut self, n: i32) -> R<()> {
        for _ in 0..n.max(0) {
            match self.byte()? {
                BYTE_WORD | BYTE_P2 => self.pos += 2,
                BYTE_P1 => self.pos += 1,
                BYTE_LONG => self.pos += 4,
                _ => {}
            }
        }
        if self.pos > self.b.len() {
            return err("skip past the blob");
        }
        Ok(())
    }

    /// `SumBytes`: the sum of the next `n` coded values.
    fn sum(&mut self, n: i32) -> R<i32> {
        let mut total = 0i32;
        for _ in 0..n.max(0) {
            total = total.wrapping_add(self.int()?);
        }
        Ok(total)
    }
}

/// What `DecodeParam` leaves behind for one cell.
struct Param {
    max: i32,
    param: i32,
}

/// `DecodeParam`: scan the (bit-field, count) pairs for the one that names `cell`.
fn decode_param(cell: usize, mut params: Cur, num_params: i32, multi: i32, variety: i32, variety_offset: i32) -> R<Param> {
    let bit = 1u32 << cell;
    let mut found = false;
    for _ in 0..num_params {
        let bits = params.int()? as u32 & 0xffff;
        if bits & bit != 0 {
            found = true;
            break;
        }
        params.int()?;
    }
    if !found {
        return Ok(Param { max: 1, param: 0 });
    }
    let bitfield = params.int()?;
    let optimized_varieties = bitfield & 1 != 0;
    let optimized_calcs = bitfield & 2 != 0;
    let max = bitfield >> 2;
    let actual = if optimized_calcs { variety } else { variety_offset };
    let mut param = if optimized_varieties { 0 } else { actual };
    if !optimized_calcs {
        param += multi;
    }
    Ok(Param { max, param })
}

/// `DecodeCell`: skip `param` values, decode one into `out`, skip the rest of the cell's `max`.
fn decode_cell(p: &Param, data: &mut Cur, out: &mut Vec<u16>) -> R<()> {
    if !(0 <= p.param && p.param < p.max) {
        return err("a parameter is out of range");
    }
    data.skip(p.param)?;
    data.value(out)?;
    data.skip(p.max - (p.param + 1))
}

/// `DecodeParamLimitsCell`: returns (the value, the variety offset).
fn decode_param_limits_cell(variety: i32, p: &Param, data: &mut Cur) -> R<(i32, i32)> {
    if !(0 <= p.param && p.param < p.max) {
        return err("a parameter limit is out of range");
    }
    let mut variety_offset = 0;
    if p.param != 0 {
        variety_offset = data.sum(p.param)?;
    }
    let value = data.int()?;
    if p.max == 1 {
        variety_offset = variety.wrapping_mul(value);
    }
    data.skip(p.max - (p.param + 1))?;
    Ok((value, variety_offset))
}

/// The state `DecodeHeaderBlock` leaves: the multi-value count and the params position.
struct Header<'a> {
    num_multi: i32,
    params: Cur<'a>,
}

fn decode_header_block<'a>(data: &mut Cur<'a>, variety: i32, limits: &mut [i32; 6]) -> R<Header<'a>> {
    let num_varieties = data.byte()? as i32;
    limits[0] = 0;
    limits[1] = num_varieties - 1;
    if !(0 <= variety && variety < num_varieties) {
        return err("variety out of range");
    }
    let num_multi = data.byte()? as i32;
    let params = *data;
    data.skip(num_multi * 2)?;
    Ok(Header { num_multi, params })
}

/// `DecodeParamLimitParam`: fills limits[slot], limits[slot+1] and returns the variety offset.
fn decode_param_limit_param(limits: &mut [i32; 6], slot: usize, h: &Header, data: &mut Cur, variety: i32, cell: usize) -> R<i32> {
    let p = decode_param(cell, h.params, h.num_multi, 0, variety, variety)?;
    let (value, variety_offset) = decode_param_limits_cell(variety, &p, data)?;
    limits[slot] = 0;
    limits[slot + 1] = value.wrapping_sub(1);
    Ok(variety_offset)
}

/// A decoded line: the 16 words of the packed struct (`TAknWindowComponentLayoutImp` / `TAknTextComponentLayoutImp`).
pub type Words = [u16; 16];

fn type_id(data: &mut Cur) -> R<(u16, bool)> {
    let b = data.byte()? as u16;
    Ok((b >> 1, b & 1 != 0))
}

fn put(words: &mut Words, from: usize, v: &[u16]) -> R<()> {
    for (i, w) in v.iter().enumerate() {
        *words.get_mut(from + i).ok_or_else(|| String::from("layout data: line overflows its struct"))? = *w;
    }
    Ok(())
}

/// `DecodeSimpleCells`.
fn simple_cells(mut data: Cur, ty: u16, cells: usize) -> R<Words> {
    let mut w: Words = [0; 16];
    w[WORDS_BEFORE_TYPE] = ty;
    let mut out = Vec::new();
    for _ in 0..cells {
        data.value(&mut out)?;
    }
    put(&mut w, WORDS_BEFORE_TYPE + 1, &out)?;
    Ok(w)
}

/// `DecodeParamCells`.
fn param_cells(mut data: Cur, ty: u16, cells: usize, variety: i32, col: i32, row: i32) -> R<Words> {
    let mut w: Words = [0; 16];
    w[WORDS_BEFORE_TYPE] = ty;
    let mut limits = [0i32; 6];
    let h = decode_header_block(&mut data, variety, &mut limits)?;
    let mut variety_offsets = [0i32; CELLS_NOT_SIMPLE];
    for (i, off) in variety_offsets.iter_mut().enumerate() {
        *off = decode_param_limit_param(&mut limits, 2 + i * 2, &h, &mut data, variety, i)?;
    }
    let (num_cols, num_rows) = (limits[3] + 1, limits[5] + 1);
    if !(0 <= col && col <= num_cols) || !(0 <= row && row <= num_rows) {
        return err("column or row out of range");
    }
    let mut out = Vec::new();
    for (ii, &is_col) in IS_COL.iter().enumerate().take(cells).skip(CELLS_NOT_SIMPLE) {
        let multi = if is_col { col } else { row };
        let col_row = if is_col { 0 } else { 1 };
        let p = decode_param(ii, h.params, h.num_multi, multi, variety, variety_offsets[col_row])?;
        decode_cell(&p, &mut data, &mut out)?;
    }
    put(&mut w, WORDS_BEFORE_TYPE + 1, &out)?;
    Ok(w)
}

/// `DecodeParams`: the line at `at`, parameterised when it has a header block.
fn params_line(blob: &[u8], at: usize, cells: usize, variety: i32, col: i32, row: i32) -> R<Words> {
    let mut data = Cur { b: blob, pos: at };
    let (ty, has_header) = type_id(&mut data)?;
    if has_header { param_cells(data, ty, cells + CELLS_NOT_SIMPLE, variety, col, row) } else { simple_cells(data, ty, cells) }
}

/// `DecodeParamLimits`: [first variety, last variety, first col, last col, first row, last row].
fn limits_line(blob: &[u8], at: usize, variety: i32) -> R<[i32; 6]> {
    let mut data = Cur { b: blob, pos: at };
    let mut limits = [0i32; 6];
    let (_, has_header) = type_id(&mut data)?;
    if has_header {
        let h = decode_header_block(&mut data, variety, &mut limits)?;
        for i in 0..2 {
            decode_param_limit_param(&mut limits, 2 + i * 2, &h, &mut data, variety, i)?;
        }
    }
    // A line without a header block has no limits to report; the device returns the struct unwritten, which is not data. Zeros.
    Ok(limits)
}

/// What a decode operation asks of the data. `at` is the line's position in the instance's blob.
pub struct Instance<'a> {
    pub blob: &'a [u8],
    /// The instance's offset into the blob (`KByteCodedData + offset`).
    pub base: usize,
    pub lookup: &'a [u16],
    /// Layout_Meta_Data::IsMirrored() (the data is stored mirrored when the layout is: unswap l and r).
    pub mirrored: bool,
}

impl Instance<'_> {
    fn line_at(&self, api: usize) -> R<usize> {
        let off = *self.lookup.get(api).ok_or_else(|| String::from("layout data: api id past the lookup"))?;
        Ok(self.base + off as usize)
    }

    /// `DecodeTable`: the line `index` of the table at api `api`.
    fn table_line(&self, api: usize, index: i32) -> R<usize> {
        let mut data = Cur { b: self.blob, pos: self.line_at(api)? };
        let n = data.int()? & 0xffff;
        if !(0 <= index && index < n) {
            return err("table line index out of range");
        }
        let at = data.pos + index as usize * 2;
        let hi = *self.blob.get(at).ok_or_else(|| String::from("layout data: table past the blob"))? as usize;
        let lo = *self.blob.get(at + 1).ok_or_else(|| String::from("layout data: table past the blob"))? as usize;
        Ok(self.base + ((hi << 8) | lo))
    }

    fn mirror_window(&self, w: &mut Words) {
        if self.mirrored {
            w.swap(7, 9);
        }
    }

    fn mirror_text(&self, w: &mut Words) {
        if self.mirrored {
            w.swap(7, 9);
            // iJ: left and right alignment swap (avkon.hrh:820-824: None 0, Center 1, Left 2, Right 3, Bidi 4).
            if w[13] == ALIGN_LEFT {
                w[13] = ALIGN_RIGHT;
            } else if w[13] == ALIGN_RIGHT {
                w[13] = ALIGN_LEFT;
            }
        }
    }

    /// A window or text line of api `api` (a table line when `table` is `Some(line index)`), parameters `(variety, col, row)`.
    pub fn line(&self, api: usize, table: Option<i32>, text: bool, variety: i32, col: i32, row: i32) -> R<Words> {
        let at = match table {
            Some(i) => self.table_line(api, i)?,
            None => self.line_at(api)?,
        };
        let w = params_line(self.blob, at, if text { CELLS_TEXT } else { CELLS_WINDOW }, variety, col, row);
        let mut w = w?;
        if text { self.mirror_text(&mut w) } else { self.mirror_window(&mut w) }
        Ok(w)
    }

    /// `ParameterLimits` / `ParameterLimitsTable`.
    pub fn limits(&self, api: usize, table: Option<i32>, variety: i32) -> R<[i32; 6]> {
        let at = match table {
            Some(i) => self.table_line(api, i)?,
            None => self.line_at(api)?,
        };
        limits_line(self.blob, at, variety)
    }

    /// `TableLimits`: the lookup entry itself, low byte = last index, high byte = first.
    pub fn table_limits(&self, api: usize) -> R<[i32; 2]> {
        let v = *self.lookup.get(api).ok_or_else(|| String::from("layout data: api id past the lookup"))?;
        Ok([(v >> 8) as i32, (v & 0xff) as i32])
    }

    /// `GetComponentTypeById`.
    pub fn component_type(&self, id: usize) -> R<u16> {
        let mut data = Cur { b: self.blob, pos: self.line_at(id)? };
        Ok(type_id(&mut data)?.0)
    }

    /// `GetParamLimitsById`.
    pub fn limits_by_id(&self, id: usize, variety: i32) -> R<[i32; 6]> {
        limits_line(self.blob, self.line_at(id)?, variety)
    }

    /// `GetWindowComponentById` / `GetTextComponentById`: the component's own type decides the cell count (text: always text).
    pub fn component(&self, id: usize, text_only: bool, variety: i32, col: i32, row: i32) -> R<Words> {
        let mut data = Cur { b: self.blob, pos: self.line_at(id)? };
        let (ty, has_header) = type_id(&mut data)?;
        let is_text = ty == COMPONENT_TYPE_TEXT;
        if text_only && !is_text {
            return err("GetTextComponentById on a component that is not text");
        }
        let cells = if is_text { CELLS_TEXT } else { CELLS_WINDOW };
        let mut w = if has_header { param_cells(data, ty, cells + CELLS_NOT_SIMPLE, variety, col, row)? } else { simple_cells(data, ty, cells)? };
        if is_text { self.mirror_text(&mut w) } else { self.mirror_window(&mut w) }
        Ok(w)
    }
}

/// TAknLayoutScalableComponentType (AknLayout2ScalableDef.h): Unknown, Screen, Container, Pane, Graphic, Text.
pub const COMPONENT_TYPE_TEXT: u16 = 5;
/// ELayoutAlignLeft / ELayoutAlignRight (avkon.hrh).
const ALIGN_LEFT: u16 = 2;
const ALIGN_RIGHT: u16 = 3;

/// The words as the 32 bytes the guest struct holds (little endian).
pub fn words_to_bytes(w: &Words) -> [u8; 32] {
    let mut b = [0u8; 32];
    for (i, v) in w.iter().enumerate() {
        b[i * 2..i * 2 + 2].copy_from_slice(&v.to_le_bytes());
    }
    b
}
