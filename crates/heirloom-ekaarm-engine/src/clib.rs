//! Symbian C standard library (`estlib.dll` / `libc.dll`): heap, C strings, stdio over the guest `Drive`,
//! `sprintf`/`vsprintf`, and basic `libm` helpers. Pure `no_std + alloc`.
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use crate::loader::Process;
use crate::natives::NativeOutcome;

fn read_cstr(p: &Process, mut addr: u32) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    while out.len() < 0x10_0000 {
        let b = p.mem.vm.read_u8(addr).map_err(|_| format!("cstr at {addr:#x} unreadable"))?;
        if b == 0 {
            return Ok(out);
        }
        out.push(b);
        addr = addr.wrapping_add(1);
    }
    Ok(out)
}

fn write_cstr(p: &mut Process, addr: u32, bytes: &[u8]) -> Result<(), String> {
    p.mem.vm.write_bytes(addr, bytes).map_err(|_| format!("cstr at {addr:#x} unwritable"))?;
    p.mem.vm.write_u8(addr.wrapping_add(bytes.len() as u32), 0).map_err(|_| format!("cstr NUL at {addr:#x} unwritable"))
}

fn next_printf_arg(p: &Process, arg_idx: &mut usize, va_ptr: Option<u32>) -> u32 {
    let idx = *arg_idx;
    *arg_idx += 1;
    if let Some(vp) = va_ptr {
        p.rd(vp.wrapping_add(idx as u32 * 4)).unwrap_or(0)
    } else if idx < 4 {
        p.cpu.regs[idx]
    } else {
        p.rd(p.cpu.regs[13].wrapping_add((idx - 4) as u32 * 4)).unwrap_or(0)
    }
}

fn format_printf(p: &Process, fmt: &[u8], mut arg_idx: usize, va_ptr: Option<u32>) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < fmt.len() {
        if fmt[i] != b'%' {
            out.push(fmt[i]);
            i += 1;
            continue;
        }
        i += 1;
        if i >= fmt.len() { break; }
        if fmt[i] == b'%' { out.push(b'%'); i += 1; continue; }
        let mut zero_pad = false;
        let mut left_align = false;
        while i < fmt.len() && matches!(fmt[i], b'-' | b'+' | b' ' | b'#' | b'0') {
            if fmt[i] == b'0' { zero_pad = true; }
            if fmt[i] == b'-' { left_align = true; }
            i += 1;
        }
        let mut width = 0usize;
        if i < fmt.len() && fmt[i] == b'*' {
            width = (next_printf_arg(p, &mut arg_idx, va_ptr) as i32).max(0) as usize;
            i += 1;
        } else {
            while i < fmt.len() && fmt[i].is_ascii_digit() {
                width = width * 10 + (fmt[i] - b'0') as usize;
                i += 1;
            }
        }
        let mut prec: Option<usize> = None;
        if i < fmt.len() && fmt[i] == b'.' {
            i += 1;
            let mut pr = 0usize;
            if i < fmt.len() && fmt[i] == b'*' {
                pr = (next_printf_arg(p, &mut arg_idx, va_ptr) as i32).max(0) as usize;
                i += 1;
            } else {
                while i < fmt.len() && fmt[i].is_ascii_digit() {
                    pr = pr * 10 + (fmt[i] - b'0') as usize;
                    i += 1;
                }
            }
            prec = Some(pr);
        }
        while i < fmt.len() && matches!(fmt[i], b'l' | b'h' | b'z') {
            i += 1;
        }
        if i >= fmt.len() { break; }
        let spec = fmt[i];
        i += 1;
        let piece: String = match spec {
            b'd' | b'i' => format!("{}", next_printf_arg(p, &mut arg_idx, va_ptr) as i32),
            b'u' => format!("{}", next_printf_arg(p, &mut arg_idx, va_ptr)),
            b'x' => format!("{:x}", next_printf_arg(p, &mut arg_idx, va_ptr)),
            b'X' => format!("{:X}", next_printf_arg(p, &mut arg_idx, va_ptr)),
            b'p' => format!("{:#x}", next_printf_arg(p, &mut arg_idx, va_ptr)),
            b'c' => {
                let c = (next_printf_arg(p, &mut arg_idx, va_ptr) & 0xFF) as u8 as char;
                format!("{c}")
            }
            b's' => {
                let ptr = next_printf_arg(p, &mut arg_idx, va_ptr);
                let mut bytes = if ptr == 0 { b"(null)".to_vec() } else { read_cstr(p, ptr).unwrap_or_default() };
                if let Some(pr) = prec { bytes.truncate(pr); }
                String::from_utf8_lossy(&bytes).into_owned()
            }
            b'f' | b'g' | b'e' => {
                if va_ptr.is_none() && (arg_idx & 1) == 1 { let _ = next_printf_arg(p, &mut arg_idx, va_ptr); }
                let lo = next_printf_arg(p, &mut arg_idx, va_ptr);
                let hi = next_printf_arg(p, &mut arg_idx, va_ptr);
                let v = f64::from_bits(((hi as u64) << 32) | (lo as u64));
                let p_n = prec.unwrap_or(6);
                format!("{v:.p_n$}")
            }
            _ => String::new(),
        };
        let pb = piece.as_bytes();
        if width > pb.len() {
            let pad = width - pb.len();
            let ch = if zero_pad && !left_align { b'0' } else { b' ' };
            if left_align {
                out.extend_from_slice(pb);
                for _ in 0..pad { out.push(b' '); }
            } else {
                for _ in 0..pad { out.push(ch); }
                out.extend_from_slice(pb);
            }
        } else {
            out.extend_from_slice(pb);
        }
    }
    out
}

