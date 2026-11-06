use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::header::{
    CPU_TYPE_X86_64, LC_DYLD_CHAINED_FIXUPS, LC_DYLD_INFO_ONLY, LC_LOAD_DYLIB,
    LC_LOAD_DYLINKER, LC_MAIN, LC_SEGMENT_64, LC_UNIXTHREAD, MH_EXECUTE, MH_MAGIC_64, MH_PIE,
};

const PAGE: u64 = 0x1000;
const CODE_FILE_OFFSET: u64 = 0x400;
const DEFAULT_BASE: u64 = 0x1_0000_0000;
const DATA_SEGMENT_INDEX: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Patch {
    GotRel32 { at: usize, import: usize },
    DataRel32 { at: usize, data_offset: usize },
}

#[derive(Debug, Clone)]
struct Import {
    name: String,
    lazy: bool,
}

#[derive(Debug, Clone)]
pub struct ImageBuilder {
    code: Vec<u8>,
    imports: Vec<Import>,
    data: Vec<u8>,
    rebase_data_offsets: Vec<usize>,
    patches: Vec<Patch>,
    pie: bool,
    arch: u32,
    unixthread: bool,
    chained: bool,
    base: u64,
}

impl Default for ImageBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ImageBuilder {
    pub fn new() -> Self {
        Self {
            code: Vec::new(),
            imports: Vec::new(),
            data: Vec::new(),
            rebase_data_offsets: Vec::new(),
            patches: Vec::new(),
            pie: true,
            arch: CPU_TYPE_X86_64,
            unixthread: false,
            chained: false,
            base: DEFAULT_BASE,
        }
    }

    pub fn code(mut self, bytes: &[u8]) -> Self {
        self.code = bytes.to_vec();
        self
    }

    pub fn import(&mut self, name: &str) -> usize {
        let index = self.imports.len();
        self.imports.push(Import {
            name: name.to_string(),
            lazy: false,
        });
        index
    }

    pub fn lazy_import(&mut self, name: &str) -> usize {
        let index = self.imports.len();
        self.imports.push(Import {
            name: name.to_string(),
            lazy: true,
        });
        index
    }

    pub fn data(mut self, bytes: &[u8]) -> Self {
        self.data = bytes.to_vec();
        self
    }

    pub fn rebase_at_data(mut self, data_offset: usize) -> Self {
        self.rebase_data_offsets.push(data_offset);
        self
    }

    pub fn pie(mut self, is_pie: bool) -> Self {
        self.pie = is_pie;
        self
    }

    pub fn arch(mut self, cputype: u32) -> Self {
        self.arch = cputype;
        self
    }

    pub fn entry_unixthread(mut self) -> Self {
        self.unixthread = true;
        self
    }

    pub fn chained_fixups_marker(mut self) -> Self {
        self.chained = true;
        self
    }

    pub fn patch(mut self, arg: Patch) -> Self {
        self.patches.push(arg);
        self
    }

