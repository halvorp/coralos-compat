//! Ring frames. Layout, every field little-endian:
//! `u16 abi_version | u16 kind | u32 payload_len | u32 token.slot | u32 token.incarnation | payload`.
//! The version is FIRST so a reader refuses an unknown layout before it parses anything else.
use crate::snapshot::{snapshot_array, snapshot_array_into, snapshot_indices_into, ArraySnapshot, Descriptor, ElemType, Role, SnapshotError};
use crate::token::Token;
use crate::wire::{Reader, WireError, Writer, ABI_VERSION};
use alloc::string::String;
use alloc::vec::Vec;

const HEADER: usize = 16;
const GL_POLYGON: u32 = 0x0009;

/// One draw: its indices and every enabled array's referenced vertices, all captured at draw time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawCommand {
    pub mode: u32,
    pub indices: crate::snapshot::IndexSnapshot,
    pub arrays: Vec<ArraySnapshot>,
}

impl DrawCommand {
    /// Capture a `glDrawArrays(mode, first, count)` NOW: the indices are `first .. first + count`, so the draw is the `glDrawElements` of
    /// those indices (GL 1.2 §2.8 defines it that way), each enabled array snapshotted over exactly that range.
    pub fn capture_arrays(mem: &[u8], mode: u32, first: u32, count: u32, arrays: &[Descriptor]) -> Result<DrawCommand, SnapshotError> {
        let last = first.checked_add(count.saturating_sub(1)).ok_or(SnapshotError::Overflow)?;
        let mut data = Vec::new();
        data.try_reserve_exact((count as usize).checked_mul(4).ok_or(SnapshotError::Overflow)?).map_err(|_| SnapshotError::Overflow)?;
        for i in 0..count {
            data.extend_from_slice(&(first + i).to_le_bytes());
        }
        let indices = crate::snapshot::IndexSnapshot { ty: ElemType::U32, count, data, range: (count > 0).then_some((first, last)) };
        let mut snaps = Vec::new();
        if let Some((lo, hi)) = indices.range {
            let n = (hi - lo).checked_add(1).ok_or(SnapshotError::Overflow)?;
            for d in arrays {
                snaps.push(snapshot_array(mem, d, lo, n)?);
            }
        }
        Ok(DrawCommand { mode, indices, arrays: snaps })
    }

    /// Capture a `glDrawElements` NOW: indices converted, arrays snapshotted over exactly `min..=max` index.
    pub fn capture(mem: &[u8], mode: u32, index_off: usize, count: u32, ty: ElemType, arrays: &[Descriptor]) -> Result<DrawCommand, SnapshotError> {
        let mut d = DrawCommand::empty();
        d.capture_into(&mut Vec::new(), mem, mode, index_off, count, ty, arrays)?;
        Ok(d)
    }

    /// A draw with no indices and no arrays (also its `Default`): the reusable buffer [`DrawCommand::capture_into`] fills.
    pub fn empty() -> DrawCommand {
        DrawCommand { mode: 0, indices: crate::snapshot::IndexSnapshot { ty: ElemType::U16, count: 0, data: Vec::new(), range: None }, arrays: Vec::new() }
    }

    /// [`DrawCommand::capture`] into this draw, reusing its buffers: array snapshots it no longer needs go to `spare` and are taken
    /// back from it, so a draw path that captures every frame allocates nothing once warm. On an error the draw is unspecified.
    pub fn capture_into(&mut self, spare: &mut Vec<ArraySnapshot>, mem: &[u8], mode: u32, index_off: usize, count: u32, ty: ElemType, arrays: &[Descriptor]) -> Result<(), SnapshotError> {
        self.mode = mode;
        snapshot_indices_into(mem, index_off, count, ty, &mut self.indices)?;
        let used = match self.indices.range {
            Some((lo, hi)) => {
                let n = (hi - lo).checked_add(1).ok_or(SnapshotError::Overflow)?;
                for (k, d) in arrays.iter().enumerate() {
                    if self.arrays.len() <= k {
                        let blank = || ArraySnapshot { role: d.role, unit: 0, size: 1, ty: d.ty, first: 0, count: 0, data: Vec::new() };
                        self.arrays.push(spare.pop().unwrap_or_else(blank));
                    }
                    snapshot_array_into(mem, d, lo, n, &mut self.arrays[k])?;
                }
                arrays.len()
            }
            None => 0,
        };
        while self.arrays.len() > used {
            spare.extend(self.arrays.pop());
        }
        Ok(())
    }
}

