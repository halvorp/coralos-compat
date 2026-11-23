//! Focused host witness tests for Symbian descriptor sync (`HBufC::Des`),
//! `TTime::FormatL`, resource listbox population, key routing/S60 Enter->OK aliasing,
//! `User::RequestComplete`, geometry (`TRect`/`TSize`), EABI f32, and `RFile::SetSize`.
use heirloom_ekaarm_engine::cpu::Memory;
use heirloom_ekaarm_engine::layout::{CODE_BASE, EXIT_SVC, STUB_SVC_BASE};
use heirloom_ekaarm_engine::{loader, Outcome};

fn arm_words(w: &[u32]) -> Vec<u8> {
    w.iter().flat_map(|x| x.to_le_bytes()).collect()
}

#[test]
fn hbufc16_des_syncs_parent_length_and_locate_replace_work() {
    let mut p = loader::bare(&[
        "_ZN7HBufC165NewLCEi",                // stub 0
        "_ZN7HBufC163DesEv",                  // stub 1 (sret r0, this r1)
        "_ZN6TDes164CopyERK7TDesC16",         // stub 2
        "_ZNK7TDesC166LocateE5TChar",         // stub 3
        "_ZN6TDes167ReplaceEiiRK7TDesC16",    // stub 4
        "_ZN12CleanupStack13PopAndDestroyEv", // stub 5
    ]).unwrap();
    let tptr = p.heap.as_mut().unwrap().alloc(16, &mut p.mem).unwrap();
    let src = p.heap.as_mut().unwrap().alloc(32, &mut p.mem).unwrap();
    let rep = p.heap.as_mut().unwrap().alloc(32, &mut p.mem).unwrap();
    // src = TBufC16("Hi %1")
    p.mem.write_u32(src, 5).unwrap();
    for (i, ch) in "Hi %1".encode_utf16().enumerate() {
        p.mem.write_u16(src + 4 + i as u32 * 2, ch).unwrap();
    }
    // rep = TBufC16("CG")
    p.mem.write_u32(rep, 2).unwrap();
    for (i, ch) in "CG".encode_utf16().enumerate() {
        p.mem.write_u16(rep + 4 + i as u32 * 2, ch).unwrap();
    }
    p.map_code(CODE_BASE, &arm_words(&[
        0xE3A0_0020,                       // mov r0, #32
        0xEF00_0000 | STUB_SVC_BASE,       // HBufC16::NewLC(32) -> r4
        0xE1A0_4000,
        0xE1A0_0005,                       // r0 = tptr (sret)
        0xE1A0_1004,                       // r1 = hbuf
        0xEF00_0000 | (STUB_SVC_BASE + 1), // hbuf->Des()
        0xE1A0_0005,                       // r0 = tptr
        0xE1A0_1006,                       // r1 = "Hi %1"
        0xEF00_0000 | (STUB_SVC_BASE + 2), // tptr.Copy("Hi %1")
        0xE1A0_0005,
        0xE3A0_1025,                       // '%'
        0xEF00_0000 | (STUB_SVC_BASE + 3), // Locate('%') -> r8
        0xE1A0_8000,
        0xE1A0_0005,                       // r0 = tptr
        0xE1A0_1008,                       // r1 = pos (3)
        0xE3A0_2002,                       // r2 = len (2)
        0xE1A0_3007,                       // r3 = "CG"
        0xEF00_0000 | (STUB_SVC_BASE + 4), // tptr.Replace(3, 2, "CG")
        // Read parent HBufC16 header word directly into r0 to verify DES_BUF_C_PTR synced it!
        0xE594_0000,                       // ldr r0, [r4]
        0xEF00_0000 | EXIT_SVC,
    ])).unwrap();
    p.cpu.regs[5] = tptr;
    p.cpu.regs[6] = src;
    p.cpu.regs[7] = rep;
    p.cpu.set_pc_interworking(CODE_BASE);
    // Must exit with parent HBufC16 length == 5 (fails without DES_BUF_C_PTR parent sync!)
    assert!(matches!(p.run(10_000), Outcome::Exited(5)));
    assert_eq!(p.cpu.regs[8], 3, "Locate('%') in \"Hi %1\" must return index 3");
    let hbuf = p.cpu.regs[4];
    let got: String = (0..5).map(|i| char::from_u32(p.mem.read_u16(hbuf + 4 + i * 2).unwrap() as u32).unwrap()).collect();
    assert_eq!(got, "Hi CG");
}

#[test]
fn tdes8_copy_raw_bytes_and_ttime_format_l_work() {
    let mut p = loader::bare(&[
        "_ZN5TDes84CopyEPKhi",                      // stub 0
        "_ZN5TTime8HomeTimeEv",                    // stub 1
        "_ZN20TShortDateFormatSpecC1Ev",           // stub 2
        "_ZNK5TTime7FormatLER6TDes16RK7TDesC16",   // stub 3
    ]).unwrap();
    let des8 = p.heap.as_mut().unwrap().alloc(32, &mut p.mem).unwrap();
    let raw = p.heap.as_mut().unwrap().alloc(16, &mut p.mem).unwrap();
    let ttime = p.heap.as_mut().unwrap().alloc(16, &mut p.mem).unwrap();
    let spec = p.heap.as_mut().unwrap().alloc(96, &mut p.mem).unwrap();
    let out = p.heap.as_mut().unwrap().alloc(64, &mut p.mem).unwrap();
    // TBuf8<16> (type 3 = EBuf)
    p.mem.write_u32(des8, 3 << 28).unwrap();
    p.mem.write_u32(des8 + 4, 16).unwrap();
    for (i, &b) in b"CHSS".iter().enumerate() { p.mem.write_u8(raw + i as u32, b).unwrap(); }
    // TBuf16<24>
    p.mem.write_u32(out, 3 << 28).unwrap();
    p.mem.write_u32(out + 4, 24).unwrap();

    p.map_code(CODE_BASE, &arm_words(&[
        0xE1A0_0004, 0xE1A0_1005, 0xE3A0_2004, 0xEF00_0000 | STUB_SVC_BASE,
        0xE1A0_0006, 0xEF00_0000 | (STUB_SVC_BASE + 1),
        0xE1A0_0007, 0xEF00_0000 | (STUB_SVC_BASE + 2),
        0xE1A0_0006, 0xE1A0_1008, 0xE1A0_2007, 0xEF00_0000 | (STUB_SVC_BASE + 3),
        0xE3A0_0000, 0xEF00_0000 | EXIT_SVC,
    ])).unwrap();
    p.cpu.regs[4] = des8;
    p.cpu.regs[5] = raw;
    p.cpu.regs[6] = ttime;
    p.cpu.regs[7] = spec;
    p.cpu.regs[8] = out;
    p.cpu.set_pc_interworking(CODE_BASE);
    assert!(matches!(p.run(10_000), Outcome::Exited(0)));

    assert_eq!(p.mem.read_u32(des8).unwrap() & 0x0FFF_FFFF, 4);
    let got8: Vec<u8> = (0..4).map(|i| p.mem.read_u8(des8 + 8 + i).unwrap()).collect();
    assert_eq!(&got8, b"CHSS");

    let out_len = (p.mem.read_u32(out).unwrap() & 0x0FFF_FFFF) as usize;
    let formatted: String = (0..out_len).map(|i| char::from_u32(p.mem.read_u16(out + 8 + i as u32 * 2).unwrap() as u32).unwrap()).collect();
    assert_eq!(formatted.len(), 10, "DD/MM/YYYY format must be 10 chars, got {formatted:?}");
    assert!(formatted.contains('/'), "formatted date must contain '/', got {formatted:?}");
}
