//! Fault / PrintFault / DosGetString: AmigaDOS error numbers to their standard messages.
//! The message texts are the user-visible strings every AmigaDOS prints for these codes (interface facts).
use alloc::format;
use alloc::string::String;
use crate::dos_err::*;
use crate::fs::*;
use crate::libtable::LibraryBuilder;
use crate::{machine::Amiga, Refusal};
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_68k::Cpu;

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("Fault", fault::<U>).named("PrintFault", print_fault::<U>)
}

/// The standard message of an error code, if AmigaDOS has one.
pub fn message(code: i32) -> Option<&'static str> {
    Some(match code {
        103 => "not enough memory available", 105 => "process table full", 114 => "bad template", 115 => "bad number",
        116 => "required argument missing", 117 => "value after keyword missing", 118 => "wrong number of arguments",
        119 => "unmatched quotes", 120 => "argument line invalid or too long", 121 => "file is not executable",
        122 => "invalid resident library", 202 => "object in use", 203 => "object already exists", 204 => "directory not found",
        205 => "object not found", 206 => "invalid window description", 209 => "packet request type unknown",
        210 => "object name invalid", 211 => "invalid object lock", 212 => "object is not of required type",
        213 => "disk not validated", 214 => "disk is write-protected", 215 => "rename across devices attempted",
        216 => "directory not empty", 217 => "too many levels", 218 => "device (or volume) is not mounted", 219 => "seek failure",
        220 => "comment is too long", 221 => "disk is full", 222 => "object is protected from deletion", 223 => "file is write protected",
        224 => "file is read protected", 225 => "not a valid DOS disk", 226 => "no disk in drive", 232 => "no more entries in directory",
        233 => "object is soft link", 234 => "object is linked", 235 => "bad loadfile hunk", 236 => "function not implemented",
        240 => "record not locked", 241 => "record lock collision", 242 => "record lock timeout", 243 => "record unlock error",
        303 => "buffer overflow", 304 => "***Break",
        _ => return None,
    })
}

/// "header: message", or "Error N" for a code with no message; empty for 0.
pub fn text(code: i32, header: Option<&str>) -> String {
    if code == 0 { return String::new(); }
    let msg = message(code).map_or_else(|| format!("Error {code}"), String::from);
    match header { Some(h) => format!("{h}: {msg}"), None => msg }
}

/// Fault(d1 = code, d2 = header or NULL, d3 = buffer, d4 = length) -> characters written.
fn fault<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (code, hp, buf, len) = (cpu.dreg(1) as i32, cpu.dreg(2), cpu.dreg(3), cpu.dreg(4));
    let header = if hp == 0 { None } else { Some(str_arg(a, hp, "Fault.header")?) };
    let t = text(code, header.as_deref());
    if len == 0 { cpu.set_d0(0); return Ok(format!("[dos] Fault {code}: no room")); }
    let n = t.len().min(len as usize - 1);
    crate::guest_str::write_z(&mut a.mem, buf, &t.as_bytes()[..n]).map_err(Refusal::mem("Fault.buffer"))?;
    cpu.set_d0(n as u32);
    Ok(format!("[dos] Fault {code} -> \"{}\"", &t[..n]))
}

/// PrintFault(d1 = code, d2 = header or NULL) -> DOSTRUE: the message and a newline to the console; IoErr becomes `code`.
fn print_fault<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (code, hp) = (cpu.dreg(1) as i32, cpu.dreg(2));
    let header = if hp == 0 { None } else { Some(str_arg(a, hp, "PrintFault.header")?) };
    let mut t = text(code, header.as_deref());
    t.push('\n');
    crate::dos::emit(a, t.as_bytes());
    set_ioerr(&mut a.mem, DosErr(code as u32));
    cpu.set_d0(DOSTRUE);
    Ok(format!("[dos] PrintFault {code}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn messages_are_standard_and_unknown_codes_say_so() {
        assert_eq!(text(205, Some("Copy")), "Copy: object not found");
        assert_eq!(text(205, None), "object not found");
        assert_eq!(text(0, Some("x")), "");
        assert_eq!(text(9999, None), "Error 9999");
    }
}
