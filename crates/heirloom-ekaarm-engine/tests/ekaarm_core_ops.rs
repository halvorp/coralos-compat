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

fn make_plain_rsc(id_offset: u32, resources: &[Vec<u8>]) -> Vec<u8> {
    let mut f = Vec::new();
    f.extend_from_slice(&0x101f_4a6bu32.to_le_bytes()); // UID1 plain
    f.extend_from_slice(&0u32.to_le_bytes());
    f.extend_from_slice(&(id_offset >> 8).to_le_bytes()); // UID3
    f.extend_from_slice(&[0; 4]);
    f.push(0);
    f.extend_from_slice(&[0, 0]);
    let mut sig = 4u32.to_le_bytes().to_vec();
    sig.extend_from_slice(&(id_offset | 1).to_le_bytes());
    let mut all = vec![sig];
    all.extend_from_slice(resources);
    f.extend(core::iter::repeat_n(0u8, all.len().div_ceil(8)));
    let mut offsets = Vec::new();
    for b in &all {
        offsets.push(f.len() as u16);
        f.extend_from_slice(b);
    }
    let index_at = f.len() as u16;
    for o in offsets { f.extend_from_slice(&o.to_le_bytes()); }
    f.extend_from_slice(&index_at.to_le_bytes());
    f
}

