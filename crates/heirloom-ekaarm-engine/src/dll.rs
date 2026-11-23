//! Generic guest E32 companion DLL loader.
//!
//! Symbian applications frequently ship application-private companion DLLs alongside the main executable in
//! `sys\bin\<name>.dll`. Rather than requiring host bridges for application-specific libraries, any imported DLL
//! whose binary exists in the application's own drive tree (`sys\bin\<dll_base>.dll`, excluding system ROM DLLs)
//! is parsed as a standard E32 image, relocated into the guest DLL address window, linked against the ordinal table
//! (and transitively against any further companion DLLs), and bound directly into every caller's import sites.
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_fmt_e32::{E32Image, OrdinalTable};
use heirloom_mem_paged32::Prot;
use crate::layout::{self, DATA_BASE, PAGE, STACK_TOP};
use crate::loader::Process;
use crate::stubs::{self, ImportSite, Stub};

struct LoadedDll {
    exports: Vec<u32>,
}

/// Resolve and load any guest companion E32 DLLs present on `p.host.drive` (`sys\bin\<dll_base>.dll`) that are
/// referenced by unresolved imports in `p.stubs`. Returns the number of guest DLLs loaded.
pub fn resolve_guest_dlls(p: &mut Process, table: &OrdinalTable) -> Result<usize, String> {
    if p.host.drive.is_none() {
        return Ok(0);
    }
    let mut loaded: BTreeMap<String, LoadedDll> = BTreeMap::new();
    let mut skipped: alloc::collections::BTreeSet<String> = alloc::collections::BTreeSet::new();

    loop {
        // Find the next distinct guest DLL referenced by an unresolved import whose image exists in the package tree.
        let mut candidate: Option<(String, Vec<u8>)> = None;
        for stub in &p.stubs {
            if stub.site.is_none() || stub.name.is_some() {
                continue;
            }
            let key = stub.dll_base.to_ascii_lowercase();
            if loaded.contains_key(&key) || skipped.contains(&key) {
                continue;
            }
            let path = format!("sys\\bin\\{key}.dll");
            let bytes = p.host.drive.as_ref().and_then(|d| d.read(&path).ok()).map(|b| b.to_vec());
            match bytes {
                Some(b) => {
                    candidate = Some((key, b));
                    break;
                }
                None => {
                    skipped.insert(key);
                }
            }
        }
        let Some((key, bytes)) = candidate else { break };
        let dll = load_one_dll(p, &key, &bytes, table)?;
        loaded.insert(key, dll);
    }

    if loaded.is_empty() {
        return Ok(0);
    }

    // Rewrite every import site (and fallback stub) targeting a loaded guest DLL to point directly at the relocated export.
    let mut patches: Vec<(usize, ImportSite, u32)> = Vec::new();
    for (idx, stub) in p.stubs.iter().enumerate() {
        let Some(site) = stub.site else { continue };
        let key = stub.dll_base.to_ascii_lowercase();
        let Some(dll) = loaded.get(&key) else { continue };
        if stub.ordinal == 0 {
            return Err(format!("guest DLL {key}: import has ordinal 0"));
        }
        let export_idx = (stub.ordinal as usize) - 1;
        let target = dll.exports.get(export_idx).copied().unwrap_or(0);
        if target == 0 {
            return Err(format!("guest DLL {key}: ordinal {} is absent (exports={})", stub.ordinal, dll.exports.len()));
        }
        patches.push((idx, site, target));
    }

    // Group site writes by chunk so each code chunk is unprotected once.
    let mut by_chunk: BTreeMap<heirloom_mem_paged32::ChunkId, Vec<(u32, u32)>> = BTreeMap::new();
    for &(_, site, target) in &patches {
        let patched = target.wrapping_add(u32::from(site.addend));
        by_chunk.entry(site.chunk).or_default().push((site.addr, patched));
    }
    for (chunk, writes) in by_chunk {
        p.mem.vm.set_prot(chunk, Prot::RW).map_err(|e| format!("guest DLL patch unprotect: {e:?}"))?;
        let mut res = Ok(());
        for (addr, word) in writes {
            if let Err(e) = p.mem.vm.write_u32(addr, word) {
                res = Err(format!("guest DLL patch write at {addr:#x}: {e:?}"));
                break;
            }
        }
        p.mem.vm.set_prot(chunk, Prot::RX).map_err(|e| format!("guest DLL patch protect: {e:?}"))?;
        res?;
    }

    // Mark resolved stubs and overwrite their fallback stub slots with an interworking branch (`LDR PC, [PC, #-4]; .word target`).
    p.mem.vm.set_prot(p.stub_chunk_id, Prot::RW).map_err(|e| format!("stub unprotect: {e:?}"))?;
    for &(idx, _, target) in &patches {
        let mut tramp = [0u8; 8];
        tramp[..4].copy_from_slice(&0xE51F_F004u32.to_le_bytes());
        tramp[4..].copy_from_slice(&target.to_le_bytes());
        let _ = p.mem.vm.write_bytes(stubs::stub_addr(idx), &tramp);
    }
    p.mem.vm.set_prot(p.stub_chunk_id, Prot::RX).map_err(|e| format!("stub protect: {e:?}"))?;

    Ok(loaded.len())
}

