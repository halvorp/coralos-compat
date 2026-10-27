//! Reading CDL instance data out of an SDK layout DLL (a PE image the S60 emulator builds: `10283389.dll` is the 360x640 touch
//! layout). The S60 5th Edition SDK's DLL IS the platform's own data: the Symbian Foundation source tree is a later revision (its
//! byte-coded blobs and its API tables differ), so the byte-coded layout lines and `KDataLookup` are taken from the DLL, and the
//! source tree only names things (instances, interfaces, packs).
//!
//! The structure is the CDL ECOM plug-in's (uiresources_plat/cdl_api/inc/CdlEngDef.h): export 1 is `ImplementationGroupProxy`,
//! whose first entry's factory returns an `SCdlMain { major, minor, TCdlArray<SCdlCustomisation>* }`; a customisation is
//! `{ id, SCdlInterface*, SCdlImpl* }` and an `SCdlImpl` is one function pointer per api id. A decode instance's function is a
//! thunk `push &KImplData; call AknLayoutScalableDecode::<op>`, and `KImplData = { lookup*, byte-coded data* }`.
use alloc::string::String;
use alloc::format;
use alloc::vec::Vec;

/// One customisation of the DLL.
#[derive(Debug, Clone)]
pub struct Customisation {
    pub id: u32,
    pub uid: u32,
    pub api_size: u32,
    /// The address of the function table (one pointer per api id).
    pub impl_va: u32,
}

pub struct Pe<'a> {
    b: &'a [u8],
    base: u32,
    /// (virtual address, virtual size, raw pointer, raw size)
    secs: Vec<(u32, u32, u32, u32)>,
}

fn rd32(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

impl<'a> Pe<'a> {
    pub fn parse(b: &'a [u8]) -> Result<Pe<'a>, String> {
        let bad = || String::from("not a PE image");
        let pe = rd32(b, 0x3c).ok_or_else(bad)? as usize;
        if b.get(pe..pe + 4) != Some(b"PE\0\0") {
            return Err(bad());
        }
        let n = u16::from_le_bytes([b[pe + 6], b[pe + 7]]) as usize;
        let opt = u16::from_le_bytes([b[pe + 20], b[pe + 21]]) as usize;
        let base = rd32(b, pe + 24 + 28).ok_or_else(bad)?;
        let mut secs = Vec::new();
        let mut o = pe + 24 + opt;
        for _ in 0..n {
            let (vs, va, rs, rp) = (rd32(b, o + 8).ok_or_else(bad)?, rd32(b, o + 12).ok_or_else(bad)?, rd32(b, o + 16).ok_or_else(bad)?, rd32(b, o + 20).ok_or_else(bad)?);
            secs.push((base + va, vs.max(rs), rp, rs));
            o += 40;
        }
        Ok(Pe { b, base, secs })
    }

    pub fn base(&self) -> u32 {
        self.base
    }

    /// The file offset of virtual address `va` (None = outside every section's raw data).
    pub fn off(&self, va: u32) -> Option<usize> {
        self.secs.iter().find(|(sva, vs, _, rs)| va >= *sva && va - sva < (*vs).min(*rs)).map(|(sva, _, rp, _)| (*rp + (va - sva)) as usize)
    }

    pub fn u32_at(&self, va: u32) -> Option<u32> {
        rd32(self.b, self.off(va)?)
    }

    pub fn bytes(&self, va: u32, n: usize) -> Option<&'a [u8]> {
        let o = self.off(va)?;
        self.b.get(o..o + n)
    }

    /// The section containing `va`: (its virtual address, its raw bytes).
    pub fn section_of(&self, va: u32) -> Option<(u32, &'a [u8])> {
        let (sva, _, rp, rs) = *self.secs.iter().find(|(sva, vs, _, rs)| va >= *sva && va - sva < (*vs).min(*rs))?;
        self.b.get(rp as usize..(rp + rs) as usize).map(|s| (sva, s))
    }

    /// `mov eax, imm32` in the first bytes of the function at `va`: the address it returns.
    fn returned_address(&self, va: u32) -> Option<u32> {
        let code = self.bytes(va, 40)?;
        let i = code.iter().position(|&c| c == 0xb8)?;
        rd32(code, i + 1)
    }

    /// Export 1's table -> the factory -> the `SCdlMain` -> its customisations.
    pub fn customisations(&self) -> Result<Vec<Customisation>, String> {
        let pe = rd32(self.b, 0x3c).ok_or("no PE header")? as usize;
        let exp_rva = rd32(self.b, pe + 24 + 96).ok_or("no export directory")?;
        let eo = self.off(self.base + exp_rva).ok_or("export directory outside the image")?;
        let aof = rd32(self.b, eo + 28).ok_or("export directory")?;
        let first = rd32(self.b, self.off(self.base + aof).ok_or("export table")?).ok_or("export table")?;
        let proxy_table = self.returned_address(self.base + first).ok_or("export 1 does not return the ECOM implementation table")?;
        let factory = self.u32_at(proxy_table + 4).ok_or("ECOM table")?;
        let main = self.returned_address(factory).ok_or("the CDL factory does not return an SCdlMain")?;
        let data = self.u32_at(main + 8).ok_or("SCdlMain")?;
        let (arr, count) = (self.u32_at(data).ok_or("TCdlArray")?, self.u32_at(data + 4).ok_or("TCdlArray")?);
        let mut out = Vec::new();
        for i in 0..count {
            let at = arr + i * 12;
            let (id, iface, imp) = (self.u32_at(at).ok_or("customisation")?, self.u32_at(at + 4).ok_or("customisation")?, self.u32_at(at + 8).ok_or("customisation")?);
            out.push(Customisation { id, uid: self.u32_at(iface + 12).ok_or("SCdlInterface")?, api_size: self.u32_at(iface + 28).ok_or("SCdlInterface")?, impl_va: imp });
        }
        Ok(out)
    }

    /// The function pointer of api `api` of `c`.
    pub fn thunk(&self, c: &Customisation, api: u32) -> Option<u32> {
        self.u32_at(c.impl_va + api * 4)
    }

    /// `&KImplData` pushed by a decode thunk (`push imm32` among its first bytes), then its lookup and data pointers.
    pub fn impl_data(&self, thunk_va: u32) -> Result<(u32, u32), String> {
        let code = self.bytes(thunk_va, 40).ok_or_else(|| format!("thunk {thunk_va:#x} outside the image"))?;
        let at = (0..code.len() - 5).find(|&i| code[i] == 0x68 && rd32(code, i + 1).is_some_and(|a| self.off(a).is_some())).ok_or_else(|| format!("thunk {thunk_va:#x} pushes no data address"))?;
        let kimpl = rd32(code, at + 1).unwrap_or(0);
        Ok((self.u32_at(kimpl).ok_or("KImplData")?, self.u32_at(kimpl + 4).ok_or("KImplData")?))
    }
}
