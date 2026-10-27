//! `TDes16::Format` / `AppendFormat` (e32des16.h): the Symbian descriptor format language over a guest argument list (`...`: r2, r3, then
//! the stack from the caller's sp). Supported: `%d %i %u %x %X %o %c %S %%`, flags `0` `-`, a width (digits or `*`), the `l` length
//! modifier (the arguments are 32-bit already) and `L` for 64-bit `d u x X`. `%s` (a zero-terminated string), `%f/%e/%g` are named refusals until a guest
//! uses them: no guessing at a layout this engine has not seen.
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use crate::loader::Process;
use crate::natives::NativeOutcome;

/// The variable arguments of an ARM EABI call: after the fixed arguments the registers r2, r3 (here, `this` and the format take r0, r1),
/// then the stack words from the caller's sp.
struct Args { next: u32, sp: u32 }

impl Args {
    fn word(&mut self, p: &Process) -> Result<u32, String> {
        let v = match self.next {
            2 => p.cpu.regs[2],
            3 => p.cpu.regs[3],
            n => p.rd(self.sp.wrapping_add((n - 4) * 4))?,
        };
        self.next += 1;
        Ok(v)
    }

    /// A 64-bit argument (`%Ld`): an even-aligned register or stack pair (AAPCS 5.5 rule C.3), low word first.
    fn dword(&mut self, p: &Process) -> Result<u64, String> {
        self.next = (self.next + 1) & !1;
        let lo = self.word(p)?;
        Ok((self.word(p)? as u64) << 32 | lo as u64)
    }
}

fn pad(out: &mut Vec<u16>, text: &[u16], width: usize, zero: bool, left: bool) {
    let fill = width.saturating_sub(text.len());
    if left {
        out.extend_from_slice(text);
        out.extend(core::iter::repeat_n(b' ' as u16, fill));
    } else if zero && text.first().is_some_and(|&c| c == b'-' as u16) {
        out.push(b'-' as u16);
        out.extend(core::iter::repeat_n(b'0' as u16, fill));
        out.extend_from_slice(&text[1..]);
    } else {
        out.extend(core::iter::repeat_n(if zero { b'0' } else { b' ' } as u16, fill));
        out.extend_from_slice(text);
    }
}

/// Expand `fmt` over the guest arguments starting at argument slot `first`.
fn expand(p: &Process, fmt: &[u16], first: u32) -> Result<Vec<u16>, String> {
    let mut args = Args { next: first, sp: p.cpu.regs[13] };
    let mut out = Vec::new();
    let mut i = 0;
    while i < fmt.len() {
        let c = fmt[i];
        i += 1;
        if c != b'%' as u16 {
            out.push(c);
            continue;
        }
        let (mut zero, mut left, mut width) = (false, false, 0usize);
        while let Some(&f) = fmt.get(i) {
            match f as u8 as char {
                '0' if f < 128 => zero = true,
                '-' if f < 128 => left = true,
                _ => break,
            }
            i += 1;
        }
        if fmt.get(i) == Some(&(b'*' as u16)) {
            width = args.word(p)? as usize;
            i += 1;
        }
        while let Some(&d) = fmt.get(i).filter(|&&d| (b'0' as u16..=b'9' as u16).contains(&d)) {
            width = width * 10 + (d - b'0' as u16) as usize;
            i += 1;
        }
        let mut long64 = false;
        while let Some(&m) = fmt.get(i).filter(|&&m| m == b'l' as u16 || m == b'L' as u16) {
            long64 |= m == b'L' as u16;
            i += 1;
        }
        let t = *fmt.get(i).ok_or_else(|| String::from("a format string ends inside a % specification"))?;
        i += 1;
        let t = char::from_u32(t as u32).unwrap_or('?');
        let text: Vec<u16> = match t {
            // `%LD`: FileBrowser's own free/total-size format ("%LD / %LD kB") on two TInt64 values. Read as the signed 64-bit decimal its use implies;
            // no euser source is available here to confirm the capital. DELETE WHEN the euser formatter's directive table is read.
            'd' | 'D' | 'i' if long64 => format!("{}", args.dword(p)? as i64).encode_utf16().collect(),
            'u' if long64 => format!("{}", args.dword(p)?).encode_utf16().collect(),
            'x' if long64 => format!("{:x}", args.dword(p)?).encode_utf16().collect(),
            'X' if long64 => format!("{:X}", args.dword(p)?).encode_utf16().collect(),
            '%' => alloc::vec![b'%' as u16],
            'd' | 'i' => format!("{}", args.word(p)? as i32).encode_utf16().collect(),
            'u' => format!("{}", args.word(p)?).encode_utf16().collect(),
            'x' => format!("{:x}", args.word(p)?).encode_utf16().collect(),
            'X' => format!("{:X}", args.word(p)?).encode_utf16().collect(),
            'o' => format!("{:o}", args.word(p)?).encode_utf16().collect(),
            'c' => alloc::vec![args.word(p)? as u16],
            'S' => { let a = args.word(p)?; crate::desc::read16(p, a)? }
            other => return Err(format!("format directive %{other} in {:?} is not implemented (no guest has used it yet)", crate::desc::to_utf8(fmt))),
        };
        pad(&mut out, &text, width, zero && t != 'S' && t != 'c', left);
    }
    Ok(out)
}

pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let append = match name {
        "_ZN6TDes166FormatE11TRefByValueIK7TDesC16Ez" => false,
        "_ZN6TDes1612AppendFormatE11TRefByValueIK7TDesC16Ez" => true,
        _ => return None,
    };
    let (this, fmt) = (p.cpu.regs[0], p.cpu.regs[1]);
    let r = crate::desc::read16(p, fmt).and_then(|f| expand(p, &f, 2)).and_then(|mut text| {
        if append {
            let mut all = crate::desc::read16(p, this)?;
            all.append(&mut text);
            text = all;
        }
        crate::desc::write16(p, this, &text)
    });
    Some(match r {
        Ok(()) => NativeOutcome::Return,
        Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {m}")),
    })
}
