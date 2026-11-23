//! QuickTime Movie Toolbox + Native `GraphicsImporter` (`Apple;Carbon;Multimedia` / `QuickTimeLib`).
//!
//! - **GraphicsImporter** (`GetGraphicsImporterForFile`, `GraphicsImportGetNaturalBounds`,
//!   `GraphicsImportGetBoundsRect`, `GraphicsImportSetBoundsRect`, `GraphicsImportSetGWorld`,
//!   `GraphicsImportGetGWorld`, `GraphicsImportDraw`, `GraphicsImportGetAsPicture`, `CloseComponent`):
//!   delegates directly to CoralOS's native `datatype.library` (`self.mac.datatype_host`) with
//!   pure-Rust fallback, blitting decoded ARGB32 pixels straight into QuickDraw `GWorld`/`GrafPort`!
//! - **Movie Toolbox** (`EnterMovies`, `NewMovie*`, `IsMovieDone`, ...): keeps the existing
//!   named refusal behavior (`invalidMovie`, -2010) when no movie decoder is attached.
use crate::clib::arg;
use crate::cpu::Cpu;
use crate::machine::MacPpc;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use heirloom_common::datatype::{decode_with_fallback, DecodedImage};
use heirloom_mac68k_engine::fm::fsspec::spec_loc;
use heirloom_mac68k_engine::qd::pictplay::draw_argb32_to_port;
use heirloom_mac68k_engine::qd::target::current_port;
use heirloom_ui_abi::GuestUi;

/// The library names a program imports QuickTime / ImageCompression from.
pub fn provided(lib: &str) -> bool {
    matches!(lib, "Apple;Carbon;Multimedia" | "QuickTimeLib")
}

/// `invalidMovie`: the movie is NIL / not a movie (Movies.h error).
pub const INVALID_MOVIE: i32 = -2010;
/// `cantFindHandler` / `badComponentInstance` errors (Components.h / ImageCompression.h).
const CANT_FIND_HANDLER: i32 = -9402;
const BAD_COMPONENT_INSTANCE: i32 = -2147450879; // 0x80008001
/// `gestaltQuickTimeVersion` ('qtim'): QuickTime 4.0 final as a NumVersion.
pub const QUICKTIME_4: u32 = 0x0400_8000;

struct ImporterInst {
    image: DecodedImage,
    bounds: [i16; 4], // top, left, bottom, right
    port: u32,
    gdevice: u32,
}

