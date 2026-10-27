//! What a list box shows, as the guest holds it: the descriptor array of item texts (`CDesC16Array`, badesca.h), the text model that
//! points at it (`CTextListBoxModel`, eiktxlbm.h) and the icon array (`CAknIconArray`, akniconarray.h). All of it lives in guest
//! memory at the SDK's layouts; the host sees it only through the projection (`listproject.rs`).
//!
//! `CDesC16Array`: `CArrayFixBase` (24 bytes, array.rs) then the `MDesC16Array` base (vptr at 24, bamdesca.h: `MdcaCount`,
//! `MdcaPoint`). Each item is one heap cell holding a `TBufC16` image (word 0 = length, data from +4), so `desc::read16` reads it and
//! `MdcaPoint` returns a `TPtrC16` into it; the array's record is the cell's address (4 bytes).
//! `CTextListBoxModel`: `CBase` vptr 0, the `MTextListBoxModel` vptr 4, `iItemTextArray` 8, `iItemArrayOwnershipType` 12 (eiktxlbm.h:73-77).
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use crate::array::{ARR_I_COUNT, ARR_SIZE};
use crate::desc::{self, DES_PTR_C, KSHIFT_DES_TYPE};
use crate::enginestub::slots_with_gaps;
use crate::loader::Process;
use crate::natives::NativeOutcome;

pub const MDESC_BASE: u32 = 24; // CArrayFixBase is 24 bytes, MDesC16Array follows (badesca.h:78 `CArrayFixBase, public MDesC16Array`)
pub const DESARR_SIZE: u32 = MDESC_BASE + 4;
pub const MODEL_I_ARRAY: u32 = 8;
pub const MODEL_I_OWNERSHIP: u32 = 12;
pub const MODEL_SIZE: u32 = 16;
const MDESC_COUNT: &str = "_ZThn24_NK12CDesC16Array9MdcaCountEv";
const MDESC_POINT: &str = "_ZThn24_NK12CDesC16Array9MdcaPointEi";
const REP_FLAT: &str = "_ZN8CBufFlat4NewLEi";
/// `CBase`: ~D1 0, ~D0 1, Extension_ 2 (the primary table of every array and the model; no other slot is called yet).
const BASE_SLOTS: usize = 3;

impl Process {
    /// `CDesC16ArrayFlat::CDesC16ArrayFlat(TInt aGranularity)`.
    fn desarr_construct(&mut self, this: u32, gran: u32) -> Result<(), String> {
        let rep = self.engine_stub(REP_FLAT)?;
        self.array_construct(this, rep, 4, gran)?;
        let vptr = self.build_vtable("CDesC16ArrayFlat", &slots_with_gaps(BASE_SLOTS, &[(0, "_ZN16CDesC16ArrayFlatD1Ev"), (1, "_ZN16CDesC16ArrayFlatD0Ev"), (2, "_ZN5CBase10Extension_EjRPvS0_")]))?;
        let mdesc = self.build_vtable("MDesC16Array", &[Some(MDESC_COUNT), Some(MDESC_POINT)])?;
        self.wr(this, vptr)?;
        self.wr(this + MDESC_BASE, mdesc)
    }

    /// `CDesC16Array::AppendL(const TDesC16&)`: a copy of the descriptor's text goes in as the last item.
    fn desarr_append(&mut self, this: u32, des: u32) -> Result<(), String> {
        let text = desc::read16(self, des)?;
        let cell = self.alloc_zeroed(4 + text.len() as u32 * 2 + 2).ok_or_else(|| String::from("out of memory for an item (leaves KErrNoMemory: EHABI unwinding is slice 1b)"))?;
        self.wr(cell, text.len() as u32)?; // type 0 = TBufC16: the data follows
        for (i, u) in text.iter().enumerate() {
            self.mem.vm.write_u16(cell + 4 + i as u32 * 2, *u).map_err(|_| String::from("item cell unwritable"))?;
        }
        let count = self.rd(this + ARR_I_COUNT)?;
        self.array_insert_record(this, count as i32, cell.to_le_bytes().to_vec())
    }

