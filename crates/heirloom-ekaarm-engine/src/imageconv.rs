//! Symbian Image Conversion Library (`imageconversion.dll`, `CImageDecoder`) bridged directly
//! to CoralOS's native `datatype.library` (`heirloom_common::datatype::NativeDatatype`).
//!
//! Heavy Symbian S60/Belle apps (X-plore, Resco Photo Viewer, browsers, games) decode PNG, JPEG,
//! GIF, WebP, SVG, and BMP images through `CImageDecoder::FileNewL` / `DataNewL` -> `FrameInfo` ->
//! `Convert(TRequestStatus*, CFbsBitmap&, ...)`. Every call decodes natively via CoralOS's
//! out-of-process `datatype.library` workers and writes the `0x00RRGGBB` pixels directly into the
//! target `CFbsBitmap` in `FbsState`.
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_common::datatype::{
    decode_with_fallback, identify_magic, DecodedImage, DT_BMP, DT_GIF, DT_JPEG, DT_PNG, DT_SVG,
    DT_WEBP,
};
use crate::fbs::{FbsBitmap, ECOLOR_16MU};
use crate::loader::Process;
use crate::natives::NativeOutcome;

/// Host-side state of open `CImageDecoder` objects.
#[derive(Default)]
pub struct ImageConvState {
    pub decoders: BTreeMap<u32, DecoderObj>,
    pub native_decodes: u64,
}

pub struct DecoderObj {
    pub image: DecodedImage,
    pub frame_info_addr: u32,
    pub native_decoded: bool,
}

fn mime_for_kind(kind: u32) -> &'static [u8] {
    match kind {
        DT_PNG => b"image/png",
        DT_JPEG => b"image/jpeg",
        DT_GIF => b"image/gif",
        DT_WEBP => b"image/webp",
        DT_BMP => b"image/bmp",
        DT_SVG => b"image/svg+xml",
        _ => b"application/octet-stream",
    }
}

fn decode_bytes(p: &mut Process, raw: &[u8]) -> Option<(DecodedImage, bool)> {
    if let Some(h) = p.datatype_host.as_deref() {
        if let Some(img) = h.decode_image(raw, 0, 0) {
            p.nat.imageconv.native_decodes += 1;
            return Some((img, true));
        }
    }
    decode_with_fallback(None, raw, 0, 0).map(|img| (img, false))
}

fn write_frame_info(p: &mut Process, addr: u32, w: i32, h: i32) -> Result<(), String> {
    // TFrameInfo ARM EABI layout (icl/imagedata.h):
    // 0..16: TRect iFrameCoordsInPixels (0, 0, w, h)
    // 16..24: TSize iFrameSizeInTwips (w*15, h*15)
    // 24..28: TInt iBitsPerPixel (32)
    // 28..32: pad (for 8-byte aligned TInt64 iDelay)
    // 32..40: TTimeIntervalMicroSeconds iDelay (0)
    // 40..44: TUint32 iFlags (EColor=1 | EFullyScaleable=4)
    // 44..52: TSize iOverallSizeInPixels (w, h)
    // 52..56: TDisplayMode iFrameDisplayMode (EColor16MU=11)
    // 56..60: TRgb iBackgroundColor (0x00FFFFFF)
    // 60..64: iFrameDataOffset (0)
    // 64..68: iCurrentDataOffset (0)
    // 68..72: iCurrentFrameState (EFrameInfoProcessingComplete=3)
    // 72..80: TSize iFrameSizeInPixels (w, h)
    p.wr(addr + 0, 0)?;
    p.wr(addr + 4, 0)?;
    p.wr(addr + 8, w as u32)?;
    p.wr(addr + 12, h as u32)?;
    p.wr(addr + 16, (w * 15) as u32)?;
    p.wr(addr + 20, (h * 15) as u32)?;
    p.wr(addr + 24, 32)?;
    p.wr(addr + 28, 0)?;
    p.wr(addr + 32, 0)?;
    p.wr(addr + 36, 0)?;
    p.wr(addr + 40, 0x05)?; // EColor | EFullyScaleable
    p.wr(addr + 44, w as u32)?;
    p.wr(addr + 48, h as u32)?;
    p.wr(addr + 52, ECOLOR_16MU)?;
    p.wr(addr + 56, 0x00FF_FFFF)?;
    p.wr(addr + 60, 0)?;
    p.wr(addr + 64, 0)?;
    p.wr(addr + 68, 3)?;
    p.wr(addr + 72, w as u32)?;
    p.wr(addr + 76, h as u32)?;
    Ok(())
}

