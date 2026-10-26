//! `MUIM_SetAsString(attr, format, values...)` (Notify.mui): `RawDoFmt` the format over the values, then `set(obj, attr, <the text>)`,
//! TRUE on success. Facts: Zune notify.c:697-725 (the text is an AllocVec block freed right after the set, so what `attr` stores is a
//! copy: a text attribute keeps its own). The message is `{ id, attr, format, val... }` and the values are RawDoFmt's data stream:
//! 16-bit for a plain conversion, 32-bit for `l`, strings and pointers ([`crate::fmt`]).
use alloc::format;
use alloc::vec::Vec;
use super::{custom, object};
use crate::{fmt, machine::Amiga, Refusal};
use heirloom_ui_abi::GuestUi;

const MAX_FORMAT: usize = 1024;

/// How many longwords follow the method id in a guest-built message at `msg`: `attr`, `format`, and as many longs of values as the
/// format consumes (RawDoFmt's own count of the stream it used, rounded up to longs).
pub fn arity<U: GuestUi>(a: &Amiga<U>, msg: u32) -> Result<u32, Refusal> {
    let word = |i: u32| a.mem.read_u32(msg + 4 * i).map_err(Refusal::mem("MUIM_SetAsString message"));
    let template = crate::guest_str::read_bytes_z(&a.mem, word(2)?, MAX_FORMAT).map_err(Refusal::mem("MUIM_SetAsString format"))?;
    Ok(2 + fmt::format(&a.mem, &template, msg + 12, false)?.used.div_ceil(4))
}

/// `args` are the longwords after the method id: `[attr, format, values...]`.
pub fn set_as_string<U: GuestUi>(a: &mut Amiga<U>, obj: u32, args: &[u32]) -> Result<u32, Refusal> {
    let (Some(&attr), Some(&format_ptr)) = (args.first(), args.get(1)) else { return Err(Refusal("MUIM_SetAsString: the message is short".into())) };
    if custom::own_class(a, obj).is_some() {
        return Err(Refusal("MUIM_SetAsString on an object of the program's own class: its OM_SET runs after the text block is freed".into()));
    }
    let template = crate::guest_str::read_bytes_z(&a.mem, format_ptr, MAX_FORMAT).map_err(Refusal::mem("MUIM_SetAsString format"))?;
    let values = &args[2..];
    let stream = a.heap.alloc(4 * values.len() as u32 + 4).ok_or_else(|| Refusal("MUIM_SetAsString: no memory for the value stream (NOMEM)".into()))?;
    let formatted = values.iter().enumerate().try_for_each(|(i, v)| a.mem.write_u32(stream + 4 * i as u32, *v).map_err(Refusal::mem("MUIM_SetAsString values")))
        .and_then(|()| fmt::format(&a.mem, &template, stream, false));
    let _ = a.heap.free(stream, 4 * values.len() as u32 + 4);
    let text: Vec<u8> = formatted?.bytes;
    let size = text.len() as u32 + 1;
    let Some(block) = a.heap.alloc(size) else { return Ok(0) }; // AllocVec failed: FALSE
    let stored = a.mem.write_bytes(block, &text).and_then(|()| a.mem.write_u8(block + text.len() as u32, 0)).map_err(Refusal::mem("MUIM_SetAsString text"))
        .and_then(|()| object::set_attr(a, obj, attr, block));
    a.heap.free(block, size).map_err(|e| Refusal(format!("MUIM_SetAsString: {e}")))?;
    stored.map(|()| 1)
}
