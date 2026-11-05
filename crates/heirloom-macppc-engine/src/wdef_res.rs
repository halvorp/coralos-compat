//! A program's own window definition as a WDEF RESOURCE of PowerPC code, chosen by a `procID` (`NewWindow` / `NewCWindow` / `GetNewWindow` /
//! `GetNewCWindow`, `procID = 16 * WDEF id + variation`). Such a resource is an ACCELERATED code resource (Mac OS Runtime Architectures,
//! "Accelerated resources"; MixedMode.h): a RoutineDescriptor (`$AAFE`, version 7) whose PowerPC record is kProcDescriptorIsRelative |
//! kFragmentNeedsPreparing, its procDescriptor the offset of a PEF container in the same resource. On first use the Mixed Mode Manager has the
//! Code Fragment Manager prepare that container and rewrites the record to the fragment's main address (absolute, prepared); this engine does
//! the same at window creation: the container is instantiated as a fragment named `WDEF <id>` (its code section recorded for identities like
//! every other), the record in the program's resource handle is rewritten, and the definition then goes through the SAME binding rule as a
//! procedure definition ([`crate::wdef_c`], coralos doctrine HEIRLOOM-CUSTOM-WINDOW-NEEDS-A-BINDING): its identity is what its code IS (code
//! section length + FNV-1a 64 + entry offset), a binding (DATA, `bindings/macppc/*.sexp`) names what its window means, and the definition's
//! own code runs for the regions and hit tests while the frame on screen is the native one. Unbound -> `[WDEF-UNSUPPORTED]` naming the
//! identity, and the 68k Window Manager's refusal stands (the call answers NIL, as before).
//!
//! Messages at creation, in the Mac OS 8.5 Window Manager's order this engine takes for every bound definition: GetFeatures, wNew, wCalcRgns,
//! GetRegion (only when the features offer it), then wDraw and ShowWindow when the program asked for a visible window. A WDEF resource of 68k
//! code (no descriptor, or a 68k record) is not handled here: the 68k binding table decides it ([`heirloom_mac68k_engine::wdef_binding`]).
use crate::cpu::Cpu;
use crate::machine::MacPpc;
use crate::wdef_c::{BoundWindow, Msg, Then, REGION_CONTENT, REGION_STRUCTURE, W_CALC_RGNS, W_DRAW, W_NEW, WR_DEF_PROC};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_mac68k_engine::pascal::{read_pstring, read_rect, Rect};
use heirloom_mac68k_engine::toolbox::WindowSpec;
use heirloom_ui_abi::GuestUi;

/// RoutineDescriptor facts (MixedMode.h): `_MixedModeMagic`, the version this engine reads, the PowerPC ISA, and the routine flags.
const MIXED_MODE_MAGIC: u16 = 0xAAFE;
const RD_VERSION: u8 = 7;
const ISA_POWERPC: u8 = 1;
const PROC_DESCRIPTOR_IS_RELATIVE: u16 = 0x1;
const FRAGMENT_NEEDS_PREPARING: u16 = 0x2;
/// RoutineDescriptor header (12 bytes) then RoutineRecords (20 bytes: procInfo 0, reserved1 4, ISA 5, routineFlags 6, procDescriptor 8, ...).
const RD_HEADER: usize = 12;
const RR_SIZE: usize = 20;

/// Where an accelerated resource's PowerPC code is: (record offset in the resource, the PEF container's offset, its flags).
pub fn accelerated_ppc_record(b: &[u8]) -> Option<(usize, usize, u16)> {
    if b.len() < RD_HEADER || u16::from_be_bytes([b[0], b[1]]) != MIXED_MODE_MAGIC || b[2] != RD_VERSION { return None; }
    let count = usize::from(u16::from_be_bytes([b[10], b[11]])) + 1;
    (0..count).map(|i| RD_HEADER + RR_SIZE * i).filter(|&r| r + RR_SIZE <= b.len()).find_map(|r| {
        let flags = u16::from_be_bytes([b[r + 6], b[r + 7]]);
        let pd = u32::from_be_bytes([b[r + 8], b[r + 9], b[r + 10], b[r + 11]]) as usize;
        (b[r + 5] == ISA_POWERPC && flags & PROC_DESCRIPTOR_IS_RELATIVE != 0).then_some((r, pd, flags))
    })
}

