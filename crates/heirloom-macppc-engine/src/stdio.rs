//! The C `FILE` stream layer over the File Manager: `fopen`/`fclose`/`fread`/`fwrite`/`fgets`/`fgetc`/`fputs`/`fputc`/`fprintf`/`fseek`/`ftell`/
//! `feof`/`ferror`/`remove`/`_iob`/`_filbuf`/`_flsbuf` and friends, as native Rust over the SAME volume the Toolbox sees (a file written by
//! `fwrite` is readable by `FSRead` and the 68k engine). Names are those the Systemless loader's import table shows real MPW/CodeWarrior PEFs using.
//!
//! * `FILE` is a 16-byte zeroed block in guest memory (MPW's `{ptr, cnt, base, flag, file}` rounded up): the program's `getc`/`putc` MACROS test
//!   `_cnt` (always 0 here) and so always call `_filbuf`/`_flsbuf`, which are served. `_iob` is an array of three of them; `stdout` (`_iob[1]`) and
//!   `stderr` (`_iob[2]`) write to the console ([`crate::clib`]), `stdin` (`_iob[0]`) is at end-of-file: there is no keyboard stream.
//! * Streams are unbuffered (every write goes to the volume at once), so `fflush` has nothing to do and `setvbuf` is accepted as the no-op it is.
//! * A path is a Mac path (`:` separated, relative to the default directory), looked up by the File Manager's own rules.
//! * NO BYTE IS EVER TRANSLATED: `fwrite`/`fputs`/`fputc`/`fprintf` store exactly the bytes the program produced and `fread`/`fgetc` return the stored bytes.
//!   Which byte is `'\\n'` is the COMPILER's convention: 0x0D for programs built by MPW or CodeWarrior against Apple's StdCLib, 0x0A for GCC/Retro68 builds;
//!   the library cannot know, so the only place it must pick is `fgets`' line end, which is the first CR **or** LF read (a CRLF file yields the CR line and
//!   then a one-byte LF line, as it would on a platform that does not translate).
//! * `fseek` past the end of a stream open for writing extends the file with zeros at once (ISO C extends it at the next write); on a read-only
//!   stream it fails. `fopen` failure returns NULL (`errno` is not modelled). `vfprintf`/`vsprintf` (a `va_list`), `fscanf`, `tmpfile`, `fgetpos`
//!   are not provided and refuse by name.
use crate::clib::arg;
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_ui_abi::GuestUi;
use heirloom_mac68k_engine::fm::err;
use heirloom_cpu_ppc::Cpu;

/// Bytes of one `FILE` and of the three-element `_iob`.
pub const FILE_BYTES: u32 = 16;
pub const IOB_BYTES: u32 = 3 * FILE_BYTES;
const EOF: u32 = u32::MAX; // -1
const FS_RD_PERM: u8 = 1;
const FS_WR_PERM: u8 = 2;
const FS_RD_WR_PERM: u8 = 3;
const FS_AT_MARK: u16 = 0;
const FS_FROM_START: u16 = 1;
const FS_FROM_LEOF: u16 = 2;
const CR: u8 = 0x0D;
const LF: u8 = 0x0A;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StdioFn { Fopen, Fclose, Fread, Fwrite, Fputs, Fputc, Fgetc, Fgets, Ungetc, Fprintf, Fseek, Ftell, Rewind, Feof, Ferror, Clearerr, Fflush, Setvbuf, Remove }

impl StdioFn {
    pub fn by_name(name: &str) -> Option<StdioFn> {
        use StdioFn::*;
        Some(match name {
            "fopen" => Fopen, "fclose" => Fclose, "fread" => Fread, "fwrite" => Fwrite, "fputs" => Fputs,
            "fputc" | "putc" | "_flsbuf" => Fputc, "fgetc" | "getc" | "_filbuf" => Fgetc, "fgets" => Fgets, "ungetc" => Ungetc, "fprintf" => Fprintf,
            "fseek" => Fseek, "ftell" => Ftell, "rewind" => Rewind, "feof" => Feof, "ferror" => Ferror, "clearerr" => Clearerr, "fflush" => Fflush,
            "setvbuf" => Setvbuf, "remove" => Remove,
            _ => return None,
        })
    }
}