fn make_decoder(p: &mut Process, raw: &[u8], label: &str) -> Result<u32, String> {
    let Some((img, native)) = decode_bytes(p, raw) else {
        return Err(format!("CImageDecoder: cannot decode {label:?} ({} bytes)", raw.len()));
    };
    let (w, h) = (img.width as i32, img.height as i32);
    let obj = p.plain_object("CImageDecoder", 0x20)?;
    let fi = p.alloc_zeroed(80).ok_or_else(|| String::from("no guest heap for TFrameInfo"))?;
    write_frame_info(p, fi, w, h)?;
    p.nat.imageconv.decoders.insert(obj, DecoderObj { image: img, frame_info_addr: fi, native_decoded: native });
    let via = if native { "native-coral-datatype" } else { "builtin-fallback" };
    p.log.push(format!("[imageconv] CImageDecoder {label:?} ({w}x{h} via {via}) -> {obj:#x}"));
    Ok(obj)
}

fn read_guest_file(p: &Process, path: &str) -> Option<Vec<u8>> {
    p.host.drive.as_ref()?.read_owned(path).ok()
}

pub fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    if !name.starts_with("_ZN13CImageDecoder") && !name.starts_with("_ZNK13CImageDecoder") {
        return None;
    }
    let refuse = |m: String| Some(NativeOutcome::Refused(format!("NEXT-NEEDED ImageConversion {name}: {m}")));
    let r0 = p.cpu.regs[0];
    let r1 = p.cpu.regs[1];
    let r2 = p.cpu.regs[2];
    let r3 = p.cpu.regs[3];

    if name.starts_with("_ZN13CImageDecoder8FileNewLER3RFsRK7TDesC16") {
        let path = match crate::desc::read16(p, r1) {
            Ok(u) => crate::desc::to_utf8(&u),
            Err(e) => return refuse(e),
        };
        let Some(raw) = read_guest_file(p, &path) else {
            return refuse(format!("CImageDecoder::FileNewL {path:?} KErrNotFound (-1)"));
        };
        match make_decoder(p, &raw, &path) {
            Ok(obj) => {
                p.cpu.regs[0] = obj;
                return Some(NativeOutcome::Return);
            }
            Err(e) => return refuse(e),
        }
    }

    if name.starts_with("_ZN13CImageDecoder8DataNewLER3RFsRK6TDesC8") {
        let raw = match crate::desc::read8(p, r1) {
            Ok(b) => b,
            Err(e) => return refuse(e),
        };
        match make_decoder(p, &raw, "<buffer>") {
            Ok(obj) => {
                p.cpu.regs[0] = obj;
                return Some(NativeOutcome::Return);
            }
            Err(e) => return refuse(e),
        }
    }

    match name {
        // TInt CImageDecoder::FrameCount() const
        "_ZNK13CImageDecoder10FrameCountEv" => {
            p.cpu.regs[0] = if p.nat.imageconv.decoders.contains_key(&r0) { 1 } else { 0 };
            Some(NativeOutcome::Return)
        }
        // const TFrameInfo& CImageDecoder::FrameInfo(TInt aFrameNumber) const
        "_ZNK13CImageDecoder9FrameInfoEi" => {
            let Some(dec) = p.nat.imageconv.decoders.get(&r0) else {
                return refuse(format!("{r0:#x} is not a CImageDecoder"));
            };
            p.cpu.regs[0] = dec.frame_info_addr;
            Some(NativeOutcome::Return)
        }
        // void CImageDecoder::Convert(TRequestStatus* aRequestStatus, CFbsBitmap& aDestination, TInt aFrameNumber)
        // void CImageDecoder::Convert(TRequestStatus* aRequestStatus, CFbsBitmap& aDestination, CFbsBitmap& aDestinationMask, TInt aFrameNumber)
        "_ZN13CImageDecoder7ConvertEP14TRequestStatusR10CFbsBitmapi"
        | "_ZN13CImageDecoder7ConvertEP14TRequestStatusR10CFbsBitmapS3_i" => {
            let has_mask = name.ends_with("S3_i");
            let status_addr = r1;
            let dst_bmp_addr = r2;
            let mask_bmp_addr = if has_mask { r3 } else { 0 };
            let Some(img) = p.nat.imageconv.decoders.get(&r0).map(|d| d.image.clone()) else {
                return refuse(format!("{r0:#x} is not a CImageDecoder"));
            };
            let (w, h) = (img.width as i32, img.height as i32);
            let mut bmp = FbsBitmap::new(w, h, ECOLOR_16MU);
            bmp.pixels = img.pixels.iter().map(|&px| px & 0x00FF_FFFF).collect();
            p.nat.fbs.bitmaps.insert(dst_bmp_addr, bmp);
            if mask_bmp_addr != 0 {
                let mut mask = FbsBitmap::new(w, h, 4); // EGray256
                mask.pixels = img.pixels.iter().map(|&px| {
                    let a = (px >> 24) & 0xFF;
                    (a << 16) | (a << 8) | a
                }).collect();
                p.nat.fbs.bitmaps.insert(mask_bmp_addr, mask);
            }
            if status_addr != 0 {
                if !p.nat.requests.is_pending(status_addr) {
                    if let Err(e) = p.request_issue(status_addr) {
                        return refuse(e);
                    }
                }
                if let Err(e) = p.request_complete(status_addr, 0) {
                    return refuse(e);
                }
            }
            p.log.push(format!("[imageconv] Convert {r0:#x} -> CFbsBitmap {dst_bmp_addr:#x} ({w}x{h})"));
            Some(NativeOutcome::Return)
        }
        // void CImageDecoder::GetMimeTypeDataL(const TDesC8& aImageData, TDes8& aMimeType)
        "_ZN13CImageDecoder16GetMimeTypeDataLERK6TDesC8R5TDes8" => {
            let raw = match crate::desc::read8(p, r0) {
                Ok(b) => b,
                Err(e) => return refuse(e),
            };
            let kind = p.datatype_host.as_deref().map(|h| h.identify(&raw)).filter(|&k| k != 0).unwrap_or_else(|| identify_magic(&raw));
            if let Err(e) = crate::desc::write8(p, r1, mime_for_kind(kind)) {
                return refuse(e);
            }
            Some(NativeOutcome::Return)
        }
        // void CImageDecoder::GetMimeTypeFileL(RFs& aFs, const TDesC16& aFileName, TDes8& aMimeType)
        "_ZN13CImageDecoder16GetMimeTypeFileLER3RFsRK7TDesC16R5TDes8" => {
            let path = match crate::desc::read16(p, r1) {
                Ok(u) => crate::desc::to_utf8(&u),
                Err(e) => return refuse(e),
            };
            let Some(raw) = read_guest_file(p, &path) else {
                return refuse(format!("GetMimeTypeFileL {path:?} KErrNotFound (-1)"));
            };
            let kind = p.datatype_host.as_deref().map(|h| h.identify(&raw)).filter(|&k| k != 0).unwrap_or_else(|| identify_magic(&raw));
            if let Err(e) = crate::desc::write8(p, r2, mime_for_kind(kind)) {
                return refuse(e);
            }
            Some(NativeOutcome::Return)
        }
        "_ZN13CImageDecoder6CancelEv" => Some(NativeOutcome::Return),
        "_ZN13CImageDecoderD0Ev" | "_ZN13CImageDecoderD1Ev" | "_ZN13CImageDecoderD2Ev" => {
            p.nat.imageconv.decoders.remove(&r0);
            p.cpu.regs[0] = r0;
            Some(NativeOutcome::Return)
        }
        _ => None,
    }
}
