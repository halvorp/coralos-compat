//! StdCLib: the C library functions a PowerPC application imports (names from a census of real MPW/CodeWarrior PEFs: memory, `mem*`/`str*`,
//! `ato*`/`strto*`, `ctype`, `rand`, `time`, `printf` and friends). They are NATIVE Rust over guest memory: a call never interprets library
//! code. Calling convention: Mac OS Runtime Architectures (the PowerPC ABI of the classic Mac OS): integer and pointer arguments in r3..r10 then the
//! parameter area 24 bytes above r1 (one word each), doubles in f1..f13 and ALSO reserving two GPR slots each (so a variadic callee finds them in
//! order), results in r3 (or f1). C semantics are the ISO C library's; where the classic library differs or this engine has no state to match
//! (`rand`'s sequence, `getenv`, locale) the difference is stated at the function.
//! NEWLINES: no byte is translated anywhere. `'\\n'` is the compiler's: 0x0D for MPW/CodeWarrior programs on Apple's StdCLib, 0x0A for GCC/Retro68
//! builds. `puts` is the one place the library must pick a byte itself (the console log is a byte stream); it appends LF (0x0A).
//! `qsort`/`bsearch`/`atexit`/`exit` call the PROGRAM back and live in [`crate::callback_c`].
//! Not provided, therefore refused by name when called: the `FILE` family (`fopen`, `fprintf`, `_iob` ...), `longjmp`, `strftime`, `signal`, `sscanf`.
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

/// A StdCLib function served natively.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CFn {
    Malloc, Calloc, Realloc, Free,
    Memcpy, Memmove, Memset, Memcmp, Memchr,
    Strlen, Strcpy, Strncpy, Strcat, Strncat, Strcmp, Strncmp, Strchr, Strrchr, Strstr, Strspn, Strcspn, Strpbrk,
    Atoi, Atol, Strtol, Strtoul, Abs, Rand, Srand,
    Sprintf, Snprintf, Printf, Puts, Putchar,
    Isalnum, Isalpha, Isascii, Iscntrl, Isdigit, Isgraph, Islower, Isprint, Ispunct, Isspace, Isupper, Isxdigit, Toupper, Tolower, Toascii,
    Getenv, Time, Clock,
    /// These call the program back: they redirect the CPU into its routine ([`crate::callback_c`]).
    Qsort, Bsearch, Atexit,
}

impl CFn {
    pub fn by_name(name: &str) -> Option<CFn> {
        use CFn::*;
        Some(match name {
            "malloc" => Malloc, "calloc" => Calloc, "realloc" => Realloc, "free" => Free,
            "memcpy" => Memcpy, "memmove" => Memmove, "memset" => Memset, "memcmp" => Memcmp, "memchr" => Memchr,
            "strlen" => Strlen, "strcpy" => Strcpy, "strncpy" => Strncpy, "strcat" => Strcat, "strncat" => Strncat, "strcmp" => Strcmp,
            "strncmp" => Strncmp, "strchr" => Strchr, "strrchr" => Strrchr, "strstr" => Strstr, "strspn" => Strspn, "strcspn" => Strcspn, "strpbrk" => Strpbrk,
            "atoi" => Atoi, "atol" => Atol, "strtol" => Strtol, "strtoul" => Strtoul, "abs" | "labs" => Abs, "rand" => Rand, "srand" => Srand,
            "sprintf" => Sprintf, "snprintf" => Snprintf, "printf" => Printf, "puts" => Puts, "putchar" => Putchar,
            "isalnum" => Isalnum, "isalpha" => Isalpha, "isascii" => Isascii, "iscntrl" => Iscntrl, "isdigit" => Isdigit, "isgraph" => Isgraph,
            "islower" => Islower, "isprint" => Isprint, "ispunct" => Ispunct, "isspace" => Isspace, "isupper" => Isupper, "isxdigit" => Isxdigit,
            "toupper" => Toupper, "tolower" => Tolower, "toascii" => Toascii,
            "getenv" => Getenv, "time" => Time, "clock" => Clock,
            "qsort" => Qsort, "bsearch" => Bsearch, "atexit" => Atexit,
            _ => return None,
        })
    }
}

