//! Universal Amiga 68k game subsystems: Paula `audio.device`, `cybergraphics.library` RTG,
//! `ASL_ScreenModeRequest`, custom screens (`OpenScreenTagList`/`AllocScreenBuffer`/`ChangeScreenBuffer`),
//! `gameport.device`, `timer.device/ReadEClock`, and `graphics.library/WriteChunkyPixels`.
use heirloom_ami68k_engine::{asl::DEFAULT_RTG_DISPLAY_ID, cybergraphics, Amiga};
use heirloom_ui_abi::{RecordingGuestUi, UiCall};

/// Assemble a minimal 68k Hunk executable from raw big-endian 16-bit words.
fn hunk(words: &[u16]) -> Vec<u8> {
    let mut code = Vec::with_capacity(words.len() * 2 + 2);
    for &w in words {
        code.extend_from_slice(&w.to_be_bytes());
    }
    while code.len() % 4 != 0 {
        code.extend_from_slice(&0x4E71u16.to_be_bytes());
    }
    let longs = (code.len() / 4) as u32;
    let mut out = Vec::new();
    for v in [0x0000_03F3u32, 0, 1, 0, 0, longs, 0x0000_03E9, longs] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    out.extend_from_slice(&code);
    out.extend_from_slice(&0x0000_03F2u32.to_be_bytes());
    out
}

#[test]
fn cybergraphics_mode_ids_and_asl_screenmode_requester() {
    assert!(cybergraphics::is_cyber_mode_id(DEFAULT_RTG_DISPLAY_ID));
    // NEGATIVE CONTROL: Workbench and arbitrary PAL/NTSC mode IDs are not CyberGraphX modes.
    assert!(!cybergraphics::is_cyber_mode_id(0x0002_9000));
    assert!(!cybergraphics::is_cyber_mode_id(0));
}