/// One open stream (the `FILE` block is only its identity).
#[derive(Debug, Clone)]
pub(crate) struct Stream { refnum: i16, write: bool, append: bool, eof: bool, error: bool, unget: Option<u8> }

enum Which { Std(u32), File }

impl<U: GuestUi> MacPpc<U> {
    fn which(&self, fp: u32, who: &str) -> Result<Which, String> {
        if let Some(&base) = self.cdata.get("_iob") {
            if fp >= base && fp < base + IOB_BYTES && (fp - base) % FILE_BYTES == 0 { return Ok(Which::Std((fp - base) / FILE_BYTES)); }
        }
        if self.rt.streams.contains_key(&fp) { Ok(Which::File) } else { Err(format!("{who}: {fp:#x} is not a FILE this library opened")) }
    }

    fn mac_path(&self, p: u32, who: &str) -> Result<Vec<u8>, String> { self.cstr(p, who) }

    fn locate_path(&self, path: &[u8]) -> Result<heirloom_mac68k_engine::fm::names::Located, i16> { self.mac.fm.locate(0, 0, Some(path)) }

    /// `fopen`: the FILE address, or 0.
    fn fopen(&mut self, path: &[u8], mode: &[u8]) -> Result<u32, String> {
        let plus = mode.contains(&b'+');
        let kind = *mode.first().ok_or_else(|| String::from("fopen: empty mode string"))?;
        if !matches!(kind, b'r' | b'w' | b'a') { return Err(format!("fopen: unsupported mode \"{}\"", String::from_utf8_lossy(mode))); }
        let mut loc = match self.locate_path(path) { Ok(l) => l, Err(_) => return Ok(0) };
        if loc.node.is_none() {
            if kind == b'r' { return Ok(0); }
            self.mac.fm.now_secs = self.mac.mem.read_u32(heirloom_mac68k_engine::lowmem::TIME).unwrap_or(0);
            if self.mac.fm.create_file(&loc).is_err() { return Ok(0); }
            loc = match self.locate_path(path) { Ok(l) => l, Err(_) => return Ok(0) };
        }
        let perm = match (kind, plus) { (b'r', false) => FS_RD_PERM, (b'r', true) | (_, true) => FS_RD_WR_PERM, _ => FS_WR_PERM };
        let refnum = match self.mac.fm.open(&loc, false, perm) { Ok(r) => r, Err(_) => return Ok(0) };
        if kind == b'w' && self.mac.fm.set_eof(refnum, 0).is_err() { let _ = self.mac.fm.close(refnum); return Ok(0); }
        let fp = self.mac.mm.new_ptr(&mut self.mac.mem, &mut self.mac.heap, FILE_BYTES, true).map_err(|e| format!("fopen: no memory for the FILE (memory error {e})"))?;
        self.rt.streams.insert(fp, Stream { refnum, write: perm != FS_RD_PERM, append: kind == b'a', eof: false, error: false, unget: None });
        self.mac.log.push(format!("[stdio] fopen \"{}\" \"{}\" -> {fp:#x} (refnum {refnum})", String::from_utf8_lossy(path), String::from_utf8_lossy(mode)));
        Ok(fp)
    }

