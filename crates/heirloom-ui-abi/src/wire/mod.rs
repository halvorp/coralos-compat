//! Versioned, bounded wire encoding for guest UI calls and native events.
//!
//! The codec owns bytes only: transport, framing across a stream, and execution of the
//! decoded operations belong to its caller. All integer fields are little-endian.

use alloc::{string::String, vec::Vec};

use crate::{
    ArrowDir, AudioFormat, AudioStreamModel, CallbackToken, ChoiceStyle, ControlKind,
    ControlModel, ControlValue, DrawOp, FontRole, GuestPoint, GuestRect, GuestUiEvent,
    GroupStyle, IconRole, Intent, Menu, MenuItem, MenuModel, Modifiers, PenRole, Plane,
    PointerButton, PointerPhase, RequestKind, RequestModel, UiCall, UiObjectRef, UiRefusal,
    WindowModel, WindowRole,
};

pub const WIRE_VERSION: u8 = 1;
pub const MAX_FRAME: u32 = 1 << 20;

const MAGIC: u16 = 0x5549;
const HEADER: usize = 8;
const KIND_CALL: u8 = 1;
const KIND_EVENT: u8 = 2;
const KIND_REFUSED: u8 = 3;
const KIND_LEASE: u8 = 4;
// Kind 5 is reserved for the later host-profile frame.
const KIND_CONTROL: u8 = 6;
const KIND_REPORT: u8 = 7;