impl Default for DrawCommand {
    fn default() -> DrawCommand { DrawCommand::empty() }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    ContextCreate,
    ContextDestroy,
    MakeCurrent,
    /// A state call by contract-table index with raw 64-bit argument words.
    Call { entry: u16, args: Vec<u64> },
    /// A call that carries bytes (a texture's pixels): the words as in `Call`, then `data`, already tight and little-endian
    /// (`crate::pixel::unpack`). Empty `data` is a call with no client pixels (`glTexImage2D(.., NULL)`).
    CallData { entry: u16, args: Vec<u64>, data: Vec<u8> },
    Draw(DrawCommand),
    FenceSet { id: u32 },
    Swap { drawable: Token },
    /// Worker to engine: this call/reason was refused (the marker text).
    Refused { text: String },
    /// Worker to engine: the answer to a request that has one (`ContextCreate` -> the new token in `words`,
    /// `glGetString` -> `text`, `glGetError` -> `words[0]`, ...). `status` 0 is success.
    Reply { status: u32, words: Vec<u64>, text: String },
    /// Worker to engine: the answer to a read back (`glReadPixels`): `words` as in `Reply`, then the pixels, tight and little-endian.
    ReplyData { status: u32, words: Vec<u64>, data: Vec<u8> },
    /// Engine to worker: batch `seq` is complete; everything before it may be retired.
    BatchEnd { seq: u64 },
    /// Worker to engine: every batch up to and including `seq` has been replayed AND finished on the GPU
    /// (`FenceBook::retire_through`).
    Retired { seq: u64 },
}

/// A frame: the context token it acts on (zero for `ContextCreate`) and its command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub token: Token,
    pub command: Command,
}

/// The frame kinds a carrier reads without decoding (the header's second field, [`peek`]).
const KIND_CALL: u16 = 4;
const KIND_DRAW: u16 = 5;
const KIND_FENCE_SET: u16 = 6;
const KIND_SWAP: u16 = 7;
const KIND_BATCH_END: u16 = 10;

fn kind_of(c: &Command) -> u16 {
    match c {
        Command::ContextCreate => 1,
        Command::ContextDestroy => 2,
        Command::MakeCurrent => 3,
        Command::Call { .. } => KIND_CALL,
        Command::Draw(_) => KIND_DRAW,
        Command::FenceSet { .. } => KIND_FENCE_SET,
        Command::Swap { .. } => KIND_SWAP,
        Command::Refused { .. } => 8,
        Command::Reply { .. } => 9,
        Command::BatchEnd { .. } => KIND_BATCH_END,
        Command::Retired { .. } => 11,
        Command::CallData { .. } => 12,
        Command::ReplyData { .. } => 13,
    }
}

/// What a carrier reads of one whole frame from its header alone, without decoding (or copying) its payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Peek {
    /// A `Command::Call` of this contract entry (its payload's first field; `None` when the payload is too short to hold it).
    Call(Option<u16>),
    Draw,
    FenceSet,
    Swap,
    BatchEnd,
    /// Any other kind (or a header too short to hold one).
    Other,
}

/// [`Peek`] of the frame `frame` (its header first; nothing is validated past the kind and a call's entry).
pub fn peek(frame: &[u8]) -> Peek {
    let Some(k) = frame.get(2..4).map(|b| u16::from_le_bytes([b[0], b[1]])) else { return Peek::Other };
    match k {
        KIND_CALL => Peek::Call(frame.get(HEADER..HEADER + 2).map(|b| u16::from_le_bytes([b[0], b[1]]))),
        KIND_DRAW => Peek::Draw,
        KIND_FENCE_SET => Peek::FenceSet,
        KIND_SWAP => Peek::Swap,
        KIND_BATCH_END => Peek::BatchEnd,
        _ => Peek::Other,
    }
}

/// Total byte length of the frame at the front of `bytes` once its 16-byte header is present: `Ok(None)` = the header is
/// not complete yet, so a streaming reader must wait for more bytes; the ABI version is checked as soon as it is seen.
pub fn peek_len(bytes: &[u8]) -> Result<Option<usize>, WireError> {
    if bytes.len() >= 2 {
        let v = u16::from_le_bytes([bytes[0], bytes[1]]);
        if v != ABI_VERSION {
            return Err(WireError::Version { got: v });
        }
    }
    if bytes.len() < HEADER {
        return Ok(None);
    }
    let len = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    HEADER.checked_add(len).map(Some).ok_or(WireError::Overflow)
}

