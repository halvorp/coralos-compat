//! Witness tests for Symbian Belle (`ekaarm`) native `datatype.library` (`ImageConversion.dll` `CImageDecoder`)
//! and SDK `imageconversion` ordinal table coverage.
use heirloom_common::datatype::{DecodedImage, NativeDatatype, DT_JPEG};
use heirloom_ekaarm_engine::cpu::Memory;
use heirloom_ekaarm_engine::drive::Drive;
use heirloom_ekaarm_engine::imageconv;
use heirloom_ekaarm_engine::loader;
use heirloom_ekaarm_engine::natives::NativeOutcome;
use heirloom_fmt_e32::OrdinalTable;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

struct CountingDatatype {
    calls: Arc<AtomicUsize>,
    w: u32,
    h: u32,
    color: u32,
}

impl NativeDatatype for CountingDatatype {
    fn identify(&self, data: &[u8]) -> u32 {
        if data.starts_with(&[0xFF, 0xD8, 0xFF]) { DT_JPEG } else { 0 }
    }
    fn decode_image(&self, data: &[u8], _max_w: u32, _max_h: u32) -> Option<DecodedImage> {
        if !data.starts_with(&[0xFF, 0xD8, 0xFF]) {
            return None;
        }
        self.calls.fetch_add(1, Ordering::SeqCst);
        Some(DecodedImage {
            width: self.w,
            height: self.h,
            pixels: vec![self.color; (self.w * self.h) as usize],
        })
    }
}

fn alloc_zeroed(p: &mut loader::Process, len: u32) -> u32 {
    let addr = p.heap.as_mut().unwrap().alloc(len, &mut p.mem).expect("alloc");
    for i in 0..len {
        p.mem.write_u8(addr + i, 0).unwrap();
    }
    addr
}

/// Allocate a guest `TBufC16` (EBufC = type 0 in top 4 bits, length in low 28 bits, followed by UTF-16LE).
fn alloc_des16(p: &mut loader::Process, s: &str) -> u32 {
    let units: Vec<u16> = s.encode_utf16().collect();
    let addr = alloc_zeroed(p, 4 + (units.len() as u32) * 2);
    p.mem.write_u32(addr, units.len() as u32).unwrap();
    for (i, &u) in units.iter().enumerate() {
        p.mem.write_u16(addr + 4 + (i as u32) * 2, u).unwrap();
    }
    addr
}

/// Allocate a guest `TBufC8` (EBufC = type 0, length in low 28 bits, followed by raw bytes).
fn alloc_des8(p: &mut loader::Process, bytes: &[u8]) -> u32 {
    let addr = alloc_zeroed(p, 4 + bytes.len() as u32);
    p.mem.write_u32(addr, bytes.len() as u32).unwrap();
    for (i, &b) in bytes.iter().enumerate() {
        p.mem.write_u8(addr + 4 + i as u32, b).unwrap();
    }
    addr
}

/// Allocate a writable `TBuf8<N>` (EBuf = type 3 in top 4 bits, length=0, max_len=N, followed by N bytes).
fn alloc_tbuf8(p: &mut loader::Process, cap: u32) -> u32 {
    let addr = alloc_zeroed(p, 8 + cap);
    p.mem.write_u32(addr, 3u32 << 28).unwrap(); // EBuf, len 0
    p.mem.write_u32(addr + 4, cap).unwrap();
    addr
}