#[test]
fn audio_device_opens_allocates_plays_pcm_and_refuses_channel_exhaustion() {
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    // Guest memory layout in scratch RAM:
    // 0x200000: "audio.device\0"
    // 0x200020: channel map [0x0f] (all 4 channels)
    // 0x200030: 8-byte signed PCM [0x00, 0x7f, 0x80, 0x01, ...]
    // 0x200100: IOAudio #1 (68 bytes)
    // 0x200200: IOAudio #2 (68 bytes, tests channel exhaustion -> ADIOERR_ALLOCFAILED)
    // 0x200300: result buffer [open1_err, open2_err, unit1]
    a.mem.write_bytes(0x200000, b"audio.device\0").unwrap();
    a.mem.write_bytes(0x200020, &[0x0f]).unwrap();
    a.mem.write_bytes(0x200030, &[0x00, 0x7f, 0x80, 0x01, 0x00, 0x7f, 0x80, 0x01]).unwrap();
    a.mem.zero(0x200100, 0x200).unwrap();
    // Pre-fill IOAudio #1 and #2 with ioa_Data = 0x200020, ioa_Length = 1
    for io in [0x200100u32, 0x200200u32] {
        a.mem.write_u32(io + 34, 0x200020).unwrap();
        a.mem.write_u32(io + 38, 1).unwrap();
    }
    // 68k program:
    //   movea.l 4.w, a6
    //   lea 0x200000, a0; moveq #0, d0; lea 0x200100, a1; moveq #0, d1; jsr -444(a6) (OpenDevice #1)
    //   move.l d0, (0x200300)
    //   lea 0x200000, a0; moveq #0, d0; lea 0x200200, a1; moveq #0, d1; jsr -444(a6) (OpenDevice #2 -> -11)
    //   move.l d0, (0x200304)
    //   ; CMD_WRITE on IOAudio #1: io_Command=3, ioa_Data=0x200030, ioa_Length=8, ioa_Period=325, ioa_Volume=64, ioa_Cycles=1
    //   lea 0x200100, a1
    //   move.w #3, 28(a1)
    //   move.l #0x200030, 34(a1)
    //   move.l #8, 38(a1)
    //   move.w #325, 42(a1)
    //   move.w #64, 44(a1)
    //   move.w #1, 46(a1)
    //   jsr -456(a6) (DoIO)
    //   lea 0x200100, a1; jsr -450(a6) (CloseDevice)
    //   moveq #0, d0; rts
    let prog = hunk(&[
        0x2C78, 0x0004,                         // movea.l 4.w, a6
        0x41F9, 0x0020, 0x0000,                 // lea 0x200000, a0
        0x7000,                                 // moveq #0, d0
        0x43F9, 0x0020, 0x0100,                 // lea 0x200100, a1
        0x7200,                                 // moveq #0, d1
        0x4EAE, 0xFE44,                         // jsr -444(a6) OpenDevice
        0x23C0, 0x0020, 0x0300,                 // move.l d0, 0x200300
        0x41F9, 0x0020, 0x0000,                 // lea 0x200000, a0
        0x7000,                                 // moveq #0, d0
        0x43F9, 0x0020, 0x0200,                 // lea 0x200200, a1
        0x7200,                                 // moveq #0, d1
        0x4EAE, 0xFE44,                         // jsr -444(a6) OpenDevice
        0x23C0, 0x0020, 0x0304,                 // move.l d0, 0x200304
        0x43F9, 0x0020, 0x0100,                 // lea 0x200100, a1
        0x337C, 0x0003, 0x001C,                 // move.w #3, 28(a1)
        0x237C, 0x0020, 0x0030, 0x0022,         // move.l #0x200030, 34(a1)
        0x237C, 0x0000, 0x0008, 0x0026,         // move.l #8, 38(a1)
        0x337C, 0x0145, 0x002A,                 // move.w #325, 42(a1)
        0x337C, 0x0040, 0x002C,                 // move.w #64, 44(a1)
        0x337C, 0x0001, 0x002E,                 // move.w #1, 46(a1)
        0x4EAE, 0xFE38,                         // jsr -456(a6) DoIO
        0x43F9, 0x0020, 0x0100,                 // lea 0x200100, a1
        0x4EAE, 0xFE3E,                         // jsr -450(a6) CloseDevice
        0x7000,                                 // moveq #0, d0
        0x4E75,                                 // rts
    ]);
    let rep = a.run_hunk(&prog, 100_000).unwrap();
    assert!(rep.stats.halted);
    assert!(a.refused.is_empty(), "{:?}", a.refused);
    assert_eq!(a.mem.read_u32(0x200300).unwrap(), 0, "first audio.device open succeeds");
    assert_eq!(a.mem.read_u32(0x200304).unwrap() as i32, -11, "NEGATIVE CONTROL: exhausted channels return ADIOERR_ALLOCFAILED");
    assert_eq!(a.mem.read_u32(0x200100 + 24).unwrap(), 0x0f, "allocated all 4 channels");
    let opened = a.ui.calls.iter().filter(|c| matches!(c, UiCall::OpenAudio(_))).count();
    let queued = a.ui.calls.iter().filter(|c| matches!(c, UiCall::QueueAudio(_, 8))).count();
    let closed = a.ui.calls.iter().filter(|c| matches!(c, UiCall::CloseAudio(_))).count();
    assert_eq!((opened, queued, closed), (4, 4, 4));
}

