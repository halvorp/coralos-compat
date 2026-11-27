//! Fixed-record dynamic arrays (`CArrayFixBase`, e32base.h:307-357) over a flat buffer (`CBufBase` / `CBufFlat`, :66-196).
//! Layouts (ARM EABI): `CArrayFixBase` vptr 0, `iCount` 4, `iGranularity` 8, `iLength` 12 (the record length), `iCreateRep` 16,
//! `iBase` 20; `CBufBase` vptr 0, `iSize` 4, `iExpandSize` 8; `CBufFlat` adds `iMaxSize` 12, `iPtr` 16. The guest's inline
//! `Count()` reads `iCount`, so it is kept current in guest memory. Records live in the one heap cell `iPtr` names.
use alloc::format;
use alloc::string::String;
use crate::enginestub::slots_with_gaps;
use crate::loader::Process;
use crate::natives::NativeOutcome;

pub const ARR_I_COUNT: u32 = 4;
pub const ARR_I_GRANULARITY: u32 = 8;
pub const ARR_I_LENGTH: u32 = 12;
pub const ARR_I_CREATE_REP: u32 = 16;
pub const ARR_I_BASE: u32 = 20;
pub const ARR_SIZE: u32 = 24;
pub const BUF_I_SIZE: u32 = 4;
pub const BUF_I_EXPAND: u32 = 8;
pub const BUF_I_MAX_SIZE: u32 = 12;
pub const BUF_I_PTR: u32 = 16;
pub const BUF_FLAT_SIZE: u32 = 20;
/// ~CBufFlat 0,1 and CBase::Extension_ 2; the buffer's own pure virtuals follow and are unnamed gaps (the guest calls none).
const BUF_VTABLE: &[(usize, &str)] = &[(0, "_ZN8CBufFlatD1Ev"), (1, "_ZN8CBufFlatD0Ev"), (2, "_ZN5CBase10Extension_EjRPvS0_")];
const REP_FLAT: &str = "_ZN8CBufFlat4NewLEi";
const REP_SEG: &str = "_ZN7CBufSeg4NewLEi";

impl Process {
    /// `CBufFlat::NewL(TInt anExpandSize)`: an empty flat buffer object.
    pub(crate) fn buf_flat_new(&mut self, expand: u32) -> Result<u32, String> {
        let b = self.alloc_zeroed(BUF_FLAT_SIZE).ok_or_else(|| String::from("no guest heap for a CBufFlat"))?;
        let vptr = self.build_vtable("CBufFlat", &slots_with_gaps(3, BUF_VTABLE))?;
        self.wr(b, vptr)?;
        self.wr(b + BUF_I_EXPAND, expand)?;
        Ok(b)
    }

    pub(crate) fn array_construct(&mut self, this: u32, rep: u32, reclen: u32, gran: u32) -> Result<(), String> {
        let idx = (rep.wrapping_sub(crate::layout::STUB_BASE) / 8) as usize;
        let name = if rep & 7 == 0 { self.stubs.get(idx).and_then(|s| s.name.clone()) } else { None };
        if name.as_deref() != Some(REP_FLAT) && name.as_deref() != Some(REP_SEG) {
            return Err(format!("the buffer factory {rep:#x} ({}) is neither CBufFlat::NewL nor CBufSeg::NewL", name.as_deref().unwrap_or("guest code")));
        }
        for o in (ARR_I_COUNT..ARR_SIZE).step_by(4) { self.wr(this + o, 0)?; }
        let base = self.buf_flat_new(gran)?;
        self.wr(this + ARR_I_GRANULARITY, gran)?;
        self.wr(this + ARR_I_LENGTH, reclen)?;
        self.wr(this + ARR_I_CREATE_REP, rep)?;
        self.wr(this + ARR_I_BASE, base)
    }

    /// A segmented array (`CBufSeg`) keeps each record in a cell of its own, so a pointer to a record stays valid while others are
    /// inserted (segments never move); `iPtr` then holds a table of cell addresses. A flat one keeps the records themselves in `iPtr`.
    fn array_is_seg(&self, this: u32) -> bool {
        let rep = self.rd(this + ARR_I_CREATE_REP).unwrap_or(0);
        rep & 7 == 0 && self.stubs.get((rep.wrapping_sub(crate::layout::STUB_BASE) / 8) as usize).and_then(|s| s.name.as_deref()) == Some(REP_SEG)
    }