#[test]
fn cimage_decoder_file_and_data_decode_via_native_datatype_into_fbs_bitmap() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut p = loader::bare(&[]).unwrap();
    let mut drive = Drive::new();
    drive.put_file("C:\\photo.jpg", vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10]).unwrap();
    p.host.drive = Some(drive);
    p.datatype_host = Some(Box::new(CountingDatatype {
        calls: calls.clone(),
        w: 12,
        h: 8,
        color: 0xFF_00_AA_22,
    }));

    // 1) CImageDecoder::FileNewL -> calls NativeDatatype::decode_image
    let path_des = alloc_des16(&mut p, "C:\\photo.jpg");
    p.cpu.regs[0] = 0x1000; // RFs&
    p.cpu.regs[1] = path_des;
    assert!(matches!(
        imageconv::native(&mut p, "_ZN13CImageDecoder8FileNewLER3RFsRK7TDesC16NS_8TOptionsE4TUidS5_S5_"),
        Some(NativeOutcome::Return)
    ));
    let dec = p.cpu.regs[0];
    assert_ne!(dec, 0);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(p.nat.imageconv.native_decodes, 1);

    // 2) FrameCount() == 1
    p.cpu.regs[0] = dec;
    assert!(matches!(imageconv::native(&mut p, "_ZNK13CImageDecoder10FrameCountEv"), Some(NativeOutcome::Return)));
    assert_eq!(p.cpu.regs[0], 1);

    // 3) FrameInfo(0) -> check ARM EABI TFrameInfo layout (iBitsPerPixel @24, iOverallSizeInPixels @44..52, iFrameDisplayMode @52, iFrameSizeInPixels @72..80)
    p.cpu.regs[0] = dec;
    p.cpu.regs[1] = 0;
    assert!(matches!(imageconv::native(&mut p, "_ZNK13CImageDecoder9FrameInfoEi"), Some(NativeOutcome::Return)));
    let fi = p.cpu.regs[0];
    assert_ne!(fi, 0);
    assert_eq!(p.mem.read_u32(fi + 24).unwrap(), 32, "TFrameInfo.iBitsPerPixel");
    assert_eq!(p.mem.read_u32(fi + 44).unwrap(), 12, "TFrameInfo.iOverallSizeInPixels.iWidth");
    assert_eq!(p.mem.read_u32(fi + 48).unwrap(), 8, "TFrameInfo.iOverallSizeInPixels.iHeight");
    assert_eq!(p.mem.read_u32(fi + 72).unwrap(), 12, "TFrameInfo.iFrameSizeInPixels.iWidth");
    assert_eq!(p.mem.read_u32(fi + 76).unwrap(), 8, "TFrameInfo.iFrameSizeInPixels.iHeight");

    // 4) Convert(TRequestStatus*, CFbsBitmap&, 0) -> populates CFbsBitmap pixels and completes status=KErrNone (0)
    let status = alloc_zeroed(&mut p, 8);
    let bmp = alloc_zeroed(&mut p, 16);
    p.cpu.regs[0] = dec;
    p.cpu.regs[1] = status;
    p.cpu.regs[2] = bmp;
    p.cpu.regs[3] = 0;
    assert!(matches!(
        imageconv::native(&mut p, "_ZN13CImageDecoder7ConvertEP14TRequestStatusR10CFbsBitmapi"),
        Some(NativeOutcome::Return)
    ));
    assert_eq!(p.mem.read_u32(status).unwrap(), 0, "TRequestStatus must complete with KErrNone");
    let fbs_bmp = p.nat.fbs.bitmaps.get(&bmp).expect("CFbsBitmap must be populated");
    assert_eq!((fbs_bmp.width, fbs_bmp.height), (12, 8));
    assert!(fbs_bmp.pixels.iter().all(|&px| px == 0x0000_AA22), "all 96 pixels must hold decoded color");

    // 5) GetMimeTypeFileL writes "image/jpeg"
    let mime_buf = alloc_tbuf8(&mut p, 64);
    p.cpu.regs[0] = 0x1000;
    p.cpu.regs[1] = path_des;
    p.cpu.regs[2] = mime_buf;
    assert!(matches!(
        imageconv::native(&mut p, "_ZN13CImageDecoder16GetMimeTypeFileLER3RFsRK7TDesC16R5TDes8"),
        Some(NativeOutcome::Return)
    ));
    let mime = heirloom_ekaarm_engine::desc::read8(&p, mime_buf).unwrap();
    assert_eq!(mime, b"image/jpeg");

    // 6) DataNewL decodes from in-memory TDesC8
    let data_des = alloc_des8(&mut p, &[0xFF, 0xD8, 0xFF, 0xDB, 0x00, 0x04]);
    p.cpu.regs[0] = 0x1000;
    p.cpu.regs[1] = data_des;
    assert!(matches!(
        imageconv::native(&mut p, "_ZN13CImageDecoder8DataNewLER3RFsRK6TDesC8NS_8TOptionsE4TUidS5_S5_"),
        Some(NativeOutcome::Return)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(p.nat.imageconv.native_decodes, 2);

    // 7) Destructor cleans up
    p.cpu.regs[0] = dec;
    assert!(matches!(imageconv::native(&mut p, "_ZN13CImageDecoderD1Ev"), Some(NativeOutcome::Return)));
    p.cpu.regs[0] = dec;
    assert!(matches!(imageconv::native(&mut p, "_ZNK13CImageDecoder10FrameCountEv"), Some(NativeOutcome::Return)));
    assert_eq!(p.cpu.regs[0], 0);

    // Negative control: without datatype_host, unknown JPEG data refuses cleanly instead of faking a bitmap
    let mut p2 = loader::bare(&[]).unwrap();
    let d2 = alloc_des8(&mut p2, &[0xFF, 0xD8, 0xFF, 0xE0]);
    p2.cpu.regs[0] = 0x1000;
    p2.cpu.regs[1] = d2;
    assert!(matches!(
        imageconv::native(&mut p2, "_ZN13CImageDecoder8DataNewLER3RFsRK6TDesC8NS_8TOptionsE4TUidS5_S5_"),
        Some(NativeOutcome::Refused(_))
    ));
}

#[test]
fn eka_ordinals_include_imageconversion_dso() {
    let tsv = include_str!("../../heirloom-fmt-e32/data/eka-ordinals.tsv");
    let tbl = OrdinalTable::parse(tsv).expect("parse eka-ordinals.tsv");
    let dll = "imageconversion{000a0000}[101f45ad].dll";
    let mut found_file_new = false;
    let mut found_convert = false;
    let mut found_frame_info = false;
    for ord in 1..=663u16 {
        if let Some(name) = tbl.name_of(dll, ord) {
            if name.starts_with("_ZN13CImageDecoder8FileNewLER3RFsRK7TDesC16") { found_file_new = true; }
            if name == "_ZN13CImageDecoder7ConvertEP14TRequestStatusR10CFbsBitmapi" { found_convert = true; }
            if name == "_ZNK13CImageDecoder9FrameInfoEi" { found_frame_info = true; }
        }
    }
    assert!(found_file_new && found_convert && found_frame_info, "imageconversion SDK ordinals must be present");
}

#[test]
fn tentry_ctor_copy_and_flags_and_execute_ld_witness() {
    let mut p = loader::bare(&[]).unwrap();
    let e1 = alloc_zeroed(&mut p, 544);
    let e2 = alloc_zeroed(&mut p, 544);
    p.mem.write_u32(e1, 0xDEAD_BEEF).unwrap();
    p.cpu.regs[0] = e1;
    assert!(matches!(heirloom_ekaarm_engine::fs::native(&mut p, "_ZN6TEntryC1Ev"), Some(NativeOutcome::Return)));
    assert_eq!(p.mem.read_u32(e1).unwrap(), 0, "TEntry::TEntry() must zero iAtt");

    p.mem.write_u32(e1, 0x10).unwrap(); // KEntryAttDir
    p.cpu.regs[0] = e1;
    assert!(matches!(heirloom_ekaarm_engine::fs::native(&mut p, "_ZNK6TEntry5IsDirEv"), Some(NativeOutcome::Return)));
    assert_eq!(p.cpu.regs[0], 1);

    p.cpu.regs[0] = e2;
    p.cpu.regs[1] = e1;
    assert!(matches!(heirloom_ekaarm_engine::fs::native(&mut p, "_ZN6TEntryaSERKS_"), Some(NativeOutcome::Return)));
    assert_eq!(p.mem.read_u32(e2).unwrap(), 0x10, "TEntry::operator= must copy iAtt");
}