impl<U: GuestUi> MacPpc<U> {
    /// The creation calls (hooked by the run loop for a PowerPC program): true = the window is being made from a bound PowerPC WDEF
    /// resource (a callback was entered); false = not that case (a system procID, a 68k or absent WDEF, an unbound definition): the 68k
    /// Window Manager's call proceeds as before.
    pub(crate) fn open_resource_window(&mut self, cpu: &mut Cpu, name: &str) -> Result<bool, String> {
        let a: Vec<u32> = cpu.gpr[3..11].to_vec();
        let err = |e: heirloom_mac68k_engine::Refusal| format!("{name}: {}", e.0);
        let (spec, color) = match name {
            "NewWindow" | "NewCWindow" => (WindowSpec {
                storage: a[0], rect: read_rect(&self.mac.mem, a[1]).map_err(err)?, title: read_pstring(&self.mac.mem, a[2]).map_err(err)?, visible: a[3] & 0xFF != 0,
                proc_id: a[4] as u16, behind: a[5], go_away: a[6] & 0xFF != 0, ref_con: a[7],
            }, name == "NewCWindow"),
            _ => {
                let id = a[0] as u16 as i16;
                let Ok(h) = self.mac.get_resource(*b"WIND", id, false) else { return Ok(false) };
                let b = self.mac.handle_bytes(h).unwrap_or_default();
                if b.len() < 19 || b.len() < 19 + usize::from(b[18]) { return Ok(false); }
                let be = |i: usize| i16::from_be_bytes([b[i], b[i + 1]]);
                (WindowSpec {
                    storage: a[1], rect: Rect { top: be(0), left: be(2), bottom: be(4), right: be(6) },
                    title: heirloom_mac68k_engine::macroman::decode(&b[19..19 + usize::from(b[18])]), proc_id: be(8) as u16, visible: be(10) != 0,
                    go_away: be(12) != 0, ref_con: u32::from_be_bytes([b[14], b[15], b[16], b[17]]), behind: a[2],
                }, name == "GetNewCWindow")
            }
        };
        let wdef_id = (spec.proc_id >> 4) as i16;
        if wdef_id <= 1 { return Ok(false); }
        let Some((handle, tv)) = self.prepare_accelerated_wdef(wdef_id)? else { return Ok(false) };
        let (tv, identity, frag) = self.tvector_identity(tv)?;
        let b = match heirloom_bind::wdef::find(&self.wdef.bindings, &identity) {
            Ok(b) => b.clone(),
            Err(why) => {
                self.mac.log.push(format!("[WDEF-UNSUPPORTED] {name} procID {}: the definition in {frag} {identity}: {why}", spec.proc_id));
                return Ok(false);
            }
        };
        let show = spec.visible;
        let var_code = u32::from(spec.proc_id & 15);
        let title = spec.title.clone();
        let win = self.mac.make_window_bound_as(WindowSpec { visible: false, ..spec }, color, (b.meaning.role, b.meaning.resizable)).map_err(err)?;
        let scratch = self.mac.mm.new_ptr(&mut self.mac.mem, &mut self.mac.heap, 16, true).map_err(|e| format!("{name}: no memory (memory error {e})"))?;
        self.put32(win + WR_DEF_PROC, handle, name)?;
        self.mac.log.push(format!("[WDEF-BOUND binding={} window={win:#x}] {name} \"{title}\" procID {} (WDEF {wdef_id} variation {var_code}): the definition in {frag} {identity} means {:?}; its own code runs for regions and hits, the frame is native",
            b.id, spec.proc_id, b.meaning));
        self.wdef.bound.insert(win, BoundWindow { binding: b.id.clone(), tv, def_handle: handle, frame_draws: 0, features: 0, scratch, var_code, owns_handle: false });
        let mut msgs = alloc::vec![Msg::Features, Msg::Plain(W_NEW), Msg::Plain(W_CALC_RGNS), Msg::Region(REGION_STRUCTURE), Msg::Region(REGION_CONTENT)];
        if show { msgs.push(Msg::Plain(W_DRAW)); }
        self.begin_wdef(cpu, win, &msgs, Then::Opened { show })?;
        Ok(true)
    }

