//! Ring frames. Layout, every field little-endian:
//! `u16 abi_version | u16 kind | u32 payload_len | u32 token.slot | u32 token.incarnation | payload`.
//! The version is FIRST so a reader refuses an unknown layout before it parses anything else.
use crate::snapshot::{snapshot_array, snapshot_indices, ArraySnapshot, Descriptor, ElemType, Role, SnapshotError};
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
        let indices = snapshot_indices(mem, index_off, count, ty)?;
        let mut snaps = Vec::new();
        if let Some((lo, hi)) = indices.range {
            let n = (hi - lo).checked_add(1).ok_or(SnapshotError::Overflow)?;
            for d in arrays {
                snaps.push(snapshot_array(mem, d, lo, n)?);
            }
        }
        Ok(DrawCommand { mode, indices, arrays: snaps })
    }
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

fn kind_of(c: &Command) -> u16 {
    match c {
        Command::ContextCreate => 1,
        Command::ContextDestroy => 2,
        Command::MakeCurrent => 3,
        Command::Call { .. } => 4,
        Command::Draw(_) => 5,
        Command::FenceSet { .. } => 6,
        Command::Swap { .. } => 7,
        Command::Refused { .. } => 8,
        Command::Reply { .. } => 9,
        Command::BatchEnd { .. } => 10,
        Command::Retired { .. } => 11,
        Command::CallData { .. } => 12,
        Command::ReplyData { .. } => 13,
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

impl Frame {
    pub fn encode(&self) -> Vec<u8> {
        let mut p = Writer::default();
        match &self.command {
            Command::ContextCreate | Command::ContextDestroy | Command::MakeCurrent => {}
            Command::Call { entry, args } => {
                p.u16(*entry);
                p.u16(0);
                p.u32(args.len() as u32);
                for a in args {
                    p.u64(*a);
                }
            }
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
        let mut w = Writer::default();
        w.u16(ABI_VERSION);
        w.u16(kind_of(&self.command));
        w.u32(p.buf.len() as u32);
        w.u32(self.token.slot);
        w.u32(self.token.incarnation);
        w.bytes(&p.buf);
        w.buf
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
            4 => {
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
            5 => Command::Draw(decode_draw(&mut p)?),
            6 => Command::FenceSet { id: p.u32()? },
            7 => Command::Swap { drawable: Token { slot: p.u32()?, incarnation: p.u32()? } },
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
            10 => Command::BatchEnd { seq: p.u64()? },
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
    let range = idata.chunks_exact(ity.bytes()).map(|c| match ity {
        ElemType::U8 => u32::from(c[0]),
        ElemType::U16 => u32::from(u16::from_le_bytes([c[0], c[1]])),
        _ => u32::from_le_bytes([c[0], c[1], c[2], c[3]]),
    }).fold(None, |a: Option<(u32, u32)>, v| Some(a.map_or((v, v), |(lo, hi)| (lo.min(v), hi.max(v)))));
    Ok(DrawCommand { mode, indices: crate::snapshot::IndexSnapshot { ty: ity, count: icount, data: idata, range }, arrays })
}
