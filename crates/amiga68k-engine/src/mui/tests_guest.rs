//! What a guest can ask of the MUI layer must never crash or exhaust the host: sizes it states, counts it passes and values it picks
//! are refused BY NAME (or computed without wrapping), never a panic, a runaway loop or an allocation the size of its claim.
use super::*;
use crate::Amiga;
use guestui_abi::{GuestRect, RecordingGuestUi, UiObjectRef};

fn machine() -> Amiga<RecordingGuestUi> { Amiga::new(RecordingGuestUi::new()).unwrap() }

fn tags(a: &mut Amiga<RecordingGuestUi>, items: &[(u32, u32)]) -> u32 {
    let p = a.heap.alloc(8 * (items.len() as u32 + 1)).unwrap();
    for (i, &(t, d)) in items.iter().chain([(0, 0)].iter()).enumerate() { a.mem.write_u32(p + 8 * i as u32, t).unwrap(); a.mem.write_u32(p + 8 * i as u32 + 4, d).unwrap(); }
    p
}
fn make(a: &mut Amiga<RecordingGuestUi>, class: &str, items: &[(u32, u32)]) -> Result<u32, crate::Refusal> { let t = tags(a, items); new_by_name(a, class, t) }
fn cstr(a: &mut Amiga<RecordingGuestUi>, s: &str) -> u32 { let p = a.heap.alloc(s.len() as u32 + 1).unwrap(); a.mem.write_bytes(p, s.as_bytes()).unwrap(); a.mem.write_u8(p + s.len() as u32, 0).unwrap(); p }
fn rect(w: i32, h: i32) -> GuestRect { GuestRect { left: 0, top: 0, width: w, height: h } }

#[test]
fn pushmethod_with_no_words_after_the_count_is_a_named_refusal_not_an_index_panic() {
    let mut a = machine();
    let app = make(&mut a, "Application.mui", &[]).unwrap();
    let e = method(&mut a, app, MUIM_Application_PushMethod, &[app, 0]).unwrap_err().0;
    assert!(e.contains("PushMethod"), "{e}");
    // CONTROL: a one-word message (the method id alone) gets past the count check.
    let r = method(&mut a, app, MUIM_Application_PushMethod, &[app, 1, MUIM_Application_NewInput]);
    assert!(!matches!(&r, Err(e) if e.0.contains("no method id")), "{r:?}");
}

#[test]
fn decreasing_a_numeric_by_i32_min_does_not_overflow() {
    let mut a = machine();
    let sl = make(&mut a, "Slider.mui", &[(MUIA_Numeric_Value, 40)]).unwrap();
    method(&mut a, sl, MUIM_Numeric_Decrease, &[i32::MIN as u32]).unwrap();
    method(&mut a, sl, MUIM_Numeric_Increase, &[i32::MIN as u32]).unwrap();
}

#[test]
fn an_imagedisplay_pen_fill_over_a_rectangle_too_big_to_count_is_refused_not_overflowed() {
    let mut a = machine();
    let spec = cstr(&mut a, "2:p5");
    let img = make(&mut a, "Imagedisplay.mui", &[(MUIA_Imagedisplay_Spec, spec)]).unwrap();
    let e = images::ops(&mut a, img, rect(i32::MAX, i32::MAX)).unwrap_err().0;
    assert!(e.contains("too large"), "{e}");
    // CONTROL: an ordinary rectangle draws.
    assert!(!images::ops(&mut a, img, rect(8, 8)).unwrap().is_empty());
}

#[test]
fn a_chunky_image_whose_geometry_overflows_or_exceeds_the_picture_bound_is_refused_by_name() {
    let mut a = machine();
    let px = a.heap.alloc(64).unwrap();
    let pal = a.heap.alloc(9).unwrap();
    let mk = |a: &mut Amiga<RecordingGuestUi>, w: u32, h: u32, modulo: u32| {
        make(a, "ChunkyImage.mui", &[(MUIA_ChunkyImage_Pixels, px), (MUIA_ChunkyImage_Palette, pal), (MUIA_ChunkyImage_NumColors, 3), (MUIA_ChunkyImage_Modulo, modulo), (MUIA_Bitmap_Width, w), (MUIA_Bitmap_Height, h)]).unwrap()
    };
    // row 1 starts at pixels + modulo: a modulo that wraps the 32-bit address space
    let wrap = mk(&mut a, 1, 2, 0xFFFF_FFF0);
    let e = images::ops(&mut a, wrap, rect(4, 4)).unwrap_err().0;
    assert!(e.contains("ChunkyImage"), "{e}");
    // a picture of 2^30 x 2^30 pixels: refused before a single pixel is read or any vector is grown
    let huge = mk(&mut a, 1 << 30, 1 << 30, 1 << 30);
    let e = images::ops(&mut a, huge, rect(4, 4)).unwrap_err().0;
    assert!(e.contains("too large"), "{e}");
}

#[test]
fn a_plane_the_host_refused_is_not_remembered_as_shown() {
    let mut a = machine();
    let spec = cstr(&mut a, "2:p5");
    let img = make(&mut a, "Imagedisplay.mui", &[(MUIA_Imagedisplay_Spec, spec)]).unwrap();
    let r = rect(8, 8);
    let ghost = UiObjectRef { id: 4242, generation: 1 }; // a window the host does not know: present_plane refuses
    assert!(images::present(&mut a, ghost, img, r).is_err());
    assert!(!a.objects.images.contains_key(&img), "nothing was shown, so nothing is cached as shown");
}