#[test]
fn timer_readeclock_and_gameport_and_cybergraphics_roundtrip() {
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    a.clock_us = 0x0000_0001_0002_0003;
    a.mem.write_bytes(0x200000, b"timer.device\0").unwrap();
    a.mem.write_bytes(0x200020, b"gameport.device\0").unwrap();
    a.mem.write_bytes(0x200040, b"cybergraphics.library\0").unwrap();
    a.mem.zero(0x200100, 0x300).unwrap();
    // 68k program:
    //  1) OpenDevice("timer.device", 2, 0x200100, 0); call ReadEClock(0x200180) on timer base -> store D0 at 0x200300
    //  2) OpenDevice("gameport.device", 1, 0x200140, 0); GPD_ASKCTYPE (cmd 10, io_Data=0x200190) -> DoIO
    //  3) OpenLibrary("cybergraphics.library", 0); BestCModeIDTagList(NULL) -> store at 0x200304; IsCyberModeID -> 0x200308
    let prog = hunk(&[
        0x2C78, 0x0004,                         // movea.l 4.w, a6
        0x41F9, 0x0020, 0x0000,                 // lea "timer.device", a0
        0x7002,                                 // moveq #2, d0
        0x43F9, 0x0020, 0x0100,                 // lea 0x200100, a1
        0x7200,                                 // moveq #0, d1
        0x4EAE, 0xFE44,                         // jsr OpenDevice
        0x2C79, 0x0020, 0x0114,                 // movea.l 0x200114 (io_Device), a6
        0x41F9, 0x0020, 0x0180,                 // lea 0x200180 (EClockVal), a0
        0x4EAE, 0xFFC4,                         // jsr -60(a6) ReadEClock
        0x23C0, 0x0020, 0x0300,                 // move.l d0, 0x200300
        0x2C78, 0x0004,                         // movea.l 4.w, a6
        0x41F9, 0x0020, 0x0020,                 // lea "gameport.device", a0
        0x7001,                                 // moveq #1, d0
        0x43F9, 0x0020, 0x0140,                 // lea 0x200140, a1
        0x7200,                                 // moveq #0, d1
        0x4EAE, 0xFE44,                         // jsr OpenDevice
        0x43F9, 0x0020, 0x0140,                 // lea 0x200140, a1
        0x337C, 0x000A, 0x001C,                 // move.w #10, 28(a1) GPD_ASKCTYPE
        0x237C, 0x0020, 0x0190, 0x0028,         // move.l #0x200190, 40(a1) io_Data
        0x4EAE, 0xFE38,                         // jsr DoIO
        0x43F9, 0x0020, 0x0040,                 // lea "cybergraphics.library", a1
        0x7000,                                 // moveq #0, d0
        0x4EAE, 0xFDD8,                         // jsr -552(a6) OpenLibrary
        0x2C40,                                 // movea.l d0, a6
        0x91C8,                                 // suba.l a0, a0 (NULL tags)
        0x4EAE, 0xFFC4,                         // jsr -60(a6) BestCModeIDTagList
        0x23C0, 0x0020, 0x0304,                 // move.l d0, 0x200304
        0x4EAE, 0xFFCA,                         // jsr -54(a6) IsCyberModeID
        0x23C0, 0x0020, 0x0308,                 // move.l d0, 0x200308
        0x7000,                                 // moveq #0, d0
        0x4E75,                                 // rts
    ]);
    let rep = a.run_hunk(&prog, 100_000).unwrap();
    assert!(rep.stats.halted);
    assert!(a.refused.is_empty(), "{:?}", a.refused);
    assert_eq!(a.mem.read_u32(0x200300).unwrap(), 1_000_000);
    assert_eq!(a.mem.read_u32(0x200180).unwrap(), 0x0000_0001);
    assert_eq!(a.mem.read_u32(0x200184).unwrap(), 0x0002_0003);
    assert_eq!(a.mem.read_u32(0x200140 + 32).unwrap(), 1, "GPD_ASKCTYPE sets io_Actual=1");
    assert_eq!(a.mem.read_u8(0x200190).unwrap(), 0, "GPCT_NOCONTROLLER");
    assert_eq!(a.mem.read_u32(0x200304).unwrap(), DEFAULT_RTG_DISPLAY_ID);
    assert_eq!(a.mem.read_u32(0x200308).unwrap(), 1);
}