fn load_one_dll(p: &mut Process, key: &str, bytes: &[u8], table: &OrdinalTable) -> Result<LoadedDll, String> {
    let mut img = E32Image::parse(bytes).map_err(|e| format!("guest DLL {key}: parse failed: {e:?}"))?;
    let h = img.header.clone();
    let code_len = img.code.len() as u32;
    let code_alloc = layout::pru(code_len).max(PAGE);
    let data_len = img.data.len() as u32;
    let bss_len = h.bss_size;
    let data_total = data_len.saturating_add(bss_len);
    let data_alloc = if data_total > 0 { layout::pru(data_total) } else { 0 };

    let code_at = p.dll_next_code;
    let data_at = p.dll_next_data;
    if code_at.saturating_add(code_alloc) > DATA_BASE {
        return Err(format!("guest DLL {key}: code window exhausted at {code_at:#x} (+{code_alloc:#x})"));
    }
    if data_alloc > 0 && data_at.saturating_add(data_alloc) > STACK_TOP - 0x0400_0000 {
        return Err(format!("guest DLL {key}: data window exhausted at {data_at:#x} (+{data_alloc:#x})"));
    }

    img.relocate(code_at, data_at).map_err(|e| format!("guest DLL {key}: relocate failed: {e:?}"))?;

    let code_delta = code_at.wrapping_sub(h.code_base);
    let data_delta = data_at.wrapping_sub(h.data_base);
    let exports: Vec<u32> = img.exports.iter().map(|&raw| {
        if raw == 0 {
            return 0;
        }
        let aligned = raw & !1;
        if aligned.wrapping_sub(h.code_base) < h.code_size {
            raw.wrapping_add(code_delta)
        } else {
            raw.wrapping_add(data_delta)
        }
    }).collect();

    let code_id = p.mem.vm.create_chunk(code_at, code_alloc, Prot::RW)
        .map_err(|e| format!("guest DLL {key}: code create: {e:?}"))?;
    p.mem.vm.commit(code_id, 0, code_alloc)
        .map_err(|e| format!("guest DLL {key}: code commit: {e:?}"))?;
    p.mem.vm.write_bytes(code_at, &img.code)
        .map_err(|e| format!("guest DLL {key}: code write: {e:?}"))?;

    if data_alloc > 0 {
        let data_id = p.mem.vm.create_chunk(data_at, data_alloc, Prot::RW)
            .map_err(|e| format!("guest DLL {key}: data create: {e:?}"))?;
        p.mem.vm.commit(data_id, 0, data_alloc)
            .map_err(|e| format!("guest DLL {key}: data commit: {e:?}"))?;
        if data_len > 0 {
            p.mem.vm.write_bytes(data_at, &img.data)
                .map_err(|e| format!("guest DLL {key}: data write: {e:?}"))?;
        }
        if bss_len > 0 {
            p.mem.vm.write_bytes(data_at + data_len, &alloc::vec![0u8; bss_len as usize])
                .map_err(|e| format!("guest DLL {key}: bss zero: {e:?}"))?;
        }
    }

    // Bind the DLL's own imports into the shared stub table and patch its code section.
    let needed: usize = img.imports.iter().map(|d| d.imports.len()).sum();
    if p.stubs.len() + needed > p.stub_slots_capacity as usize {
        return Err(format!("guest DLL {key}: stub table capacity {} exceeded (have {}, need +{needed})", p.stub_slots_capacity, p.stubs.len()));
    }
    p.mem.vm.set_prot(p.stub_chunk_id, Prot::RW).map_err(|e| format!("guest DLL {key}: stub unprotect: {e:?}"))?;
    let mut stub_write_err: Option<String> = None;
    for imp_dll in &img.imports {
        let imp_base = stubs::dll_base(&imp_dll.name);
        for imp in &imp_dll.imports {
            let idx = p.stubs.len();
            if let Err(e) = p.mem.vm.write_bytes(stubs::stub_addr(idx), &stubs::encode_stub(idx)) {
                stub_write_err = Some(format!("guest DLL {key}: stub write: {e:?}"));
                break;
            }
            let site_addr = code_at.wrapping_add(imp.site);
            if let Err(e) = p.mem.vm.write_u32(site_addr, stubs::stub_addr(idx).wrapping_add(imp.addend as u32)) {
                stub_write_err = Some(format!("guest DLL {key}: import site patch at {site_addr:#x}: {e:?}"));
                break;
            }
            p.stubs.push(Stub {
                dll_base: imp_base.clone(),
                dll: imp_dll.name.clone(),
                ordinal: imp.ordinal,
                name: table.name_of(&imp_dll.name, imp.ordinal).map(String::from),
                site: Some(ImportSite { chunk: code_id, addr: site_addr, addend: imp.addend }),
            });
        }
        if stub_write_err.is_some() {
            break;
        }
    }
    p.mem.vm.set_prot(p.stub_chunk_id, Prot::RX).map_err(|e| format!("guest DLL {key}: stub protect: {e:?}"))?;
    if let Some(e) = stub_write_err {
        return Err(e);
    }

    p.mem.vm.set_prot(code_id, Prot::RX).map_err(|e| format!("guest DLL {key}: code protect: {e:?}"))?;
    p.dll_code_ranges.push((code_at, code_len));
    if (h.exception_descriptor & 1) != 0 {
        p.exception_descriptors.push(code_at.wrapping_add(h.exception_descriptor & !1));
    }
    p.dll_next_code = code_at.saturating_add(code_alloc);
    p.dll_next_data = data_at.saturating_add(data_alloc);
    p.log.push(format!("[dll] loaded {key}.dll at code={code_at:#x} (+{code_len:#x}) data={data_at:#x} (+{data_total:#x}) exports={} imports={needed}", exports.len()));
    Ok(LoadedDll { exports })
}