#[derive(Default)]
pub struct QtState {
    last_err: i32,
    refusals: u32,
    next_ci: u32,
    importers: BTreeMap<u32, ImporterInst>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QtFn {
    EnterMovies, ExitMovies, GetMoviesError, ClearMoviesStickyError, NewMovie, NewMovieFromFile, MovieImportFile, DisposeMovie,
    OpenMovieFile, CloseMovieFile, PrerollMovie, SetMovieRate, GetMovieTime,
    GetMovieBox, SetMovieBox, SetMovieGWorld, StartMovie, StopMovie, MoviesTask, IsMovieDone, GoToBeginningOfMovie, SetMovieVolume,
    GetGraphicsImporterForFile, GraphicsImportGetNaturalBounds, GraphicsImportGetBoundsRect, GraphicsImportSetBoundsRect,
    GraphicsImportSetGWorld, GraphicsImportGetGWorld, GraphicsImportDraw, GraphicsImportGetAsPicture, CloseComponent,
}

impl QtFn {
    pub fn by_name(name: &str) -> Option<QtFn> {
        use QtFn::*;
        Some(match name {
            "EnterMovies" => EnterMovies, "ExitMovies" => ExitMovies, "GetMoviesError" => GetMoviesError,
            "ClearMoviesStickyError" => ClearMoviesStickyError, "NewMovie" => NewMovie, "NewMovieFromFile" => NewMovieFromFile,
            "MovieImportFile" => MovieImportFile, "DisposeMovie" => DisposeMovie, "OpenMovieFile" => OpenMovieFile,
            "CloseMovieFile" => CloseMovieFile, "PrerollMovie" => PrerollMovie, "SetMovieRate" => SetMovieRate,
            "GetMovieTime" => GetMovieTime, "GetMovieBox" => GetMovieBox, "SetMovieBox" => SetMovieBox,
            "SetMovieGWorld" => SetMovieGWorld, "StartMovie" => StartMovie, "StopMovie" => StopMovie, "MoviesTask" => MoviesTask,
            "IsMovieDone" => IsMovieDone, "GoToBeginningOfMovie" => GoToBeginningOfMovie, "SetMovieVolume" => SetMovieVolume,
            "GetGraphicsImporterForFile" => GetGraphicsImporterForFile,
            "GraphicsImportGetNaturalBounds" => GraphicsImportGetNaturalBounds,
            "GraphicsImportGetBoundsRect" => GraphicsImportGetBoundsRect,
            "GraphicsImportSetBoundsRect" => GraphicsImportSetBoundsRect,
            "GraphicsImportSetGWorld" => GraphicsImportSetGWorld,
            "GraphicsImportGetGWorld" => GraphicsImportGetGWorld,
            "GraphicsImportDraw" => GraphicsImportDraw,
            "GraphicsImportGetAsPicture" => GraphicsImportGetAsPicture,
            "CloseComponent" => CloseComponent,
            _ => return None,
        })
    }
}

fn write_rect<U: GuestUi>(m: &mut MacPpc<U>, addr: u32, r: [i16; 4]) -> Result<(), String> {
    if addr == 0 {
        return Ok(());
    }
    for (i, &v) in r.iter().enumerate() {
        m.mac.mem.write_u16(addr + (i * 2) as u32, v as u16).map_err(|e| format!("qt rect: {e}"))?;
    }
    Ok(())
}

fn read_rect<U: GuestUi>(m: &MacPpc<U>, addr: u32) -> Result<[i16; 4], String> {
    let mut r = [0i16; 4];
    for (i, slot) in r.iter_mut().enumerate() {
        *slot = m.mac.mem.read_u16(addr + (i * 2) as u32).map_err(|e| format!("qt rect: {e}"))? as i16;
    }
    Ok(r)
}

impl<U: GuestUi> MacPpc<U> {
    pub(crate) fn qt_call(&mut self, f: QtFn, cpu: &mut Cpu) -> Result<(), String> {
        use QtFn::*;
        let a0 = arg(self, cpu, 0)?;
        let a1 = arg(self, cpu, 1)?;
        let a2 = arg(self, cpu, 2)?;
        let (ret, err, line): (u32, i32, Option<String>) = match f {
            EnterMovies => {
                (0, 0, Some(String::from("[qt] EnterMovies: QuickTime GraphicsImporter (native datatype.library) ready; movie playback refused by name")))
            }
            ExitMovies => (0, 0, None),
            GetMoviesError => (self.qt.last_err as u32, self.qt.last_err, None),
            ClearMoviesStickyError => (0, self.qt.last_err, None),
            NewMovie | NewMovieFromFile | MovieImportFile | OpenMovieFile => {
                self.qt.refusals += 1;
                if matches!(f, NewMovieFromFile | OpenMovieFile) && a0 != 0 {
                    if f == NewMovieFromFile {
                        let _ = self.mac.mem.write_u32(a0, 0);
                    } else if a1 != 0 {
                        let _ = self.mac.mem.write_u16(a1, 0);
                    }
                }
                let ret = if f == NewMovie { 0 } else { INVALID_MOVIE as u32 };
                (ret, INVALID_MOVIE, Some(format!("[qt] {f:?}: refused - this engine has no movie decoder (invalidMovie); the program goes on without the movie [QT-MOVIE REFUSED n={}]", self.qt.refusals)))
            }
            IsMovieDone => (1, INVALID_MOVIE, None),
            DisposeMovie | CloseMovieFile | PrerollMovie | SetMovieRate | GetMovieTime | GetMovieBox | SetMovieBox | SetMovieGWorld | StartMovie | StopMovie | MoviesTask | GoToBeginningOfMovie | SetMovieVolume => {
                (0, INVALID_MOVIE, None)
            }

            // OSErr GetGraphicsImporterForFile(const FSSpec *theFile, ComponentInstance *ci)
            GetGraphicsImporterForFile => {
                let raw = spec_loc(&self.mac, a0).ok().and_then(|l| l.node).and_then(|node| {
                    let len = self.mac.fm.tree.file_len(node)? as usize;
                    let mut buf = vec![0u8; len.min(32 * 1024 * 1024)];
                    let n = self.mac.fm.tree.read_at(node, 0, &mut buf)?;
                    buf.truncate(n);
                    Some(buf)
                });
                let decoded = raw.as_deref().and_then(|bytes| {
                    if let Some(h) = self.mac.datatype_host.as_deref() {
                        if let Some(img) = h.decode_image(bytes, 0, 0) {
                            self.mac.native_datatype_decodes += 1;
                            return Some((img, true));
                        }
                    }
                    decode_with_fallback(None, bytes, 0, 0).map(|img| (img, false))
                });
                if let Some((img, native)) = decoded {
                    self.qt.next_ci = self.qt.next_ci.wrapping_add(1).max(1);
                    let ci = 0x5154_0000 | self.qt.next_ci;
                    let (w, h) = (img.width.min(i16::MAX as u32) as i16, img.height.min(i16::MAX as u32) as i16);
                    self.qt.importers.insert(ci, ImporterInst { image: img, bounds: [0, 0, h, w], port: 0, gdevice: 0 });
                    if a1 != 0 {
                        let _ = self.mac.mem.write_u32(a1, ci);
                    }
                    let via = if native { "native-coral-datatype" } else { "builtin-fallback" };
                    (0, 0, Some(format!("[qt] GetGraphicsImporterForFile -> {ci:#x} ({w}x{h} via {via})")))
                } else {
                    if a1 != 0 {
                        let _ = self.mac.mem.write_u32(a1, 0);
                    }
                    (CANT_FIND_HANDLER as u32, CANT_FIND_HANDLER, Some(String::from("[qt] GetGraphicsImporterForFile -> cantFindHandler")))
                }
            }

            // ComponentResult GraphicsImportGetNaturalBounds(ComponentInstance ci, Rect *boundsRect)
            GraphicsImportGetNaturalBounds => {
                if let Some(inst) = self.qt.importers.get(&a0) {
                    let r = [0, 0, inst.image.height.min(i16::MAX as u32) as i16, inst.image.width.min(i16::MAX as u32) as i16];
                    write_rect(self, a1, r)?;
                    (0, 0, None)
                } else {
                    (BAD_COMPONENT_INSTANCE as u32, BAD_COMPONENT_INSTANCE, None)
                }
            }

            // ComponentResult GraphicsImportGetBoundsRect(ComponentInstance ci, Rect *boundsRect)
            GraphicsImportGetBoundsRect => {
                if let Some(inst) = self.qt.importers.get(&a0) {
                    let r = inst.bounds;
                    write_rect(self, a1, r)?;
                    (0, 0, None)
                } else {
                    (BAD_COMPONENT_INSTANCE as u32, BAD_COMPONENT_INSTANCE, None)
                }
            }

            // ComponentResult GraphicsImportSetBoundsRect(ComponentInstance ci, const Rect *boundsRect)
            GraphicsImportSetBoundsRect => {
                if self.qt.importers.contains_key(&a0) {
                    let r = if a1 != 0 { read_rect(self, a1)? } else { [0, 0, 0, 0] };
                    if let Some(inst) = self.qt.importers.get_mut(&a0) {
                        inst.bounds = r;
                    }
                    (0, 0, None)
                } else {
                    (BAD_COMPONENT_INSTANCE as u32, BAD_COMPONENT_INSTANCE, None)
                }
            }

            // ComponentResult GraphicsImportSetGWorld(ComponentInstance ci, CGrafPtr port, GDHandle gd)
            GraphicsImportSetGWorld => {
                if let Some(inst) = self.qt.importers.get_mut(&a0) {
                    inst.port = a1;
                    inst.gdevice = a2;
                    (0, 0, None)
                } else {
                    (BAD_COMPONENT_INSTANCE as u32, BAD_COMPONENT_INSTANCE, None)
                }
            }

            // ComponentResult GraphicsImportGetGWorld(ComponentInstance ci, CGrafPtr *port, GDHandle *gd)
            GraphicsImportGetGWorld => {
                if let Some(inst) = self.qt.importers.get(&a0) {
                    let (port, gd) = (inst.port, inst.gdevice);
                    if a1 != 0 { let _ = self.mac.mem.write_u32(a1, port); }
                    if a2 != 0 { let _ = self.mac.mem.write_u32(a2, gd); }
                    (0, 0, None)
                } else {
                    (BAD_COMPONENT_INSTANCE as u32, BAD_COMPONENT_INSTANCE, None)
                }
            }

            // ComponentResult GraphicsImportDraw(ComponentInstance ci)
            GraphicsImportDraw => {
                if let Some(inst) = self.qt.importers.get(&a0) {
                    let port = if inst.port != 0 { inst.port } else { current_port(&self.mac).unwrap_or(0) };
                    let (w, h) = (inst.image.width, inst.image.height);
                    let pixels = inst.image.pixels.clone();
                    let b = inst.bounds;
                    if port != 0 {
                        draw_argb32_to_port(&mut self.mac, port, w, h, &pixels, b).map_err(|r| r.0)?;
                    }
                    (0, 0, Some(format!("[qt] GraphicsImportDraw {a0:#x} ({w}x{h}) -> port {port:#x}")))
                } else {
                    (BAD_COMPONENT_INSTANCE as u32, BAD_COMPONENT_INSTANCE, None)
                }
            }

            // ComponentResult GraphicsImportGetAsPicture(ComponentInstance ci, PicHandle *picture)
            GraphicsImportGetAsPicture => {
                if let Some(inst) = self.qt.importers.get(&a0) {
                    // Build a minimal PICT v2 with DirectBitsRect ($009A, packType 1 raw 32-bit)
                    let (w, h) = (inst.image.width.min(i16::MAX as u32) as u16, inst.image.height.min(i16::MAX as u32) as u16);
                    let rb = w.saturating_mul(4);
                    let mut pic = alloc::vec::Vec::with_capacity(128 + inst.image.pixels.len() * 4);
                    let put16 = |v: &mut alloc::vec::Vec<u8>, x: u16| v.extend_from_slice(&x.to_be_bytes());
                    for x in [0u16, 0, 0, h, w, 0x0011, 0x02FF, 0x0C00] { put16(&mut pic, x); }
                    pic.extend_from_slice(&[0u8; 24]);
                    // DirectBitsRect ($009A)
                    for x in [0x009Au16, 0, 0x00FF, 0x8000 | rb, 0, 0, h, w] { put16(&mut pic, x); }
                    for x in [0u16, 1, 0, 0, 0x48, 0, 0x48, 0, 16, 32, 4, 8, 0, 0, 0, 0, 0, 0] { put16(&mut pic, x); }
                    for x in [0u16, 0, h, w, 0, 0, h, w, 0] { put16(&mut pic, x); }
                    for px in &inst.image.pixels { pic.extend_from_slice(&px.to_be_bytes()); }
                    if pic.len() & 1 != 0 { pic.push(0); }
                    put16(&mut pic, 0x00FF);
                    let hnd = self.mac.mm.new_handle(&mut self.mac.mem, &mut self.mac.heap, pic.len() as u32, false).map_err(|e| format!("PicHandle: {e}"))?;
                    let body = self.mac.mem.read_u32(hnd).map_err(|e| format!("PicHandle ptr: {e}"))?;
                    self.mac.mem.write_bytes(body, &pic).map_err(|e| format!("PicHandle bytes: {e}"))?;
                    if a1 != 0 {
                        let _ = self.mac.mem.write_u32(a1, hnd);
                    }
                    (0, 0, Some(format!("[qt] GraphicsImportGetAsPicture {a0:#x} ({w}x{h}) -> {hnd:#x}")))
                } else {
                    (BAD_COMPONENT_INSTANCE as u32, BAD_COMPONENT_INSTANCE, None)
                }
            }

            // OSErr CloseComponent(ComponentInstance aComponentInstance)
            CloseComponent => {
                self.qt.importers.remove(&a0);
                (0, 0, Some(format!("[qt] CloseComponent {a0:#x}")))
            }
        };
        if !matches!(f, GetMoviesError | ClearMoviesStickyError) {
            self.qt.last_err = err;
        } else if f == ClearMoviesStickyError {
            self.qt.last_err = 0;
        }
        if let Some(l) = line {
            self.mac.log.push(l);
        }
        cpu.gpr[3] = ret;
        Ok(())
    }
}