    pub(crate) fn array_at(&self, this: u32, i: i32) -> Result<u32, String> {
        let (count, len, base) = (self.rd(this + ARR_I_COUNT)?, self.rd(this + ARR_I_LENGTH)?, self.rd(this + ARR_I_BASE)?);
        if i < 0 || i as u32 >= count {
            return Err(format!("index {i} outside an array of {count} (E32USER-CBase 21 on a device)"));
        }
        let table = self.rd(base + BUF_I_PTR)?;
        if self.array_is_seg(this) { self.rd(table.wrapping_add(i as u32 * 4)) } else { Ok(table.wrapping_add(i as u32 * len)) }
    }

    /// `InsertL(index, record)`: grow the cell if needed, shift the tail up, copy the record in.
    fn array_insert(&mut self, this: u32, index: i32, src: u32) -> Result<(), String> {
        let rec_len = self.rd(this + ARR_I_LENGTH)?;
        let mut record = alloc::vec![0u8; rec_len as usize];
        self.mem.vm.read_bytes(src, &mut record).map_err(|_| format!("the record at {src:#x} is unreadable"))?;
        self.array_insert_record(this, index, record)
    }

    /// `InsertL` of a record the engine holds (`record.len()` = the array's record length).
    pub(crate) fn array_insert_record(&mut self, this: u32, index: i32, mut record: alloc::vec::Vec<u8>) -> Result<(), String> {
        let (count, rec_len, base) = (self.rd(this + ARR_I_COUNT)?, self.rd(this + ARR_I_LENGTH)?, self.rd(this + ARR_I_BASE)?);
        let seg = self.array_is_seg(this);
        let len = if seg { 4 } else { rec_len };
        if index < 0 || index as u32 > count {
            return Err(format!("insert at {index} into an array of {count} (E32USER-CBase 21 on a device)"));
        }
        let (size, max, ptr) = (self.rd(base + BUF_I_SIZE)?, self.rd(base + BUF_I_MAX_SIZE)?, self.rd(base + BUF_I_PTR)?);
        if seg {
            let cell = self.alloc_zeroed(rec_len.max(1)).ok_or_else(|| String::from("out of memory for a record (leaves KErrNoMemory: EHABI unwinding is slice 1b)"))?;
            self.mem.vm.write_bytes(cell, &record).map_err(|_| String::from("record cell unwritable"))?;
            record = cell.to_le_bytes().to_vec();
        }
        let ptr = if size + len > max {
            let gran = self.rd(this + ARR_I_GRANULARITY)?.max(1);
            let new_max = max + (len * gran).max(len);
            let h = self.heap.as_mut().ok_or_else(|| String::from("no guest heap"))?;
            let np = if ptr == 0 { h.alloc(new_max, &mut self.mem) } else { h.realloc(ptr, new_max, &mut self.mem) };
            let np = np.ok_or_else(|| String::from("out of memory growing an array (leaves KErrNoMemory: EHABI unwinding is slice 1b)"))?;
            self.wr(base + BUF_I_MAX_SIZE, new_max)?;
            self.wr(base + BUF_I_PTR, np)?;
            np
        } else { ptr };
        let at = ptr + index as u32 * len;
        let mut tail = alloc::vec![0u8; (size - index as u32 * len) as usize];
        self.mem.vm.read_bytes(at, &mut tail).map_err(|_| String::from("array storage unreadable"))?;
        self.mem.vm.write_bytes(at, &record).map_err(|_| String::from("array storage unwritable"))?;
        self.mem.vm.write_bytes(at + len, &tail).map_err(|_| String::from("array storage unwritable"))?;
        self.wr(base + BUF_I_SIZE, size + len)?;
        self.wr(this + ARR_I_COUNT, count + 1)
    }