    /// Read up to `n` bytes from stream `fp`.
    fn stream_read(&mut self, fp: u32, n: u32, who: &str) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        let s = self.rt.streams.get_mut(&fp).ok_or_else(|| format!("{who}: {fp:#x} is not a FILE this library opened"))?;
        if n > 0 { if let Some(b) = s.unget.take() { out.push(b); } }
        let refnum = s.refnum;
        let want = n - out.len() as u32;
        if want > 0 {
            match self.mac.fm.read(refnum, FS_AT_MARK, 0, want) {
                Ok(r) => {
                    out.extend_from_slice(&r.data);
                    let s = self.rt.streams.get_mut(&fp).expect("checked");
                    if r.result == err::EOF_ERR || (r.data.len() as u32) < want { s.eof = true; }
                }
                Err(_) => { self.rt.streams.get_mut(&fp).expect("checked").error = true; }
            }
        }
        Ok(out)
    }

    fn stream_write(&mut self, fp: u32, data: &[u8], who: &str) -> Result<bool, String> {
        match self.which(fp, who)? {
            Which::Std(0) => Ok(false),
            Which::Std(_) => { self.console_write(data); Ok(true) }
            Which::File => {
                let s = self.rt.streams.get(&fp).expect("which");
                if !s.write { self.rt.streams.get_mut(&fp).expect("which").error = true; return Ok(false); }
                let (refnum, mode) = (s.refnum, if s.append { FS_FROM_LEOF } else { FS_AT_MARK });
                let ok = self.mac.fm.write(refnum, mode, 0, data).is_ok();
                if !ok { self.rt.streams.get_mut(&fp).expect("which").error = true; }
                Ok(ok)
            }
        }
    }

    /// Serve `f` for the call the CPU is making; the result goes in r3.
    pub(crate) fn stdio_call(&mut self, f: StdioFn, cpu: &mut Cpu) -> Result<(), String> {
        use StdioFn::*;
        let name = format!("{f:?}").to_lowercase();
        let a = |m: &Self, i: usize| arg(m, cpu, i);
        let (a0, a1, a2, a3) = (a(self, 0)?, a(self, 1)?, a(self, 2).unwrap_or(0), a(self, 3).unwrap_or(0));
        let r: u32 = match f {
            Fopen => { let (p, m) = (self.mac_path(a0, "fopen")?, self.cstr(a1, "fopen")?); self.fopen(&p, &m)? }
            Fclose => match self.which(a0, "fclose")? {
                Which::Std(_) => 0,
                Which::File => {
                    let s = self.rt.streams.remove(&a0).expect("which");
                    let _ = self.mac.fm.close(s.refnum);
                    let _ = self.mac.mm.dispose_ptr(&mut self.mac.heap, a0);
                    0
                }
            },
            Fread => {
                let (size, n) = (a1, a2);
                let total = size.checked_mul(n).ok_or_else(|| String::from("fread: size * nmemb overflows"))?;
                match self.which(a3, "fread")? {
                    Which::Std(_) => 0,
                    Which::File => {
                        let data = self.stream_read(a3, total, "fread")?;
                        let whole = if size == 0 { 0 } else { data.len() as u32 / size };
                        self.put_bytes(a0, &data, "fread")?;
                        whole
                    }
                }
            }
            Fwrite => {
                let (size, n) = (a1, a2);
                let total = size.checked_mul(n).ok_or_else(|| String::from("fwrite: size * nmemb overflows"))?;
                let data = self.get_bytes(a0, total, "fwrite")?;
                if self.stream_write(a3, &data, "fwrite")? { n } else { 0 }
            }
            Fputs => { let s = self.cstr(a0, "fputs")?; if self.stream_write(a1, &s, "fputs")? { 1 } else { EOF } }
            Fputc => { let c = a0 as u8; if self.stream_write(a1, &[c], "fputc")? { u32::from(c) } else { EOF } }
            Fgetc => match self.which(a0, "fgetc")? {
                Which::Std(_) => EOF,
                Which::File => { let b = self.stream_read(a0, 1, "fgetc")?; b.first().map_or(EOF, |&c| u32::from(c)) }
            },
            Fgets => {
                // fgets(buf, n, fp): at most n-1 bytes, stopping after the first CR or LF (either compiler's '\n'); NULL when nothing was read
                let (buf, n) = (a0, a1);
                if n == 0 { 0 } else {
                    if matches!(self.which(a2, "fgets")?, Which::Std(_)) { cpu.gpr[3] = 0; return Ok(()); }
                    let mut line = Vec::new();
                    while (line.len() as u32) + 1 < n {
                        let b = self.stream_read(a2, 1, "fgets")?;
                        let Some(&c) = b.first() else { break };
                        line.push(c);
                        if c == CR || c == LF { break; }
                    }
                    if line.is_empty() { 0 } else { let mut z = line; z.push(0); self.put_bytes(buf, &z, "fgets")?; buf }
                }
            }
            Ungetc => match self.which(a1, "ungetc")? {
                Which::Std(_) => EOF,
                Which::File => {
                    if a0 == EOF { EOF } else {
                        let s = self.rt.streams.get_mut(&a1).expect("which");
                        if s.unget.is_some() { EOF } else { s.unget = Some(a0 as u8); s.eof = false; a0 & 0xFF }
                    }
                }
            },
            Fprintf => { let s = self.format_c(cpu, 1, a1)?; if self.stream_write(a0, &s, "fprintf")? { s.len() as u32 } else { EOF } }
            Fseek => match self.which(a0, "fseek")? {
                Which::Std(_) => EOF,
                Which::File => {
                    let s = self.rt.streams.get(&a0).expect("which").clone();
                    let (len, mark) = (self.mac.fm.eof(s.refnum).unwrap_or(0), self.mac.fm.mark(s.refnum).unwrap_or(0));
                    let pending = u32::from(s.unget.is_some());
                    let base = match a2 { 0 => 0i64, 1 => i64::from(mark) - i64::from(pending), 2 => i64::from(len), _ => { cpu.gpr[3] = EOF; return Ok(()); } };
                    let abs = base + i64::from(a1 as i32);
                    let mut ok = abs >= 0 && abs <= i64::from(u32::MAX);
                    if ok && abs as u64 > u64::from(len) { ok = s.write && self.mac.fm.set_eof(s.refnum, abs as u32).is_ok(); }
                    if ok { ok = self.mac.fm.set_mark(s.refnum, FS_FROM_START, abs as i32).is_ok(); }
                    let s = self.rt.streams.get_mut(&a0).expect("which");
                    if ok { s.eof = false; s.unget = None; }
                    if ok { 0 } else { EOF }
                }
            },
            Ftell => match self.which(a0, "ftell")? {
                Which::Std(_) => EOF,
                Which::File => { let s = &self.rt.streams[&a0]; self.mac.fm.mark(s.refnum).map_or(EOF, |m| m - u32::from(s.unget.is_some())) }
            },
            Rewind => {
                if let Which::File = self.which(a0, "rewind")? {
                    let refnum = self.rt.streams[&a0].refnum;
                    let _ = self.mac.fm.set_mark(refnum, FS_FROM_START, 0);
                    let s = self.rt.streams.get_mut(&a0).expect("which");
                    s.eof = false; s.error = false; s.unget = None;
                }
                0
            }
            Feof => match self.which(a0, "feof")? { Which::Std(0) => 1, Which::Std(_) => 0, Which::File => u32::from(self.rt.streams[&a0].eof) },
            Ferror => match self.which(a0, "ferror")? { Which::Std(_) => 0, Which::File => u32::from(self.rt.streams[&a0].error) },
            Clearerr => { if let Which::File = self.which(a0, "clearerr")? { let s = self.rt.streams.get_mut(&a0).expect("which"); s.eof = false; s.error = false; } 0 }
            // streams are unbuffered: nothing to flush (fflush(NULL) = all streams); setvbuf is accepted as the no-op it is
            Fflush | Setvbuf => { if !(f == Fflush && a0 == 0) { self.which(a0, &name)?; } 0 }
            Remove => {
                let p = self.mac_path(a0, "remove")?;
                match self.locate_path(&p) {
                    Ok(l) if l.node.is_some() => if self.mac.fm.delete(&l).is_ok() { 0 } else { EOF },
                    _ => EOF,
                }
            }
        };
        cpu.gpr[3] = r;
        Ok(())
    }
}