fn c_file_path(p: &Process, raw: &str) -> String {
    let priv_dir = format!("C:\\private\\{:08x}\\", p.uid3);
    crate::fs::resolve_drive_path(&raw.replace('/', "\\"), &priv_dir)
}

pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let [r0, r1, r2, r3] = [p.cpu.regs[0], p.cpu.regs[1], p.cpu.regs[2], p.cpu.regs[3]];
    let refuse = |m: String| NativeOutcome::Refused(format!("NEXT-NEEDED clib {name}: {m}"));
    Some(match name {
        "malloc" => {
            let Some(h) = p.heap.as_mut() else { return Some(refuse("heap not initialised".into())); };
            p.cpu.regs[0] = h.alloc(r0.max(1), &mut p.mem).unwrap_or(0);
            NativeOutcome::Return
        }
        "calloc" => {
            let total = r0.saturating_mul(r1).max(1);
            p.cpu.regs[0] = p.alloc_zeroed(total).unwrap_or(0);
            NativeOutcome::Return
        }
        "realloc" => {
            if r0 == 0 {
                let Some(h) = p.heap.as_mut() else { return Some(refuse("heap not initialised".into())); };
                p.cpu.regs[0] = h.alloc(r1.max(1), &mut p.mem).unwrap_or(0);
            } else if r1 == 0 {
                if let Some(h) = p.heap.as_mut() { let _ = h.free(r0); }
                p.cpu.regs[0] = 0;
            } else {
                let Some(h) = p.heap.as_mut() else { return Some(refuse("heap not initialised".into())); };
                p.cpu.regs[0] = h.realloc(r0, r1, &mut p.mem).unwrap_or(0);
            }
            NativeOutcome::Return
        }
        "free" => {
            if r0 != 0 {
                if let Some(h) = p.heap.as_mut() { let _ = h.free(r0); }
            }
            NativeOutcome::Return
        }
        "strlen" => {
            match read_cstr(p, r0) {
                Ok(b) => { p.cpu.regs[0] = b.len() as u32; NativeOutcome::Return }
                Err(m) => refuse(m),
            }
        }
        "strcpy" => {
            match read_cstr(p, r1).and_then(|b| write_cstr(p, r0, &b)) {
                Ok(()) => { p.cpu.regs[0] = r0; NativeOutcome::Return }
                Err(m) => refuse(m),
            }
        }
        "strncpy" => {
            let max = r2 as usize;
            let mut src = match read_cstr(p, r1) { Ok(b) => b, Err(m) => return Some(refuse(m)) };
            src.truncate(max);
            src.resize(max, 0);
            if p.mem.vm.write_bytes(r0, &src).is_err() { return Some(refuse(format!("dst {r0:#x} unwritable"))); }
            p.cpu.regs[0] = r0;
            NativeOutcome::Return
        }
        "strcat" => {
            match read_cstr(p, r0).and_then(|mut dst| {
                let src = read_cstr(p, r1)?;
                dst.extend_from_slice(&src);
                write_cstr(p, r0, &dst)
            }) {
                Ok(()) => { p.cpu.regs[0] = r0; NativeOutcome::Return }
                Err(m) => refuse(m),
            }
        }
        "strncat" => {
            let max = r2 as usize;
            match read_cstr(p, r0).and_then(|mut dst| {
                let mut src = read_cstr(p, r1)?;
                src.truncate(max);
                dst.extend_from_slice(&src);
                write_cstr(p, r0, &dst)
            }) {
                Ok(()) => { p.cpu.regs[0] = r0; NativeOutcome::Return }
                Err(m) => refuse(m),
            }
        }
        "strcmp" => {
            match (read_cstr(p, r0), read_cstr(p, r1)) {
                (Ok(a), Ok(b)) => {
                    p.cpu.regs[0] = match a.cmp(&b) {
                        core::cmp::Ordering::Less => (-1i32) as u32,
                        core::cmp::Ordering::Equal => 0,
                        core::cmp::Ordering::Greater => 1,
                    };
                    NativeOutcome::Return
                }
                (Err(m), _) | (_, Err(m)) => refuse(m),
            }
        }
        "strncmp" => {
            let n = r2 as usize;
            match (read_cstr(p, r0), read_cstr(p, r1)) {
                (Ok(mut a), Ok(mut b)) => {
                    a.truncate(n);
                    b.truncate(n);
                    p.cpu.regs[0] = match a.cmp(&b) {
                        core::cmp::Ordering::Less => (-1i32) as u32,
                        core::cmp::Ordering::Equal => 0,
                        core::cmp::Ordering::Greater => 1,
                    };
                    NativeOutcome::Return
                }
                (Err(m), _) | (_, Err(m)) => refuse(m),
            }
        }
        "strchr" => {
            let needle = (r1 & 0xFF) as u8;
            match read_cstr(p, r0) {
                Ok(b) => {
                    p.cpu.regs[0] = if needle == 0 {
                        r0.wrapping_add(b.len() as u32)
                    } else {
                        b.iter().position(|&c| c == needle).map(|i| r0.wrapping_add(i as u32)).unwrap_or(0)
                    };
                    NativeOutcome::Return
                }
                Err(m) => refuse(m),
            }
        }
        "strrchr" => {
            let needle = (r1 & 0xFF) as u8;
            match read_cstr(p, r0) {
                Ok(b) => {
                    p.cpu.regs[0] = if needle == 0 {
                        r0.wrapping_add(b.len() as u32)
                    } else {
                        b.iter().rposition(|&c| c == needle).map(|i| r0.wrapping_add(i as u32)).unwrap_or(0)
                    };
                    NativeOutcome::Return
                }
                Err(m) => refuse(m),
            }
        }
        "strstr" => {
            match (read_cstr(p, r0), read_cstr(p, r1)) {
                (Ok(hay), Ok(needle)) => {
                    p.cpu.regs[0] = if needle.is_empty() {
                        r0
                    } else {
                        hay.windows(needle.len()).position(|w| w == needle.as_slice()).map(|i| r0.wrapping_add(i as u32)).unwrap_or(0)
                    };
                    NativeOutcome::Return
                }
                (Err(m), _) | (_, Err(m)) => refuse(m),
            }
        }
        "atoi" => {
            let s = read_cstr(p, r0).unwrap_or_default();
            let trimmed = String::from_utf8_lossy(&s);
            let t = trimmed.trim_start();
            let (neg, rest) = if let Some(r) = t.strip_prefix('-') { (true, r) } else if let Some(r) = t.strip_prefix('+') { (false, r) } else { (false, t) };
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            let v: i32 = digits.parse::<i32>().unwrap_or(0);
            p.cpu.regs[0] = if neg { v.wrapping_neg() } else { v } as u32;
            NativeOutcome::Return
        }
        "tolower" => { p.cpu.regs[0] = ((r0 & 0xFF) as u8).to_ascii_lowercase() as u32; NativeOutcome::Return }
        "toupper" => { p.cpu.regs[0] = ((r0 & 0xFF) as u8).to_ascii_uppercase() as u32; NativeOutcome::Return }
        "isdigit" => { p.cpu.regs[0] = u32::from(((r0 & 0xFF) as u8).is_ascii_digit()); NativeOutcome::Return }
        "isalpha" => { p.cpu.regs[0] = u32::from(((r0 & 0xFF) as u8).is_ascii_alphabetic()); NativeOutcome::Return }
        "isalnum" => { p.cpu.regs[0] = u32::from(((r0 & 0xFF) as u8).is_ascii_alphanumeric()); NativeOutcome::Return }
        "isspace" => { p.cpu.regs[0] = u32::from(((r0 & 0xFF) as u8).is_ascii_whitespace()); NativeOutcome::Return }
        "isprint" => { p.cpu.regs[0] = u32::from(((r0 & 0xFF) as u8).is_ascii_graphic() || (r0 & 0xFF) == b' ' as u32); NativeOutcome::Return }
        "isxdigit" => { p.cpu.regs[0] = u32::from(((r0 & 0xFF) as u8).is_ascii_hexdigit()); NativeOutcome::Return }
        "isupper" => { p.cpu.regs[0] = u32::from(((r0 & 0xFF) as u8).is_ascii_uppercase()); NativeOutcome::Return }
        "islower" => { p.cpu.regs[0] = u32::from(((r0 & 0xFF) as u8).is_ascii_lowercase()); NativeOutcome::Return }
        "abs" => { p.cpu.regs[0] = (r0 as i32).wrapping_abs() as u32; NativeOutcome::Return }
        "CloseSTDLIB" => { NativeOutcome::Return }
        "strcasecmp" => {
            match (read_cstr(p, r0), read_cstr(p, r1)) {
                (Ok(a), Ok(b)) => {
                    let al: Vec<u8> = a.iter().map(|c| c.to_ascii_lowercase()).collect();
                    let bl: Vec<u8> = b.iter().map(|c| c.to_ascii_lowercase()).collect();
                    p.cpu.regs[0] = match al.cmp(&bl) {
                        core::cmp::Ordering::Less => (-1i32) as u32,
                        core::cmp::Ordering::Equal => 0,
                        core::cmp::Ordering::Greater => 1,
                    };
                    NativeOutcome::Return
                }
                (Err(m), _) | (_, Err(m)) => refuse(m),
            }
        }
        "strncasecmp" => {
            let n = r2 as usize;
            match (read_cstr(p, r0), read_cstr(p, r1)) {
                (Ok(mut a), Ok(mut b)) => {
                    a.truncate(n);
                    b.truncate(n);
                    let al: Vec<u8> = a.iter().map(|c| c.to_ascii_lowercase()).collect();
                    let bl: Vec<u8> = b.iter().map(|c| c.to_ascii_lowercase()).collect();
                    p.cpu.regs[0] = match al.cmp(&bl) {
                        core::cmp::Ordering::Less => (-1i32) as u32,
                        core::cmp::Ordering::Equal => 0,
                        core::cmp::Ordering::Greater => 1,
                    };
                    NativeOutcome::Return
                }
                (Err(m), _) | (_, Err(m)) => refuse(m),
            }
        }
        "strcspn" => {
            match (read_cstr(p, r0), read_cstr(p, r1)) {
                (Ok(s), Ok(rej)) => {
                    let pos = s.iter().position(|c| rej.contains(c)).unwrap_or(s.len());
                    p.cpu.regs[0] = pos as u32;
                    NativeOutcome::Return
                }
                (Err(m), _) | (_, Err(m)) => refuse(m),
            }
        }
        "strtok" => {
            let delims = match read_cstr(p, r1) { Ok(b) => b, Err(m) => return Some(refuse(m)) };
            let mut cur = if r0 != 0 { r0 } else { p.nat.clib_strtok_pos };
            if cur == 0 {
                p.cpu.regs[0] = 0;
                return Some(NativeOutcome::Return);
            }
            // Skip leading delimiters.
            loop {
                let b = p.mem.vm.read_u8(cur).unwrap_or(0);
                if b == 0 {
                    p.nat.clib_strtok_pos = 0;
                    p.cpu.regs[0] = 0;
                    return Some(NativeOutcome::Return);
                }
                if !delims.contains(&b) { break; }
                cur = cur.wrapping_add(1);
            }
            let start = cur;
            loop {
                let b = p.mem.vm.read_u8(cur).unwrap_or(0);
                if b == 0 {
                    p.nat.clib_strtok_pos = 0;
                    break;
                }
                if delims.contains(&b) {
                    let _ = p.mem.vm.write_u8(cur, 0);
                    p.nat.clib_strtok_pos = cur.wrapping_add(1);
                    break;
                }
                cur = cur.wrapping_add(1);
            }
            p.cpu.regs[0] = start;
            NativeOutcome::Return
        }
        "strtol" | "strtoul" => {
            let unsigned = name == "strtoul";
            let base_arg = r2 as i32;
            let bytes = match read_cstr(p, r0) { Ok(b) => b, Err(m) => return Some(refuse(m)) };
            let mut i = 0usize;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() { i += 1; }
            let mut neg = false;
            if i < bytes.len() && (bytes[i] == b'-' || bytes[i] == b'+') {
                neg = bytes[i] == b'-';
                i += 1;
            }
            let base: u32 = if base_arg == 0 {
                if i + 1 < bytes.len() && bytes[i] == b'0' && (bytes[i + 1] == b'x' || bytes[i + 1] == b'X') {
                    i += 2; 16
                } else if i < bytes.len() && bytes[i] == b'0' {
                    8
                } else { 10 }
            } else {
                if base_arg == 16 && i + 1 < bytes.len() && bytes[i] == b'0' && (bytes[i + 1] == b'x' || bytes[i + 1] == b'X') {
                    i += 2;
                }
                base_arg.clamp(2, 36) as u32
            };
            let start_digits = i;
            let mut acc: u64 = 0;
            while i < bytes.len() {
                let d = match bytes[i] {
                    b'0'..=b'9' => (bytes[i] - b'0') as u32,
                    b'a'..=b'z' => (bytes[i] - b'a' + 10) as u32,
                    b'A'..=b'Z' => (bytes[i] - b'A' + 10) as u32,
                    _ => break,
                };
                if d >= base { break; }
                acc = acc.wrapping_mul(base as u64).wrapping_add(d as u64);
                i += 1;
            }
            let end_off = if i == start_digits { 0 } else { i };
            if r1 != 0 { let _ = p.wr(r1, r0.wrapping_add(end_off as u32)); }
            let val = if neg { (acc as u32).wrapping_neg() } else { acc as u32 };
            p.cpu.regs[0] = if unsigned { val } else { val };
            NativeOutcome::Return
        }
        "sscanf" => {
            let input = match read_cstr(p, r0) { Ok(b) => b, Err(m) => return Some(refuse(m)) };
            let fmt = match read_cstr(p, r1) { Ok(b) => b, Err(m) => return Some(refuse(m)) };
            let matched = run_sscanf(p, &input, &fmt);
            p.cpu.regs[0] = matched as u32;
            NativeOutcome::Return
        }
        "memcmp" => {
            let n = r2 as usize;
            let mut a = alloc::vec![0u8; n];
            let mut b = alloc::vec![0u8; n];
            if n > 0 && (p.mem.vm.read_bytes(r0, &mut a).is_err() || p.mem.vm.read_bytes(r1, &mut b).is_err()) {
                return Some(refuse("memcmp fault".into()));
            }
            p.cpu.regs[0] = match a.cmp(&b) {
                core::cmp::Ordering::Less => (-1i32) as u32,
                core::cmp::Ordering::Equal => 0,
                core::cmp::Ordering::Greater => 1,
            };
            NativeOutcome::Return
        }
        "memchr" => {
            let needle = (r1 & 0xFF) as u8;
            let n = r2 as usize;
            let mut buf = alloc::vec![0u8; n];
            if n > 0 && p.mem.vm.read_bytes(r0, &mut buf).is_err() {
                return Some(refuse("memchr fault".into()));
            }
            p.cpu.regs[0] = buf.iter().position(|&c| c == needle).map(|i| r0.wrapping_add(i as u32)).unwrap_or(0);
            NativeOutcome::Return
        }
        "_e32memcpy" => {
            let n = r2 as usize;
            let mut buf = alloc::vec![0u8; n];
            if n > 0 && (p.mem.vm.read_bytes(r1, &mut buf).is_err() || p.mem.vm.write_bytes(r0, &buf).is_err()) {
                return Some(refuse("_e32memcpy fault".into()));
            }
            p.cpu.regs[0] = r0;
            NativeOutcome::Return
        }
        "fflush" => { p.cpu.regs[0] = 0; NativeOutcome::Return }
        "fprintf" => { p.cpu.regs[0] = 0; NativeOutcome::Return }
        "fgetc" => {
            let h = p.rd(r0).unwrap_or(0);
            let ch = p.nat.objs.open_files.get_mut(&h).and_then(|f| {
                if f.pos < f.data.len() { let b = f.data[f.pos]; f.pos += 1; Some(b as u32) } else { None }
            });
            p.cpu.regs[0] = ch.unwrap_or((-1i32) as u32);
            NativeOutcome::Return
        }
        "fputc" => {
            let ch = (r0 & 0xFF) as u8;
            let h = p.rd(r1).unwrap_or(0);
            if let Some(f) = p.nat.objs.open_files.get_mut(&h) {
                if f.pos >= f.data.len() { f.data.resize(f.pos + 1, 0); }
                f.data[f.pos] = ch;
                f.pos += 1;
                let (path, data) = (f.path.clone(), f.data.clone());
                if let Some(drv) = p.host.drive.as_mut() { let _ = drv.put_file(&path, data); }
            }
            p.cpu.regs[0] = ch as u32;
            NativeOutcome::Return
        }
        "ungetc" => {
            let h = p.rd(r1).unwrap_or(0);
            if let Some(f) = p.nat.objs.open_files.get_mut(&h) {
                if f.pos > 0 { f.pos -= 1; }
            }
            p.cpu.regs[0] = r0 & 0xFF;
            NativeOutcome::Return
        }
        "fgets" => {
            // char* fgets(char* s, int size, FILE* stream)
            let max = (r1 as i32).max(0) as usize;
            let h = p.rd(r2).unwrap_or(0);
            if max <= 1 || h == 0 {
                p.cpu.regs[0] = 0;
                return Some(NativeOutcome::Return);
            }
            let line = p.nat.objs.open_files.get_mut(&h).and_then(|f| {
                if f.pos >= f.data.len() { return None; }
                let mut out = Vec::new();
                while f.pos < f.data.len() && out.len() + 1 < max {
                    let b = f.data[f.pos];
                    f.pos += 1;
                    out.push(b);
                    if b == b'\n' { break; }
                }
                Some(out)
            });
            match line {
                Some(bytes) => {
                    if let Err(m) = write_cstr(p, r0, &bytes) { return Some(refuse(m)); }
                    p.cpu.regs[0] = r0;
                }
                None => { p.cpu.regs[0] = 0; }
            }
            NativeOutcome::Return
        }
        "time" => {
            let secs = (p.nat.clock_us / 1_000_000) as u32 + 1_262_304_000;
            if r0 != 0 { let _ = p.wr(r0, secs); }
            p.cpu.regs[0] = secs;
            NativeOutcome::Return
        }
        "gettimeofday" => {
            let secs = (p.nat.clock_us / 1_000_000) as u32 + 1_262_304_000;
            let usec = (p.nat.clock_us % 1_000_000) as u32;
            if r0 != 0 { let _ = p.wr(r0, secs); let _ = p.wr(r0.wrapping_add(4), usec); }
            p.cpu.regs[0] = 0;
            NativeOutcome::Return
        }
        "puts" => { p.cpu.regs[0] = 0; NativeOutcome::Return }
        "rand" => {
            let mut s = p.nat.clib_rng;
            if s == 0 { s = 0x1234_5678; }
            s = s.wrapping_mul(1103515245).wrapping_add(12345);
            p.nat.clib_rng = s;
            p.cpu.regs[0] = (s >> 16) & 0x7FFF;
            NativeOutcome::Return
        }
        "srand" => { p.nat.clib_rng = r0.max(1); NativeOutcome::Return }
        "sprintf" => {
            let fmt = match read_cstr(p, r1) { Ok(b) => b, Err(m) => return Some(refuse(m)) };
            let out = format_printf(p, &fmt, 2, None);
            if let Err(m) = write_cstr(p, r0, &out) { return Some(refuse(m)); }
            p.cpu.regs[0] = out.len() as u32;
            NativeOutcome::Return
        }
        "snprintf" => {
            let max = r1 as usize;
            let fmt = match read_cstr(p, r2) { Ok(b) => b, Err(m) => return Some(refuse(m)) };
            let out = format_printf(p, &fmt, 3, None);
            if max > 0 {
                let n = out.len().min(max - 1);
                if let Err(m) = write_cstr(p, r0, &out[..n]) { return Some(refuse(m)); }
            }
            p.cpu.regs[0] = out.len() as u32;
            NativeOutcome::Return
        }
        "vsprintf" => {
            let fmt = match read_cstr(p, r1) { Ok(b) => b, Err(m) => return Some(refuse(m)) };
            let out = format_printf(p, &fmt, 0, Some(r2));
            if let Err(m) = write_cstr(p, r0, &out) { return Some(refuse(m)); }
            p.cpu.regs[0] = out.len() as u32;
            NativeOutcome::Return
        }
        "fopen" => {
            let raw = match read_cstr(p, r0) { Ok(b) => String::from_utf8_lossy(&b).into_owned(), Err(m) => return Some(refuse(m)) };
            let mode = match read_cstr(p, r1) { Ok(b) => String::from_utf8_lossy(&b).into_owned(), Err(m) => return Some(refuse(m)) };
            let path = c_file_path(p, &raw);
            let write_mode = mode.contains('w') || mode.contains('a');
            let existing = p.host.drive.as_ref().and_then(|d| {
                d.read_owned(&path).ok().or_else(|| {
                    let tail = raw.rsplit(['\\', '/']).next().unwrap_or(&raw);
                    let priv_dir = format!("C:\\private\\{:08x}\\", p.uid3);
                    d.read_owned(&format!("{priv_dir}{tail}")).ok()
                })
            });
            let data = match (existing, write_mode) {
                (Some(d), false) => d,
                (_, true) => {
                    let init = if mode.contains('a') { p.host.drive.as_ref().and_then(|d| d.read_owned(&path).ok()).unwrap_or_default() } else { Vec::new() };
                    if let Some(drv) = p.host.drive.as_mut() { let _ = drv.put_file(&path, init.clone()); }
                    init
                }
                (None, false) => {
                    p.cpu.regs[0] = 0;
                    return Some(NativeOutcome::Return);
                }
            };
            let pos = if mode.contains('a') { data.len() } else { 0 };
            p.nat.objs.next_file_handle += 1;
            let h = p.nat.objs.next_file_handle;
            p.nat.objs.open_files.insert(h, crate::objects::OpenFile { path, data, pos });
            // Allocate a tiny guest FILE marker holding the handle so non-null FILE* pointers work.
            let fptr = p.alloc_zeroed(8).unwrap_or(0);
            if fptr != 0 { let _ = p.wr(fptr, h); }
            p.cpu.regs[0] = fptr;
            NativeOutcome::Return
        }
        "fread" => {
            // size_t fread(void* ptr, size_t size, size_t nmemb, FILE* stream)
            let h = p.rd(r3).unwrap_or(0);
            let total = r1.saturating_mul(r2) as usize;
            if r1 == 0 || r2 == 0 || h == 0 {
                p.cpu.regs[0] = 0;
                return Some(NativeOutcome::Return);
            }
            let chunk = match p.nat.objs.open_files.get_mut(&h) {
                Some(f) => {
                    let avail = f.data.len().saturating_sub(f.pos);
                    let take = avail.min(total);
                    let slice = f.data[f.pos..f.pos + take].to_vec();
                    f.pos += take;
                    slice
                }
                None => Vec::new(),
            };
            if !chunk.is_empty() && p.mem.vm.write_bytes(r0, &chunk).is_err() {
                return Some(refuse(format!("fread dst {r0:#x} unwritable")));
            }
            p.cpu.regs[0] = (chunk.len() / r1 as usize) as u32;
            NativeOutcome::Return
        }
        "fwrite" => {
            let h = p.rd(r3).unwrap_or(0);
            let total = r1.saturating_mul(r2) as usize;
            if r1 == 0 || r2 == 0 || h == 0 {
                p.cpu.regs[0] = 0;
                return Some(NativeOutcome::Return);
            }
            let mut buf = alloc::vec![0u8; total];
            if p.mem.vm.read_bytes(r0, &mut buf).is_err() {
                return Some(refuse(format!("fwrite src {r0:#x} unreadable")));
            }
            if let Some(f) = p.nat.objs.open_files.get_mut(&h) {
                let end = f.pos + buf.len();
                if end > f.data.len() { f.data.resize(end, 0); }
                f.data[f.pos..end].copy_from_slice(&buf);
                f.pos = end;
                let (path, data) = (f.path.clone(), f.data.clone());
                if let Some(drv) = p.host.drive.as_mut() { let _ = drv.put_file(&path, data); }
            }
            p.cpu.regs[0] = r2;
            NativeOutcome::Return
        }
        "fseek" => {
            let h = p.rd(r0).unwrap_or(0);
            let off = r1 as i32 as isize;
            let ok = if let Some(f) = p.nat.objs.open_files.get_mut(&h) {
                let base = match r2 {
                    0 => 0isize,
                    1 => f.pos as isize,
                    2 => f.data.len() as isize,
                    _ => -1,
                };
                let target = base + off;
                if target >= 0 { f.pos = target as usize; true } else { false }
            } else { false };
            p.cpu.regs[0] = if ok { 0 } else { (-1i32) as u32 };
            NativeOutcome::Return
        }
        "ftell" => {
            let h = p.rd(r0).unwrap_or(0);
            p.cpu.regs[0] = p.nat.objs.open_files.get(&h).map(|f| f.pos as u32).unwrap_or((-1i32) as u32);
            NativeOutcome::Return
        }
        "fclose" => {
            if r0 != 0 {
                if let Ok(h) = p.rd(r0) { p.nat.objs.open_files.remove(&h); }
                if let Some(heap) = p.heap.as_mut() { let _ = heap.free(r0); }
            }
            p.cpu.regs[0] = 0;
            NativeOutcome::Return
        }
        "sqrt" => {
            let v = f64::from_bits(((r1 as u64) << 32) | (r0 as u64));
            let res = if v <= 0.0 { 0.0 } else {
                let mut g = v;
                for _ in 0..32 { g = 0.5 * (g + v / g); }
                g
            };
            let bits = res.to_bits();
            p.cpu.regs[0] = bits as u32;
            p.cpu.regs[1] = (bits >> 32) as u32;
            NativeOutcome::Return
        }
        "atan2" => {
            let y = f64::from_bits(((r1 as u64) << 32) | (r0 as u64));
            let x = f64::from_bits(((r3 as u64) << 32) | (r2 as u64));
            let res = approx_atan2(y, x);
            let bits = res.to_bits();
            p.cpu.regs[0] = bits as u32;
            p.cpu.regs[1] = (bits >> 32) as u32;
            NativeOutcome::Return
        }
        "memcpy" => {
            let n = r2 as usize;
            let mut buf = alloc::vec![0u8; n];
            if n > 0 {
                if p.mem.vm.read_bytes(r1, &mut buf).is_err() || p.mem.vm.write_bytes(r0, &buf).is_err() {
                    return Some(refuse("memcpy fault".into()));
                }
            }
            p.cpu.regs[0] = r0;
            NativeOutcome::Return
        }
        "memmove" => {
            let n = r2 as usize;
            let mut buf = alloc::vec![0u8; n];
            if n > 0 {
                if p.mem.vm.read_bytes(r1, &mut buf).is_err() || p.mem.vm.write_bytes(r0, &buf).is_err() {
                    return Some(refuse("memmove fault".into()));
                }
            }
            p.cpu.regs[0] = r0;
            NativeOutcome::Return
        }
        "memset" => {
            let n = r2 as usize;
            if n > 0 {
                let buf = alloc::vec![(r1 & 0xFF) as u8; n];
                if p.mem.vm.write_bytes(r0, &buf).is_err() { return Some(refuse("memset fault".into())); }
            }
            p.cpu.regs[0] = r0;
            NativeOutcome::Return
        }
        _ => return None,
    })
}