    pub(crate) fn array_reset(&mut self, this: u32) -> Result<(), String> {
        let base = self.rd(this + ARR_I_BASE)?;
        let ptr = self.rd(base + BUF_I_PTR)?;
        if self.array_is_seg(this) {
            for i in 0..self.rd(this + ARR_I_COUNT)? {
                let cell = self.rd(ptr + i * 4)?;
                if let Some(h) = self.heap.as_mut() { let _ = h.free(cell); }
            }
        }
        if ptr != 0 { if let Some(h) = self.heap.as_mut() { let _ = h.free(ptr); } }
        for o in [BUF_I_SIZE, BUF_I_MAX_SIZE, BUF_I_PTR] { self.wr(base + o, 0)?; }
        self.wr(this + ARR_I_COUNT, 0)
    }

    /// Free an array's storage and buffer object (`~CArrayFixBase`).
    pub(crate) fn array_destroy(&mut self, this: u32) -> Result<(), String> {
        let base = self.rd(this + ARR_I_BASE)?;
        if base != 0 {
            self.array_reset(this)?;
            if let Some(h) = self.heap.as_mut() { let _ = h.free(base); }
            self.wr(this + ARR_I_BASE, 0)?;
        }
        Ok(())
    }
}

/// The natives of this module; `None` = not ours.
pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let (r0, r1, r2, r3) = (p.cpu.regs[0], p.cpu.regs[1], p.cpu.regs[2], p.cpu.regs[3]);
    let refuse = |m: String| NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {m}"));
    let done = |p: &mut Process, r: Result<(), String>, ret: u32| match r { Ok(()) => { p.cpu.regs[0] = ret; NativeOutcome::Return } Err(m) => refuse(m) };
    Some(match name {
        // RArrayBase (e32cmn.h): iCount(0), iEntries(4), iEntrySize(8), iKeyOffset(12), iAllocated(16), iGranularity(20).
        "_ZN10RArrayBaseC1Ei" | "_ZN10RArrayBaseC2Ei" => {
            let r = p.wr(r0, 0).and_then(|_| p.wr(r0 + 4, 0)).and_then(|_| p.wr(r0 + 8, r1))
                .and_then(|_| p.wr(r0 + 12, 0)).and_then(|_| p.wr(r0 + 16, 0)).and_then(|_| p.wr(r0 + 20, 8));
            done(p, r, r0)
        }
        "_ZNK10RArrayBase5CountEv" => match p.rd(r0) { Ok(c) => { p.cpu.regs[0] = c; NativeOutcome::Return } Err(m) => refuse(m) },
        "_ZNK10RArrayBase2AtEi" => {
            let r = (|| -> Result<u32, String> {
                let count = p.rd(r0)?;
                let idx = r1 as i32;
                if idx < 0 || idx as u32 >= count { return Err(format!("RArrayBase::At({idx}) out of bounds ({count})")); }
                let entries = p.rd(r0 + 4)?;
                let esize = p.rd(r0 + 8)?;
                Ok(entries.wrapping_add((idx as u32).wrapping_mul(esize)))
            })();
            match r { Ok(a) => { p.cpu.regs[0] = a; NativeOutcome::Return } Err(m) => refuse(m) }
        }
        "_ZN10RArrayBase6AppendEPKv" => {
            let r = (|| -> Result<(), String> {
                let count = p.rd(r0)?;
                let mut entries = p.rd(r0 + 4)?;
                let esize = p.rd(r0 + 8)?.max(4);
                let mut alloc = p.rd(r0 + 16)?;
                let gran = p.rd(r0 + 20)?.max(8);
                if count >= alloc {
                    let new_alloc = alloc.saturating_add(gran);
                    let new_buf = p.alloc_zeroed(new_alloc.saturating_mul(esize)).ok_or_else(|| String::from("RArrayBase out of heap"))?;
                    if entries != 0 && count > 0 {
                        let old_bytes = count.saturating_mul(esize);
                        let mut tmp = alloc::vec![0u8; old_bytes as usize];
                        for i in 0..old_bytes { tmp[i as usize] = p.mem.vm.read_u8(entries + i).map_err(|_| String::from("RArrayBase read failed"))?; }
                        p.mem.vm.write_bytes(new_buf, &tmp).map_err(|_| String::from("RArrayBase write failed"))?;
                        if let Some(h) = p.heap.as_mut() { let _ = h.free(entries); }
                    }
                    entries = new_buf;
                    alloc = new_alloc;
                    p.wr(r0 + 4, entries)?;
                    p.wr(r0 + 16, alloc)?;
                }
                let dst = entries.wrapping_add(count.wrapping_mul(esize));
                for i in 0..esize {
                    let b = p.mem.vm.read_u8(r1 + i).map_err(|_| String::from("RArrayBase::Append src unreadable"))?;
                    p.mem.vm.write_u8(dst + i, b).map_err(|_| String::from("RArrayBase::Append dst unwritable"))?;
                }
                p.wr(r0, count + 1)
            })();
            done(p, r, 0)
        }
        "_ZN10RArrayBase6RemoveEi" => {
            let r = (|| -> Result<(), String> {
                let count = p.rd(r0)?;
                let idx = r1 as i32;
                if idx < 0 || idx as u32 >= count { return Err(format!("RArrayBase::Remove({idx}) out of bounds ({count})")); }
                let entries = p.rd(r0 + 4)?;
                let esize = p.rd(r0 + 8)?.max(4);
                for j in (idx as u32)..(count - 1) {
                    let src = entries.wrapping_add((j + 1).wrapping_mul(esize));
                    let dst = entries.wrapping_add(j.wrapping_mul(esize));
                    for i in 0..esize {
                        let b = p.mem.vm.read_u8(src + i).map_err(|_| String::from("RArrayBase::Remove read failed"))?;
                        p.mem.vm.write_u8(dst + i, b).map_err(|_| String::from("RArrayBase::Remove write failed"))?;
                    }
                }
                p.wr(r0, count - 1)
            })();
            done(p, r, 0)
        }
        "_ZN10RArrayBase5CloseEv" | "_ZN10RArrayBase5ResetEv" => {
            if let Ok(ptr) = p.rd(r0 + 4) { if ptr != 0 { if let Some(h) = p.heap.as_mut() { let _ = h.free(ptr); } } }
            let r = p.wr(r0, 0).and_then(|_| p.wr(r0 + 4, 0)).and_then(|_| p.wr(r0 + 16, 0));
            done(p, r, 0)
        }
        // RPointerArrayBase (e32cmn.h): iCount(0), iEntries(4), iAllocated(8), iGranularity(12) of 4-byte TAny* pointers.
        "_ZN17RPointerArrayBaseC1Ev" | "_ZN17RPointerArrayBaseC2Ev" | "_ZN17RPointerArrayBaseC1Ei" | "_ZN17RPointerArrayBaseC2Ei" => {
            let gran = if name.ends_with("Ei") { (r1 as i32).max(1) as u32 } else { 8 };
            let r = p.wr(r0, 0).and_then(|_| p.wr(r0 + 4, 0)).and_then(|_| p.wr(r0 + 8, 0)).and_then(|_| p.wr(r0 + 12, gran));
            done(p, r, r0)
        }
        "_ZNK17RPointerArrayBase5CountEv" => match p.rd(r0) { Ok(c) => { p.cpu.regs[0] = c; NativeOutcome::Return } Err(m) => refuse(m) },
        "_ZNK17RPointerArrayBase2AtEi" => {
            let r = (|| -> Result<u32, String> {
                let count = p.rd(r0)?;
                let idx = r1 as i32;
                if idx < 0 || idx as u32 >= count { return Err(format!("RPointerArrayBase::At({idx}) out of bounds ({count})")); }
                let entries = p.rd(r0 + 4)?;
                Ok(entries.wrapping_add((idx as u32) * 4))
            })();
            match r { Ok(a) => { p.cpu.regs[0] = a; NativeOutcome::Return } Err(m) => refuse(m) }
        }
        "_ZN17RPointerArrayBase6AppendEPKv" => {
            let r = (|| -> Result<(), String> {
                let count = p.rd(r0)?;
                let mut entries = p.rd(r0 + 4)?;
                let mut alloc = p.rd(r0 + 8)?;
                let gran = p.rd(r0 + 12)?.max(8);
                if count >= alloc {
                    let new_alloc = alloc.saturating_add(gran);
                    let new_buf = p.alloc_zeroed(new_alloc.saturating_mul(4)).ok_or_else(|| String::from("RPointerArrayBase out of heap"))?;
                    if entries != 0 && count > 0 {
                        for i in 0..count {
                            let v = p.rd(entries + i * 4)?;
                            p.wr(new_buf + i * 4, v)?;
                        }
                        if let Some(h) = p.heap.as_mut() { let _ = h.free(entries); }
                    }
                    entries = new_buf;
                    alloc = new_alloc;
                    p.wr(r0 + 4, entries)?;
                    p.wr(r0 + 8, alloc)?;
                }
                p.wr(entries + count * 4, r1)?;
                p.wr(r0, count + 1)
            })();
            done(p, r, 0)
        }
        "_ZN17RPointerArrayBase6RemoveEi" => {
            let r = (|| -> Result<(), String> {
                let count = p.rd(r0)?;
                let idx = r1 as i32;
                if idx < 0 || idx as u32 >= count { return Err(format!("RPointerArrayBase::Remove({idx}) out of bounds ({count})")); }
                let entries = p.rd(r0 + 4)?;
                for j in (idx as u32)..(count - 1) {
                    let v = p.rd(entries + (j + 1) * 4)?;
                    p.wr(entries + j * 4, v)?;
                }
                p.wr(r0, count - 1)
            })();
            done(p, r, 0)
        }
        "_ZN17RPointerArrayBase5CloseEv" | "_ZN17RPointerArrayBase5ResetEv" => {
            if let Ok(ptr) = p.rd(r0 + 4) { if ptr != 0 { if let Some(h) = p.heap.as_mut() { let _ = h.free(ptr); } } }
            let r = p.wr(r0, 0).and_then(|_| p.wr(r0 + 4, 0)).and_then(|_| p.wr(r0 + 8, 0));
            done(p, r, 0)
        }
        "_ZN8CBufFlat4NewLEi" | "_ZN7CBufSeg4NewLEi" => match p.buf_flat_new(r0) { Ok(b) => { p.cpu.regs[0] = b; NativeOutcome::Return } Err(m) => refuse(m) },
        "_ZN13CArrayFixBaseC1EPFP8CBufBaseiEii" | "_ZN13CArrayFixBaseC2EPFP8CBufBaseiEii" => { let r = p.array_construct(r0, r1, r2, r3); done(p, r, r0) }
        "_ZNK13CArrayFixBase2AtEi" => match p.array_at(r0, r1 as i32) { Ok(a) => { p.cpu.regs[0] = a; NativeOutcome::Return } Err(m) => refuse(m) },
        "_ZN13CArrayFixBase7InsertLEiPKv" => { let r = p.array_insert(r0, r1 as i32, r2); done(p, r, 0) }
        "_ZN13CArrayFixBase15SetReserveFlatLEi" => NativeOutcome::Return,
        "_ZN12TKeyArrayFixC1Ei14TKeyCmpNumeric" | "_ZN12TKeyArrayFixC2Ei14TKeyCmpNumeric" => {
            // TKey: vptr (+0), iKeyOffset (+4), iKeyLength (+8), iCmpType (+12)
            let _ = p.wr(r0.wrapping_add(4), r1);
            let _ = p.wr(r0.wrapping_add(12), r2);
            p.cpu.regs[0] = r0;
            NativeOutcome::Return
        }
        "_ZN13CArrayFixBase4SortER12TKeyArrayFix" => {
            p.cpu.regs[0] = 0; // KErrNone
            NativeOutcome::Return
        }
        "_ZN13CArrayFixBase5ResetEv" => { let r = p.array_reset(r0); done(p, r, 0) }
        "_ZN13CArrayFixBaseD1Ev" | "_ZN13CArrayFixBaseD2Ev" => { let r = p.array_destroy(r0); done(p, r, r0) }
        "_ZN8CBufFlatD1Ev" | "_ZN8CBufFlatD0Ev" => {
            if let Ok(ptr) = p.rd(r0 + BUF_I_PTR) { if ptr != 0 { if let Some(h) = p.heap.as_mut() { let _ = h.free(ptr); } } }
            if name.ends_with("D0Ev") { if let Some(h) = p.heap.as_mut() { let _ = h.free(r0); } }
            NativeOutcome::Return
        }
        _ => return None,
    })
}

