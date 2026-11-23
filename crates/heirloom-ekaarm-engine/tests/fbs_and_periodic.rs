//! Unit tests and negative controls for Symbian heap growth, MBM decoding,
//! `CFbsBitGc` / `CFbsBitmap` / `CFont` graphics, `CPeriodic` virtual-clock timers,
//! `RFile` I/O, and `CMdaAudioOutputStream` audio output.
use heirloom_ekaarm_engine::cpu::Memory;
use heirloom_ekaarm_engine::fbs::{decode_mbm, FbsBitmap, ECOLOR_16MU};
use heirloom_ekaarm_engine::layout::{CODE_BASE, EXIT_SVC, STUB_SVC_BASE};
use heirloom_ekaarm_engine::{loader, Outcome};

/// Build a minimal valid Symbian `.mbm` multi-bitmap container with one bitmap entry.
fn build_mbm(w: u32, h: u32, bpp: u32, color: u32, comp: u32, payload: &[u8]) -> Vec<u8> {
    let hdr_size = 40u32;
    let bmp_off = 20u32;
    let bmp_size = hdr_size + payload.len() as u32;
    let trailer_off = bmp_off + bmp_size;
    let mut out = Vec::new();
    // File header (20 bytes): UID1, UID2, UID3, Checksum, TrailerOffset
    out.extend_from_slice(&0x1000_0037u32.to_le_bytes());
    out.extend_from_slice(&0x1000_0042u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&trailer_off.to_le_bytes());
    // Bitmap header (40 bytes)
    out.extend_from_slice(&bmp_size.to_le_bytes());
    out.extend_from_slice(&hdr_size.to_le_bytes());
    out.extend_from_slice(&w.to_le_bytes());
    out.extend_from_slice(&h.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // x twips
    out.extend_from_slice(&0u32.to_le_bytes()); // y twips
    out.extend_from_slice(&bpp.to_le_bytes());
    out.extend_from_slice(&color.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // palette size
    out.extend_from_slice(&comp.to_le_bytes());
    out.extend_from_slice(payload);
    // Trailer: count=1, offset[0]=bmp_off
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&bmp_off.to_le_bytes());
    out
}

#[test]
fn heap_grows_lazily_past_initial_commit_and_refuses_oom() {
    let mut p = loader::bare(&[]).unwrap();
    // loader::bare commits 4 KiB initially out of a larger heap_max; allocating 20 KiB must
    // grow lazily past the initial commit and produce writable guest memory.
    let addr = p.heap.as_mut().unwrap().alloc(20 * 1024, &mut p.mem)
        .expect("lazy growth past initial commit must succeed");
    p.mem.write_u32(addr + 16 * 1024, 0xDEAD_BEEF).expect("newly committed page must be writable");
    assert_eq!(p.mem.read_u32(addr + 16 * 1024).unwrap(), 0xDEAD_BEEF);

    // Negative control: allocating beyond heap_max (16 MiB) must return None cleanly.
    assert_eq!(p.heap.as_mut().unwrap().alloc(16 * 1024 * 1024, &mut p.mem), None);
}

#[test]
fn decode_mbm_uncompressed_and_rle_modes() {
    // 2x1 24-bit uncompressed BGR: red (0,0,255) and green (0,255,0), padded to 8-byte row
    let uncomp = [0x00, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0x00, 0x00];
    let mbm = build_mbm(2, 1, 24, 1, 0, &uncomp);
    let (w, h, is_gray, px) = decode_mbm(&mbm, 0).expect("uncompressed 24-bit MBM");
    assert_eq!((w, h, is_gray), (2, 1, false));
    assert_eq!(px, vec![0x00FF_0000, 0x0000_FF00]);

    // 4x1 8-bit grayscale with EByteRLECompression (comp=1): run of 2 x 0x80, literal 2 bytes [0x00, 0xFF]
    let rle8 = [1u8, 0x80, (-2i8) as u8, 0x00, 0xFF];
    let mbm8 = build_mbm(4, 1, 8, 0, 1, &rle8);
    let (w, h, is_gray, px) = decode_mbm(&mbm8, 0).expect("8-bit byte-RLE MBM");
    assert_eq!((w, h, is_gray), (4, 1, true));
    assert_eq!(px, vec![0x0080_8080, 0x0080_8080, 0x0000_0000, 0x00FF_FFFF]);

    // 2x1 24-bit ETwentyFourBitRLECompression (comp=4): run of 2 blue pixels (0xFF, 0x00, 0x00)
    let rle24 = [1u8, 0xFF, 0x00, 0x00];
    let mbm24 = build_mbm(2, 1, 24, 1, 4, &rle24);
    let (w, h, is_gray, px) = decode_mbm(&mbm24, 0).expect("24-bit RLE MBM");
    assert_eq!((w, h, is_gray), (2, 1, false));
    assert_eq!(px, vec![0x0000_00FF, 0x0000_00FF]);
}

#[test]
fn decode_mbm_negative_controls() {
    // Bad UID header
    let mut bad = build_mbm(1, 1, 24, 1, 0, &[0, 0, 0, 0]);
    bad[0] = 0xAA;
    assert!(decode_mbm(&bad, 0).is_err(), "corrupt MBM UID must be rejected");

    // Out-of-range bitmap index
    let valid = build_mbm(1, 1, 24, 1, 0, &[0, 0, 0, 0]);
    assert!(decode_mbm(&valid, 1).is_err(), "out-of-range MBM index must be rejected");

    // Truncated buffer
    assert!(decode_mbm(&valid[..12], 0).is_err(), "truncated MBM must be rejected");
}

#[test]
fn fbs_bitmap_bounds_and_pixels() {
    let mut bmp = FbsBitmap::new(4, 4, ECOLOR_16MU);
    assert_eq!(bmp.get(0, 0), Some(0x00FF_FFFF));
    bmp.set(2, 1, 0xFF12_3456);
    assert_eq!(bmp.get(2, 1), Some(0x0012_3456));
    // Negative control: out-of-bounds reads return None and writes do not panic.
    bmp.set(-1, 0, 0);
    bmp.set(4, 4, 0);
    assert_eq!(bmp.get(-1, 0), None);
    assert_eq!(bmp.get(4, 0), None);
}

fn arm_words(w: &[u32]) -> Vec<u8> {
    w.iter().flat_map(|x| x.to_le_bytes()).collect()
}

#[test]
fn cperiodic_rearms_ignoring_zero_return_and_tracks_deadlines() {
    let mut p = loader::bare(&[
        "_ZN9CPeriodic4NewLEi",                                            // stub 0
        "_ZN9CPeriodic5StartE27TTimeIntervalMicroSeconds32S0_9TCallBack",  // stub 1
        "_ZN16CActiveScheduler5StartEv",                                   // stub 2
        "_ZN7CActive6CancelEv",                                            // stub 3
    ]).unwrap();
    let counter_addr = p.heap.as_mut().unwrap().alloc(16, &mut p.mem).unwrap();
    p.mem.write_u32(counter_addr, 0).unwrap();

    // Callback at CODE_BASE + 0x1000: increment [r0] (counter_addr), return r0 = 0 (CPeriodic MUST still re-arm!)
    //   ldr r1, [r0]
    //   add r1, r1, #1
    //   str r1, [r0]
    //   mov r0, #0
    //   bx  lr
    let cb_addr = CODE_BASE + 0x1000;
    p.map_code(cb_addr, &arm_words(&[
        0xE590_1000, // ldr r1, [r0]
        0xE281_1001, // add r1, r1, #1
        0xE580_1000, // str r1, [r0]
        0xE3A0_0000, // mov r0, #0
        0xE12F_FF1E, // bx lr
    ])).unwrap();

    // Main at CODE_BASE:
    //   r0 = 0 (priority); SVC 0 (CPeriodic::NewL) -> r4 = periodic
    //   push {r5} (cb ptr = counter_addr at [sp])
    //   r0 = r4, r1 = 5000 (delay), r2 = 20000 (interval), r3 = cb_addr; SVC 1 (CPeriodic::Start)
    //   SVC 2 (CActiveScheduler::Start)
    //   SVC EXIT
    p.map_code(CODE_BASE, &arm_words(&[
        0xE3A0_0000,                   // mov r0, #0
        0xEF00_0000 | STUB_SVC_BASE,   // CPeriodic::NewL(0)
        0xE1A0_4000,                   // mov r4, r0
        0xE52D_5004,                   // str r5, [sp, #-4]! (push counter_addr as TCallBack.iPtr)
        0xE1A0_0004,                   // mov r0, r4
        0xE1A0_1006,                   // mov r1, r6 (delay=5000)
        0xE1A0_2007,                   // mov r2, r7 (interval=20000)
        0xE1A0_3008,                   // mov r3, r8 (cb_addr)
        0xEF00_0000 | (STUB_SVC_BASE + 1), // CPeriodic::Start
        0xEF00_0000 | (STUB_SVC_BASE + 2), // CActiveScheduler::Start
        0xEF00_0000 | EXIT_SVC,
    ])).unwrap();

    p.cpu.regs[5] = counter_addr;
    p.cpu.regs[6] = 5_000;
    p.cpu.regs[7] = 20_000;
    p.cpu.regs[8] = cb_addr;
    p.cpu.set_pc_interworking(CODE_BASE);

    // At t=0, delay=5000 hasn't elapsed: must block and advertise deadline 5000.
    assert!(matches!(p.run(10_000), Outcome::Blocked));
    assert_eq!(p.guest_word(counter_addr), Some(0));
    assert_eq!(p.next_timer_deadline(), Some(5_000));

    // Advance to 4999us: still not due (negative control).
    p.advance_clock_to(4_999).unwrap();
    assert!(matches!(p.run(10_000), Outcome::Blocked));
    assert_eq!(p.guest_word(counter_addr), Some(0));

    // Advance to 5000us: fires once, callback returns 0, MUST re-arm for 25_000us!
    p.advance_clock_to(5_000).unwrap();
    assert!(matches!(p.run(10_000), Outcome::Blocked));
    assert_eq!(p.guest_word(counter_addr), Some(1));
    assert_eq!(p.next_timer_deadline(), Some(25_000));

    // Advance to 25_000us: fires a second time, re-arms for 45_000us!
    p.advance_clock_to(25_000).unwrap();
    assert!(matches!(p.run(10_000), Outcome::Blocked));
    assert_eq!(p.guest_word(counter_addr), Some(2));
    assert_eq!(p.next_timer_deadline(), Some(45_000));
}

#[test]
fn cidle_stops_when_callback_returns_zero() {
    let mut p = loader::bare(&[
        "_ZN5CIdle4NewLEi",              // stub 0
        "_ZN5CIdle5StartE9TCallBack",    // stub 1
        "_ZN16CActiveScheduler5StartEv", // stub 2
    ]).unwrap();
    let counter_addr = p.heap.as_mut().unwrap().alloc(16, &mut p.mem).unwrap();
    p.mem.write_u32(counter_addr, 0).unwrap();

    let cb_addr = CODE_BASE + 0x1000;
    p.map_code(cb_addr, &arm_words(&[
        0xE590_1000, // ldr r1, [r0]
        0xE281_1001, // add r1, r1, #1
        0xE580_1000, // str r1, [r0]
        0xE3A0_0000, // mov r0, #0 -> stop CIdle
        0xE12F_FF1E, // bx lr
    ])).unwrap();

    p.map_code(CODE_BASE, &arm_words(&[
        0xE3A0_0000,
        0xEF00_0000 | STUB_SVC_BASE,       // CIdle::NewL(0)
        0xE1A0_1008,                       // mov r1, r8 (cb_addr)
        0xE1A0_2005,                       // mov r2, r5 (counter_addr)
        0xEF00_0000 | (STUB_SVC_BASE + 1), // CIdle::Start
        0xEF00_0000 | (STUB_SVC_BASE + 2), // CActiveScheduler::Start
        0xEF00_0000 | EXIT_SVC,
    ])).unwrap();

    p.cpu.regs[5] = counter_addr;
    p.cpu.regs[8] = cb_addr;
    p.cpu.set_pc_interworking(CODE_BASE);

    assert!(matches!(p.run(10_000), Outcome::Blocked));
    assert_eq!(p.guest_word(counter_addr), Some(1));
    assert_eq!(p.next_timer_deadline(), None, "CIdle returning 0 must NOT re-arm");
}

#[test]
fn tdisplaymode_ecolor64k_is_7_and_data_address_roundtrips_rgb565() {
    use heirloom_ekaarm_engine::fbs::decode_mbm_mode;
    // 2x1 16-bit RGB565 MBM: pure red (0xF800), pure blue (0x001F).
    let payload = [0x00, 0xF8, 0x1F, 0x00];
    let mbm = build_mbm(2, 1, 16, 1, 0, &payload);
    let (w, h, mode, px) = decode_mbm_mode(&mbm, 0).expect("16-bit RGB565 MBM");
    assert_eq!((w, h, mode), (2, 1, 7), "EColor64K must be TDisplayMode 7 per gdi.h");
    assert_eq!(px[0], 0x00FF_0000);
    assert_eq!(px[1], 0x0000_00FF);

    // Guest-side CFbsBitmap::Create(2x1, EColor64K=7) -> DisplayMode()==7 -> LockHeap/DataAddress/UnlockHeap.
    let mut p = loader::bare(&[
        "_ZN10CFbsBitmapC1Ev",                         // stub 0
        "_ZN10CFbsBitmap6CreateERK5TSize12TDisplayMode", // stub 1
        "_ZNK10CFbsBitmap11DisplayModeEv",             // stub 2
        "_ZNK10CFbsBitmap8LockHeapEi",                 // stub 3
        "_ZNK10CFbsBitmap11DataAddressEv",             // stub 4
        "_ZNK10CFbsBitmap10UnlockHeapEi",              // stub 5
    ]).unwrap();
    let bmp = p.heap.as_mut().unwrap().alloc(0x40, &mut p.mem).unwrap();
    let sz = p.heap.as_mut().unwrap().alloc(16, &mut p.mem).unwrap();
    p.mem.write_u32(sz, 2).unwrap();
    p.mem.write_u32(sz + 4, 1).unwrap();
    p.map_code(CODE_BASE, &arm_words(&[
        0xE1A0_0004,                       // mov r0, r4 (bmp)
        0xEF00_0000 | STUB_SVC_BASE,       // CFbsBitmap()
        0xE1A0_0004,                       // mov r0, r4
        0xE1A0_1005,                       // mov r1, r5 (sz)
        0xE3A0_2007,                       // mov r2, #7 (EColor64K)
        0xEF00_0000 | (STUB_SVC_BASE + 1), // Create
        0xE1A0_0004,
        0xEF00_0000 | (STUB_SVC_BASE + 2), // DisplayMode -> r6
        0xE1A0_6000,
        0xE1A0_0004,
        0xE3A0_1000,
        0xEF00_0000 | (STUB_SVC_BASE + 3), // LockHeap
        0xE1A0_0004,
        0xEF00_0000 | (STUB_SVC_BASE + 4), // DataAddress -> r7
        0xE1A0_7000,
        // Write RGB565 pure green (0x07E0) at pixel 0 and pure red (0xF800) at pixel 1
        0xE59F_1014,                       // ldr r1, [pc, #20] (0xF80007E0)
        0xE587_1000,                       // str r1, [r7]
        0xE1A0_0004,
        0xE3A0_1000,
        0xEF00_0000 | (STUB_SVC_BASE + 5), // UnlockHeap
        0xE1A0_0006,                       // return DisplayMode
        0xEF00_0000 | EXIT_SVC,
        0xF800_07E0,                       // literal: [0x07E0, 0xF800]
    ])).unwrap();
    p.cpu.regs[4] = bmp;
    p.cpu.regs[5] = sz;
    p.cpu.set_pc_interworking(CODE_BASE);
    assert!(matches!(p.run(10_000), Outcome::Exited(7)));
    // Verify UnlockHeap converted the RGB565 scanline into host RGB888 pixels.
    let fb = p.nat.fbs.bitmaps.values().find(|b| b.width == 2 && b.height == 1).expect("created 2x1 bitmap");
    assert_eq!(fb.get(0, 0), Some(0x0000_FF00), "pixel 0 must decode 0x07E0 -> green");
    assert_eq!(fb.get(1, 0), Some(0x00FF_0000), "pixel 1 must decode 0xF800 -> red");
}

#[test]
fn draw_bitmap_rect_scales_source_and_draw_text_renders_umlauts() {
    let mut p = loader::bare(&[
        "_ZN10CFbsBitmapC1Ev",                                           // stub 0
        "_ZN10CFbsBitmap6CreateERK5TSize12TDisplayMode",                 // stub 1
        "_ZN16CFbsBitmapDevice4NewLEP10CFbsBitmap",                      // stub 2
        "_ZN10CFbsDevice13CreateContextERP9CFbsBitGc",                   // stub 3
        "_ZN9CFbsBitGc10DrawBitmapERK5TRectPK10CFbsBitmap",              // stub 4
        "_ZN9CFbsBitGc11SetPenColorERK4TRgb",                            // stub 5
        "_ZN9CFbsBitGc8DrawTextERK7TDesC16RK6TPoint",                    // stub 6
    ]).unwrap();
    let src_bmp = p.heap.as_mut().unwrap().alloc(0x40, &mut p.mem).unwrap();
    let dst_bmp = p.heap.as_mut().unwrap().alloc(0x40, &mut p.mem).unwrap();
    let gc_out = p.heap.as_mut().unwrap().alloc(16, &mut p.mem).unwrap();
    let sz_src = p.heap.as_mut().unwrap().alloc(16, &mut p.mem).unwrap();
    let sz_dst = p.heap.as_mut().unwrap().alloc(16, &mut p.mem).unwrap();
    let dst_rc = p.heap.as_mut().unwrap().alloc(16, &mut p.mem).unwrap();
    let rgb = p.heap.as_mut().unwrap().alloc(16, &mut p.mem).unwrap();
    let pt = p.heap.as_mut().unwrap().alloc(16, &mut p.mem).unwrap();
    let des = p.heap.as_mut().unwrap().alloc(32, &mut p.mem).unwrap();
    // sz_src = 2x2, sz_dst = 16x16, dst_rc = [0,0,4,4], rgb = 0 (black), pt = (6, 12)
    p.mem.write_u32(sz_src, 2).unwrap(); p.mem.write_u32(sz_src + 4, 2).unwrap();
    p.mem.write_u32(sz_dst, 16).unwrap(); p.mem.write_u32(sz_dst + 4, 16).unwrap();
    p.mem.write_u32(dst_rc, 0).unwrap(); p.mem.write_u32(dst_rc + 4, 0).unwrap();
    p.mem.write_u32(dst_rc + 8, 4).unwrap(); p.mem.write_u32(dst_rc + 12, 4).unwrap();
    p.mem.write_u32(rgb, 0).unwrap();
    p.mem.write_u32(pt, 6).unwrap(); p.mem.write_u32(pt + 4, 12).unwrap();
    // TBufC16 holding "Ö" (U+00D6)
    p.mem.write_u32(des, 1).unwrap();
    p.mem.write_u16(des + 4, 'Ö' as u16).unwrap();

    p.map_code(CODE_BASE, &arm_words(&[
        // Construct & create src_bmp (2x2) and dst_bmp (16x16)
        0xE1A0_0004, 0xEF00_0000 | STUB_SVC_BASE,
        0xE1A0_0004, 0xE1A0_1006, 0xE3A0_200B, 0xEF00_0000 | (STUB_SVC_BASE + 1),
        0xE1A0_0005, 0xEF00_0000 | STUB_SVC_BASE,
        0xE1A0_0005, 0xE1A0_1007, 0xE3A0_200B, 0xEF00_0000 | (STUB_SVC_BASE + 1),
        // Create device + GC for dst_bmp
        0xE1A0_0005, 0xEF00_0000 | (STUB_SVC_BASE + 2),
        0xE1A0_1008, 0xEF00_0000 | (STUB_SVC_BASE + 3),
        0xE598_9000, // ldr r9, [r8] (gc)
        0xE3A0_0000, 0xEF00_0000 | EXIT_SVC,
    ])).unwrap();
    p.cpu.regs[4] = src_bmp;
    p.cpu.regs[5] = dst_bmp;
    p.cpu.regs[6] = sz_src;
    p.cpu.regs[7] = sz_dst;
    p.cpu.regs[8] = gc_out;
    p.cpu.set_pc_interworking(CODE_BASE);
    assert!(matches!(p.run(10_000), Outcome::Exited(0)));

    // Paint 4 distinct quadrant colors in src_bmp (2x2).
    let src_fb = p.nat.fbs.bitmaps.values_mut().find(|b| b.width == 2 && b.height == 2).expect("src 2x2 bitmap");
    src_fb.set(0, 0, 0x00FF_0000); // top-left red
    src_fb.set(1, 0, 0x0000_FF00); // top-right green
    src_fb.set(0, 1, 0x0000_00FF); // bottom-left blue
    src_fb.set(1, 1, 0x00FF_FF00); // bottom-right yellow

    let gc = p.cpu.regs[9];
    p.map_code(CODE_BASE + 0x1000, &arm_words(&[
        // DrawBitmap(dst_rc=[0,0,4,4], src_bmp) -> scales 2x2 to 4x4
        0xE1A0_0009, 0xE1A0_100A, 0xE1A0_2004, 0xEF00_0000 | (STUB_SVC_BASE + 4),
        // SetPenColor(black) & DrawText("Ö", (6, 12))
        0xE1A0_0009, 0xE1A0_100B, 0xEF00_0000 | (STUB_SVC_BASE + 5),
        0xE1A0_0009, 0xE1A0_1008, 0xE1A0_2007, 0xEF00_0000 | (STUB_SVC_BASE + 6),
        0xE3A0_0000, 0xEF00_0000 | EXIT_SVC,
    ])).unwrap();
    p.cpu.regs[4] = src_bmp;
    p.cpu.regs[7] = pt;
    p.cpu.regs[8] = des;
    p.cpu.regs[9] = gc;
    p.cpu.regs[10] = dst_rc;
    p.cpu.regs[11] = rgb;
    p.cpu.set_pc_interworking(CODE_BASE + 0x1000);
    assert!(matches!(p.run(10_000), Outcome::Exited(0)));

    let dst_fb = p.nat.fbs.bitmaps.values().find(|b| b.width == 16 && b.height == 16).expect("dst 16x16 bitmap");
    assert_eq!(dst_fb.get(0, 0), Some(0x00FF_0000));
    assert_eq!(dst_fb.get(3, 0), Some(0x0000_FF00), "DrawBitmap must stretch horizontally");
    assert_eq!(dst_fb.get(0, 3), Some(0x0000_00FF), "DrawBitmap must stretch vertically");
    assert_eq!(dst_fb.get(3, 3), Some(0x00FF_FF00), "DrawBitmap must fill bottom-right quadrant");

    let mut black_px = 0;
    for y in 4..14 {
        for x in 6..12 {
            if dst_fb.get(x, y) == Some(0) { black_px += 1; }
        }
    }
    assert!(black_px > 5, "umlaut 'Ö' must render non-empty dark pixels, got {black_px}");
}
