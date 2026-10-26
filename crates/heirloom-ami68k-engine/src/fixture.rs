//! An ORIGINAL tiny Hunk executable (hand-assembled m68k) used by tests and the runner.
//!
//! Program: OpenLibrary("intuition.library"); OpenWindow(NewWindow "Hello" 10,20,320x200);
//! store the handle in BSS; SetMenuStrip(window, Project{Open ^O, Quit ^Q}); CloseWindow;
//! CloseLibrary; rts.
use alloc::vec::Vec;
use alloc::vec;
use heirloom_fmt_hunk::writer::{be16, be32, build, Hunk};

/// Data-hunk builder: 4-aligned allocations, plus relocs for pointers into the same hunk.
struct Data { b: Vec<u8>, relocs: Vec<(u32, u32)> }
impl Data {
    fn alloc(&mut self, n: usize) -> u32 {
        while self.b.len() % 4 != 0 { self.b.push(0); }
        let at = self.b.len() as u32;
        self.b.resize(self.b.len() + n, 0);
        at
    }
    fn cstr(&mut self, s: &str) -> u32 {
        let at = self.alloc(s.len() + 1);
        self.b[at as usize..at as usize + s.len()].copy_from_slice(s.as_bytes());
        at
    }
    fn w16(&mut self, at: u32, v: u16) { self.b[at as usize..at as usize + 2].copy_from_slice(&v.to_be_bytes()); }
    fn w32(&mut self, at: u32, v: u32) { self.b[at as usize..at as usize + 4].copy_from_slice(&v.to_be_bytes()); }
    /// Store a pointer to offset `target` of this hunk (relocated at load time).
    fn ptr(&mut self, at: u32, target: u32) { self.w32(at, target); self.relocs.push((1, at)); }
}

const WFLAGS: u32 = 0x0F; // SIZEGADGET | DRAGBAR | DEPTHGADGET | CLOSEGADGET

/// `title_ptr_override`: write this raw value into NewWindow.Title instead of a relocated title.
pub fn build_hunk(title_ptr_override: Option<u32>) -> Vec<u8> {
    let mut d = Data { b: vec![], relocs: vec![] };
    let name = d.cstr("intuition.library");
    let nw = d.alloc(48);
    for (i, v) in [10u16, 20, 320, 200].into_iter().enumerate() { d.w16(nw + 2 * i as u32, v); }
    d.w32(nw + 14, WFLAGS);
    let title = d.cstr("Hello");
    match title_ptr_override { Some(raw) => d.w32(nw + 26, raw), None => d.ptr(nw + 26, title) }
    d.w16(nw + 46, 1); // Type = WBENCHSCREEN

    // Menu "Project" -> items "Open" (^O) and "Quit" (^Q); both ITEMTEXT|COMMSEQ|ITEMENABLED.
    let menu = d.alloc(30);
    let mname = d.cstr("Project");
    d.ptr(menu + 14, mname);
    d.w16(menu + 12, 1); // MENUENABLED
    let mut prev_next: Option<u32> = None;
    for (label, key) in [("Open", b'O'), ("Quit", b'Q')] {
        let it = d.alloc(34);
        let itext = d.alloc(20);
        let s = d.cstr(label);
        d.ptr(itext + 12, s);
        d.w16(it + 12, 0x0002 | 0x0004 | 0x0010);
        d.ptr(it + 18, itext);
        d.b[it as usize + 26] = key;
        match prev_next { None => d.ptr(menu + 18, it), Some(p) => d.ptr(p, it) }
        prev_next = Some(it);
    }

    // ---- code ----
    let (mut c, mut relocs) = (Vec::new(), vec![]);
    be16(&mut c, 0x2C78); be16(&mut c, 0x0004);                                   // movea.l 4.w,a6        ; ExecBase
    be16(&mut c, 0x43F9); relocs.push((1, c.len() as u32)); be32(&mut c, name);    // lea libname,a1
    be16(&mut c, 0x7000);                                                         // moveq #0,d0
    be16(&mut c, 0x4EAE); be16(&mut c, 0xFDD8);                                   // jsr -552(a6)          ; OpenLibrary
    be16(&mut c, 0x4A80);                                                         // tst.l d0
    let beq_at = c.len();
    be16(&mut c, 0x6700);                                                         // beq.s done (patched)
    be16(&mut c, 0x2A40);                                                         // movea.l d0,a5         ; keep lib base
    be16(&mut c, 0x2C40);                                                         // movea.l d0,a6
    be16(&mut c, 0x41F9); relocs.push((1, c.len() as u32)); be32(&mut c, nw);      // lea newwin,a0
    be16(&mut c, 0x4EAE); be16(&mut c, 0xFF34);                                   // jsr -204(a6)          ; OpenWindow
    be16(&mut c, 0x23C0); relocs.push((2, c.len() as u32)); be32(&mut c, 0);       // move.l d0,bssvar
    be16(&mut c, 0x2440);                                                         // movea.l d0,a2         ; window
    be16(&mut c, 0x204A);                                                         // movea.l a2,a0
    be16(&mut c, 0x43F9); relocs.push((1, c.len() as u32)); be32(&mut c, menu);    // lea menu,a1
    be16(&mut c, 0x4EAE); be16(&mut c, 0xFEF8);                                   // jsr -264(a6)          ; SetMenuStrip
    be16(&mut c, 0x204A);                                                         // movea.l a2,a0
    be16(&mut c, 0x4EAE); be16(&mut c, 0xFFB8);                                   // jsr -72(a6)           ; CloseWindow
    be16(&mut c, 0x224D);                                                         // movea.l a5,a1
    be16(&mut c, 0x2C78); be16(&mut c, 0x0004);                                   // movea.l 4.w,a6
    be16(&mut c, 0x4EAE); be16(&mut c, 0xFE62);                                   // jsr -414(a6)          ; CloseLibrary
    let skip = (c.len() - (beq_at + 2)) as u8;
    c[beq_at + 1] = skip;                                                         // done:
    be16(&mut c, 0x4E75);                                                         // rts

    build(&[Hunk::code(c, relocs), Hunk::data(d.b, d.relocs), Hunk::bss(4)], None)
}