#[test]
fn listbox_construct_from_resource_populates_array_id_and_tbuf_reads_raw_utf16() {
    use heirloom_ekaarm_engine::drive::Drive;
    use heirloom_ui_abi::{RecordingGuestUi, UiCall};

    const OFF: u32 = 0x1234_5000;
    // Resource 2: raw UTF-16LE TBUF ("Play") — no leading length byte!
    let tbuf: Vec<u8> = "Play".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    // Resource 3: ARRAY of 2 LTEXT items ("New Game", "Load Game") — 1-byte length + odd pad + UTF-16LE
    let mut arr = 2u16.to_le_bytes().to_vec();
    for s in ["New Game", "Load Game"] {
        let u: Vec<u16> = s.encode_utf16().collect();
        arr.push(u.len() as u8);
        if arr.len() % 2 == 1 { arr.push(0xAB); }
        for c in u { arr.extend_from_slice(&c.to_le_bytes()); }
    }
    // Resource 4: STRUCT LISTBOX { BYTE version=0; LONG flags=0; WORD h=5; WORD w=10; LLINK array_id=OFF|3; }
    let mut lb_res = vec![0u8];
    lb_res.extend_from_slice(&0u32.to_le_bytes());
    lb_res.extend_from_slice(&5u16.to_le_bytes());
    lb_res.extend_from_slice(&10u16.to_le_bytes());
    lb_res.extend_from_slice(&(OFF | 3).to_le_bytes());

    const EIK_OFF: u32 = 0x19bd_6000;
    let mut color_list = (EIK_OFF | 6).to_le_bytes().to_vec();
    color_list.extend_from_slice(&(EIK_OFF | 7).to_le_bytes());
    let mut colors = 59u16.to_le_bytes().to_vec();
    for i in 0u16..59 { colors.extend_from_slice(&i.to_le_bytes()); colors.extend_from_slice(&[i as u8; 3]); }
    let eikpriv = make_plain_rsc(EIK_OFF, &[vec![0; 8], color_list, vec![0; 2], vec![0; 2], colors, 0u16.to_le_bytes().to_vec()]);
    let test_rsc = make_plain_rsc(OFF, &[tbuf, arr, lb_res]);

    struct TestRom { eikpriv: Vec<u8>, test_rsc: Vec<u8> }
    impl heirloom_ekaarm_engine::drive::RomSource for TestRom {
        fn read(&self, path: &str) -> Option<Vec<u8>> {
            if path.eq_ignore_ascii_case("\\resource\\uiklaf\\eikpriv.rsc") { Some(self.eikpriv.clone()) }
            else if path.eq_ignore_ascii_case("\\resource\\test.rsc") { Some(self.test_rsc.clone()) }
            else { None }
        }
        fn exists(&self, path: &str) -> bool { self.read(path).is_some() }
        fn list(&self, path: &str) -> Option<Vec<String>> {
            if path.eq_ignore_ascii_case("\\resource") { Some(vec!["uiklaf".into(), "test.rsc".into()]) }
            else if path.eq_ignore_ascii_case("\\resource\\uiklaf") { Some(vec!["eikpriv.rsc".into()]) }
            else { None }
        }
    }
    let mut drive = Drive::new();
    drive.set_rom(Box::new(TestRom { eikpriv, test_rsc }));

    let mut p = loader::bare(&[
        "_ZNK7CCoeEnv20ReadResourceAsDes16LER6TDes16i",                                // stub 0
        "_ZN22CAknSingleStyleListBoxC1Ev",                                            // stub 1
        "_ZNK7CCoeEnv22CreateResourceReaderLCER15TResourceReaderi",                    // stub 2
        "_ZN24CEikFormattedCellListBox22ConstructFromResourceLER15TResourceReader",   // stub 3
        "_ZNK24CEikFormattedCellListBox5ModelEv",                                     // stub 4
        "_ZNK17CTextListBoxModel13NumberOfItemsEv",                                   // stub 5
        "_ZN22CAknMessageQueryDialog4NewLER7TDesC16RKN15CAknQueryDialog5TToneE",      // stub 6
        "_ZN10CEikDialog5RunLDEv",                                                    // stub 7
    ]).unwrap();
    p.host.drive = Some(drive);
    p.host.ui = Some(Box::new(RecordingGuestUi::new()));

    let out_des = p.heap.as_mut().unwrap().alloc(64, &mut p.mem).unwrap();
    p.mem.write_u32(out_des, 3 << 28).unwrap();
    p.mem.write_u32(out_des + 4, 24).unwrap();
    let lb = p.heap.as_mut().unwrap().alloc(0x100, &mut p.mem).unwrap();
    let reader = p.heap.as_mut().unwrap().alloc(32, &mut p.mem).unwrap();
    let tone = p.heap.as_mut().unwrap().alloc(8, &mut p.mem).unwrap();

    p.map_code(CODE_BASE, &arm_words(&[
        // ReadResourceAsDes16L(out_des, OFF|2)
        0xE3A0_0000, 0xE1A0_1004, 0xE1A0_2007, 0xEF00_0000 | STUB_SVC_BASE,
        // Construct listbox + ConstructFromResourceL(OFF|4)
        0xE1A0_0005, 0xEF00_0000 | (STUB_SVC_BASE + 1),
        0xE3A0_0000, 0xE1A0_1006, 0xE1A0_2008, 0xEF00_0000 | (STUB_SVC_BASE + 2),
        0xE1A0_0005, 0xE1A0_1006, 0xEF00_0000 | (STUB_SVC_BASE + 3),
        // model = lb->Model(); r9 = model->NumberOfItems()
        0xE1A0_0005, 0xEF00_0000 | (STUB_SVC_BASE + 4),
        0xEF00_0000 | (STUB_SVC_BASE + 5),
        0xE1A0_9000,
        // dlg = CAknMessageQueryDialog::NewL(out_des, tone); dlg->RunLD()
        0xE1A0_0004, 0xE1A0_100A, 0xEF00_0000 | (STUB_SVC_BASE + 6),
        0xEF00_0000 | (STUB_SVC_BASE + 7),
        0xE1A0_0009, 0xEF00_0000 | EXIT_SVC,
    ])).unwrap();
    p.cpu.regs[4] = out_des;
    p.cpu.regs[5] = lb;
    p.cpu.regs[6] = reader;
    p.cpu.regs[7] = OFF | 2;
    p.cpu.regs[8] = OFF | 4;
    p.cpu.regs[10] = tone;
    p.cpu.set_pc_interworking(CODE_BASE);

    // RunLD blocks waiting for host dialog response.
    let out = p.run(10_000);
    assert!(matches!(out, Outcome::Blocked), "expected Blocked, got {out:?}");
    // Verify STRUCT LISTBOX populated 2 items from array_id (fails without array_id parsing!).
    assert_eq!(p.cpu.regs[9], 2, "STRUCT LISTBOX array_id must populate 2 model items");
    // Verify ReadResourceAsDes16L decoded raw UTF-16LE TBUF ("Play") and CAknMessageQueryDialog::NewL kept it!
    let ui = p.host.ui.as_ref().unwrap().as_any().downcast_ref::<RecordingGuestUi>().unwrap();
    let req_text = ui.calls.iter().find_map(|c| match c {
        UiCall::Request(_, m) => Some(m.text.clone()),
        _ => None,
    });
    assert_eq!(req_text.as_deref(), Some("Play"), "TBUF read + MessageQueryDialog::NewL must project \"Play\"");
}