    pub fn build(&self) -> BuiltImage {
        let base = self.base;
        let code_vmaddr = base + CODE_FILE_OFFSET;
        let text_filesize = round_up(CODE_FILE_OFFSET + self.code.len() as u64, PAGE);
        let data_segment_vmaddr = base + text_filesize;
        let got_vmaddr = data_segment_vmaddr;
        let got_len = self.imports.len() as u64 * 8;
        let data_vmaddr = got_vmaddr + got_len;
        let data_filesize = round_up(got_len + self.data.len() as u64, PAGE);
        let linkedit_vmaddr = data_segment_vmaddr + data_filesize;
        let linkedit_fileoff = text_filesize + data_filesize;

        let rebase = self.rebase_stream(got_len);
        let bind = self.bind_stream();
        let lazy_bind = self.lazy_bind_stream();
        let rebase_off = linkedit_fileoff as u32;
        let bind_off = rebase_off + rebase.len() as u32;
        let lazy_bind_off = bind_off + bind.len() as u32;
        let linkedit_filesize = (rebase.len() + bind.len() + lazy_bind.len()) as u64;
        let linkedit_vmsize = round_up(linkedit_filesize, PAGE);

        let mut commands = Vec::new();
        write_segment(
            &mut commands,
            &SegmentOut {
                name: "__PAGEZERO",
                vmaddr: 0,
                vmsize: base,
                fileoff: 0,
                filesize: 0,
                maxprot: 0,
                initprot: 0,
                sections: &[],
            },
        );
        write_segment(
            &mut commands,
            &SegmentOut {
                name: "__TEXT",
                vmaddr: base,
                vmsize: text_filesize,
                fileoff: 0,
                filesize: text_filesize,
                maxprot: 5,
                initprot: 5,
                sections: &[SectionOut {
                sectname: "__text",
                segname: "__TEXT",
                addr: code_vmaddr,
                size: self.code.len() as u64,
                offset: CODE_FILE_OFFSET as u32,
                flags: 0x8000_0400,
            }],
            },
        );
        write_segment(
            &mut commands,
            &SegmentOut {
                name: "__DATA",
                vmaddr: data_segment_vmaddr,
                vmsize: data_filesize,
                fileoff: text_filesize,
                filesize: data_filesize,
                maxprot: 3,
                initprot: 3,
                sections: &[
                SectionOut {
                    sectname: "__got",
                    segname: "__DATA",
                    addr: got_vmaddr,
                    size: got_len,
                    offset: text_filesize as u32,
                    flags: 6,
                },
                SectionOut {
                    sectname: "__data",
                    segname: "__DATA",
                    addr: data_vmaddr,
                    size: self.data.len() as u64,
                    offset: (text_filesize + got_len) as u32,
                    flags: 0,
                },
            ],
            },
        );
        write_segment(
            &mut commands,
            &SegmentOut {
                name: "__LINKEDIT",
                vmaddr: linkedit_vmaddr,
                vmsize: linkedit_vmsize,
                fileoff: linkedit_fileoff,
                filesize: linkedit_filesize,
                maxprot: 1,
                initprot: 1,
                sections: &[],
            },
        );

        push_u32(&mut commands, LC_DYLD_INFO_ONLY);
        push_u32(&mut commands, 48);
        for (offset, size) in [
            (rebase_off, rebase.len() as u32),
            (bind_off, bind.len() as u32),
            (0, 0),
            (lazy_bind_off, lazy_bind.len() as u32),
            (0, 0),
        ] {
            push_u32(&mut commands, offset);
            push_u32(&mut commands, size);
        }

        write_dylinker(&mut commands, "/usr/lib/dyld");
        if self.unixthread {
            push_u32(&mut commands, LC_UNIXTHREAD);
            push_u32(&mut commands, 184);
            push_u32(&mut commands, 4);
            push_u32(&mut commands, 42);
            for register in 0..21 {
                push_u64(
                    &mut commands,
                    if register == 16 { code_vmaddr } else { 0 },
                );
            }
        } else {
            push_u32(&mut commands, LC_MAIN);
            push_u32(&mut commands, 24);
            push_u64(&mut commands, CODE_FILE_OFFSET);
            push_u64(&mut commands, 0);
        }
        write_dylib(&mut commands, "/usr/lib/libSystem.B.dylib");
        if self.chained {
            push_u32(&mut commands, LC_DYLD_CHAINED_FIXUPS);
            push_u32(&mut commands, 16);
            push_u32(&mut commands, 0);
            push_u32(&mut commands, 0);
        }

        let ncmds = 8 + usize::from(self.chained);
        debug_assert!(32 + commands.len() <= CODE_FILE_OFFSET as usize);
        let mut code = self.code.clone();
        for patch in &self.patches {
            let (at, target) = match *patch {
                Patch::GotRel32 { at, import } => (at, got_vmaddr + import as u64 * 8),
                Patch::DataRel32 { at, data_offset } => {
                    (at, data_vmaddr + data_offset as u64)
                }
            };
            if let Some(end) = at.checked_add(4) {
                if end <= code.len() {
                    let next_ip = code_vmaddr + at as u64 + 4;
                    let displacement = (target as i64).wrapping_sub(next_ip as i64) as i32;
                    code[at..end].copy_from_slice(&displacement.to_le_bytes());
                }
            }
        }

        let final_size = linkedit_fileoff + linkedit_filesize;
        let mut bytes = Vec::with_capacity(final_size as usize);
        push_u32(&mut bytes, MH_MAGIC_64);
        push_u32(&mut bytes, self.arch);
        push_u32(&mut bytes, 3);
        push_u32(&mut bytes, MH_EXECUTE);
        push_u32(&mut bytes, ncmds as u32);
        push_u32(&mut bytes, commands.len() as u32);
        push_u32(&mut bytes, if self.pie { MH_PIE } else { 0 });
        push_u32(&mut bytes, 0);
        bytes.extend_from_slice(&commands);
        bytes.resize(CODE_FILE_OFFSET as usize, 0);
        bytes.extend_from_slice(&code);
        bytes.resize(text_filesize as usize, 0);
        bytes.resize(bytes.len() + got_len as usize, 0);
        bytes.extend_from_slice(&self.data);
        bytes.resize(linkedit_fileoff as usize, 0);
        bytes.extend_from_slice(&rebase);
        bytes.extend_from_slice(&bind);
        bytes.extend_from_slice(&lazy_bind);

        BuiltImage {
            bytes,
            text_vmaddr: base,
            code_vmaddr,
            got_vmaddr,
            data_vmaddr,
            imports: self.imports.iter().map(|import| import.name.clone()).collect(),
        }
    }

