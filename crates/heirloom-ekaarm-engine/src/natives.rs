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
    let name = stub_name.as_deref().unwrap_or("");

    // RAllocator / RHeap members need a guest-resident allocator object with a vtable, which this engine does not build
    // (User::Allocator is refused by name for the same reason), so they are refused before anything dereferences one.
    // DELETE WHEN the first corpus image's next refusal is an RAllocator member or User::Allocator (then build the object).
    if name.starts_with("_ZN9RAllocator") || name.starts_with("_ZN5RHeap") {
        return NativeOutcome::Refused(format!(
            "NEXT-NEEDED euser/{} {} (no guest RAllocator object yet)", ordinal, name
        ));
    }

    let r0 = proc.cpu.regs[0];
    let r1 = proc.cpu.regs[1];
    match name {
        "_ZN8UserHeap15SetupThreadHeapEiR24SStdEpocThreadCreateInfo" => {
            let info_ptr = proc.cpu.regs[1];
            let allocator = match proc.mem.vm.read_u32(info_ptr + 0x30) {
                Ok(v) => v,
                Err(_) => return NativeOutcome::Refused(format!(
                    "NEXT-NEEDED euser SetupThreadHeap: fault reading aInfo+0x30"
                )),
            };
            if proc.heap.is_none() {
                if allocator == 0 {
                    let heap_min = proc.mem.vm.read_u32(info_ptr + 0x34).unwrap_or(0);
                    let heap_max = proc.mem.vm.read_u32(info_ptr + 0x38).unwrap_or(0);
                    let already = crate::layout::pru(heap_min);
                    proc.heap = Some(crate::heap::Heap::new(
                        proc.heap_chunk_id,
                        crate::layout::HEAP_BASE,
                        heap_max,
                        already,
                    ));
                } else {
                    return NativeOutcome::Refused(format!(
                        "NEXT-NEEDED euser SetupThreadHeap with foreign allocator {:#x}", allocator
                    ));
                }
            }
            proc.cpu.regs[0] = 0;
            NativeOutcome::Return
        }
        "_ZN4User5AllocEi" => {
            let len = proc.cpu.regs[0] as i32 as u32;
            match proc.heap.as_mut() {
                Some(h) => {
                    proc.cpu.regs[0] = h.alloc(len, &mut proc.mem).unwrap_or(0);
                    NativeOutcome::Return
                }
                None => NativeOutcome::Refused(
                    "NEXT-NEEDED euser/User::Alloc before heap setup".into()
                ),
            }
        }
        "_ZN4User6AllocZEi" => {
            let len = proc.cpu.regs[0] as i32 as u32;
            match proc.heap.as_mut() {
                Some(h) => {
                    let addr = match h.alloc(len, &mut proc.mem) {
                        Some(a) => a,
                        None => {
                            proc.cpu.regs[0] = 0;
                            return NativeOutcome::Return;
                        }
                    };
                    let total = h.len_of(addr).unwrap_or(0);
                    for i in 0..total {
                        let _ = proc.mem.vm.write_u8(addr + i, 0);
                    }
                    proc.cpu.regs[0] = addr;
                    NativeOutcome::Return
                }
                None => NativeOutcome::Refused(
                    "NEXT-NEEDED euser/User::AllocZ before heap setup".into()
                ),
            }
        }
        "_ZN4User4FreeEPv" => {
            let addr = proc.cpu.regs[0];
            if addr == 0 {
                proc.cpu.regs[0] = 0;
                return NativeOutcome::Return;
            }
            match proc.heap.as_mut() {
                Some(h) => match h.free(addr) {
                    Ok(()) => {
                        proc.cpu.regs[0] = 0;
                        NativeOutcome::Return
                    }
                    Err(_) => NativeOutcome::Refused(format!(
                        "NEXT-NEEDED euser/User::Free unknown ptr {:#010x}", addr
                    )),
                },
                None => NativeOutcome::Refused(
                    "NEXT-NEEDED euser/User::Free before heap setup".into()
                ),
            }
        }
        "_ZN4User7ReAllocEPvii" => {
            let addr = proc.cpu.regs[0];
            let new_len = proc.cpu.regs[1] as i32 as u32;
            let _mode = proc.cpu.regs[2] as i32;
            match proc.heap.as_mut() {
                Some(h) => {
                    if addr == 0 {
                        proc.cpu.regs[0] = h.alloc(new_len, &mut proc.mem).unwrap_or(0);
                    } else {
                        proc.cpu.regs[0] = h.realloc(addr, new_len, &mut proc.mem).unwrap_or(0);
                    }
                    NativeOutcome::Return
                }
                None => NativeOutcome::Refused(
                    "NEXT-NEEDED euser/User::ReAlloc before heap setup".into()
                ),
            }
        }
        "_ZN4User8AllocLenEPKv" => {
            let addr = proc.cpu.regs[0];
            match proc.heap.as_ref() {
                Some(h) => {
                    proc.cpu.regs[0] = h.len_of(addr).unwrap_or(0);
                    NativeOutcome::Return
                }
                None => NativeOutcome::Refused(
                    "NEXT-NEEDED euser/User::AllocLen before heap setup".into()
                ),
            }
        }
        "_ZN4User4ExitEi" => NativeOutcome::Exit(r0 as i32),
        "_ZN4User5PanicERK7TDesC16i" => {
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
        "" => NativeOutcome::Refused(format!(
            "NEXT-NEEDED {}/{} <unnamed-ordinal>", dll_base, ordinal
        )),
        _ => NativeOutcome::Refused(format!(
            "NEXT-NEEDED {}/{} {}", dll_base, ordinal, name
        )),
    }
}