fn run_sscanf(p: &mut Process, input: &[u8], fmt: &[u8]) -> i32 {
    let mut ii = 0usize;
    let mut fi = 0usize;
    let mut arg_idx = 2usize;
    let mut matched = 0i32;
    while fi < fmt.len() {
        if fmt[fi].is_ascii_whitespace() {
            while fi < fmt.len() && fmt[fi].is_ascii_whitespace() { fi += 1; }
            while ii < input.len() && input[ii].is_ascii_whitespace() { ii += 1; }
            continue;
        }
        if fmt[fi] != b'%' {
            if ii >= input.len() || input[ii] != fmt[fi] { break; }
            ii += 1;
            fi += 1;
            continue;
        }
        fi += 1;
        if fi >= fmt.len() { break; }
        if fmt[fi] == b'%' {
            if ii >= input.len() || input[ii] != b'%' { break; }
            ii += 1; fi += 1; continue;
        }
        let suppress = if fmt[fi] == b'*' { fi += 1; true } else { false };
        let mut width: Option<usize> = None;
        while fi < fmt.len() && fmt[fi].is_ascii_digit() {
            width = Some(width.unwrap_or(0) * 10 + (fmt[fi] - b'0') as usize);
            fi += 1;
        }
        while fi < fmt.len() && matches!(fmt[fi], b'l' | b'h') { fi += 1; }
        if fi >= fmt.len() { break; }
        let spec = fmt[fi];
        fi += 1;
        match spec {
            b'd' | b'i' | b'u' | b'x' | b'X' => {
                while ii < input.len() && input[ii].is_ascii_whitespace() { ii += 1; }
                let start = ii;
                let max_end = width.map(|w| (ii + w).min(input.len())).unwrap_or(input.len());
                let mut neg = false;
                if ii < max_end && (input[ii] == b'-' || input[ii] == b'+') {
                    neg = input[ii] == b'-';
                    ii += 1;
                }
                let base: u32 = match spec {
                    b'x' | b'X' => {
                        if ii + 1 < max_end && input[ii] == b'0' && (input[ii + 1] == b'x' || input[ii + 1] == b'X') { ii += 2; }
                        16
                    }
                    b'i' => {
                        if ii + 1 < max_end && input[ii] == b'0' && (input[ii + 1] == b'x' || input[ii + 1] == b'X') { ii += 2; 16 }
                        else if ii < max_end && input[ii] == b'0' { 8 }
                        else { 10 }
                    }
                    _ => 10,
                };
                let dstart = ii;
                let mut acc: u64 = 0;
                while ii < max_end {
                    let d = match input[ii] {
                        b'0'..=b'9' => (input[ii] - b'0') as u32,
                        b'a'..=b'f' => (input[ii] - b'a' + 10) as u32,
                        b'A'..=b'F' => (input[ii] - b'A' + 10) as u32,
                        _ => break,
                    };
                    if d >= base { break; }
                    acc = acc.wrapping_mul(base as u64).wrapping_add(d as u64);
                    ii += 1;
                }
                if ii == dstart {
                    if start == input.len() && matched == 0 { return -1; }
                    break;
                }
                if !suppress {
                    let dst = next_printf_arg(p, &mut arg_idx, None);
                    let val = if neg { (acc as u32).wrapping_neg() } else { acc as u32 };
                    let _ = p.wr(dst, val);
                    matched += 1;
                }
            }
            b's' => {
                while ii < input.len() && input[ii].is_ascii_whitespace() { ii += 1; }
                let max_end = width.map(|w| (ii + w).min(input.len())).unwrap_or(input.len());
                let start = ii;
                while ii < max_end && !input[ii].is_ascii_whitespace() { ii += 1; }
                if ii == start { break; }
                if !suppress {
                    let dst = next_printf_arg(p, &mut arg_idx, None);
                    let _ = write_cstr(p, dst, &input[start..ii]);
                    matched += 1;
                }
            }
            b'c' => {
                let count = width.unwrap_or(1);
                if ii + count > input.len() { break; }
                if !suppress {
                    let dst = next_printf_arg(p, &mut arg_idx, None);
                    let _ = p.mem.vm.write_bytes(dst, &input[ii..ii + count]);
                    matched += 1;
                }
                ii += count;
            }
            b'n' => {
                if !suppress {
                    let dst = next_printf_arg(p, &mut arg_idx, None);
                    let _ = p.wr(dst, ii as u32);
                }
            }
            _ => break,
        }
    }
    matched
}

