use alloc::string::String;
use alloc::vec::Vec;

pub const MH_MAGIC_64: u32 = 0xfeed_facf;
pub const FAT_MAGIC: u32 = 0xcafe_babe;
pub const CPU_TYPE_X86_64: u32 = 0x0100_0007;
pub const MH_EXECUTE: u32 = 2;
pub const MH_DYLIB: u32 = 6;
pub const MH_PIE: u32 = 0x0020_0000;

pub const LC_SEGMENT_64: u32 = 0x19;
pub const LC_LOAD_DYLINKER: u32 = 0x0e;
pub const LC_LOAD_DYLIB: u32 = 0x0c;
pub const LC_UNIXTHREAD: u32 = 0x05;
pub const LC_MAIN: u32 = 0x8000_0028;
pub const LC_DYLD_INFO: u32 = 0x22;
pub const LC_DYLD_INFO_ONLY: u32 = 0x8000_0022;
pub const LC_DYLD_CHAINED_FIXUPS: u32 = 0x8000_0034;

pub const VM_PROT_READ: u32 = 1;
pub const VM_PROT_WRITE: u32 = 2;
pub const VM_PROT_EXECUTE: u32 = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MachoError {
    Truncated(&'static str),
    BadMagic(u32),
    NotX86_64 { cputype: u32 },
    FatWithoutX86_64,
    BadLoadCommand { index: u32, why: &'static str },
    SegmentOutOfFile { segment: String },
    SegmentVmOverflow { segment: String },
    SegmentsOverlap,
    BadOpcodeStream {
        stream: &'static str,
        at: usize,
        why: &'static str,
    },
    TargetOutsideSegment { stream: &'static str },
}

impl core::fmt::Display for MachoError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Truncated(what) => write!(f, "truncated {what}"),
            Self::BadMagic(magic) => write!(f, "bad Mach-O magic 0x{magic:08x}"),
            Self::NotX86_64 { cputype } => {
                write!(f, "not an x86_64 Mach-O (cputype 0x{cputype:08x})")
            }
            Self::FatWithoutX86_64 => f.write_str("fat file has no x86_64 slice"),
            Self::BadLoadCommand { index, why } => {
                write!(f, "bad load command {index}: {why}")
            }
            Self::SegmentOutOfFile { segment } => {
                write!(f, "segment {segment} extends past the file")
            }
            Self::SegmentVmOverflow { segment } => {
                write!(f, "segment {segment} has an invalid VM range")
            }
            Self::SegmentsOverlap => f.write_str("segments overlap in VM space"),
            Self::BadOpcodeStream { stream, at, why } => {
                write!(f, "bad {stream} opcode stream at {at}: {why}")
            }
            Self::TargetOutsideSegment { stream } => {
                write!(f, "{stream} target lies outside its segment")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub sectname: String,
    pub segname: String,
    pub addr: u64,
    pub size: u64,
    pub offset: u32,
    pub flags: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub name: String,
    pub vmaddr: u64,
    pub vmsize: u64,
    pub fileoff: u64,
    pub filesize: u64,
    pub maxprot: u32,
    pub initprot: u32,
    pub sections: Vec<Section>,
}

impl Segment {
    /// Whether the segment is mapped with execute permission (`initprot` carries `VM_PROT_EXECUTE`).
    pub fn is_executable(&self) -> bool {
        self.initprot & VM_PROT_EXECUTE != 0
    }

    pub fn is_pagezero(&self) -> bool {
        self.name == "__PAGEZERO"
    }

    pub fn vm_end(&self) -> Option<u64> {
        self.vmaddr.checked_add(self.vmsize)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    Main { entryoff: u64, stacksize: u64 },
    UnixThread { rip: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DyldInfo {
    pub rebase: (u32, u32),
    pub bind: (u32, u32),
    pub weak_bind: (u32, u32),
    pub lazy_bind: (u32, u32),
    pub export: (u32, u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dylib {
    pub path: String,
    pub current_version: u32,
    pub compatibility_version: u32,
}

#[derive(Debug, Clone)]
pub struct MachO<'a> {
    pub slice: &'a [u8],
    pub filetype: u32,
    pub flags: u32,
    pub segments: Vec<Segment>,
    pub entry: Option<Entry>,
    pub dylibs: Vec<Dylib>,
    pub dyld_info: Option<DyldInfo>,
    pub has_chained_fixups: bool,
    pub dylinker: Option<String>,
}

impl<'a> MachO<'a> {
    pub fn parse(file: &'a [u8]) -> Result<MachO<'a>, MachoError> {
        if file.len() < 4 {
            return Err(MachoError::Truncated("magic"));
        }
        let first = [file[0], file[1], file[2], file[3]];
        if u32::from_be_bytes(first) == FAT_MAGIC {
            return Self::parse_fat(file);
        }
        let magic = u32::from_le_bytes(first);
        if magic != MH_MAGIC_64 {
            return Err(MachoError::BadMagic(magic));
        }
        Self::parse_thin(file)
    }

    fn parse_fat(file: &'a [u8]) -> Result<MachO<'a>, MachoError> {
        let count = read_u32_be(file, 4)? as usize;
        let table_size = count
            .checked_mul(20)
            .ok_or(MachoError::Truncated("fat arch table"))?;
        let table_end = 8usize
            .checked_add(table_size)
            .ok_or(MachoError::Truncated("fat arch table"))?;
        if table_end > file.len() {
            return Err(MachoError::Truncated("fat arch table"));
        }

        for i in 0..count {
            let at = 8 + i * 20;
            if read_u32_be(file, at)? != CPU_TYPE_X86_64 {
                continue;
            }
            let offset = read_u32_be(file, at + 8)? as u64;
            let size = read_u32_be(file, at + 12)? as u64;
            let end = offset
                .checked_add(size)
                .ok_or(MachoError::Truncated("fat x86_64 slice"))?;
            if end > file.len() as u64 {
                return Err(MachoError::Truncated("fat x86_64 slice"));
            }
            return Self::parse_thin(&file[offset as usize..end as usize]);
        }
        Err(MachoError::FatWithoutX86_64)
    }

    fn parse_thin(file: &'a [u8]) -> Result<MachO<'a>, MachoError> {
        if file.len() < 32 {
            return Err(MachoError::Truncated("mach header"));
        }
        let cputype = read_u32(file, 4, "mach header")?;
        if cputype != CPU_TYPE_X86_64 {
            return Err(MachoError::NotX86_64 { cputype });
        }
        let filetype = read_u32(file, 12, "mach header")?;
        let ncmds = read_u32(file, 16, "mach header")?;
        let sizeofcmds = read_u32(file, 20, "mach header")? as usize;
        let flags = read_u32(file, 24, "mach header")?;
        let cmds_end = 32usize
            .checked_add(sizeofcmds)
            .ok_or(MachoError::Truncated("load commands"))?;
        if cmds_end > file.len() {
            return Err(MachoError::Truncated("load commands"));
        }

        let mut result = MachO {
            slice: file,
            filetype,
            flags,
            segments: Vec::new(),
            entry: None,
            dylibs: Vec::new(),
            dyld_info: None,
            has_chained_fixups: false,
            dylinker: None,
        };

        let mut pos = 32usize;
        for index in 0..ncmds {
            if pos.checked_add(8).map_or(true, |end| end > cmds_end) {
                return Err(MachoError::BadLoadCommand {
                    index,
                    why: "truncated command header",
                });
            }
            let cmd = read_u32(file, pos, "load command")?;
            let cmdsize = read_u32(file, pos + 4, "load command")? as usize;
            if cmdsize < 8 {
                return Err(MachoError::BadLoadCommand {
                    index,
                    why: "cmdsize is less than 8",
                });
            }
            if cmdsize % 8 != 0 {
                return Err(MachoError::BadLoadCommand {
                    index,
                    why: "cmdsize is not a multiple of 8",
                });
            }
            let next = pos
                .checked_add(cmdsize)
                .ok_or(MachoError::BadLoadCommand {
                    index,
                    why: "cmdsize overflows",
                })?;
            if next > cmds_end {
                return Err(MachoError::BadLoadCommand {
                    index,
                    why: "command overruns sizeofcmds",
                });
            }

            match cmd {
                LC_SEGMENT_64 => parse_segment(file, pos, cmdsize, index, &mut result)?,
                LC_LOAD_DYLIB => {
                    if cmdsize < 24 {
                        return Err(bad_command(index, "dylib command is too small"));
                    }
                    let name_offset = read_u32(file, pos + 8, "dylib")?;
                    let current_version = read_u32(file, pos + 16, "dylib")?;
                    let compatibility_version = read_u32(file, pos + 20, "dylib")?;
                    let path = command_string(file, pos, next, name_offset, index)?;
                    result.dylibs.push(Dylib {
                        path,
                        current_version,
                        compatibility_version,
                    });
                }
                LC_LOAD_DYLINKER => {
                    if cmdsize < 12 {
                        return Err(bad_command(index, "dylinker command is too small"));
                    }
                    let name_offset = read_u32(file, pos + 8, "dylinker")?;
                    result.dylinker = Some(command_string(
                        file,
                        pos,
                        next,
                        name_offset,
                        index,
                    )?);
                }
                LC_MAIN => {
                    if cmdsize < 24 {
                        return Err(bad_command(index, "LC_MAIN is too small"));
                    }
                    result.entry = Some(Entry::Main {
                        entryoff: read_u64(file, pos + 8, "LC_MAIN")?,
                        stacksize: read_u64(file, pos + 16, "LC_MAIN")?,
                    });
                }
                LC_UNIXTHREAD => {
                    if cmdsize < 152 {
                        return Err(bad_command(index, "LC_UNIXTHREAD is too small"));
                    }
                    let flavor = read_u32(file, pos + 8, "LC_UNIXTHREAD")?;
                    if flavor != 4 {
                        return Err(bad_command(index, "not x86_THREAD_STATE64"));
                    }
                    result.entry = Some(Entry::UnixThread {
                        rip: read_u64(file, pos + 144, "LC_UNIXTHREAD")?,
                    });
                }
                LC_DYLD_INFO | LC_DYLD_INFO_ONLY => {
                    if cmdsize < 48 {
                        return Err(bad_command(index, "dyld info command is too small"));
                    }
                    result.dyld_info = Some(DyldInfo {
                        rebase: (
                            read_u32(file, pos + 8, "dyld info")?,
                            read_u32(file, pos + 12, "dyld info")?,
                        ),
                        bind: (
                            read_u32(file, pos + 16, "dyld info")?,
                            read_u32(file, pos + 20, "dyld info")?,
                        ),
                        weak_bind: (
                            read_u32(file, pos + 24, "dyld info")?,
                            read_u32(file, pos + 28, "dyld info")?,
                        ),
                        lazy_bind: (
                            read_u32(file, pos + 32, "dyld info")?,
                            read_u32(file, pos + 36, "dyld info")?,
                        ),
                        export: (
                            read_u32(file, pos + 40, "dyld info")?,
                            read_u32(file, pos + 44, "dyld info")?,
                        ),
                    });
                }
                LC_DYLD_CHAINED_FIXUPS => result.has_chained_fixups = true,
                _ => {}
            }
            pos = next;
        }

        let mut ordered: Vec<&Segment> = result
            .segments
            .iter()
            .filter(|segment| !segment.is_pagezero() && segment.vmsize != 0)
            .collect();
        ordered.sort_by_key(|segment| segment.vmaddr);
        for pair in ordered.windows(2) {
            if pair[0].vm_end().unwrap_or(u64::MAX) > pair[1].vmaddr {
                return Err(MachoError::SegmentsOverlap);
            }
        }
        Ok(result)
    }

    pub fn segment_index(&self, name: &str) -> Option<usize> {
        self.segments.iter().position(|segment| segment.name == name)
    }

    pub fn segment_bytes(&self, segment: usize) -> &'a [u8] {
        let Some(segment) = self.segments.get(segment) else {
            return &[];
        };
        let Ok(start) = usize::try_from(segment.fileoff) else {
            return &[];
        };
        let Ok(size) = usize::try_from(segment.filesize) else {
            return &[];
        };
        let Some(end) = start.checked_add(size) else {
            return &[];
        };
        self.slice.get(start..end).unwrap_or(&[])
    }

    pub fn vm_span(&self) -> Option<(u64, u64)> {
        let mut low = u64::MAX;
        let mut high = 0u64;
        let mut any = false;
        for segment in self
            .segments
            .iter()
            .filter(|segment| !segment.is_pagezero())
        {
            low = low.min(segment.vmaddr);
            high = high.max(segment.vm_end()?);
            any = true;
        }
        any.then_some((low, high))
    }

    pub fn is_pie(&self) -> bool {
        self.flags & MH_PIE != 0
    }
}

fn parse_segment(
    file: &[u8],
    pos: usize,
    cmdsize: usize,
    index: u32,
    result: &mut MachO<'_>,
) -> Result<(), MachoError> {
    if cmdsize < 72 {
        return Err(bad_command(index, "segment command is too small"));
    }
    let body = pos + 8;
    let name = name16(file, body, "segment name")?;
    let vmaddr = read_u64(file, body + 16, "segment")?;
    let vmsize = read_u64(file, body + 24, "segment")?;
    let fileoff = read_u64(file, body + 32, "segment")?;
    let filesize = read_u64(file, body + 40, "segment")?;
    let maxprot = read_u32(file, body + 48, "segment")?;
    let initprot = read_u32(file, body + 52, "segment")?;
    let nsects = read_u32(file, body + 56, "segment")? as usize;
    let sections_size = nsects
        .checked_mul(80)
        .ok_or_else(|| bad_command(index, "section count overflows"))?;
    let required = 72usize
        .checked_add(sections_size)
        .ok_or_else(|| bad_command(index, "section count overflows"))?;
    if required > cmdsize {
        return Err(bad_command(index, "sections overrun cmdsize"));
    }

    let file_end = fileoff
        .checked_add(filesize)
        .ok_or_else(|| MachoError::SegmentOutOfFile {
            segment: name.clone(),
        })?;
    if file_end > file.len() as u64 {
        return Err(MachoError::SegmentOutOfFile {
            segment: name.clone(),
        });
    }
    if vmaddr.checked_add(vmsize).is_none() || filesize > vmsize {
        return Err(MachoError::SegmentVmOverflow { segment: name });
    }

    let mut sections = Vec::new();
    for section in 0..nsects {
        let at = body + 64 + section * 80;
        sections.push(Section {
            sectname: name16(file, at, "section name")?,
            segname: name16(file, at + 16, "section segment name")?,
            addr: read_u64(file, at + 32, "section")?,
            size: read_u64(file, at + 40, "section")?,
            offset: read_u32(file, at + 48, "section")?,
            flags: read_u32(file, at + 64, "section")?,
        });
    }
    result.segments.push(Segment {
        name,
        vmaddr,
        vmsize,
        fileoff,
        filesize,
        maxprot,
        initprot,
        sections,
    });
    Ok(())
}

fn bad_command(index: u32, why: &'static str) -> MachoError {
    MachoError::BadLoadCommand { index, why }
}

fn command_string(
    file: &[u8],
    command: usize,
    end: usize,
    offset: u32,
    index: u32,
) -> Result<String, MachoError> {
    let start = command
        .checked_add(offset as usize)
        .ok_or_else(|| bad_command(index, "string offset overflows"))?;
    if start >= end {
        return Err(bad_command(index, "string offset is outside command"));
    }
    let bytes = &file[start..end];
    let length = bytes
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| bad_command(index, "unterminated command string"))?;
    Ok(String::from_utf8_lossy(&bytes[..length]).into_owned())
}

fn name16(file: &[u8], offset: usize, what: &'static str) -> Result<String, MachoError> {
    let end = offset
        .checked_add(16)
        .ok_or(MachoError::Truncated(what))?;
    let bytes = file.get(offset..end).ok_or(MachoError::Truncated(what))?;
    let length = bytes.iter().position(|byte| *byte == 0).unwrap_or(16);
    Ok(String::from_utf8_lossy(&bytes[..length]).into_owned())
}

fn read_u32(file: &[u8], offset: usize, what: &'static str) -> Result<u32, MachoError> {
    let end = offset
        .checked_add(4)
        .ok_or(MachoError::Truncated(what))?;
    let bytes = file.get(offset..end).ok_or(MachoError::Truncated(what))?;
    Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
}

fn read_u64(file: &[u8], offset: usize, what: &'static str) -> Result<u64, MachoError> {
    let end = offset
        .checked_add(8)
        .ok_or(MachoError::Truncated(what))?;
    let bytes = file.get(offset..end).ok_or(MachoError::Truncated(what))?;
    Ok(u64::from_le_bytes(bytes.try_into().unwrap()))
}

fn read_u32_be(file: &[u8], offset: usize) -> Result<u32, MachoError> {
    let end = offset
        .checked_add(4)
        .ok_or(MachoError::Truncated("fat header"))?;
    let bytes = file
        .get(offset..end)
        .ok_or(MachoError::Truncated("fat header"))?;
    Ok(u32::from_be_bytes(bytes.try_into().unwrap()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writer::ImageBuilder;
    use alloc::vec;

    const BASE: u64 = 0x1_0000_0000;
    // Byte offsets inside the writer's image: commands start at 32; __PAGEZERO is 72 bytes, __TEXT 152, __DATA 232.
    const TEXT_CMD: usize = 32 + 72;
    const DATA_CMD: usize = TEXT_CMD + 152;

    fn good() -> crate::writer::BuiltImage {
        let mut b = ImageBuilder::new().code(&[0x90; 16]);
        b.import("_write");
        b.build()
    }

    fn put_u64(bytes: &mut [u8], at: usize, v: u64) {
        bytes[at..at + 8].copy_from_slice(&v.to_le_bytes());
    }

    fn put_u32(bytes: &mut [u8], at: usize, v: u32) {
        bytes[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    #[test]
    fn parse_roundtrips_segments_and_entry() {
        let image = good();
        let m = MachO::parse(&image.bytes).unwrap();
        let names: Vec<_> = m.segments.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["__PAGEZERO", "__TEXT", "__DATA", "__LINKEDIT"]);
        let text = &m.segments[1];
        assert_eq!((text.vmaddr, text.vmsize, text.initprot, text.maxprot), (BASE, 0x1000, 5, 5));
        assert!(text.is_executable());
        assert_eq!(text.sections.len(), 1);
        assert_eq!(text.sections[0].sectname, "__text");
        assert_eq!(text.sections[0].addr, BASE + 0x400);
        assert_eq!(text.sections[0].size, 16);
        let data = &m.segments[2];
        assert_eq!((data.vmaddr, data.vmsize, data.initprot), (BASE + 0x1000, 0x1000, 3));
        assert!(!data.is_executable());
        assert!(m.segments[0].is_pagezero());
        assert_eq!(m.segments[0].vmsize, BASE);
        assert_eq!(m.entry, Some(Entry::Main { entryoff: 0x400, stacksize: 0 }));
        assert_eq!(m.dylibs.len(), 1);
        assert_eq!(m.dylibs[0].path, "/usr/lib/libSystem.B.dylib");
        assert_eq!(m.dylinker.as_deref(), Some("/usr/lib/dyld"));
        assert!(m.is_pie());
        assert_eq!(m.filetype, MH_EXECUTE);
        assert_eq!(m.vm_span(), Some((BASE, BASE + 0x3000)));
        assert_eq!(m.segment_index("__DATA"), Some(2));
        assert_eq!(m.segment_index("__NOPE"), None);
        assert_eq!(m.segment_bytes(1).len(), 0x1000);
        assert!(!m.has_chained_fixups);
    }

    #[test]
    fn non_pie_is_reported() {
        let image = ImageBuilder::new().pie(false).build();
        assert!(!MachO::parse(&image.bytes).unwrap().is_pie());
    }

    #[test]
    fn rejects_bad_magic() {
        let mut bytes = good().bytes;
        bytes[..4].copy_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
        assert_eq!(MachO::parse(&bytes).unwrap_err(), MachoError::BadMagic(0xefbe_adde));
    }

    #[test]
    fn rejects_arm64() {
        let image = ImageBuilder::new().arch(0x0100_000c).build();
        assert_eq!(MachO::parse(&image.bytes).unwrap_err(), MachoError::NotX86_64 { cputype: 0x0100_000c });
    }

    #[test]
    fn rejects_truncated_header_and_commands() {
        let bytes = good().bytes;
        let cmds_end = 32 + u32::from_le_bytes(bytes[20..24].try_into().unwrap()) as usize;
        for n in 0..cmds_end {
            assert!(MachO::parse(&bytes[..n]).is_err(), "a {n}-byte prefix parsed");
        }
    }

    #[test]
    fn rejects_segment_past_end_of_file() {
        let mut bytes = good().bytes;
        let len = bytes.len() as u64;
        put_u64(&mut bytes, TEXT_CMD + 8 + 16 + 8 + 8 + 8, len + 0x1000); // __TEXT filesize
        assert_eq!(
            MachO::parse(&bytes).unwrap_err(),
            MachoError::SegmentOutOfFile { segment: "__TEXT".into() }
        );
    }

    #[test]
    fn rejects_file_range_that_overflows() {
        let mut bytes = good().bytes;
        put_u64(&mut bytes, TEXT_CMD + 8 + 16 + 16, u64::MAX); // __TEXT fileoff
        assert!(matches!(MachO::parse(&bytes).unwrap_err(), MachoError::SegmentOutOfFile { .. }));
    }

    #[test]
    fn rejects_vm_range_that_overflows() {
        let mut bytes = good().bytes;
        put_u64(&mut bytes, TEXT_CMD + 8 + 16, u64::MAX - 0x10); // __TEXT vmaddr
        assert!(matches!(MachO::parse(&bytes).unwrap_err(), MachoError::SegmentVmOverflow { .. }));
    }

    #[test]
    fn rejects_misaligned_and_overrunning_cmdsize() {
        let mut bytes = good().bytes;
        put_u32(&mut bytes, TEXT_CMD + 4, 12);
        assert!(matches!(
            MachO::parse(&bytes).unwrap_err(),
            MachoError::BadLoadCommand { index: 1, why: "cmdsize is not a multiple of 8" }
        ));
        let mut bytes = good().bytes;
        put_u32(&mut bytes, TEXT_CMD + 4, 0x4000);
        assert!(matches!(
            MachO::parse(&bytes).unwrap_err(),
            MachoError::BadLoadCommand { index: 1, why: "command overruns sizeofcmds" }
        ));
        let mut bytes = good().bytes;
        put_u32(&mut bytes, TEXT_CMD + 4, 0);
        assert!(matches!(MachO::parse(&bytes).unwrap_err(), MachoError::BadLoadCommand { index: 1, .. }));
    }

    #[test]
    fn rejects_overlapping_segments() {
        let mut bytes = good().bytes;
        put_u64(&mut bytes, DATA_CMD + 8 + 16, BASE); // __DATA vmaddr = __TEXT vmaddr
        assert_eq!(MachO::parse(&bytes).unwrap_err(), MachoError::SegmentsOverlap);
    }

    #[test]
    fn rejects_sections_that_overrun_their_command() {
        let mut bytes = good().bytes;
        put_u32(&mut bytes, TEXT_CMD + 8 + 56, 9); // __TEXT nsects = 9
        assert!(matches!(MachO::parse(&bytes).unwrap_err(), MachoError::BadLoadCommand { index: 1, .. }));
    }

    fn fat(records: &[(u32, usize, usize)], payload: &[u8], payload_at: usize) -> Vec<u8> {
        let mut f = Vec::new();
        f.extend_from_slice(&0xcafe_babe_u32.to_be_bytes());
        f.extend_from_slice(&(records.len() as u32).to_be_bytes());
        for &(cpu, off, size) in records {
            f.extend_from_slice(&cpu.to_be_bytes());
            f.extend_from_slice(&3u32.to_be_bytes());
            f.extend_from_slice(&(off as u32).to_be_bytes());
            f.extend_from_slice(&(size as u32).to_be_bytes());
            f.extend_from_slice(&12u32.to_be_bytes());
        }
        f.resize(payload_at, 0);
        f.extend_from_slice(payload);
        f
    }

    #[test]
    fn fat_file_selects_x86_64() {
        let image = good().bytes;
        let file = fat(&[(0x0100_000c, 0x800, 16), (CPU_TYPE_X86_64, 0x1000, image.len())], &image, 0x1000);
        let m = MachO::parse(&file).unwrap();
        assert_eq!(m.slice, &image[..]);
        assert_eq!(m.segments.len(), 4);
        let only_arm = fat(&[(0x0100_000c, 0x1000, image.len())], &image, 0x1000);
        assert_eq!(MachO::parse(&only_arm).unwrap_err(), MachoError::FatWithoutX86_64);
        let past_end = fat(&[(CPU_TYPE_X86_64, 0x1000, image.len() + 1)], &image, 0x1000);
        assert!(matches!(MachO::parse(&past_end).unwrap_err(), MachoError::Truncated(_)));
    }

    #[test]
    fn unixthread_entry_reads_rip() {
        let image = ImageBuilder::new().code(&[0x90; 8]).entry_unixthread().build();
        let m = MachO::parse(&image.bytes).unwrap();
        assert_eq!(m.entry, Some(Entry::UnixThread { rip: image.code_vmaddr }));
    }

    #[test]
    fn chained_fixups_marker_is_reported() {
        let image = ImageBuilder::new().chained_fixups_marker().build();
        assert!(MachO::parse(&image.bytes).unwrap().has_chained_fixups);
    }

    #[test]
    fn dyld_info_pairs_are_read() {
        let mut b = ImageBuilder::new();
        b.import("_write");
        let image = b.build();
        let info = MachO::parse(&image.bytes).unwrap().dyld_info.unwrap();
        assert!(info.bind.1 > 0, "a bind stream was written");
        assert!(info.bind.0 as usize + info.bind.1 as usize <= image.bytes.len());
    }

    #[test]
    fn flipping_header_bytes_never_panics() {
        let bytes = good().bytes;
        for i in 0..0x200 {
            let mut b = bytes.clone();
            b[i] = 0xFF;
            if let Ok(m) = MachO::parse(&b) {
                let _ = (m.rebases(), m.binds(), m.vm_span());
                for s in 0..m.segments.len() {
                    let _ = m.segment_bytes(s);
                }
            }
        }
    }
}