#[test]
fn enter_aliases_to_s60_device3_only_when_unconsumed_and_user_request_complete_signals() {
    let mut p = loader::bare(&[
        "_ZN4User15RequestCompleteERP14TRequestStatusi",               // stub 0
        "_ZN11CCoeControl14OfferKeyEventLERK9TKeyEvent10TEventCode",  // stub 1
        "_ZN9CCoeAppUi14HandleKeyEventLERK9TKeyEvent10TEventCode",    // stub 2
        "_ZN9CAknAppUi14HandleWsEventLERK8TWsEventP11CCoeControl",    // stub 3
        "_ZN20CActiveSchedulerWaitC1Ev",                              // stub 4
        "_ZNK20CActiveSchedulerWait9IsStartedEv",                     // stub 5
    ]).unwrap();

    // Test 1: User::RequestComplete clears pointer to NULL, writes reason, and signals once.
    let status = p.heap.as_mut().unwrap().alloc(16, &mut p.mem).unwrap();
    let status_ptr = p.heap.as_mut().unwrap().alloc(16, &mut p.mem).unwrap();
    let waiter = p.heap.as_mut().unwrap().alloc(32, &mut p.mem).unwrap();
    p.mem.write_u32(status, 0x8000_0001).unwrap(); // KRequestPending
    p.mem.write_u32(status_ptr, status).unwrap();

    // Test 2: Guest AppUi whose HandleKeyEventL rejects EKeyEnter (0x0d) -> expects retry as EKeyDevice3 (0xf845).
    let appui_vt = CODE_BASE + 0x4000;
    let appui = p.heap.as_mut().unwrap().alloc(64, &mut p.mem).unwrap();
    let ws_ev = p.heap.as_mut().unwrap().alloc(64, &mut p.mem).unwrap();
    let log_buf = p.heap.as_mut().unwrap().alloc(32, &mut p.mem).unwrap(); // [count, code0, code1]
    // TWsEvent: type=1 (EEventKey), iEventData at +16: iCode=0x0d, iScanCode=0x03
    p.mem.write_u32(ws_ev, 1).unwrap();
    p.mem.write_u32(ws_ev + 16, 0x000d).unwrap();
    p.mem.write_u32(ws_ev + 20, 0x0003).unwrap();

    // Guest HandleKeyEventL(r0=appui, r1=TKeyEvent*, r2=TEventCode) at CODE_BASE + 0x1000:
    // logs iCode to log_buf[1 + count], increments count, returns 1 iff iCode == 0xf845 else 0.
    let handler_addr = CODE_BASE + 0x1000;
    p.map_code(handler_addr, &arm_words(&[
        0xE591_3000, // word 0: ldr r3, [r1]        ; iCode
        0xE59F_C020, // word 1: ldr r12, [pc, #32]  ; word 11 = log_buf
        0xE59C_2000, // word 2: ldr r2, [r12]       ; count
        0xE282_0001, // word 3: add r0, r2, #1
        0xE58C_0000, // word 4: str r0, [r12]       ; count++
        0xE78C_3100, // word 5: str r3, [r12, r0, lsl #2] ; log_buf[count] = iCode
        0xE59F_0010, // word 6: ldr r0, [pc, #16]   ; word 12 = 0xf845 (EKeyDevice3)
        0xE153_0000, // word 7: cmp r3, r0
        0x03A0_0001, // word 8: moveq r0, #1        ; EKeyWasConsumed
        0x13A0_0000, // word 9: movne r0, #0        ; EKeyWasNotConsumed
        0xE12F_FF1E, // word 10: bx lr
        log_buf,     // word 11
        0x0000_F845, // word 12
    ])).unwrap();
    let mut vt_words = vec![0u32; 16];
    vt_words[7] = handler_addr;
    p.map_code(appui_vt, &arm_words(&vt_words)).unwrap();
    p.mem.write_u32(appui, appui_vt).unwrap();

    p.map_code(CODE_BASE, &arm_words(&[
        0xE1A0_0004, 0xE3A0_1000, 0xEF00_0000 | STUB_SVC_BASE,       // User::RequestComplete(&status, 0)
        0xE1A0_0005, 0xEF00_0000 | (STUB_SVC_BASE + 4),              // CActiveSchedulerWait()
        0xE1A0_0005, 0xEF00_0000 | (STUB_SVC_BASE + 5),              // IsStarted() -> r8
        0xE1A0_8000,
        0xE3A0_0000, 0xE3A0_1000, 0xE3A0_2001, 0xEF00_0000 | (STUB_SVC_BASE + 1), // base OfferKeyEventL -> r9
        0xE1A0_9000,
        0xE1A0_0006, 0xE1A0_1007, 0xE3A0_2000, 0xEF00_0000 | (STUB_SVC_BASE + 3), // HandleWsEventL
        0xE3A0_0000, 0xEF00_0000 | EXIT_SVC,
    ])).unwrap();
    p.cpu.regs[4] = status_ptr;
    p.cpu.regs[5] = waiter;
    p.cpu.regs[6] = appui;
    p.cpu.regs[7] = ws_ev;
    p.cpu.set_pc_interworking(CODE_BASE);
    assert!(matches!(p.run(10_000), Outcome::Exited(0)));

    assert_eq!(p.mem.read_u32(status).unwrap(), 0, "RequestComplete must write reason 0");
    assert_eq!(p.mem.read_u32(status_ptr).unwrap(), 0, "RequestComplete must NULL caller pointer");
    assert_eq!(p.nat.requests.signals, 1, "RequestComplete must signal scheduler once");
    assert_eq!(p.cpu.regs[8], 0, "fresh CActiveSchedulerWait must not be started");
    assert_eq!(p.cpu.regs[9], 0, "base CCoeControl::OfferKeyEventL must return EKeyWasNotConsumed");
    // Verify HandleKeyEventL saw 0x0d first, rejected it, and then received S60 EKeyDevice3 (0xf845)!
    assert_eq!(p.mem.read_u32(log_buf).unwrap(), 2, "unconsumed EKeyEnter must retry once as EKeyDevice3");
    assert_eq!(p.mem.read_u32(log_buf + 4).unwrap(), 0x000d);
    assert_eq!(p.mem.read_u32(log_buf + 8).unwrap(), 0xf845);

    // Negative control: when HandleKeyEventL consumes 0x0d directly (returns 1), it must NOT retry!
    p.mem.write_u32(log_buf, 0).unwrap();
    p.mem.write_u32(ws_ev + 16, 0x000d).unwrap();
    let consume_addr = CODE_BASE + 0x2000;
    p.map_code(consume_addr, &arm_words(&[
        0xE59F_C010, // ldr r12, [pc, #16] (log_buf)
        0xE59C_2000,
        0xE282_2001,
        0xE58C_2000,
        0xE3A0_0001, // return EKeyWasConsumed (1)
        0xE12F_FF1E,
        log_buf,
    ])).unwrap();
    let consume_vt = CODE_BASE + 0x5000;
    let mut vt2 = vec![0u32; 16];
    vt2[7] = consume_addr;
    p.map_code(consume_vt, &arm_words(&vt2)).unwrap();
    p.mem.write_u32(appui, consume_vt).unwrap();
    p.map_code(CODE_BASE + 0x3000, &arm_words(&[
        0xE1A0_0006, 0xE1A0_1007, 0xE3A0_2000, 0xEF00_0000 | (STUB_SVC_BASE + 3),
        0xE3A0_0000, 0xEF00_0000 | EXIT_SVC,
    ])).unwrap();
    p.cpu.regs[6] = appui;
    p.cpu.regs[7] = ws_ev;
    p.cpu.set_pc_interworking(CODE_BASE + 0x3000);
    assert!(matches!(p.run(10_000), Outcome::Exited(0)));
    assert_eq!(p.mem.read_u32(log_buf).unwrap(), 1, "consumed EKeyEnter must NOT trigger EKeyDevice3 retry");
}
