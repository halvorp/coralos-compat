use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use crate::loader::Process;

/// The host-side state of the natives (no globals: it lives in the `Process`).
#[derive(Default)]
pub struct NativeState {
    /// `UserSvr::DllSetTls` / `DllTls`: (handle, DLL UID) -> pointer.
    pub tls: BTreeMap<(u32, u32), u32>,
    /// The `TCppRTExceptionsGlobals` objects the guest constructed, in order.
    pub exception_globals: Vec<u32>,
    /// The guest-resident `CApaProcess` placeholder (zeroed, vtable pointer 0), allocated on first use by the application start.
    pub apa_process: Option<u32>,
    /// `CleanupStack`: the pointers pushed and not yet popped (e32base.h:2944-2948 `PushL` x3, `Pop`, `Pop(TInt)`).
    pub cleanup: Vec<crate::cleanup::CleanupItem>,
    /// Cleanup-stack depths at each open TRAP bracket, and the guest block standing for the thread's `TTrapHandler` (`trap.rs`).
    pub trap_marks: Vec<u32>,
    pub trap_handler: Option<u32>,
    /// The guest sentinel of the active scheduler's queue (`sched.rs`).
    pub sched_head: Option<u32>,
    /// Outstanding requests and the thread's request semaphore (`request.rs`).
    pub requests: crate::request::RequestState,
    /// The window server's event queue for this process (`wsevent.rs`).
    pub ws: crate::wsevent::WsState,
    /// ROM resource files by id offset (`resource.rs`), built on first need.
    pub rom_resources: Option<alloc::collections::BTreeMap<u32, String>>,
    /// The Avkon shell objects (`shell.rs`).
    pub shell: crate::shell::ShellState,
    /// Host-only state of guest objects, by address and allocation generation (`objects.rs`).
    pub objs: crate::objects::Objects,
    /// The CDL engine's data and customisation stacks (`cdl.rs`).
    pub cdl: crate::cdl::CdlState,
    /// The opaque `CFont` guest objects by logical font id (`listdata.rs`).
    pub fonts: BTreeMap<u32, u32>,
    /// The icon library's guest constants (`icons.rs`).
    pub icons: crate::icons::IconState,
    /// Font & Bitmap Server (`fbscli`) bitmaps (`fbs.rs`).
    pub fbs: crate::fbs::FbsState,
    /// Image Conversion Library (`imageconversion.dll` `CImageDecoder`, `imageconv.rs`).
    pub imageconv: crate::imageconv::ImageConvState,
    /// Virtual monotonic clock in microseconds, driven by `GuestMachine::advance_clock_to`.
    pub clock_us: u64,
    /// Active `CPeriodic` timers keyed by guest object address (`sched.rs`).
    pub periodic_timers: BTreeMap<u32, crate::sched::PeriodicTimer>,
    /// Active screen dimensions `(width, height)` in pixels (`None` = S60 5th Ed default `360x640`;
    /// set to `352x416` when a legacy non-touch S60 3rd Ed app calls `CAknAppUi::BaseConstructL(0)`).
    pub screen_size: Option<(i32, i32)>,
    /// A modal `GuestUi::request` currently blocking `CEikDialog::RunLD` (`dialog.rs`): `(request_ref, dialog_addr)`.
    pub pending_request: Option<(heirloom_ui_abi::RequestRef, u32)>,
    /// PRNG state for C stdlib `rand()` / `srand()` (`clib.rs`).
    pub clib_rng: u32,
    /// Saved cursor for C stdlib `strtok()` (`clib.rs`).
    pub clib_strtok_pos: u32,
    /// Active ARM EHABI `User::Leave` unwinding state (`ehabi.rs`).
    pub active_leave: Option<crate::ehabi::LeaveState>,
    /// Reusable guest `XLeaveException` object address (`ehabi.rs`).
    pub leave_exc_obj: Option<u32>,
    /// Open `RResourceFile` buffers keyed by guest object address (`cone.rs`).
    pub rsc_files: BTreeMap<u32, Vec<u8>>,
    /// Active thread `RAllocator*` for `User::SwitchAllocator` / `UserHeap::ChunkHeap`.
    pub current_allocator: u32,
}

/// A host extension to the natives: asked first with the import's mangled name; `Some` = handled. Used by tests to stand in
/// for an OS call that calls guest code; the engine itself leaves it `None`.
pub type NativeHook = Box<dyn FnMut(&mut Process, &str) -> Option<NativeOutcome>>;

#[derive(Debug)]
pub enum NativeOutcome {
    Return,
    Blocked,
    Exit(i32),
    Refused(String),
}