/// Start a frame at the end of `out`: its header with the payload length still 0 ([`finish`] patches it).
fn begin(out: &mut Vec<u8>, kind: u16, token: Token) -> (Writer, usize) {
    let start = out.len();
    let mut p = Writer { buf: core::mem::take(out) };
    p.u16(ABI_VERSION);
    p.u16(kind);
    p.u32(0);
    p.u32(token.slot);
    p.u32(token.incarnation);
    (p, start)
}

fn finish(mut p: Writer, start: usize, out: &mut Vec<u8>) {
    let len = (p.buf.len() - start - HEADER) as u32;
    p.buf[start + 4..start + 8].copy_from_slice(&len.to_le_bytes());
    *out = p.buf;
}

fn call_payload(p: &mut Writer, entry: u16, args: &[u64]) {
    p.u16(entry);
    p.u16(0);
    p.u32(args.len() as u32);
    for a in args {
        p.u64(*a);
    }
}

impl Frame {
    /// The frame's bytes as a new vector (the same bytes [`Frame::encode_into`] appends).
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.encode_into(&mut out);
        out
    }

    /// Append the frame's bytes to `out`: the header first with its length patched once the payload is written, so a
    /// queue of frames grows in place with no per-frame buffer (the engine encodes thousands of GL calls a frame).
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        let (mut p, start) = begin(out, kind_of(&self.command), self.token);
        match &self.command {
            Command::ContextCreate | Command::ContextDestroy | Command::MakeCurrent => {}
            Command::Call { entry, args } => call_payload(&mut p, *entry, args),
            Command::CallData { entry, args, data } => {
                p.u16(*entry);
                p.u16(0);
                p.u32(args.len() as u32);
                for a in args {
                    p.u64(*a);
                }
                p.bytes(data);
            }
            Command::ReplyData { status, words, data } => {
                p.u32(*status);
                p.u32(words.len() as u32);
                for w in words {
                    p.u64(*w);
                }
                p.bytes(data);
            }
            Command::Draw(d) => {
                p.u32(d.mode);
                p.u32(d.indices.ty as u32);
                p.u32(d.indices.count);
                p.u32(d.arrays.len() as u32);
                p.bytes(&d.indices.data);
                for a in &d.arrays {
                    p.u8(a.role as u8);
                    p.u8(a.unit);
                    p.u8(a.size);
                    p.u8(0);
                    p.u32(a.ty as u32);
                    p.u32(a.first);
                    p.u32(a.count);
                    p.u32(a.data.len() as u32);
                    p.bytes(&a.data);
                }
            }
            Command::FenceSet { id } => p.u32(*id),
            Command::Swap { drawable } => {
                p.u32(drawable.slot);
                p.u32(drawable.incarnation);
            }
            Command::Refused { text } => p.bytes(text.as_bytes()),
            Command::Reply { status, words, text } => {
                p.u32(*status);
                p.u32(words.len() as u32);
                for w in words {
                    p.u64(*w);
                }
                p.bytes(text.as_bytes());
            }
            Command::BatchEnd { seq } | Command::Retired { seq } => p.u64(*seq),
        }
        finish(p, start, out);
    }

    /// Append a `Command::Call { entry, args }` frame for `token` from borrowed words - the same bytes as encoding that frame, with
    /// no `Vec` of arguments to build first (the engine's per-call path: ~5,000 a Warcraft III frame, so the frame is written
    /// with one capacity check and its length known up front instead of a check per field and a patch).
    pub fn encode_call_into(out: &mut Vec<u8>, token: Token, entry: u16, args: &[u64]) {
        let len = 8 + 8 * args.len();
        out.reserve(HEADER + len);
        let mut h = [0u8; HEADER + 8];
        h[0..2].copy_from_slice(&ABI_VERSION.to_le_bytes());
        h[2..4].copy_from_slice(&KIND_CALL.to_le_bytes());
        h[4..8].copy_from_slice(&(len as u32).to_le_bytes());
        h[8..12].copy_from_slice(&token.slot.to_le_bytes());
        h[12..16].copy_from_slice(&token.incarnation.to_le_bytes());
        h[16..18].copy_from_slice(&entry.to_le_bytes());
        h[20..24].copy_from_slice(&(args.len() as u32).to_le_bytes());
        out.extend_from_slice(&h);
        for a in args {
            out.extend_from_slice(&a.to_le_bytes());
        }
    }

    /// Capture a `glDrawElements(mode, count, ty, indices at index_off)` out of guest memory `mem` STRAIGHT INTO its frame at the end
    /// of `out`: the same bytes as `Frame { token, command: Command::Draw(DrawCommand::capture(..)) }.encode_into(out)`, written
    /// once - each index and array range is copied from guest memory into the frame and converted to the wire order there, and the
    /// index range is found in the same pass (no snapshot buffers, no second copy: a Warcraft III frame's ~300 draws carry
    /// ~470 KB). On an error `out` is as it was.
    pub fn encode_draw_elements_into(out: &mut Vec<u8>, token: Token, mem: &[u8], mode: u32, index_off: usize, count: u32, ty: ElemType, arrays: &[Descriptor]) -> Result<(), SnapshotError> {
        let start = out.len();
        let r = encode_draw_body(out, token, mem, mode, index_off, count, ty, arrays);
        if r.is_err() {
            out.truncate(start);
        }
        r
    }

    /// Decode ONE frame from the front of `bytes`; returns it and the bytes consumed.
    pub fn decode(bytes: &[u8]) -> Result<(Frame, usize), WireError> {
        let mut r = Reader::new(bytes);
        let version = r.u16()?;
        if version != ABI_VERSION {
            return Err(WireError::Version { got: version });
        }
        let kind = r.u16()?;
        let len = r.u32()? as usize;
        let token = Token { slot: r.u32()?, incarnation: r.u32()? };
        let mut p = Reader::new(r.take(len)?);
        let command = match kind {
            1 => Command::ContextCreate,
            2 => Command::ContextDestroy,
            3 => Command::MakeCurrent,
            KIND_CALL => {
                let entry = p.u16()?;
                p.u16()?;
                let n = p.u32()? as usize;
                // Size the allocation from bytes actually present, never from the declared count.
                if n.checked_mul(8).ok_or(WireError::Overflow)? != p.remaining() {
                    return Err(WireError::LengthMismatch);
                }
                let mut args = Vec::with_capacity(n);
                for _ in 0..n {
                    args.push(p.u64()?);
                }
                Command::Call { entry, args }
            }
            KIND_DRAW => Command::Draw(decode_draw(&mut p)?),
            KIND_FENCE_SET => Command::FenceSet { id: p.u32()? },
            KIND_SWAP => Command::Swap { drawable: Token { slot: p.u32()?, incarnation: p.u32()? } },
            8 => {
                let t = p.take(p.remaining())?;
                Command::Refused { text: String::from_utf8(t.to_vec()).map_err(|_| WireError::BadValue("refusal text is not UTF-8"))? }
            }
            9 => {
                let status = p.u32()?;
                let n = p.u32()? as usize;
                if n.checked_mul(8).ok_or(WireError::Overflow)? > p.remaining() {
                    return Err(WireError::LengthMismatch);
                }
                let mut words = Vec::with_capacity(n);
                for _ in 0..n {
                    words.push(p.u64()?);
                }
                let t = p.take(p.remaining())?;
                Command::Reply { status, words, text: String::from_utf8(t.to_vec()).map_err(|_| WireError::BadValue("reply text is not UTF-8"))? }
            }
            KIND_BATCH_END => Command::BatchEnd { seq: p.u64()? },
            11 => Command::Retired { seq: p.u64()? },
            12 => {
                let entry = p.u16()?;
                p.u16()?;
                let n = p.u32()? as usize;
                // Size the allocation from bytes actually present, never from the declared count.
                if n.checked_mul(8).ok_or(WireError::Overflow)? > p.remaining() {
                    return Err(WireError::LengthMismatch);
                }
                let mut args = Vec::with_capacity(n);
                for _ in 0..n {
                    args.push(p.u64()?);
                }
                Command::CallData { entry, args, data: p.take(p.remaining())?.to_vec() }
            }
            13 => {
                let status = p.u32()?;
                let n = p.u32()? as usize;
                if n.checked_mul(8).ok_or(WireError::Overflow)? > p.remaining() {
                    return Err(WireError::LengthMismatch);
                }
                let mut words = Vec::with_capacity(n);
                for _ in 0..n {
                    words.push(p.u64()?);
                }
                Command::ReplyData { status, words, data: p.take(p.remaining())?.to_vec() }
            }
            k => return Err(WireError::UnknownKind(k)),
        };
        if p.remaining() != 0 {
            return Err(WireError::LengthMismatch);
        }
        Ok((Frame { token, command }, HEADER + len))
    }
}