    /// The item cell at `index` of the descriptor array at `this`.
    fn desarr_cell(&self, this: u32, index: i32) -> Result<u32, String> {
        let record = self.array_at(this, index)?;
        self.rd(record)
    }

    fn desarr_reset(&mut self, this: u32) -> Result<(), String> {
        for i in 0..self.rd(this + ARR_I_COUNT)? {
            let cell = self.desarr_cell(this, i as i32)?;
            if let Some(h) = self.heap.as_mut() { let _ = h.free(cell); }
        }
        self.array_reset(this)
    }

    /// The item texts (UTF-16) of the `MDesC16Array` at `iface` (what the guest passed: the secondary base of an engine-built
    /// `CDesC16Array`). A guest-implemented one answers through its own virtuals, which need a call back into the guest: refused by name.
    pub(crate) fn mdesc_items(&self, iface: u32) -> Result<Vec<Vec<u16>>, String> {
        if iface == 0 { return Ok(Vec::new()); }
        let vptr = self.rd(iface)?;
        let first = self.rd(vptr)?;
        let ours = first >= crate::layout::STUB_BASE && first & 7 == 0 && self.stubs.get(((first - crate::layout::STUB_BASE) / 8) as usize).and_then(|s| s.name.as_deref()) == Some(MDESC_COUNT);
        if !ours {
            return Err(format!("the MDesC16Array at {iface:#x} is the guest's own implementation (vtable {vptr:#x}): reading its items needs the guest's MdcaCount/MdcaPoint called back"));
        }
        let this = iface - MDESC_BASE;
        (0..self.rd(this + ARR_I_COUNT)?).map(|i| desc::read16(self, self.desarr_cell(this, i as i32)?)).collect()
    }

    /// A `CTextListBoxModel` (`CTextListBoxModel::ConstructL(NULL, ELbmOwnsItemArray)`): no item array yet.
    pub(crate) fn make_text_model(&mut self) -> Result<u32, String> {
        let m = self.alloc_zeroed(MODEL_SIZE).ok_or_else(|| String::from("no guest heap for the list box model"))?;
        let primary = self.build_vtable("CTextListBoxModel", &slots_with_gaps(BASE_SLOTS, &[(0, "_ZN17CTextListBoxModelD1Ev"), (1, "_ZN17CTextListBoxModelD0Ev"), (2, "_ZN5CBase10Extension_EjRPvS0_")]))?;
        let mtext = self.build_vtable("MTextListBoxModel", &[])?;
        self.wr(m, primary)?;
        self.wr(m + 4, mtext)?;
        Ok(m)
    }

    /// The `CDesC16Array` interface pointer the model at `model` was given (0 = none).
    pub(crate) fn model_array(&self, model: u32) -> Result<u32, String> {
        self.rd(model + MODEL_I_ARRAY)
    }
}

pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let (r0, r1, r2) = (p.cpu.regs[0], p.cpu.regs[1], p.cpu.regs[2]);
    let refuse = |m: String| NativeOutcome::Refused(format!("NEXT-NEEDED {name}: {m}"));
    let ret = |p: &mut Process, r: Result<u32, String>| match r { Ok(v) => { p.cpu.regs[0] = v; NativeOutcome::Return } Err(m) => refuse(m) };
    Some(match name {
        // CDesC16ArrayFlat(TInt aGranularity) returns this.
        "_ZN16CDesC16ArrayFlatC1Ei" | "_ZN16CDesC16ArrayFlatC2Ei" => { let r = p.desarr_construct(r0, r1); ret(p, r.map(|_| r0)) }
        "_ZN12CDesC16Array7AppendLERK7TDesC16" => { let r = p.desarr_append(r0, r1); ret(p, r.map(|_| 0)) }
        "_ZN12CDesC16Array5ResetEv" => { let r = p.desarr_reset(r0); ret(p, r.map(|_| 0)) }
        // TInt MdcaCount() const; the thunk's `this` is the MDesC16Array base.
        "_ZNK12CDesC16Array9MdcaCountEv" | MDESC_COUNT => {
            let this = if name == MDESC_COUNT { r0 - MDESC_BASE } else { r0 };
            let r = p.rd(this + ARR_I_COUNT); ret(p, r)
        }
        // TPtrC16 MdcaPoint(TInt) const: a class return, so r0 is the result's address, r1 `this`, r2 the index. The result points into the cell.
        "_ZNK12CDesC16Array9MdcaPointEi" | MDESC_POINT => {
            let this = if name == MDESC_POINT { r1 - MDESC_BASE } else { r1 };
            let r = p.desarr_cell(this, r2 as i32).and_then(|cell| {
                let len = p.rd(cell)?;
                p.wr(r0, len | DES_PTR_C << KSHIFT_DES_TYPE)?;
                p.wr(r0 + 4, cell + 4)?;
                Ok(r0)
            });
            ret(p, r)
        }
        "_ZN16CDesC16ArrayFlatD1Ev" | "_ZN16CDesC16ArrayFlatD0Ev" | "_ZN12CDesC16ArrayD1Ev" => {
            let r = p.desarr_reset(r0).and_then(|_| p.array_destroy(r0));
            if r.is_ok() && name.ends_with("D0Ev") { if let Some(h) = p.heap.as_mut() { let _ = h.free(r0); } }
            ret(p, r.map(|_| r0))
        }
        // CAknIconArray(TInt aGranularity): a `CArrayPtrFlat<CGulIcon>`, vptr 0 (the icons are appended by the inline templates' InsertL).
        "_ZN13CAknIconArrayC1Ei" | "_ZN13CAknIconArrayC2Ei" => {
            let r = p.engine_stub(REP_FLAT).and_then(|rep| p.array_construct(r0, rep, 4, r1))
                .and_then(|_| p.build_vtable("CAknIconArray", &slots_with_gaps(BASE_SLOTS, &[(0, "_ZN13CAknIconArrayD1Ev"), (1, "_ZN13CAknIconArrayD0Ev"), (2, "_ZN5CBase10Extension_EjRPvS0_")])))
                .and_then(|vptr| p.wr(r0, vptr));
            ret(p, r.map(|_| r0))
        }
        "_ZN13CAknIconArrayD1Ev" | "_ZN13CAknIconArrayD0Ev" => match p.rd(r0 + ARR_I_COUNT) {
            Ok(0) => {
                let r = p.array_destroy(r0);
                if r.is_ok() && name.ends_with("D0Ev") { if let Some(h) = p.heap.as_mut() { let _ = h.free(r0); } }
                ret(p, r.map(|_| r0))
            }
            Ok(n) => refuse(format!("destroying an icon array of {n} icons: the icons (CGulIcon) are not modelled, so they cannot be deleted")),
            Err(m) => refuse(m),
        },
        // CTextListBoxModel (eiktxlbm.h:55-60).
        "_ZN17CTextListBoxModel16SetItemTextArrayEP12MDesC16Array" => { let r = p.wr(r0 + MODEL_I_ARRAY, r1); ret(p, r.map(|_| 0)) }
        "_ZN17CTextListBoxModel16SetOwnershipTypeE31TListBoxModelItemArrayOwnership" => { let r = p.wr(r0 + MODEL_I_OWNERSHIP, r1); ret(p, r.map(|_| 0)) }
        "_ZNK17CTextListBoxModel13ItemTextArrayEv" => { let r = p.model_array(r0); ret(p, r) }
        _ => return None,
    })
}

// ARR_SIZE is the CArrayFixBase part of the object the guest allocates; a CDesC16ArrayFlat adds the MDesC16Array vptr.
const _: () = assert!(ARR_SIZE == MDESC_BASE);