    fn rebase_stream(&self, got_len: u64) -> Vec<u8> {
        let mut stream = Vec::new();
        for &offset in &self.rebase_data_offsets {
            stream.extend_from_slice(&[0x11, 0x20 | DATA_SEGMENT_INDEX]);
            encode_uleb(got_len + offset as u64, &mut stream);
            stream.push(0x51);
        }
        stream.push(0);
        pad_stream(stream)
    }

    fn bind_stream(&self) -> Vec<u8> {
        let mut stream = Vec::new();
        for (index, import) in self.imports.iter().enumerate().filter(|(_, import)| !import.lazy) {
            stream.extend_from_slice(&[0x11, 0x40]);
            stream.extend_from_slice(import.name.as_bytes());
            stream.extend_from_slice(&[0, 0x51, 0x70 | DATA_SEGMENT_INDEX]);
            encode_uleb(index as u64 * 8, &mut stream);
            stream.push(0x90);
        }
        stream.push(0);
        pad_stream(stream)
    }

    fn lazy_bind_stream(&self) -> Vec<u8> {
        let mut stream = Vec::new();
        for (index, import) in self.imports.iter().enumerate().filter(|(_, import)| import.lazy) {
            stream.push(0x70 | DATA_SEGMENT_INDEX);
            encode_uleb(index as u64 * 8, &mut stream);
            stream.extend_from_slice(&[0x11, 0x40]);
            stream.extend_from_slice(import.name.as_bytes());
            stream.extend_from_slice(&[0, 0x90, 0]);
        }
        pad_stream(stream)
    }
}

#[derive(Debug, Clone)]
pub struct BuiltImage {
    pub bytes: Vec<u8>,
    pub text_vmaddr: u64,
    pub code_vmaddr: u64,
    pub got_vmaddr: u64,
    pub data_vmaddr: u64,
    pub imports: Vec<String>,
}

impl BuiltImage {
    pub fn got_slot_vmaddr(&self, import: usize) -> u64 {
        self.got_vmaddr + import as u64 * 8
    }
}

struct SectionOut {
    sectname: &'static str,
    segname: &'static str,
    addr: u64,
    size: u64,
    offset: u32,
    flags: u32,
}

/// One LC_SEGMENT_64 to emit.
struct SegmentOut<'a> {
    name: &'a str,
    vmaddr: u64,
    vmsize: u64,
    fileoff: u64,
    filesize: u64,
    maxprot: u32,
    initprot: u32,
    sections: &'a [SectionOut],
}

