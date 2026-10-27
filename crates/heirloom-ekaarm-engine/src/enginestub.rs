//! Engine-owned stubs and virtual tables. A library class the GUEST instantiates (`new CAknDoubleLargeStyleListBox`) gets its
//! virtual table from the library; an object the ENGINE builds for the guest (the screen device `CCoeEnv` owns, a list box
//! constructed by a native) needs one in guest memory too, because guest code dispatches through it. Each entry is a stub
//! (`SVC; BX LR`, exactly like an import) that dispatches by mangled name through the same natives as every import, so a slot
//! nobody implemented stops the run with a named `NEXT-NEEDED` instead of jumping into zeroes.
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_mem_paged32::Prot;
use crate::layout::{ENGINE_STUB_SLOTS, STUB_BASE};
use crate::loader::Process;
use crate::stubs::{encode_stub, stub_addr, Stub};

/// The `dll_base` the stubs the engine hands out are filed under in the stub table.
pub const ENGINE_DLL: &str = "engine";

impl Process {
    /// The address of a stub dispatching to the native named `name`: the image's own import of that name if it has one, else a new
    /// engine stub (at most [`ENGINE_STUB_SLOTS`] of them).
    pub fn engine_stub(&mut self, name: &str) -> Result<u32, String> {
        if let Some(i) = self.stubs.iter().position(|s| s.name.as_deref() == Some(name)) {
            return Ok(stub_addr(i));
        }
        let idx = self.stubs.len();
        let engine_made = self.stubs.iter().filter(|s| s.dll_base == ENGINE_DLL).count() as u32;
        let slot_limit_hit = engine_made >= ENGINE_STUB_SLOTS;
        let chunk_end = (STUB_BASE as u64) + ((idx as u64) + 1) * 8;
        if slot_limit_hit || chunk_end > STUB_BASE as u64 + (self.stub_capacity() as u64) {
            return Err(format!("the engine stub table is full ({ENGINE_STUB_SLOTS} engine stubs): cannot add {name}"));
        }
        let vm = &mut self.mem.vm;
        vm.set_prot(self.stub_chunk_id, Prot::RW).map_err(|e| format!("stub page unprotect: {e:?}"))?;
        let wrote = vm.write_bytes(stub_addr(idx), &encode_stub(idx));
        vm.set_prot(self.stub_chunk_id, Prot::RX).map_err(|e| format!("stub page protect: {e:?}"))?;
        wrote.map_err(|e| format!("stub write: {e:?}"))?;
        self.stubs.push(Stub { dll_base: String::from(ENGINE_DLL), dll: String::from(ENGINE_DLL), ordinal: 0, name: Some(String::from(name)) });
        Ok(stub_addr(idx))
    }

    /// Bytes the stub chunk was created with (imports and the engine's reserve).
    fn stub_capacity(&self) -> u32 {
        let imports = self.stubs.iter().filter(|s| s.dll_base != ENGINE_DLL).count() as u32;
        crate::layout::pru((imports + ENGINE_STUB_SLOTS) * 8)
    }

    /// Build a virtual table in guest memory: `slots[i]` is the mangled name of the native for slot `i`; `None` is a slot nobody
    /// verified, which dispatches to a stub named `<class>::vslot<i>` (a named refusal when called). The two words before the vptr
    /// (offset-to-top, typeinfo) are zero: nothing here is thrown or `dynamic_cast`. Returns the vptr.
    pub fn build_vtable(&mut self, class: &str, slots: &[Option<&str>]) -> Result<u32, String> {
        let base = self.alloc_zeroed(8 + slots.len() as u32 * 4).ok_or_else(|| format!("no guest heap for the {class} virtual table"))?;
        let vptr = base + 8;
        for (i, slot) in slots.iter().enumerate() {
            let name: String = slot.map(String::from).unwrap_or_else(|| format!("{class}::vslot{i}"));
            let addr = self.engine_stub(&name)?;
            self.mem.vm.write_u32(vptr + i as u32 * 4, addr).map_err(|_| format!("{class} virtual table is unwritable"))?;
        }
        Ok(vptr)
    }
}

/// Convenience for a table whose first `named.len()` slots are known and the rest (up to `total`) are not.
pub fn slots_with_gaps<'a>(total: usize, named: &[(usize, &'a str)]) -> Vec<Option<&'a str>> {
    let mut v = alloc::vec![None; total];
    for &(i, n) in named {
        if i < total {
            v[i] = Some(n);
        }
    }
    v
}

impl Process {
    /// Diagnostic: the first `n` slots of the vtable of the guest object at `obj`, each as an address and, for a stub, its name.
    pub fn describe_vtable(&self, obj: u32, n: u32) -> String {
        let vptr = self.mem.vm.read_u32(obj).unwrap_or(0);
        let mut s = format!("obj {obj:#x} vptr {vptr:#x}");
        for i in 0..n {
            let v = self.mem.vm.read_u32(vptr.wrapping_add(i * 4)).unwrap_or(0);
            let name = if v >= STUB_BASE && v & 7 == 0 { self.stubs.get(((v - STUB_BASE) / 8) as usize).and_then(|x| x.name.clone()).unwrap_or_default() } else { String::new() };
            s.push_str(&format!("\n  [{i}] {v:#x} {name}"));
        }
        s
    }
}