/// The natives of each module, asked in turn: the first that knows the mangled `name` answers.
fn module_native(proc: &mut Process, name: &str) -> Option<NativeOutcome> {
    if let Some(o) = crate::appstart::native(proc, name) { return Some(o); }
    if let Some(o) = crate::cone::native(proc, name) { return Some(o); }
    if let Some(o) = crate::appui::native(proc, name) { return Some(o); }
    if let Some(o) = crate::view::native(proc, name) { return Some(o); }
    if let Some(o) = crate::remcon::native(proc, name) { return Some(o); }
    if let Some(o) = crate::viewappui::native(proc, name) { return Some(o); }
    if let Some(o) = crate::dialog::native(proc, name) { return Some(o); }
    if let Some(o) = crate::trap::native(proc, name) { return Some(o); }
    if let Some(o) = crate::ehabi::native(proc, name) { return Some(o); }
    if let Some(o) = crate::screen::native(proc, name) { return Some(o); }
    if let Some(o) = crate::desc::native(proc, name) { return Some(o); }
    if let Some(o) = crate::avkon::native(proc, name) { return Some(o); }
    if let Some(o) = crate::fs::native(proc, name) { return Some(o); }
    if let Some(o) = crate::cleanup::native(proc, name) { return Some(o); }
    if let Some(o) = crate::dictstore::native(proc, name) { return Some(o); }
    if let Some(o) = crate::sched::native(proc, name) { return Some(o); }
    if let Some(o) = crate::ctrlstack::native(proc, name) { return Some(o); }
    if let Some(o) = crate::schedrun::native(proc, name) { return Some(o); }
    if let Some(o) = crate::wsevent::native(proc, name) { return Some(o); }
    if let Some(o) = crate::keyroute::native(proc, name) { return Some(o); }
    if let Some(o) = crate::mixins::native(proc, name) { return Some(o); }
    if let Some(o) = crate::tz::native(proc, name) { return Some(o); }
    if let Some(o) = crate::timer::native(proc, name) { return Some(o); }
    if let Some(o) = crate::control::native(proc, name) { return Some(o); }
    if let Some(o) = crate::etel::native(proc, name) { return Some(o); }
    if let Some(o) = crate::geom::native(proc, name) { return Some(o); }
    if let Some(o) = crate::shell::native(proc, name) { return Some(o); }
    if let Some(o) = crate::window::native(proc, name) { return Some(o); }
    if let Some(o) = crate::toolbar::native(proc, name) { return Some(o); }
    if let Some(o) = crate::time::native(proc, name) { return Some(o); }
    if let Some(o) = crate::imageconv::native(proc, name) { return Some(o); }
    if let Some(o) = crate::array::native(proc, name) { return Some(o); }
    if let Some(o) = crate::fileman::native(proc, name) { return Some(o); }
    if let Some(o) = crate::cenrep::native(proc, name) { return Some(o); }
    if let Some(o) = crate::cdl::native(proc, name) { return Some(o); }
    if let Some(o) = crate::navi::native(proc, name) { return Some(o); }
    if let Some(o) = crate::avkonlayout::native(proc, name) { return Some(o); }
    if let Some(o) = crate::fpeabi::native(proc, name) { return Some(o); }
    if let Some(o) = crate::hal::native(proc, name) { return Some(o); }
    if let Some(o) = crate::listbox::native(proc, name) { return Some(o); }
    if let Some(o) = crate::listkey::native(proc, name) { return Some(o); }
    if let Some(o) = crate::tchar::native(proc, name) { return Some(o); }
    if let Some(o) = crate::listmodel::native(proc, name) { return Some(o); }
    if let Some(o) = crate::listdata::native(proc, name) { return Some(o); }
    if let Some(o) = crate::colorlist::native(proc, name) { return Some(o); }
    if let Some(o) = crate::icons::native(proc, name) { return Some(o); }
    if let Some(o) = crate::fbs::native(proc, name) { return Some(o); }
    if let Some(o) = crate::audio::native(proc, name) { return Some(o); }
    if let Some(o) = crate::aeabimem::native(proc, name) { return Some(o); }
    if let Some(o) = crate::clib::native(proc, name) { return Some(o); }
    if let Some(o) = crate::format::native(proc, name) { return Some(o); }
    None
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
    if let Some(t) = proc.trace.as_mut() {
        let r = &proc.cpu.regs;
        t.push(format!("{} {}/{} {name} r0-r3={:#x} {:#x} {:#x} {:#x} lr={:#x}", t.len(), dll_base, ordinal, r[0], r[1], r[2], r[3], r[14]));
    }

    // RAllocator / RHeap members need a guest-resident allocator object with a vtable, which this engine does not build
    // (User::Allocator is refused by name for the same reason), so they are refused before anything dereferences one.
    // DELETE WHEN the first corpus image's next refusal is an RAllocator member or User::Allocator (then build the object).
    if name.starts_with("_ZN9RAllocator") || name.starts_with("_ZN5RHeap") {
        return NativeOutcome::Refused(format!(
            "NEXT-NEEDED euser/{} {} (no guest RAllocator object yet)", ordinal, name
        ));
    }

    // A guest call past the end of an engine-built vtable (enginestub.rs guard slot): named, never a jump into the next table.
    if name.starts_with(crate::enginestub::VTABLE_OVERRUN) {
        return NativeOutcome::Refused(format!("NEXT-NEEDED {name}"));
    }

    if let Some(mut hook) = proc.native_hook.take() {
        let handled = hook(proc, name);
        proc.native_hook = Some(hook);
        if let Some(outcome) = handled {
            return outcome;
        }
    }
    if let Some(outcome) = module_native(proc, name) {
        return outcome;
    }

    let r0 = proc.cpu.regs[0];
    let r1 = proc.cpu.regs[1];
    match name {
        // TBufBase16(TInt aMaxLength) / TBufBase8(TInt): `TDes(EBuf, 0, aMaxLength)` (e32des16.h:332 TDes16(aType, aLength, aMaxLength)). A
        // descriptor has no vptr: word 0 is iLength | type << KShiftDesType16 (e32des16.h:16 = 28), word 1 is iMaxLength (:341).
        // DES_TYPE_EBUF: the TType enum is internal to the SDK; EKA2L1 utils/des.h:50-55 (facts) lists buf_const 0, ptr_const 1, ptr 2, buf 3.
        "_ZN10TBufBase16C1Ei" | "_ZN10TBufBase16C2Ei" | "_ZN9TBufBase8C1Ei" | "_ZN9TBufBase8C2Ei" => {
            const DES_TYPE_EBUF: u32 = 3;
            const KSHIFT_DES_TYPE: u32 = 28; // e32des16.h:16
            let ok = proc.mem.vm.write_u32(r0, DES_TYPE_EBUF << KSHIFT_DES_TYPE).is_ok() && proc.mem.vm.write_u32(r0.wrapping_add(4), r1).is_ok();
            if ok { NativeOutcome::Return } else { NativeOutcome::Refused(format!("NEXT-NEEDED euser/{name}: descriptor at {r0:#x} is unwritable")) }
        }
        // TBufBase16(TInt aLength, TInt aMaxLength) / TBufBase8(TInt, TInt): `TDes(EBuf, aLength, aMaxLength)` (e32des16.h:332): the same two words with the length set.
        "_ZN10TBufBase16C1Eii" | "_ZN10TBufBase16C2Eii" | "_ZN9TBufBase8C1Eii" | "_ZN9TBufBase8C2Eii" => {
            const DES_TYPE_EBUF: u32 = 3;
            const KSHIFT_DES_TYPE: u32 = 28; // e32des16.h:16
            let r2 = proc.cpu.regs[2];
            let ok = proc.mem.vm.write_u32(r0, r1 | DES_TYPE_EBUF << KSHIFT_DES_TYPE).is_ok() && proc.mem.vm.write_u32(r0.wrapping_add(4), r2).is_ok();
            if ok { NativeOutcome::Return } else { NativeOutcome::Refused(format!("NEXT-NEEDED euser/{name}: descriptor at {r0:#x} is unwritable")) }
        }
        // TVersion(TInt aMajor, TInt aMinor, TInt aBuild): e32cmn.h layout is { TInt8 iMajor; TInt8 iMinor; TInt16 iBuild; }.
        "_ZN8TVersionC1Eiii" | "_ZN8TVersionC2Eiii" => {
            let major = proc.cpu.regs[1] as u8;
            let minor = proc.cpu.regs[2] as u8;
            let build = proc.cpu.regs[3] as u16;
            let word = (major as u32) | ((minor as u32) << 8) | ((build as u32) << 16);
            if proc.mem.vm.write_u32(r0, word).is_ok() {
                NativeOutcome::Return
            } else {
                NativeOutcome::Refused(format!("NEXT-NEEDED euser/{name}: TVersion at {r0:#x} is unwritable"))
            }
        }
        // TSecureId RProcess::SecureId() const (e32std.h): returns the process's UID3 as a 32-bit TSecureId.
        "_ZNK8RProcess8SecureIdEv" => {
            proc.cpu.regs[0] = proc.uid3;
            NativeOutcome::Return
        }
        // TFileName RProcess::FileName() const / CApaApplication::DllName() const: ARM EABI returns TBuf<256> via sret in r0.
        "_ZNK8RProcess8FileNameEv" | "_ZNK15CApaApplication7DllNameEv" => {
            const KMAX_FILE_NAME: u32 = 256;
            let app_stem = proc.host.drive.as_ref()
                .and_then(|d| crate::resource::registration(d, proc.uid3).ok())
                .map(|r| r.app_file)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| String::from("app"));
            let full = if app_stem.to_ascii_lowercase().ends_with(".exe") {
                format!("c:\\sys\\bin\\{app_stem}")
            } else {
                format!("c:\\sys\\bin\\{app_stem}.exe")
            };
            let units: Vec<u16> = full.encode_utf16().collect();
            let header_ok = proc.mem.vm.write_u32(r0, crate::desc::DES_BUF << crate::desc::KSHIFT_DES_TYPE).is_ok()
                && proc.mem.vm.write_u32(r0.wrapping_add(4), KMAX_FILE_NAME).is_ok();
            if !header_ok {
                return NativeOutcome::Refused(format!("NEXT-NEEDED euser/{name}: sret TFileName at {r0:#x} is unwritable"));
            }
            match crate::desc::write16(proc, r0, &units) {
                Ok(()) => NativeOutcome::Return,
                Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED euser/{name}: {m}")),
            }
        }
        // RApaLsSession(): RSessionBase -> RHandleBase { iHandle } = 0 (e32std.h:3153, :4100), then apgcli.h:224 iExtension = 0: two zero words.
        "_ZN13RApaLsSessionC1Ev" => {
            let ok = proc.mem.vm.write_u32(r0, 0).is_ok() && proc.mem.vm.write_u32(r0.wrapping_add(4), 0).is_ok();
            if ok { NativeOutcome::Return } else { NativeOutcome::Refused(format!("NEXT-NEEDED apgrfx/{name}: session object at {r0:#x} is unwritable")) }
        }
        // drtaeabi's exception-globals object: recorded, `this` returned. Unwinding itself is slice 1b (see the leaves below).
        "_ZN23TCppRTExceptionsGlobalsC1Ev" => {
            proc.nat.exception_globals.push(r0);
            NativeOutcome::Return
        }
        // Per-process initialisation of the EKA2 user side: nothing in this engine needs initialising, so it returns.
        "_ZN4User11InitProcessEv" | "_ZN4User16CompressAllHeapsEv" => {
            proc.cpu.regs[0] = 0;
            NativeOutcome::Return
        }
        // TLanguage User::Language() (e32const.h): ELangEnglish = 1.
        "_ZN4User8LanguageEv" => {
            proc.cpu.regs[0] = 1;
            NativeOutcome::Return
        }
        // TInt Math::Rand(TInt64& aSeed) (e32math.h): Symbian 64-bit LCG (seed = seed * 0x5DEECE66D + 0xB, return (seed >> 33) & 0x7FFF_FFFF).
        "_ZN4Math4RandERx" => {
            let (Ok(lo), Ok(hi)) = (proc.rd(r0), proc.rd(r0.wrapping_add(4))) else {
                return NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: seed at {r0:#x} unreadable"));
            };
            let seed = (u64::from(lo) | (u64::from(hi) << 32))
                .wrapping_mul(0x5DEE_CE66_D)
                .wrapping_add(0xB);
            let _ = proc.wr(r0, seed as u32);
            let _ = proc.wr(r0.wrapping_add(4), (seed >> 32) as u32);
            proc.cpu.regs[0] = ((seed >> 33) & 0x7FFF_FFFF) as u32;
            NativeOutcome::Return
        }
        // RHeap* UserHeap::ChunkHeap(const TDesC* aName, TInt aMinLength, TInt aMaxLength, TInt aGrowBy, TInt aAlign, TBool aSingleThread) (e32std.h):
        // returns an opaque non-null RAllocator* backed by the process's unified guest heap.
        "_ZN8UserHeap9ChunkHeapEPK7TDesC16iiiii" => {
            let Some(alloc_obj) = proc.alloc_zeroed(16) else {
                return NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: out of heap"));
            };
            proc.cpu.regs[0] = alloc_obj;
            NativeOutcome::Return
        }
        // RAllocator* User::SwitchAllocator(RAllocator* aA) (e32std.h): switches the thread's allocator handle and returns the previous one.
        "_ZN4User15SwitchAllocatorEP10RAllocator" => {
            if proc.nat.current_allocator == 0 {
                if let Some(base) = proc.alloc_zeroed(16) {
                    proc.nat.current_allocator = base;
                }
            }
            let prev = proc.nat.current_allocator;
            if r0 != 0 {
                proc.nat.current_allocator = r0;
            }
            proc.cpu.regs[0] = prev;
            NativeOutcome::Return
        }
        // void* __dynamic_cast(const void* sub, const __class_type_info* src, const __class_type_info* dst, ptrdiff_t src2dst_offset):
        // ARM EABI RTTI downcast; return sub pointer when non-null.
        "__dynamic_cast" => {
            proc.cpu.regs[0] = r0;
            NativeOutcome::Return
        }
        // TInt memcompare(const TUint8* aLeft, TInt aLeftLen, const TUint8* aRight, TInt aRightLen) (euser/1952):
        "memcompare" => {
            let (lptr, llen, rptr, rlen) = (r0, r1 as i32, proc.cpu.regs[2], proc.cpu.regs[3] as i32);
            let n = llen.max(0).min(rlen.max(0)) as u32;
            let mut cmp = 0i32;
            for i in 0..n {
                let (Ok(a), Ok(b)) = (proc.mem.vm.read_u8(lptr.wrapping_add(i)), proc.mem.vm.read_u8(rptr.wrapping_add(i))) else {
                    return NativeOutcome::Refused(format!("NEXT-NEEDED euser memcompare: unreadable at +{i}"));
                };
                if a != b {
                    cmp = i32::from(a) - i32::from(b);
                    break;
                }
            }
            if cmp == 0 {
                cmp = llen - rlen;
            }
            proc.cpu.regs[0] = cmp as u32;
            NativeOutcome::Return
        }
        // unsigned int sleep(unsigned int seconds) (ESTLIB/238): advance virtual clock and return 0.
        "sleep" => {
            proc.nat.clock_us = proc.nat.clock_us.saturating_add(u64::from(r0) * 1_000_000);
            proc.cpu.regs[0] = 0;
            NativeOutcome::Return
        }
        // int uncompress(Bytef *dest, uLongf *destLen, const Bytef *source, uLong sourceLen) (EZLib/69, RFC 1950 zlib):
        // Z_OK=0, Z_DATA_ERROR=-3, Z_BUF_ERROR=-5.
        "uncompress" => {
            let (dest, dest_len_ptr, src_ptr, src_len) = (r0, r1, proc.cpu.regs[2], proc.cpu.regs[3]);
            let Ok(max_out) = proc.rd(dest_len_ptr) else {
                return NativeOutcome::Refused(format!("NEXT-NEEDED ezlib uncompress: destLen at {dest_len_ptr:#x} unreadable"));
            };
            if src_len > 16 * 1024 * 1024 {
                proc.cpu.regs[0] = (-3i32) as u32;
                return NativeOutcome::Return;
            }
            let mut src = alloc::vec![0u8; src_len as usize];
            if !src.is_empty() && proc.mem.vm.read_bytes(src_ptr, &mut src).is_err() {
                return NativeOutcome::Refused(format!("NEXT-NEEDED ezlib uncompress: src {src_ptr:#x}..+{src_len} unreadable"));
            }
            match miniz_oxide::inflate::decompress_to_vec_zlib(&src) {
                Ok(out) => {
                    if out.len() as u32 > max_out {
                        proc.cpu.regs[0] = (-5i32) as u32; // Z_BUF_ERROR
                    } else if !out.is_empty() && proc.mem.vm.write_bytes(dest, &out).is_err() {
                        return NativeOutcome::Refused(format!("NEXT-NEEDED ezlib uncompress: dest {dest:#x} unwritable"));
                    } else {
                        let _ = proc.wr(dest_len_ptr, out.len() as u32);
                        proc.cpu.regs[0] = 0; // Z_OK
                    }
                }
                Err(_) => {
                    proc.cpu.regs[0] = (-3i32) as u32; // Z_DATA_ERROR
                }
            }
            NativeOutcome::Return
        }
        // TUint32 Math::Random() (e32math.h): deterministic PRNG.
        "_ZN4Math6RandomEv" => {
            let mut s = proc.nat.clib_rng;
            if s == 0 { s = 0x2468_ACE1; }
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            proc.nat.clib_rng = s;
            proc.cpu.regs[0] = s;
            NativeOutcome::Return
        }
        // TInt Math::Sqrt(TReal& aTrg, const TReal& aSrc) (e32math.h): IEEE-754 f64 square root.
        "_ZN4Math4SqrtERdRKd" => {
            let (Ok(lo), Ok(hi)) = (proc.rd(r1), proc.rd(r1.wrapping_add(4))) else {
                return NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: src unreadable"));
            };
            let x = f64::from_bits(u64::from(lo) | (u64::from(hi) << 32));
            if x.is_nan() || x < 0.0 {
                proc.cpu.regs[0] = (-6i32) as u32; // KErrArgument
            } else {
                let mut g = if x == 0.0 { 0.0 } else { f64::from_bits((x.to_bits() >> 1) + (0x3ff0_0000_0000_0000u64 >> 1)) };
                if x > 0.0 && x.is_finite() {
                    for _ in 0..8 {
                        g = 0.5 * (g + x / g);
                    }
                } else {
                    g = x;
                }
                let bits = g.to_bits();
                let _ = proc.wr(r0, bits as u32);
                let _ = proc.wr(r0.wrapping_add(4), (bits >> 32) as u32);
                proc.cpu.regs[0] = 0;
            }
            NativeOutcome::Return
        }
        // TInt Math::Log(TReal& aTrg, const TReal& aSrc) (base-10 log) & Math::Exp(TReal& aTrg, const TReal& aSrc)
        "_ZN4Math3LogERdRKd" | "_ZN4Math3ExpERdRKd" => {
            let (Ok(lo), Ok(hi)) = (proc.rd(r1), proc.rd(r1.wrapping_add(4))) else {
                return NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: src unreadable"));
            };
            let x = f64::from_bits(u64::from(lo) | (u64::from(hi) << 32));
            let res: Result<f64, i32> = if name.contains("3Log") {
                if !x.is_finite() || x <= 0.0 {
                    Err(-6) // KErrArgument
                } else {
                    // Range reduction: x = m * 2^e with m in [1, 2), ln(m) via atanh series, log10(x) = ln(x)/ln(10)
                    let bits = x.to_bits();
                    let e = ((bits >> 52) & 0x7FF) as i32 - 1023;
                    let m = f64::from_bits((bits & 0x000F_FFFF_FFFF_FFFF) | 0x3FF0_0000_0000_0000);
                    let t = (m - 1.0) / (m + 1.0);
                    let t2 = t * t;
                    let mut term = t;
                    let mut sum = 0.0;
                    for k in 0..10 {
                        sum += term / (2 * k + 1) as f64;
                        term *= t2;
                    }
                    let ln_x = 2.0 * sum + (e as f64) * core::f64::consts::LN_2;
                    Ok(ln_x / core::f64::consts::LN_10)
                }
            } else {
                if x.is_nan() {
                    Err(-6)
                } else if x > 709.78 {
                    Err(-9) // KErrOverflow
                } else if x < -745.0 {
                    Ok(0.0)
                } else {
                    // Range reduction: exp(x) = 2^k * exp(r), |r| <= ln(2)/2
                    let k = (x / core::f64::consts::LN_2 + if x >= 0.0 { 0.5 } else { -0.5 }) as i32;
                    let r = x - (k as f64) * core::f64::consts::LN_2;
                    let mut term = 1.0;
                    let mut sum = 1.0;
                    for n in 1..16 {
                        term *= r / (n as f64);
                        sum += term;
                    }
                    let scale_exp = (k + 1023).clamp(1, 2046) as u64;
                    Ok(sum * f64::from_bits(scale_exp << 52))
                }
            };
            match res {
                Ok(v) => {
                    let bits = v.to_bits();
                    let _ = proc.wr(r0, bits as u32);
                    let _ = proc.wr(r0.wrapping_add(4), (bits >> 32) as u32);
                    proc.cpu.regs[0] = 0;
                }
                Err(e) => proc.cpu.regs[0] = e as u32,
            }
            NativeOutcome::Return
        }
        "_ZN11TDblQueLink5DequeEv" => {
            if let (Ok(next), Ok(prev)) = (proc.rd(r0), proc.rd(r0.wrapping_add(4))) {
                if next != 0 && prev != 0 {
                    let _ = proc.wr(prev, next);
                    let _ = proc.wr(next.wrapping_add(4), prev);
                    let _ = proc.wr(r0, 0);
                    let _ = proc.wr(r0.wrapping_add(4), 0);
                }
            }
            NativeOutcome::Return
        }
        // Layout: TDblQueBase { iHead: { iNext: +0, iPrev: +4 }, iOffset: +8 }
        //         TDblQueIterBase { iOffset: +0, iHead: +4, iNext: +8 }
        "_ZN11TDblQueBaseC2Ei" | "_ZN11TDblQueBaseC1Ei" => {
            let _ = proc.wr(r0, r0);
            let _ = proc.wr(r0.wrapping_add(4), r0);
            let _ = proc.wr(r0.wrapping_add(8), r1);
            proc.cpu.regs[0] = r0;
            NativeOutcome::Return
        }
        "_ZN11TDblQueBase9DoAddLastEPv" | "_ZN11TDblQueBase10DoAddFirstEPv" | "_ZN11TDblQueBase13DoAddPriorityEPv" => {
            let off = proc.rd(r0.wrapping_add(8)).unwrap_or(0);
            let link = r1.wrapping_add(off);
            let head = r0;
            if name.contains("DoAddFirst") {
                let first = proc.rd(head).unwrap_or(head);
                let _ = proc.wr(link, first);
                let _ = proc.wr(link.wrapping_add(4), head);
                let _ = proc.wr(first.wrapping_add(4), link);
                let _ = proc.wr(head, link);
            } else if name.contains("DoAddPriority") {
                // TPriQueLink: iNext (+0), iPrev (+4), iPriority (+8) — insert descending by priority.
                let prio = proc.rd(link.wrapping_add(8)).unwrap_or(0) as i32;
                let mut cur = proc.rd(head).unwrap_or(head);
                let mut steps = 0;
                while cur != head && cur != 0 && steps < 4096 {
                    let cp = proc.rd(cur.wrapping_add(8)).unwrap_or(0) as i32;
                    if prio > cp { break; }
                    cur = proc.rd(cur).unwrap_or(head);
                    steps += 1;
                }
                let prev = proc.rd(cur.wrapping_add(4)).unwrap_or(head);
                let _ = proc.wr(link, cur);
                let _ = proc.wr(link.wrapping_add(4), prev);
                let _ = proc.wr(prev, link);
                let _ = proc.wr(cur.wrapping_add(4), link);
            } else {
                let last = proc.rd(head.wrapping_add(4)).unwrap_or(head);
                let _ = proc.wr(link, head);
                let _ = proc.wr(link.wrapping_add(4), last);
                let _ = proc.wr(last, link);
                let _ = proc.wr(head.wrapping_add(4), link);
            }
            NativeOutcome::Return
        }
        "_ZN15TDblQueIterBaseC2ER11TDblQueBase" | "_ZN15TDblQueIterBaseC1ER11TDblQueBase" => {
            let off = proc.rd(r1.wrapping_add(8)).unwrap_or(0);
            let first = proc.rd(r1).unwrap_or(r1);
            let _ = proc.wr(r0, off);
            let _ = proc.wr(r0.wrapping_add(4), r1);
            let _ = proc.wr(r0.wrapping_add(8), first);
            proc.cpu.regs[0] = r0;
            NativeOutcome::Return
        }
        "_ZN15TDblQueIterBase10SetToFirstEv" => {
            let head = proc.rd(r0.wrapping_add(4)).unwrap_or(0);
            let first = proc.rd(head).unwrap_or(head);
            let _ = proc.wr(r0.wrapping_add(8), first);
            NativeOutcome::Return
        }
        "_ZN15TDblQueIterBase9DoPostIncEv" => {
            let off = proc.rd(r0).unwrap_or(0);
            let head = proc.rd(r0.wrapping_add(4)).unwrap_or(0);
            let cur = proc.rd(r0.wrapping_add(8)).unwrap_or(head);
            if cur == head || cur == 0 {
                proc.cpu.regs[0] = 0;
            } else {
                let next = proc.rd(cur).unwrap_or(head);
                let _ = proc.wr(r0.wrapping_add(8), next);
                proc.cpu.regs[0] = cur.wrapping_sub(off);
            }
            NativeOutcome::Return
        }
        // TBool User::QueryVersionSupported(const TVersion& aCurrent, const TVersion& aRequested)
        "_ZN4User21QueryVersionSupportedERK8TVersionS2_" => {
            proc.cpu.regs[0] = 1;
            NativeOutcome::Return
        }
        // void User::After(TTimeIntervalMicroSeconds32 aInterval) / User::AfterHighRes: advance virtual clock.
        "_ZN4User5AfterE27TTimeIntervalMicroSeconds32"
        | "_ZN4User12AfterHighResE27TTimeIntervalMicroSeconds32" => {
            proc.nat.clock_us = proc.nat.clock_us.saturating_add(r0 as u64);
            NativeOutcome::Return
        }
        // TBool TUid::operator==(const TUid& aUid) const / operator!=(const TUid&) const
        "_ZNK4TUideqERKS_" | "_ZNK4TUidneERKS_" => {
            let a = proc.rd(r0).unwrap_or(0);
            let b = proc.rd(r1).unwrap_or(0);
            let eq = a == b;
            proc.cpu.regs[0] = u32::from(if name.contains("neERKS_") { !eq } else { eq });
            NativeOutcome::Return
        }
        // RProperty (e32property.h): Publish & Subscribe stub — Attach succeeds, Subscribe stays pending, Get returns KErrNotFound.
        "_ZN9RProperty6AttachE4TUidj10TOwnerType" => {
            let _ = proc.wr(r0, 1);
            proc.cpu.regs[0] = 0;
            NativeOutcome::Return
        }
        "_ZN9RProperty9SubscribeER14TRequestStatus" => {
            let _ = proc.wr(r1, crate::request::KREQUEST_PENDING);
            NativeOutcome::Return
        }
        "_ZN9RProperty6CancelEv" | "_ZN9RProperty5CloseEv" => {
            if name.ends_with("CloseEv") { let _ = proc.wr(r0, 0); }
            NativeOutcome::Return
        }
        "_ZN9RProperty3GetERi" | "_ZN9RProperty3GetER5TDes8" | "_ZN9RProperty3GetER6TDes16"
        | "_ZN9RProperty3GetE4TUidjRi" | "_ZN9RProperty3GetE4TUidjR5TDes8" | "_ZN9RProperty3GetE4TUidjR6TDes16" => {
            proc.cpu.regs[0] = (-1i32) as u32; // KErrNotFound
            NativeOutcome::Return
        }
        // REComSession (ecom.h): no dynamic ECom plugins registered on desktop host; list returns empty, CreateImplementationL leaves KErrNotFound.
        "_ZN12REComSession20ListImplementationsLE4TUidR13RPointerArrayI26CImplementationInformationE"
        | "_ZN12REComSession20ListImplementationsLE4TUidRK19TEComResolverParamsR13RPointerArrayI26CImplementationInformationE"
        | "_ZN12REComSession20ListImplementationsLE4TUidRK19TEComResolverParamsS0_R13RPointerArrayI26CImplementationInformationE"
        | "_ZN12REComSession10FinalCloseEv" => NativeOutcome::Return,
        "_ZN12REComSession21CreateImplementationLE4TUidlPv" => {
            match proc.plain_object("CEComImpl", 0x40) {
                Ok(obj) => {
                    let key_off = r1;
                    if key_off < 0x3C { let _ = proc.wr(obj.wrapping_add(key_off), r0); }
                    proc.cpu.regs[0] = obj;
                    NativeOutcome::Return
                }
                Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED ecom {name}: {m}")),
            }
        }
        "_ZN12REComSession23DestroyedImplementationE4TUid" => NativeOutcome::Return,
        // TFindHandleBase / TFindServer (e32std.h): no background servers matched.
        "_ZN15TFindHandleBaseC1ERK7TDesC16" | "_ZN15TFindHandleBaseC2ERK7TDesC16" => {
            let _ = proc.wr(r0, 0);
            proc.cpu.regs[0] = r0;
            NativeOutcome::Return
        }
        "_ZN11TFindServer4NextER4TBufILi256EE" => {
            proc.cpu.regs[0] = (-1i32) as u32; // KErrNotFound
            NativeOutcome::Return
        }
        // RSessionBase::CreateSession / DoSendReceive (e32std.h): non-bridged Symbian servers are absent.
        "_ZN12RSessionBase13CreateSessionERK7TDesC16RK8TVersioni" => {
            let _ = proc.wr(r0, 0);
            proc.cpu.regs[0] = (-1i32) as u32; // KErrNotFound
            NativeOutcome::Return
        }
        "_ZNK12RSessionBase13DoSendReceiveEiPK8TIpcArgs" => {
            proc.cpu.regs[0] = (-5i32) as u32; // KErrNotSupported
            NativeOutcome::Return
        }
        // RConnectionMonitor / RHTTPSession: offline desktop host (no cellular bearer events).
        "_ZN18RConnectionMonitor8ConnectLEv"
        | "_ZN18RConnectionMonitor12NotifyEventLER26MConnectionMonitorObserver"
        | "_ZN18RConnectionMonitor5CloseEv"
        | "_ZN12RHTTPSession5OpenLEv"
        | "_ZN12RHTTPSession5CloseEv" => NativeOutcome::Return,
        // CMsvSession::OpenAsyncL (msvapi.h): messaging store has no SMS/MMS events on desktop host.
        "_ZN11CMsvSession10OpenAsyncLER19MMsvSessionObserver" => {
            match proc.plain_object("CMsvSession", 0x20) {
                Ok(obj) => { proc.cpu.regs[0] = obj; NativeOutcome::Return }
                Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED msgs {name}: {m}")),
            }
        }
        // CAknLongTapDetector::NewL (aknlongtapdetector.h): touch long-tap helper.
        "_ZN19CAknLongTapDetector4NewLEP27MAknLongTapDetectorCallBack" => {
            match proc.plain_object("CAknLongTapDetector", 0x30) {
                Ok(obj) => { proc.cpu.regs[0] = obj; NativeOutcome::Return }
                Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED avkon {name}: {m}")),
            }
        }
        // Symbian Sensor Framework (sensrvclient / sensrvutil): no hardware accelerometer/proximity sensors on desktop host.
        "_ZN18TSensrvChannelInfoC1Ev" | "_ZN18TSensrvChannelInfoC2Ev" => {
            let _ = proc.mem.vm.write_bytes(r0, &[0u8; 0x40]);
            proc.cpu.regs[0] = r0;
            NativeOutcome::Return
        }
        // Symbian Sensor Framework channel enumerator (sensrvclient ordinal 3: NewL, vtable slot 3: FindChannelsL).
        s if s.starts_with("_ZN20CSensrvChannel") && s.ends_with("4NewLEv") => {
            let slots = crate::enginestub::slots_with_gaps(5, &[
                (0, "sensrv::enum_dtor1"),
                (1, "sensrv::enum_dtor0"),
                (2, "_ZN5CBase10Extension_EjRPvS0_"),
                (3, "sensrv::find_channels"),
            ]);
            let r = proc.alloc_zeroed(16)
                .ok_or_else(|| String::from("no heap for sensrv enumerator"))
                .and_then(|obj| {
                    let vptr = proc.build_vtable("SensrvEnum", &slots)?;
                    proc.wr(obj, vptr)?;
                    Ok(obj)
                });
            match r {
                Ok(obj) => { proc.cpu.regs[0] = obj; NativeOutcome::Return }
                Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED sensrvclient {name}: {m}")),
            }
        }
        "sensrv::find_channels" => {
            // Leave the output RArray empty (0 hardware sensors found).
            NativeOutcome::Return
        }
        "sensrv::enum_dtor1" | "sensrv::enum_dtor0" => {
            if name.ends_with("dtor0") { if let Some(h) = proc.heap.as_mut() { let _ = h.free(r0); } }
            NativeOutcome::Return
        }
        // CServer2 / CSession2 / RMessagePtr2 (e32base.h): local Symbian IPC server base classes.
        "_ZN8CServer2C2EiNS_11TServerTypeE" => {
            let _ = crate::sched::native(proc, "_ZN7CActiveC2Ei");
            proc.cpu.regs[0] = r0;
            NativeOutcome::Return
        }
        "_ZN8CServer26StartLERK7TDesC16" => NativeOutcome::Return,
        "_ZN8CServer2D2Ev" | "_ZN8CServer28DoCancelEv" | "_ZN8CServer24RunLEv" => {
            proc.cpu.regs[0] = r0;
            NativeOutcome::Return
        }
        "_ZN8CServer28RunErrorEi" | "_ZN8CServer210Extension_EjRPvS0_" | "_ZN9CSession210Extension_EjRPvS0_" => {
            proc.cpu.regs[0] = 0;
            NativeOutcome::Return
        }
        "_ZN9CSession2C2Ev" | "_ZN9CSession2D2Ev" => {
            proc.cpu.regs[0] = r0;
            NativeOutcome::Return
        }
        "_ZN9CSession27CreateLEv"
        | "_ZN9CSession210DisconnectERK9RMessage2"
        | "_ZN9CSession212ServiceErrorERK9RMessage2i"
        | "_ZNK12RMessagePtr28CompleteEi" => NativeOutcome::Return,
        "_ZN9CSession214CountResourcesEv" | "_ZNK12RMessagePtr26ClientER7RThread10TOwnerType" => {
            proc.cpu.regs[0] = 0;
            NativeOutcome::Return
        }
        // RApaLsSession / TApaTask / TApaTaskList (apgcli.h / apgtask.h).
        "_ZN13RApaLsSession7ConnectEv"
        | "_ZN13RApaLsSession5CloseEv"
        | "_ZN13RApaLsSession13StartDocumentERK7TDesC164TUidR9TThreadIdNS_11TLaunchTypeE" => {
            proc.cpu.regs[0] = 0;
            NativeOutcome::Return
        }
        "_ZN8TApaTaskC1ER10RWsSession" | "_ZN12TApaTaskListC1ER10RWsSession" => {
            let _ = proc.wr(r0, r1);
            proc.cpu.regs[0] = r0;
            NativeOutcome::Return
        }
        "_ZN11CMsvSession18CloseMessageServerEv" => NativeOutcome::Return,
        // void UserSvr::ScreenInfo(TDes8& aInfo) (e32event.h TScreenInfoV01: 24 bytes)
        "_ZN7UserSvr10ScreenInfoER5TDes8" => {
            let (sw, sh) = proc.screen_size();
            let words = [0u32, 0, 0, 0, sw as u32, sh as u32];
            let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
            match crate::desc::write8(proc, r0, &bytes) {
                Ok(()) => NativeOutcome::Return,
                Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {m}")),
            }
        }
        // UserSvr::DllSetTls(aHandle, aDllUid, aPtr) / DllSetTls(aHandle, aPtr) -> KErrNone;
        // UserSvr::DllTls(aHandle, aDllUid) / DllTls(aHandle) -> the pointer (0 = none); DllFreeTls(aHandle).
        "_ZN7UserSvr9DllSetTlsEiiPv" => {
            let ptr = proc.cpu.regs[2];
            proc.nat.tls.insert((r0, r1), ptr);
            proc.cpu.regs[0] = 0;
            NativeOutcome::Return
        }
        "_ZN7UserSvr9DllSetTlsEiPv" => {
            proc.nat.tls.insert((r0, 0), r1);
            proc.cpu.regs[0] = 0;
            NativeOutcome::Return
        }
        "_ZN7UserSvr6DllTlsEii" => {
            proc.cpu.regs[0] = proc.nat.tls.get(&(r0, r1)).copied().unwrap_or(0);
            NativeOutcome::Return
        }
        "_ZN7UserSvr6DllTlsEi" => {
            proc.cpu.regs[0] = proc.nat.tls.get(&(r0, 0)).copied().unwrap_or(0);
            NativeOutcome::Return
        }
        "_ZN7UserSvr10DllFreeTlsEi" => {
            proc.nat.tls.remove(&(r0, 0));
            NativeOutcome::Return
        }
        // C++ non-leave throw runtime (non-XLeaveException C++ exceptions are still refused by name).
        "__cxa_throw" => NativeOutcome::Refused(String::from("NEXT-NEEDED leave __cxa_throw: non-leave C++ throw is not implemented")),
        "__cxa_allocate_exception" => NativeOutcome::Refused(String::from("NEXT-NEEDED leave __cxa_allocate_exception: non-leave C++ throw is not implemented")),
        // CBase::operator new (inline in e32base.inl) is User::AllocZ / AllocZL: zero-filled. The L forms leave on failure.
        "_ZN4User7AllocZLEi" | "_ZN4User6AllocLEi" | "_Znwj" | "_Znaj" => {
            let len = proc.cpu.regs[0] as i32 as u32;
            if proc.heap.is_none() {
                return NativeOutcome::Refused(format!("NEXT-NEEDED euser/{name} before heap setup"));
            }
            match proc.alloc_zeroed(len) {
                Some(a) => { proc.cpu.regs[0] = a; NativeOutcome::Return }
                None => NativeOutcome::Refused(format!("NEXT-NEEDED leave {name}({len}): out of memory leaves KErrNoMemory (EHABI unwinding is slice 1b)")),
            }
        }
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
            if proc.heap.is_none() {
                return NativeOutcome::Refused("NEXT-NEEDED euser/User::AllocZ before heap setup".into());
            }
            proc.cpu.regs[0] = proc.alloc_zeroed(len).unwrap_or(0);
            NativeOutcome::Return
        }
        "_ZN6RDebug5PrintE11TRefByValueIK7TDesC16Ez" | "_ZN6RDebug6PrintfEPKcz" => NativeOutcome::Return,
        // Local synchronization primitives (single-threaded cooperative engine: CreateLocal/Wait/Signal succeed cleanly).
        "_ZN10RSemaphore11CreateLocalEi10TOwnerType"
        | "_ZN6RMutex11CreateLocalE10TOwnerType" => {
            let _ = proc.mem.vm.write_u32(proc.cpu.regs[0], 1);
            proc.cpu.regs[0] = 0;
            NativeOutcome::Return
        }
        "_ZN10RSemaphore4WaitEv"
        | "_ZN10RSemaphore6SignalEv"
        | "_ZN10RSemaphore6SignalEi"
        | "_ZN6RMutex4WaitEv"
        | "_ZN6RMutex6SignalEv" => NativeOutcome::Return,
        // Cooperative worker RThread (runs entry(arg) on Resume() until it enters CActiveScheduler::Start() or exits).
        // TInt RThread::Create(const TDesC16& aName, TThreadFunction aFunction, TInt aStackSize, TInt aMinHeapSize, TInt aMaxHeapSize, TAny* aPtr, TOwnerType aType)
        "_ZN7RThread6CreateERK7TDesC16PFiPvEiiiS3_10TOwnerType" => {
            let sp = proc.cpu.regs[13];
            let entry = proc.cpu.regs[2];
            let stack_size = proc.cpu.regs[3].clamp(0x2000, 0x10000);
            let arg = proc.mem.vm.read_u32(sp + 8).unwrap_or(0);
            proc.nat.objs.next_thread_handle += 1;
            let h = proc.nat.objs.next_thread_handle;
            proc.nat.objs.threads.insert(h, crate::objects::ThreadEntry { entry, arg, stack_size, resumed: false });
            let _ = proc.mem.vm.write_u32(proc.cpu.regs[0], h);
            proc.cpu.regs[0] = 0;
            NativeOutcome::Return
        }
        "_ZNK7RThread6ResumeEv" => {
            let h = proc.mem.vm.read_u32(proc.cpu.regs[0]).unwrap_or(0);
            let launch = proc.nat.objs.threads.get_mut(&h).and_then(|t| {
                if !t.resumed && t.entry != 0 {
                    t.resumed = true;
                    Some((t.entry, t.arg, t.stack_size))
                } else {
                    None
                }
            });
            if let Some((entry, arg, stack_size)) = launch {
                if let Some(stack_base) = proc.alloc_zeroed(stack_size) {
                    let top = (stack_base + stack_size) & !7;
                    if proc.call_guest(entry, &[arg], crate::callback::Continuation::ThreadStart).is_ok() {
                        proc.cpu.regs[13] = top;
                    }
                }
            }
            NativeOutcome::Return
        }
        // void RThread::RequestComplete(TRequestStatus*& aStatus, TInt aReason) const
        // void User::RequestComplete(TRequestStatus*& aStatus, TInt aReason)
        "_ZNK7RThread15RequestCompleteERP14TRequestStatusi"
        | "_ZN4User15RequestCompleteERP14TRequestStatusi" => {
            let (status_ptr_addr, reason) = if name.starts_with("_ZN4User") {
                (proc.cpu.regs[0], proc.cpu.regs[1])
            } else {
                (proc.cpu.regs[1], proc.cpu.regs[2])
            };
            if let Ok(status_addr) = proc.mem.vm.read_u32(status_ptr_addr) {
                if status_addr != 0 {
                    let _ = proc.mem.vm.write_u32(status_addr, reason);
                    let _ = proc.mem.vm.write_u32(status_ptr_addr, 0);
                    proc.nat.requests.signals += 1;
                }
            }
            NativeOutcome::Return
        }
        "_ZNK7RThread11SetPriorityE15TThreadPriority"
        | "_ZN7RThread4KillEi"
        | "_ZN7RThread9TerminateEi" => NativeOutcome::Return,
        // CTrapCleanup::New() / CActiveScheduler::CActiveScheduler() / CActiveScheduler::Install(CActiveScheduler*)
        "_ZN12CTrapCleanup3NewEv" | "_ZN16CActiveSchedulerC1Ev" => {
            if name == "_ZN12CTrapCleanup3NewEv" {
                let obj = proc.plain_object("CTrapCleanup", 16).unwrap_or(0);
                proc.cpu.regs[0] = obj;
            }
            NativeOutcome::Return
        }
        "_ZN12CTrapCleanupD1Ev" | "_ZN12CTrapCleanupD0Ev" => {
            if name.ends_with("D0Ev") {
                if let Some(heap) = proc.heap.as_mut() { let _ = heap.free(proc.cpu.regs[0]); }
            }
            NativeOutcome::Return
        }
        "_ZN16CActiveScheduler7InstallEPS_" => NativeOutcome::Return,
        "tolower" => {
            let c = proc.cpu.regs[0] & 0xFF;
            proc.cpu.regs[0] = if (b'A'..=b'Z').contains(&(c as u8)) { c + 32 } else { c };
            NativeOutcome::Return
        }
        "toupper" => {
            let c = proc.cpu.regs[0] & 0xFF;
            proc.cpu.regs[0] = if (b'a'..=b'z').contains(&(c as u8)) { c - 32 } else { c };
            NativeOutcome::Return
        }
        "_ZN4User12StringLengthEPKh" => {
            let mut ptr = proc.cpu.regs[0];
            let mut n = 0u32;
            while n < 0x10000 && proc.mem.vm.read_u8(ptr).unwrap_or(0) != 0 {
                ptr = ptr.wrapping_add(1);
                n += 1;
            }
            proc.cpu.regs[0] = n;
            NativeOutcome::Return
        }
        "_ZN4User12StringLengthEPKt" => {
            let mut ptr = proc.cpu.regs[0];
            let mut n = 0u32;
            while n < 0x10000 && proc.mem.vm.read_u16(ptr).unwrap_or(0) != 0 {
                ptr = ptr.wrapping_add(2);
                n += 1;
            }
            proc.cpu.regs[0] = n;
            NativeOutcome::Return
        }
        "_ZN4User9TickCountEv" => {
            proc.cpu.regs[0] = (proc.nat.clock_us / 15_625).saturating_add(proc.insns / 50_000) as u32;
            NativeOutcome::Return
        }
        "_ZN4User10NTickCountEv" => {
            proc.cpu.regs[0] = (proc.nat.clock_us / 1_000).saturating_add(proc.insns / 3_200) as u32;
            NativeOutcome::Return
        }
        "_ZN4User11FastCounterEv" => {
            proc.cpu.regs[0] = proc.nat.clock_us.saturating_add(proc.insns) as u32;
            NativeOutcome::Return
        }
        "_ZN4User19ResetInactivityTimeEv" => NativeOutcome::Return,
        // RLibrary (e32std.h): dynamic optional plugin loading; absent DLLs return KErrNotFound (-1).
        "_ZN8RLibrary4LoadERK7TDesC16S2_" | "_ZN8RLibrary4LoadERK7TDesC16" => {
            let _ = proc.mem.vm.write_u32(proc.cpu.regs[0], 0);
            proc.cpu.regs[0] = (-1i32) as u32; // KErrNotFound
            NativeOutcome::Return
        }
        "_ZNK8RLibrary6LookupEi" => {
            proc.cpu.regs[0] = 0;
            NativeOutcome::Return
        }
        "_ZN8RLibrary5CloseEv" => {
            let _ = proc.mem.vm.write_u32(proc.cpu.regs[0], 0);
            NativeOutcome::Return
        }
        // CAknAppUiBase::KeySounds() / CAknKeySoundSystem::{PushContextL,PopContext,PlaySound}: key click feedback (no-op on host).
        "_ZNK13CAknAppUiBase9KeySoundsEv" => {
            let p = proc.alloc_zeroed(16).unwrap_or(0);
            proc.cpu.regs[0] = p;
            NativeOutcome::Return
        }
        "_ZN18CAknKeySoundSystem12PushContextLEi"
        | "_ZN18CAknKeySoundSystem10PopContextEv"
        | "_ZN18CAknKeySoundSystem9PlaySoundEi" => NativeOutcome::Return,
        "_ZN4User4FreeEPv" | "_ZdlPv" | "_ZdaPv" => {
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
        // RVCT Thumb 8-bit jump-table helper (`drtrvct2_2/28`): index in r3, table at (lr & !1) where
        // table[0] = case count N, table[1..=N] = halfword offsets for cases 0..N-1, table[N+1] = default.
        // Branches to ((lr & !1) + 2 * offset) | 1 via the import stub's BX LR.
        "__ARM_switch8" => {
            let table = proc.cpu.regs[14] & !1;
            let Ok(count) = proc.mem.vm.read_u8(table) else {
                return NativeOutcome::Refused(format!("NEXT-NEEDED {dll_base}/{ordinal} __ARM_switch8: table at {table:#x} unreadable"));
            };
            let idx = if proc.cpu.regs[3] < u32::from(count) { proc.cpu.regs[3] } else { u32::from(count) };
            let Ok(off) = proc.mem.vm.read_u8(table.wrapping_add(1).wrapping_add(idx)) else {
                return NativeOutcome::Refused(format!("NEXT-NEEDED {dll_base}/{ordinal} __ARM_switch8: entry {idx} at {table:#x} unreadable"));
            };
            proc.cpu.regs[14] = table.wrapping_add(u32::from(off) * 2) | 1;
            NativeOutcome::Return
        }
        "_ZN4User11LeaveIfNullEPv" => {
            if r0 == 0 {
                NativeOutcome::Refused(format!("NEXT-NEEDED leave {name}(NULL): KErrNoMemory (EHABI unwinding is slice 1b)"))
            } else {
                proc.cpu.regs[0] = r0;
                NativeOutcome::Return
            }
        }
        "_ZN4User4ExitEi" => NativeOutcome::Exit(r0 as i32),
        "_ZN4User5PanicERK7TDesC16i" => match crate::desc::read16(proc, r0) {
            Ok(units) => NativeOutcome::Refused(format!(
                "NEXT-NEEDED {}/{} _ZN4User5PanicE (panic=\"{}\" reason={})",
                dll_base, ordinal, crate::desc::to_utf8(&units[..units.len().min(64)]), r1 as i32
            )),
            Err(e) => NativeOutcome::Refused(format!("NEXT-NEEDED {}/{} _ZN4User5PanicE ({e})", dll_base, ordinal)),
        },
        "" => match (dll_base.as_str(), ordinal) {
            // S60 5th / Symbian^3 ws32 ordinal 609: RWindowBase::EnableAdvancedPointers() (multi-touch enable, void).
            ("ws32", 609) => NativeOutcome::Return,
            // Nokia OOM Monitor (goommonitor.dll): ROomMonitorSession::Connect() (ord 10) / RequestFreeMemory(TInt) (ord 23) -> KErrNone.
            ("goommonitor", 10) | ("goommonitor", 23) => { proc.cpu.regs[0] = 0; NativeOutcome::Return }
            // S60 imut.dll (imcvcodc.h) TImCodecB64: standard RFC 4648 Base64 codec.
            // Ordinal 27 = TImCodecB64::TImCodecB64(), 24 = TImCodecB64::Initialise(),
            // 25 = TInt TImCodecB64::Decode(const TDesC8& aSrcString, TDes8& rDestString).
            ("imut", 27) => { proc.cpu.regs[0] = r0; NativeOutcome::Return }
            ("imut", 24) => NativeOutcome::Return,
            ("imut", 25) => {
                let dst = proc.cpu.regs[2];
                let res = crate::desc::read8(proc, r1).and_then(|src| {
                    let mut out = Vec::with_capacity(src.len() * 3 / 4 + 1);
                    let mut buf = 0u32;
                    let mut bits = 0u32;
                    for &b in &src {
                        let v = match b {
                            b'A'..=b'Z' => b - b'A',
                            b'a'..=b'z' => b - b'a' + 26,
                            b'0'..=b'9' => b - b'0' + 52,
                            b'+' => 62,
                            b'/' => 63,
                            b'=' | b'\r' | b'\n' | b' ' | b'\t' => continue,
                            _ => continue,
                        };
                        buf = (buf << 6) | u32::from(v);
                        bits += 6;
                        if bits >= 8 {
                            bits -= 8;
                            out.push(((buf >> bits) & 0xff) as u8);
                        }
                    }
                    crate::desc::write8(proc, dst, &out)
                });
                match res {
                    Ok(()) => { proc.cpu.regs[0] = 0; NativeOutcome::Return }
                    Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED imut/24 TImCodecB64::Decode: {m}")),
                }
            }
            _ => NativeOutcome::Refused(format!(
                "NEXT-NEEDED {}/{} <unnamed-ordinal>", dll_base, ordinal
            )),
        },
        _ => NativeOutcome::Refused(format!(
            "NEXT-NEEDED {}/{} {}", dll_base, ordinal, name
        )),
    }
}
impl Process {
    /// Allocate `len` bytes from the guest heap, all of them zero (the whole block, as `AllocLen` reports it).
    pub(crate) fn alloc_zeroed(&mut self, len: u32) -> Option<u32> {
        let h = self.heap.as_mut()?;
        let addr = h.alloc(len, &mut self.mem)?;
        let total = h.len_of(addr).unwrap_or(0);
        for i in 0..total {
            let _ = self.mem.vm.write_u8(addr + i, 0);
        }
        Some(addr)
    }
}