fn write_segment(output: &mut Vec<u8>, seg: &SegmentOut<'_>) {
    let SegmentOut { name, vmaddr, vmsize, fileoff, filesize, maxprot, initprot, sections } = *seg;
    push_u32(output, LC_SEGMENT_64);
    push_u32(output, (72 + sections.len() * 80) as u32);
    push_name(output, name);
    push_u64(output, vmaddr);
    push_u64(output, vmsize);
    push_u64(output, fileoff);
    push_u64(output, filesize);
    push_u32(output, maxprot);
    push_u32(output, initprot);
    push_u32(output, sections.len() as u32);
    push_u32(output, 0);
    for section in sections {
        push_name(output, section.sectname);
        push_name(output, section.segname);
        push_u64(output, section.addr);
        push_u64(output, section.size);
        push_u32(output, section.offset);
        push_u32(output, 0);
        push_u32(output, 0);
        push_u32(output, 0);
        push_u32(output, section.flags);
        push_u32(output, 0);
        push_u32(output, 0);
        push_u32(output, 0);
    }
}

fn write_dylinker(output: &mut Vec<u8>, path: &str) {
    let size = round_up(12 + path.len() as u64 + 1, 8) as usize;
    push_u32(output, LC_LOAD_DYLINKER);
    push_u32(output, size as u32);
    push_u32(output, 12);
    output.extend_from_slice(path.as_bytes());
    output.push(0);
    output.resize(output.len() + size - 12 - path.len() - 1, 0);
}

fn write_dylib(output: &mut Vec<u8>, path: &str) {
    let size = round_up(24 + path.len() as u64 + 1, 8) as usize;
    push_u32(output, LC_LOAD_DYLIB);
    push_u32(output, size as u32);
    push_u32(output, 24);
    push_u32(output, 0);
    push_u32(output, 0x1_0000);
    push_u32(output, 0x1_0000);
    output.extend_from_slice(path.as_bytes());
    output.push(0);
    output.resize(output.len() + size - 24 - path.len() - 1, 0);
}

fn push_name(output: &mut Vec<u8>, name: &str) {
    let start = output.len();
    output.extend_from_slice(name.as_bytes());
    output.resize(start + 16, 0);
}

fn push_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(output: &mut Vec<u8>, value: u64) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn encode_uleb(mut value: u64, output: &mut Vec<u8>) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        output.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn pad_stream(mut stream: Vec<u8>) -> Vec<u8> {
    while stream.len() % 8 != 0 {
        stream.push(0);
    }
    stream
}

fn round_up(value: u64, alignment: u64) -> u64 {
    (value + alignment - 1) & !(alignment - 1)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::MachO;

    #[test]
    fn patch_stores_rip_relative_displacement() {
        let mut b = ImageBuilder::new().code(&[0xFF, 0x15, 0, 0, 0, 0, 0xC3]).data(&[1, 2, 3, 4]);
        let write = b.import("_write");
        let b = b.patch(Patch::GotRel32 { at: 2, import: write });
        let image = b.build();
        let stored = i32::from_le_bytes(image.bytes[0x402..0x406].try_into().unwrap());
        let want = image.got_slot_vmaddr(write) as i64 - (image.code_vmaddr as i64 + 2 + 4);
        assert_eq!(stored as i64, want);
        assert_eq!(want, 0x1000 - 0x400 - 6, "the GOT is one page above the code's page");
        // the same code with a data patch lands on the data bytes after the GOT
        let b2 = ImageBuilder::new().code(&[0x48, 0x8D, 0x35, 0, 0, 0, 0, 0xC3]).data(&[7; 4]).patch(Patch::DataRel32 { at: 3, data_offset: 0 });
        let image2 = b2.build();
        let stored = i32::from_le_bytes(image2.bytes[0x403..0x407].try_into().unwrap());
        assert_eq!(stored as i64, image2.data_vmaddr as i64 - (image2.code_vmaddr as i64 + 3 + 4));
        assert!(MachO::parse(&image2.bytes).is_ok());
    }

    #[test]
    fn data_bytes_land_after_the_got_slots() {
        let mut b = ImageBuilder::new().data(&[0xAB, 0xCD]);
        b.import("_a");
        b.import("_b");
        let image = b.build();
        let m = MachO::parse(&image.bytes).unwrap();
        let data = m.segment_bytes(2);
        assert_eq!(&data[16..18], &[0xAB, 0xCD]);
        assert_eq!(image.data_vmaddr, image.got_vmaddr + 16);
        assert_eq!(image.imports, vec!["_a".to_string(), "_b".to_string()]);
    }
}
