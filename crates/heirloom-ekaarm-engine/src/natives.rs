use alloc::format;
use alloc::string::String;
use crate::loader::Process;

pub enum NativeOutcome {
    Return,
    Exit(i32),
    Refused(String),
}

pub fn call(proc: &mut Process, stub_index: usize) -> NativeOutcome {
    let stub = match proc.stubs.get(stub_index) {
        Some(stub) => stub,
        None => return NativeOutcome::Refused(String::from("NEXT-NEEDED <bad stub index>")),
    };
    let dll_base = stub.dll_base.clone();
    let ordinal = stub.ordinal;
    let stub_name = stub.name.clone();

    let r0 = proc.cpu.regs[0];
    let r1 = proc.cpu.regs[1];
    match stub_name.as_deref() {
        Some("_ZN4User4ExitEi") => NativeOutcome::Exit(r0 as i32),
        Some("_ZN4User5PanicERK7TDesC16i") => {
            let word0 = match proc.mem.vm.read_u32(r0) {
                Ok(word) => word,
                Err(_) => return NativeOutcome::Refused(format!(
                    "NEXT-NEEDED {}/{} _ZN4User5PanicE (TDesC16 read fault at 0x{:08X})",
                    dll_base, ordinal, r0
                )),
            };
            let len = word0 & 0x0FFF_FFFF;
            let typ = (word0 >> 28) & 0xF;
            let data_ptr = if typ == 0 {
                r0.wrapping_add(4)
            } else if typ == 1 {
                match proc.mem.vm.read_u32(r0.wrapping_add(4)) {
                    Ok(ptr) => ptr,
                    Err(_) => return NativeOutcome::Refused(format!(
                        "NEXT-NEEDED {}/{} _ZN4User5PanicE (TDesC16 pointer read fault)",
                        dll_base, ordinal
                    )),
                }
            } else {
                return NativeOutcome::Refused(format!(
                    "NEXT-NEEDED {}/{} _ZN4User5PanicE (TDesC16 type {} unsupported)",
                    dll_base, ordinal, typ
                ));
            };

            let mut panic = String::new();
            for i in 0..core::cmp::min(len, 64) {
                let addr = data_ptr.wrapping_add(i.wrapping_mul(2));
                let ch = match proc.mem.vm.read_u16(addr) {
                    Ok(ch) => ch,
                    Err(_) => return NativeOutcome::Refused(format!(
                        "NEXT-NEEDED {}/{} _ZN4User5PanicE (TDesC16 data read fault at 0x{:08X})",
                        dll_base, ordinal, addr
                    )),
                };
                panic.push(core::char::from_u32(ch as u32).unwrap_or('\u{FFFD}'));
            }
            NativeOutcome::Refused(format!(
                "NEXT-NEEDED {}/{} _ZN4User5PanicE (panic=\"{}\" reason={})",
                dll_base, ordinal, panic, r1 as i32
            ))
        }
        Some(name) => NativeOutcome::Refused(format!(
            "NEXT-NEEDED {}/{} {}", dll_base, ordinal, name
        )),
        None => NativeOutcome::Refused(format!(
            "NEXT-NEEDED {}/{} <unnamed-ordinal>", dll_base, ordinal
        )),
    }
}