fn approx_atan2(y: f64, x: f64) -> f64 {
    const PI: f64 = core::f64::consts::PI;
    const FRAC_PI_2: f64 = core::f64::consts::FRAC_PI_2;
    if x == 0.0 {
        return if y > 0.0 { FRAC_PI_2 } else if y < 0.0 { -FRAC_PI_2 } else { 0.0 };
    }
    let atan = |z: f64| -> f64 {
        let abs = if z < 0.0 { -z } else { z };
        let t = if abs <= 1.0 {
            let z2 = z * z;
            z * (1.0 - z2 * (1.0 / 3.0 - z2 * (1.0 / 5.0 - z2 * (1.0 / 7.0 - z2 * (1.0 / 9.0 - z2 / 11.0)))))
        } else {
            let inv = 1.0 / z;
            let i2 = inv * inv;
            let a = inv * (1.0 - i2 * (1.0 / 3.0 - i2 * (1.0 / 5.0 - i2 * (1.0 / 7.0 - i2 * (1.0 / 9.0 - i2 / 11.0)))));
            if z > 0.0 { FRAC_PI_2 - a } else { -FRAC_PI_2 - a }
        };
        t
    };
    let a = atan(y / x);
    if x > 0.0 { a } else if y >= 0.0 { a + PI } else { a - PI }
}