/// [`Frame::encode_draw_elements_into`] without the rollback.
#[allow(clippy::too_many_arguments)]
fn encode_draw_body(out: &mut Vec<u8>, token: Token, mem: &[u8], mode: u32, index_off: usize, count: u32, ty: ElemType, arrays: &[Descriptor]) -> Result<(), SnapshotError> {
    if !matches!(ty, ElemType::U8 | ElemType::U16 | ElemType::U32) {
        return Err(SnapshotError::BadIndexType);
    }
    let ibytes = (count as usize).checked_mul(ty.bytes()).ok_or(SnapshotError::Overflow)?;
    let iend = index_off.checked_add(ibytes).ok_or(SnapshotError::Overflow)?;
    if iend > mem.len() {
        return Err(SnapshotError::OutOfRange);
    }
    let start = out.len();
    let mut h = [0u8; HEADER + 16];
    h[0..2].copy_from_slice(&ABI_VERSION.to_le_bytes());
    h[2..4].copy_from_slice(&KIND_DRAW.to_le_bytes());
    h[8..12].copy_from_slice(&token.slot.to_le_bytes());
    h[12..16].copy_from_slice(&token.incarnation.to_le_bytes());
    h[16..20].copy_from_slice(&mode.to_le_bytes());
    h[20..24].copy_from_slice(&(ty as u32).to_le_bytes());
    h[24..28].copy_from_slice(&count.to_le_bytes());
    // the array count (h[28..32]) is patched below: none when the draw has no index
    out.extend_from_slice(&h);
    let at = out.len();
    out.extend_from_slice(&mem[index_off..iend]);
    let range = indices_to_wire(ty, &mut out[at..]);
    if let Some((lo, hi)) = range {
        let n = (hi - lo).checked_add(1).ok_or(SnapshotError::Overflow)?;
        for d in arrays {
            encode_array(out, mem, d, lo, n)?;
        }
        out[start + 28..start + 32].copy_from_slice(&(arrays.len() as u32).to_le_bytes());
    }
    let len = (out.len() - start - HEADER) as u32;
    out[start + 4..start + 8].copy_from_slice(&len.to_le_bytes());
    Ok(())
}