    /// WDEF `id` from the resource chain, when it is an accelerated resource of PowerPC code: its handle and the TVector of its definition,
    /// preparing it first if Mixed Mode has not (the record is then rewritten absolute + prepared, as the Mixed Mode Manager leaves it).
    /// None for a WDEF that is absent or 68k code.
    pub(crate) fn prepare_accelerated_wdef(&mut self, id: i16) -> Result<Option<(u32, u32)>, String> {
        let Ok(h) = self.mac.get_resource(*b"WDEF", id, false) else { return Ok(None) };
        let Some(bytes) = self.mac.handle_bytes(h) else { return Ok(None) };
        let ptr = self.mac.mm.block(h).map(|b| b.ptr).ok_or_else(|| format!("WDEF {id}: the resource handle has no block"))?;
        // an already-prepared record (absolute) names the TVector itself
        if u16::from_be_bytes([bytes.first().copied().unwrap_or(0), bytes.get(1).copied().unwrap_or(0)]) == MIXED_MODE_MAGIC && bytes.len() >= RD_HEADER + RR_SIZE {
            let r = RD_HEADER;
            let flags = u16::from_be_bytes([bytes[r + 6], bytes[r + 7]]);
            if bytes[r + 5] == ISA_POWERPC && flags & (PROC_DESCRIPTOR_IS_RELATIVE | FRAGMENT_NEEDS_PREPARING) == 0 {
                return Ok(Some((h, u32::from_be_bytes([bytes[r + 8], bytes[r + 9], bytes[r + 10], bytes[r + 11]]))));
            }
        }
        let Some((rec, pef_at, flags)) = accelerated_ppc_record(&bytes) else { return Ok(None) };
        let pef = bytes.get(pef_at..).filter(|p| p.starts_with(b"Joy!peff")).ok_or_else(|| format!("WDEF {id}: its PowerPC record names offset {pef_at:#x}, where there is no PEF container"))?;
        let lib = format!("WDEF {id}");
        self.provide_library(&lib, pef);
        self.ensure_loaded(&lib);
        let main = match self.libs.get(lib.as_str()) {
            Some(crate::machine::LibSlot::Loaded(_, Some(main))) => *main,
            Some(crate::machine::LibSlot::Failed(why)) => return Err(format!("WDEF {id}: its PEF container does not prepare ({why})")),
            _ => return Err(format!("WDEF {id}: its PEF container has no main symbol (the definition's entry)")),
        };
        let new_flags = flags & !(PROC_DESCRIPTOR_IS_RELATIVE | FRAGMENT_NEEDS_PREPARING);
        self.mac.mem.write_u16(ptr + rec as u32 + 6, new_flags).map_err(|e| format!("WDEF {id}: {e}"))?;
        self.put32(ptr + rec as u32 + 8, main, "WDEF")?;
        self.mac.log.push(format!("[cfm] WDEF {id}: accelerated resource prepared ({} byte PEF container at +{pef_at:#x}) -> main {main:#x}; its routine record is now absolute", pef.len()));
        Ok(Some((h, main)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accelerated_records_are_found_by_isa_and_flags() {
        let mut rd = alloc::vec![0xAA, 0xFE, 7, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        rd.extend_from_slice(&[0, 0, 0x3B, 0xB0, 0, 1, 0, 7, 0, 0, 0, 0x20, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(accelerated_ppc_record(&rd), Some((12, 0x20, 7)));
        // NEGATIVES: 68k code (no descriptor), a 68k record, an absolute record, another descriptor version
        assert_eq!(accelerated_ppc_record(b"\x4e\x56\x00\x00 68k code ......"), None);
        let mut m68k = rd.clone(); m68k[12 + 5] = 0; assert_eq!(accelerated_ppc_record(&m68k), None);
        let mut abs = rd.clone(); abs[12 + 7] = 4; assert_eq!(accelerated_ppc_record(&abs), None);
        let mut v6 = rd; v6[2] = 6; assert_eq!(accelerated_ppc_record(&v6), None);
    }

    use crate::wdef_c::{CodeSection, W_GET_FEATURES, W_HIT};
    use heirloom_bind::{WindowDefBinding, WindowDefIdentity, WindowMeaning};
    use heirloom_ui_abi::{RecordingGuestUi, WindowRole};
    type M = MacPpc<RecordingGuestUi>;
    const SP: u32 = 0x0007_0000;
    const R: u32 = 0x4000;
    const SEC: u32 = 0x0003_0000;
    const CODE: u32 = SEC + 0x40;
    const TV: u32 = 0x0003_8000;
    const DIGEST: u64 = 0x0bad_cafe_f00d_0001;

    /// A machine whose resource chain holds WDEF 200 = a PREPARED accelerated resource (absolute PowerPC record -> TV, code in a recorded
    /// section) and, when `m68k`, a 68k WDEF 201 instead; a binding for the section when `bound`.
    fn setup(bound: bool) -> M {
        let mut m: M = MacPpc::new(RecordingGuestUi::new()).unwrap();
        m.implicit_graf();
        m.toolbox_call("InitWindows", &[]);
        m.mac.mem.write_u32(TV, CODE).unwrap();
        m.mac.mem.write_u32(TV + 4, 0x0003_9000).unwrap();
        m.wdef.code.push(CodeSection { fragment: "WDEF 200".into(), base: SEC, len: 0x100, fnv1a64: DIGEST });
        let mut rd = alloc::vec![0xAA, 0xFE, 7, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        rd.extend_from_slice(&[0, 0, 0x3B, 0xB0, 0, 1, 0, 4]);
        rd.extend_from_slice(&TV.to_be_bytes());
        rd.extend_from_slice(&[0; 8]);
        let fork = heirloom_fmt_resfork::writer::ForkBuilder::new().add(b"WDEF", 200, None, &rd).add(b"WDEF", 201, None, b"\x4E\x75 68k code").build();
        m.mac.rm.open_fork(heirloom_mac68k_engine::resmgr::APP_REF_NUM, heirloom_fmt_resfork::ResFork::parse(fork).unwrap());
        if bound {
            m.add_window_def_binding(WindowDefBinding { id: "test-frameless-1".into(), identity: WindowDefIdentity::FragmentCode { code_length: 0x100, code_fnv1a64: DIGEST, entry_offset: 0x40 },
                meaning: WindowMeaning { role: WindowRole::Utility, titled: false, movable: false, closable: false, resizable: false } });
        }
        m
    }

    /// NewCWindow(storage 0, &bounds, "\p", visible, procID, behind -1, goAway, refCon) as the run loop hands it over.
    fn new_c_window(m: &mut M, proc_id: u32, visible: bool) -> (Cpu, bool) {
        for (i, v) in [40i16, 40, 140, 240].iter().enumerate() { m.mac.mem.write_u16(R + 2 * i as u32, *v as u16).unwrap(); }
        m.mac.mem.write_u8(R + 16, 0).unwrap();
        let mut cpu = Cpu::new(0x0005_0000, SP);
        cpu.lr = 0x0005_1230;
        for (i, a) in [0, R, R + 16, u32::from(visible), proc_id, u32::MAX, 0, 0x1234].iter().enumerate() { cpu.gpr[3 + i] = *a; }
        let entered = m.open_resource_window(&mut cpu, "NewCWindow").unwrap();
        (cpu, entered)
    }

    #[test]
    fn a_bound_wdef_resource_gets_its_messages_with_its_variation_and_the_window_is_returned() {
        let mut m = setup(true);
        let (mut cpu, entered) = new_c_window(&mut m, 200 * 16 + 3, true);
        assert!(entered);
        let mut seen = Vec::new();
        while cpu.pc == CODE {
            assert_eq!((cpu.gpr[3], cpu.gpr[2]), (3, 0x0003_9000), "the procID's variation, the definition's TOC");
            seen.push(cpu.gpr[5]);
            let win = cpu.gpr[4];
            if cpu.gpr[5] == W_CALC_RGNS {
                // a frameless definition: structure = content = the port rectangle, global
                for off in [heirloom_mac68k_engine::winregions::WR_STRUC_RGN, heirloom_mac68k_engine::winregions::WR_CONT_RGN] {
                    let h = m.mac.mem.read_u32(win + off).unwrap();
                    m.mac.write_region(h, &heirloom_raster::Region::from_rect(heirloom_raster::Rect::new(40, 40, 240, 140))).unwrap();
                }
            }
            cpu.gpr[3] = 0;
            m.callback_returned(&mut cpu).unwrap();
        }
        assert_eq!(seen, [W_GET_FEATURES, W_NEW, W_CALC_RGNS, W_DRAW], "no GetRegion (features 0), Draw before the window is shown");
        let win = cpu.gpr[3];
        assert!(m.mac.windows.get(&win).is_some_and(|w| w.visible && w.native.is_some()), "the window is returned and shown");
        assert_eq!(cpu.pc, 0x0005_1230, "back to the caller");
        assert_eq!(m.mac.mem.read_u32(win + WR_DEF_PROC).unwrap(), m.mac.get_resource(*b"WDEF", 200, false).unwrap(), "windowDefProc is the WDEF resource handle");
        assert!(m.mac.log.iter().any(|l| l.contains("[WDEF-BOUND binding=test-frameless-1") && l.contains("variation 3")));
        let _ = W_HIT;
    }

    #[test]
    fn unbound_or_68k_wdef_resources_are_left_to_the_window_manager_refusal() {
        // NEGATIVE: the same definition with no binding is not admitted: named with its identity, nothing made, the 68k path refuses as before
        let mut m = setup(false);
        let before = m.mac.windows.len();
        let (_, entered) = new_c_window(&mut m, 200 * 16, false);
        assert!(!entered);
        assert_eq!(m.mac.windows.len(), before);
        assert!(m.mac.log.iter().any(|l| l.contains("[WDEF-UNSUPPORTED] NewCWindow procID 3200") && l.contains("(entry-offset 64)")), "{:?}", m.mac.log);
        // NEGATIVE: a 68k WDEF resource and a system procID are not this path's
        let mut m = setup(true);
        assert!(!new_c_window(&mut m, 201 * 16, false).1);
        assert!(!new_c_window(&mut m, 0, false).1);
    }
}