/// The word-sized argument `i` of the current call: r3..r10, then the parameter area.
pub(crate) fn arg<U: GuestUi>(m: &MacPpc<U>, cpu: &Cpu, i: usize) -> Result<u32, String> {
    if i < 8 { return Ok(cpu.gpr[3 + i]); }
    let at = cpu.gpr[1].wrapping_add(24 + 4 * i as u32);
    m.mac.mem.read_u32(at).map_err(|e| format!("argument {i} is in the stack parameter area at {at:#x}: {e}"))
}

impl<U: GuestUi> MacPpc<U> {
    /// The bytes of the NUL-terminated string at `p` (without the NUL).
    pub fn cstr(&self, p: u32, who: &str) -> Result<Vec<u8>, String> {
        let mut v = Vec::new();
        loop {
            let b = self.mac.mem.read_u8(p.wrapping_add(v.len() as u32)).map_err(|_| format!("{who}: the string at {p:#x} runs out of guest RAM before its NUL"))?;
            if b == 0 { return Ok(v); }
            v.push(b);
        }
    }
    pub fn put_bytes(&mut self, at: u32, b: &[u8], who: &str) -> Result<(), String> {
        self.mac.mem.write_bytes(at, b).map_err(|_| format!("{who}: {} bytes at {at:#x} leave guest RAM", b.len()))
    }
    pub fn get_bytes(&self, at: u32, n: u32, who: &str) -> Result<Vec<u8>, String> {
        let s = self.mac.mem.span(at, n).map_err(|_| format!("{who}: {n} bytes at {at:#x} leave guest RAM"))?;
        self.mac.mem.bytes(s).map(<[u8]>::to_vec).map_err(|_| format!("{who}: {n} bytes at {at:#x} leave guest RAM"))
    }

    fn alloc(&mut self, n: u32, clear: bool) -> u32 {
        // malloc(0) still returns a unique pointer (C allows either; programs compare against NULL)
        self.mac.mm.new_ptr(&mut self.mac.mem, &mut self.mac.heap, n.max(1), clear).unwrap_or(0)
    }