/// The (min, max) of little-endian indices of type `ty`; `None` when there are none. A plain min/max loop per width (the
/// worker decodes ~300 draws a Warcraft III frame: an `Option` fold per index was ~0.6M host instructions a frame).
pub fn index_range(ty: ElemType, b: &[u8]) -> Option<(u32, u32)> {
    if b.is_empty() {
        return None;
    }
    let (mut lo, mut hi) = (u32::MAX, 0u32);
    match ty {
        ElemType::U8 => b.iter().for_each(|&v| { lo = lo.min(u32::from(v)); hi = hi.max(u32::from(v)); }),
        ElemType::U16 => b.chunks_exact(2).for_each(|c| { let v = u32::from(u16::from_le_bytes([c[0], c[1]])); lo = lo.min(v); hi = hi.max(v); }),
        _ => b.chunks_exact(4).for_each(|c| { let v = u32::from_le_bytes([c[0], c[1], c[2], c[3]]); lo = lo.min(v); hi = hi.max(v); }),
    }
    (lo <= hi).then_some((lo, hi))
}

/// Convert indices in place from the guest's order to the wire's and return their (min, max) - one pass.
fn indices_to_wire(ty: ElemType, b: &mut [u8]) -> Option<(u32, u32)> {
    if b.is_empty() {
        return None;
    }
    let (mut lo, mut hi) = (u32::MAX, 0u32);
    match ty {
        ElemType::U8 => b.iter().for_each(|&v| { lo = lo.min(u32::from(v)); hi = hi.max(u32::from(v)); }),
        ElemType::U16 => b.chunks_exact_mut(2).for_each(|c| {
            let v = u16::from_be_bytes([c[0], c[1]]);
            c.copy_from_slice(&v.to_le_bytes());
            lo = lo.min(u32::from(v));
            hi = hi.max(u32::from(v));
        }),
        _ => b.chunks_exact_mut(4).for_each(|c| {
            let v = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
            c.copy_from_slice(&v.to_le_bytes());
            lo = lo.min(v);
            hi = hi.max(v);
        }),
    }
    Some((lo, hi))
}