// Structs carry a type tag; an enum's variant tag identifies its value and shape.
const T_OBJECT: u8 = 0x01;
const T_TOKEN: u8 = 0x02;
const T_RECT: u8 = 0x03;
const T_WINDOW: u8 = 0x04;
const T_MENU_ITEM: u8 = 0x05;
const T_MENU: u8 = 0x06;
const T_MENU_MODEL: u8 = 0x07;
const T_CONTROL: u8 = 0x08;
const T_REQUEST: u8 = 0x09;
const T_AUDIO: u8 = 0x0a;
const T_POINT: u8 = 0x0b;
const T_PLANE: u8 = 0x0c;
const T_MODIFIERS: u8 = 0x0d;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    Call(UiCall),
    Event(GuestUiEvent),
    /// A late host refusal of the call `token` named. Its reason is OWNED (a host's text crossing a process boundary is
    /// data, never a `&'static str` this side would have to leak to hold); the proxy maps it onto a `UiRefusal` and logs it.
    Refused { token: CallbackToken, why: WireRefusal },
    Lease { window: UiObjectRef, region: u32, generation: u32 },
    /// A host -> engine machine-session call ([`session`]).
    Control(Control),
    /// An engine -> host machine-session answer ([`session`]).
    Report(Report),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireError {
    BadMagic,
    BadVersion(u8),
    UnknownKind(u8),
    TooLong { what: &'static str, len: u32 },
    Truncated(&'static str),
    UnknownTag { what: &'static str, tag: u8 },
    BadUtf8(&'static str),
    TrailingBytes,
}

impl WireError {
    pub fn name(&self) -> &'static str {
        match self {
            Self::BadMagic => "BadMagic",
            Self::BadVersion(_) => "BadVersion",
            Self::UnknownKind(_) => "UnknownKind",
            Self::TooLong { .. } => "TooLong",
            Self::Truncated(_) => "Truncated",
            Self::UnknownTag { .. } => "UnknownTag",
            Self::BadUtf8(_) => "BadUtf8",
            Self::TrailingBytes => "TrailingBytes",
        }
    }
}

// UiRefusal deliberately retains its public &'static str fields. Intern decoded
// reasons instead of leaking one allocation per frame; duplicate reasons are reused,
// and both storage and the number of distinct reasons have hard limits.


fn put_len(out: &mut Vec<u8>, len: usize) {
    assert!(len <= u32::MAX as usize, "wire collection exceeds u32");
    out.extend_from_slice(&(len as u32).to_le_bytes());
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_i32(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_bool(out: &mut Vec<u8>, value: bool) {
    out.push(value as u8);
}

fn put_usize(out: &mut Vec<u8>, value: usize) {
    put_u64(out, value as u64);
}

fn put_string(out: &mut Vec<u8>, value: &str) {
    put_len(out, value.len());
    out.extend_from_slice(value.as_bytes());
}

fn put_vec<T>(
    out: &mut Vec<u8>,
    values: &[T],
    mut put: impl FnMut(&mut Vec<u8>, &T),
) {
    put_len(out, values.len());
    for value in values {
        put(out, value);
    }
}

fn put_option<T>(
    out: &mut Vec<u8>,
    value: &Option<T>,
    mut put: impl FnMut(&mut Vec<u8>, &T),
) {
    match value {
        None => out.push(0),
        Some(value) => {
            out.push(1);
            put(out, value);
        }
    }
}

mod call;
mod model;
mod reader;
pub mod session;
pub mod canvas; // the shared canvas arena layout (pixels beside the ring, never on it)
use call::*;
pub use session::{Control, Report, WireStep, BLOB_CHUNK};
use session::*;
pub use model::WireRefusal;
use model::*;
use reader::*;

/// Append one frame. A payload over [`MAX_FRAME`] is refused BY NAME (and nothing is appended), never dropped silently.
fn emit(out: &mut Vec<u8>, kind: u8, payload: Vec<u8>) -> Result<(), WireError> {
    if payload.len() > MAX_FRAME as usize {
        return Err(WireError::TooLong { what: "frame payload", len: u32::try_from(payload.len()).unwrap_or(u32::MAX) });
    }
    out.extend_from_slice(&MAGIC.to_le_bytes());
    out.push(WIRE_VERSION);
    out.push(kind);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&payload);
    Ok(())
}

pub fn encode_call(call: &UiCall, out: &mut Vec<u8>) -> Result<(), WireError> {
    let mut payload = Vec::new();
    put_call(&mut payload, call);
    emit(out, KIND_CALL, payload)
}

pub fn encode_event(event: &GuestUiEvent, out: &mut Vec<u8>) -> Result<(), WireError> {
    let mut payload = Vec::new();
    put_event(&mut payload, event);
    emit(out, KIND_EVENT, payload)
}

pub fn encode_refused(token: CallbackToken, why: &UiRefusal, out: &mut Vec<u8>) -> Result<(), WireError> {
    let mut payload = Vec::new();
    put_token(&mut payload, &token);
    put_refusal(&mut payload, why);
    emit(out, KIND_REFUSED, payload)
}

pub fn encode_lease(window: UiObjectRef, region: u32, generation: u32, out: &mut Vec<u8>) -> Result<(), WireError> {
    let mut payload = Vec::new();
    put_object(&mut payload, &window);
    put_u32(&mut payload, region);
    put_u32(&mut payload, generation);
    emit(out, KIND_LEASE, payload)
}

pub fn encode_control(control: &Control, out: &mut Vec<u8>) -> Result<(), WireError> {
    let mut payload = Vec::new();
    put_machine_control(&mut payload, control);
    emit(out, KIND_CONTROL, payload)
}

pub fn encode_report(report: &Report, out: &mut Vec<u8>) -> Result<(), WireError> {
    let mut payload = Vec::new();
    put_machine_report(&mut payload, report);
    emit(out, KIND_REPORT, payload)
}

pub fn decode(bytes: &[u8]) -> Result<(Frame, usize), WireError> {
    if bytes.len() < HEADER {
        return Err(WireError::Truncated("frame header"));
    }
    if u16::from_le_bytes([bytes[0], bytes[1]]) != MAGIC {
        return Err(WireError::BadMagic);
    }
    if bytes[2] != WIRE_VERSION {
        return Err(WireError::BadVersion(bytes[2]));
    }
    let kind = bytes[3];
    let len = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    if len > MAX_FRAME {
        return Err(WireError::TooLong { what: "frame", len });
    }
    let end = HEADER + len as usize;
    if bytes.len() < end {
        return Err(WireError::Truncated("frame payload"));
    }
    let mut reader = Reader::new(&bytes[HEADER..end]);
    let frame = match kind {
        KIND_CALL => Frame::Call(get_call(&mut reader)?),
        KIND_EVENT => Frame::Event(get_event(&mut reader)?),
        KIND_REFUSED => Frame::Refused {
            token: get_token(&mut reader)?,
            why: get_refusal(&mut reader)?,
        },
        KIND_LEASE => Frame::Lease {
            window: get_object(&mut reader)?,
            region: reader.u32()?,
            generation: reader.u32()?,
        },
        KIND_CONTROL => Frame::Control(get_machine_control(&mut reader)?),
        KIND_REPORT => Frame::Report(get_machine_report(&mut reader)?),
        _ => return Err(WireError::UnknownKind(kind)),
    };
    if reader.remaining() != 0 {
        return Err(WireError::TrailingBytes);
    }
    Ok((frame, end))
}
pub mod proxy;
pub mod host;
pub mod loopback;