    /// Serve `f` for the call the CPU is making; the result goes in r3.
    pub(crate) fn c_call(&mut self, f: CFn, cpu: &mut Cpu) -> Result<(), String> {
        use CFn::*;
        let name = format!("{f:?}").to_lowercase();
        let a = |m: &Self, i: usize| arg(m, cpu, i);
        let (a0, a1, a2) = (a(self, 0)?, a(self, 1)?, a(self, 2)?);
        match f {
            Qsort => { let a3 = a(self, 3)?; return self.start_qsort(cpu, a0, a1, a2, a3); }
            Bsearch => { let (a3, a4) = (a(self, 3)?, a(self, 4)?); return self.start_bsearch(cpu, a0, a1, a2, a3, a4); }
            Atexit => { self.register_atexit(a0)?; cpu.gpr[3] = 0; return Ok(()); }
            _ => {}
        }
        let r: u32 = match f {
            Malloc => self.alloc(a0, false),
            Calloc => match a0.checked_mul(a1) { Some(n) => self.alloc(n, true), None => 0 },
            Free => {
                if a0 != 0 { self.mac.mm.dispose_ptr(&mut self.mac.heap, a0).map_err(|e| format!("free: {a0:#x} is not a block malloc returned (memory error {e})"))?; }
                0
            }
            Realloc => {
                if a0 == 0 { self.alloc(a1, false) }
                else if a1 == 0 { self.mac.mm.dispose_ptr(&mut self.mac.heap, a0).map_err(|e| format!("realloc: {a0:#x} is not a block malloc returned (memory error {e})"))?; 0 }
                else {
                    let old = self.mac.mm.ptr_size(a0).map_err(|e| format!("realloc: {a0:#x} is not a block malloc returned (memory error {e})"))?;
                    let p = self.alloc(a1, false);
                    if p != 0 {
                        let keep = self.get_bytes(a0, old.min(a1), "realloc")?;
                        self.put_bytes(p, &keep, "realloc")?;
                        self.mac.mm.dispose_ptr(&mut self.mac.heap, a0).map_err(|e| format!("realloc: freeing the old block: memory error {e}"))?;
                    }
                    p
                }
            }
            Memcpy | Memmove => { let b = self.get_bytes(a1, a2, &name)?; self.put_bytes(a0, &b, &name)?; a0 }
            Memset => { let b = alloc::vec![a1 as u8; a2 as usize]; self.put_bytes(a0, &b, &name)?; a0 }
            Memcmp => { let (x, y) = (self.get_bytes(a0, a2, &name)?, self.get_bytes(a1, a2, &name)?); ord(x.cmp(&y)) }
            Memchr => { let b = self.get_bytes(a0, a2, &name)?; b.iter().position(|&c| c == a1 as u8).map_or(0, |i| a0 + i as u32) }
            Strlen => self.cstr(a0, &name)?.len() as u32,
            Strcpy => { let mut s = self.cstr(a1, &name)?; s.push(0); self.put_bytes(a0, &s, &name)?; a0 }
            Strncpy => {
                let s = self.cstr(a1, &name)?;
                let mut out = alloc::vec![0u8; a2 as usize]; // padded with NULs to n
                let k = s.len().min(a2 as usize);
                out[..k].copy_from_slice(&s[..k]);
                self.put_bytes(a0, &out, &name)?; a0
            }
            Strcat => { let end = a0 + self.cstr(a0, &name)?.len() as u32; let mut s = self.cstr(a1, &name)?; s.push(0); self.put_bytes(end, &s, &name)?; a0 }
            Strncat => {
                let end = a0 + self.cstr(a0, &name)?.len() as u32;
                let s = self.cstr(a1, &name)?;
                let mut s = s[..s.len().min(a2 as usize)].to_vec();
                s.push(0);
                self.put_bytes(end, &s, &name)?; a0
            }
            Strcmp => { let (x, y) = (self.cstr(a0, &name)?, self.cstr(a1, &name)?); ord(x.cmp(&y)) }
            Strncmp => {
                let (x, y) = (self.cstr(a0, &name)?, self.cstr(a1, &name)?);
                let n = a2 as usize;
                ord(x[..x.len().min(n)].cmp(&y[..y.len().min(n)]))
            }
            Strchr => { let mut s = self.cstr(a0, &name)?; s.push(0); s.iter().position(|&c| c == a1 as u8).map_or(0, |i| a0 + i as u32) }
            Strrchr => { let mut s = self.cstr(a0, &name)?; s.push(0); s.iter().rposition(|&c| c == a1 as u8).map_or(0, |i| a0 + i as u32) }
            Strstr => {
                let (h, n) = (self.cstr(a0, &name)?, self.cstr(a1, &name)?);
                if n.is_empty() { a0 } else { h.windows(n.len()).position(|w| w == n.as_slice()).map_or(0, |i| a0 + i as u32) }
            }
            Strspn => { let (s, set) = (self.cstr(a0, &name)?, self.cstr(a1, &name)?); s.iter().take_while(|c| set.contains(c)).count() as u32 }
            Strcspn => { let (s, set) = (self.cstr(a0, &name)?, self.cstr(a1, &name)?); s.iter().take_while(|c| !set.contains(c)).count() as u32 }
            Strpbrk => { let (s, set) = (self.cstr(a0, &name)?, self.cstr(a1, &name)?); s.iter().position(|c| set.contains(c)).map_or(0, |i| a0 + i as u32) }
            Atoi | Atol => { let s = self.cstr(a0, &name)?; crate::clib_num::strtol(&s, 10).0 as u32 }
            Strtol | Strtoul => {
                let s = self.cstr(a0, &name)?;
                let (v, used, overflow) = crate::clib_num::strtol_full(&s, a2, f == Strtoul);
                if a1 != 0 { self.mac.mem.write_u32(a1, a0 + used as u32).map_err(|_| format!("{name}: endptr {a1:#x} leaves guest RAM"))?; }
                if overflow { self.set_errno(34); } // ERANGE
                v
            }
            Abs => (a0 as i32).wrapping_abs() as u32,
            Rand => { if !self.rt.rand_seeded { self.rt.rand = 1; self.rt.rand_seeded = true; } self.rt.rand = self.rt.rand.wrapping_mul(1_103_515_245).wrapping_add(12_345); (self.rt.rand >> 16) & 0x7FFF }
            Srand => { self.rt.rand = a0; self.rt.rand_seeded = true; 0 }
            Sprintf => { let s = self.format_c(cpu, 1, a1)?; let mut b = s.clone(); b.push(0); self.put_bytes(a0, &b, &name)?; s.len() as u32 }
            Snprintf => {
                let s = self.format_c(cpu, 2, a2)?;
                if a1 > 0 { let k = s.len().min(a1 as usize - 1); let mut b = s[..k].to_vec(); b.push(0); self.put_bytes(a0, &b, &name)?; }
                s.len() as u32
            }
            Printf => { let s = self.format_c(cpu, 0, a0)?; self.console_write(&s); s.len() as u32 }
            Puts => { let mut s = self.cstr(a0, &name)?; s.push(b'\n'); self.console_write(&s); 1 }
            Putchar => { self.console_write(&[a0 as u8]); a0 & 0xFF }
            Isalnum => ct(a0, |c| c.is_ascii_alphanumeric()), Isalpha => ct(a0, |c| c.is_ascii_alphabetic()),
            Isascii => u32::from(a0 < 128), Iscntrl => ct(a0, |c| c.is_ascii_control()), Isdigit => ct(a0, |c| c.is_ascii_digit()),
            Isgraph => ct(a0, |c| c.is_ascii_graphic()), Islower => ct(a0, |c| c.is_ascii_lowercase()),
            Isprint => ct(a0, |c| c.is_ascii_graphic() || c == b' '), Ispunct => ct(a0, |c| c.is_ascii_punctuation()),
            Isspace => ct(a0, |c| matches!(c, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r')), Isupper => ct(a0, |c| c.is_ascii_uppercase()),
            Isxdigit => ct(a0, |c| c.is_ascii_hexdigit()),
            Toupper => if a0 < 256 { u32::from((a0 as u8).to_ascii_uppercase()) } else { a0 },
            Tolower => if a0 < 256 { u32::from((a0 as u8).to_ascii_lowercase()) } else { a0 },
            Toascii => a0 & 0x7F,
            // There is no process environment: every variable is unset (NULL), which is what a program without an environment sees.
            Getenv => 0,
            // time_t on the classic Mac is seconds since 1904-01-01 (the Time low-memory global, which the engine's clock keeps)
            Time => {
                let now = self.mac.mem.read_u32(heirloom_mac68k_engine::lowmem::TIME).unwrap_or(0);
                if a0 != 0 { self.mac.mem.write_u32(a0, now).map_err(|_| format!("time: {a0:#x} leaves guest RAM"))?; }
                now
            }
            // clock() counts CLOCKS_PER_SEC = 60 ticks per second: TickCount
            Clock => self.mac.ev.ticks,
            Qsort | Bsearch | Atexit => unreachable!("handled above"),
        };
        cpu.gpr[3] = r;
        Ok(())
    }

    /// `errno` is a StdCLib data symbol (not yet provided); the value is recorded for when it is.
    fn set_errno(&mut self, e: u32) { self.rt.errno = e; }

    pub(crate) fn console_write(&mut self, b: &[u8]) {
        self.console.extend_from_slice(b);
        self.mac.log.push(format!("[console] {}", String::from_utf8_lossy(b).trim_end_matches('\n')));
    }
}

fn ord(o: core::cmp::Ordering) -> u32 { match o { core::cmp::Ordering::Less => (-1i32) as u32, core::cmp::Ordering::Equal => 0, core::cmp::Ordering::Greater => 1 } }
/// ctype in the C locale: the argument is an int in 0..=255 (or EOF); anything else is false.
fn ct(c: u32, f: impl Fn(u8) -> bool) -> u32 { u32::from(c < 256 && f(c as u8)) }