/// One array's frame record and its vertices `first .. first+count` at the end of `out`, as `snapshot_array_into` + the draw's
/// encoding would write them (the same checks, in the same order).
fn encode_array(out: &mut Vec<u8>, mem: &[u8], d: &Descriptor, first: u32, count: u32) -> Result<(), SnapshotError> {
    if !(1..=4).contains(&d.size) {
        return Err(SnapshotError::BadSize(d.size));
    }
    let vbytes = (d.size as usize).checked_mul(d.ty.bytes()).ok_or(SnapshotError::Overflow)?;
    let stride = if d.stride == 0 { vbytes } else { d.stride as usize };
    let total = (count as usize).checked_mul(vbytes).ok_or(SnapshotError::Overflow)?;
    let begin = (first as usize).checked_mul(stride).and_then(|o| o.checked_add(d.offset)).ok_or(SnapshotError::Overflow)?;
    let last = (count as usize - 1).checked_mul(stride).and_then(|o| o.checked_add(begin)).ok_or(SnapshotError::Overflow)?;
    let end = last.checked_add(vbytes).ok_or(SnapshotError::Overflow)?;
    if end > mem.len() {
        return Err(SnapshotError::OutOfRange);
    }
    let mut rec = [0u8; 20];
    rec[0] = d.role as u8;
    rec[1] = d.unit;
    rec[2] = d.size;
    rec[4..8].copy_from_slice(&(d.ty as u32).to_le_bytes());
    rec[8..12].copy_from_slice(&first.to_le_bytes());
    rec[12..16].copy_from_slice(&count.to_le_bytes());
    rec[16..20].copy_from_slice(&(total as u32).to_le_bytes());
    out.reserve(rec.len() + total);
    out.extend_from_slice(&rec);
    let at = out.len();
    if stride == vbytes {
        out.extend_from_slice(&mem[begin..end]);
    } else {
        for i in 0..count as usize {
            let a = begin + i * stride; // bounded by the proof above
            out.extend_from_slice(&mem[a..a + vbytes]);
        }
    }
    crate::snapshot::to_wire_order(d.ty, &mut out[at..]);
    Ok(())
}

fn decode_draw(p: &mut Reader) -> Result<DrawCommand, WireError> {
    let mode = p.u32()?;
    if mode > GL_POLYGON {
        return Err(WireError::BadValue("draw mode"));
    }
    let ity = ElemType::from_gl(p.u32()?).filter(|t| matches!(t, ElemType::U8 | ElemType::U16 | ElemType::U32)).ok_or(WireError::BadValue("index type"))?;
    let icount = p.u32()?;
    let narrays = p.u32()? as usize;
    let ilen = (icount as usize).checked_mul(ity.bytes()).ok_or(WireError::Overflow)?;
    let idata = p.take(ilen)?.to_vec();
    let mut arrays = Vec::new();
    for _ in 0..narrays {
        let role = Role::from_u8(p.u8()?).ok_or(WireError::BadValue("array role"))?;
        let unit = p.u8()?;
        let size = p.u8()?;
        p.u8()?;
        let ty = ElemType::from_gl(p.u32()?).ok_or(WireError::BadValue("array type"))?;
        let first = p.u32()?;
        let count = p.u32()?;
        let dlen = p.u32()? as usize;
        if !(1..=4).contains(&size) {
            return Err(WireError::BadValue("array size"));
        }
        let want = (count as usize).checked_mul(size as usize).and_then(|n| n.checked_mul(ty.bytes())).ok_or(WireError::Overflow)?;
        if dlen != want {
            return Err(WireError::LengthMismatch);
        }
        arrays.push(ArraySnapshot { role, unit, size, ty, first, count, data: p.take(dlen)?.to_vec() });
    }
    let range = index_range(ity, &idata);
    Ok(DrawCommand { mode, indices: crate::snapshot::IndexSnapshot { ty: ity, count: icount, data: idata, range }, arrays })
}
