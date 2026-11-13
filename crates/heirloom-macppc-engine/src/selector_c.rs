//! Stack-selector Toolbox routines by C name: the List Manager (`_Pack0`: `LNew`, `LAddRow`, `LClick` ...) and the styled TextEdit
//! dispatch (`_TEDispatch`: `TEGetHeight` ...). On a 68k these are ONE trap each whose routine is a selector word pushed last; the
//! 68k engine's selector bodies read the arguments off that frame. A PowerPC caller passes C arguments in r3..; Mixed Mode
//! ([`heirloom_mac68k_engine::package::by_value`]) builds the frame a 68k caller would have pushed and runs the SAME body, so a list
//! or a styled text record behaves identically from either processor. A routine whose body refuses is named in `refused` (the
//! program goes on with 0, as a 68k caller's frame is popped and goes on); the result register is the 68k result slot's value.
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use heirloom_mac68k_engine::package::by_value;
use heirloom_mac68k_engine::textedit::styled;
use heirloom_mac68k_engine::translate::lists::{self, Frame, Res};
use heirloom_ui_abi::GuestUi;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelFn {
    /// A `_Pack0` routine, by its frame in the List Manager's table.
    List(u16),
    /// A `_TEDispatch` routine, by selector.
    Te(u16),
}

impl SelFn {
    pub fn by_name(name: &str) -> Option<SelFn> {
        if let Some(f) = lists::frame_named(name) { return Some(SelFn::List(f.sel)); }
        styled::SEL_FRAMES.iter().find(|f| f.1 == name).map(|f| SelFn::Te(f.0))
    }
    pub fn name(self) -> &'static str {
        match self {
            SelFn::List(s) => list_frame(s).name,
            SelFn::Te(s) => styled::SEL_FRAMES[s as usize].1,
        }
    }
}

fn list_frame(sel: u16) -> &'static Frame { lists::FRAMES.iter().find(|f| f.sel == sel).expect("SelFn::List is made from the table") }

impl<U: GuestUi> MacPpc<U> {
    /// Run `f` with the C arguments in `regs`; the result goes to r3.
    pub(crate) fn sel_call(&mut self, f: SelFn, regs: &mut [u32; 32]) -> Result<(), String> {
        let (sel, layout) = match f { SelFn::List(s) => (s, list_frame(s).args), SelFn::Te(s) => (s, styled::SEL_ARGS[s as usize]) };
        let values = self.arg_values(layout.len(), regs)?;
        let out = match f {
            SelFn::List(_) => by_value(&mut self.mac, sel, layout, &values, |m, sp| lists::run_selector(m, sel, sp)).map(|(v, log)| {
                let r = match list_frame(sel).res { Res::Boolean => (v >> 8) & 0xFF, Res::Integer => v as u16 as i16 as i32 as u32, Res::Long => v, Res::None => 0 };
                (r, log)
            }),
            SelFn::Te(_) => by_value(&mut self.mac, sel, layout, &values, |m, sp| styled::run_selector(m, sel, sp)).map(|(ret, log)| (ret.value(), format!("[textedit] {log}"))),
        };
        regs[3] = match out {
            Ok((r, log)) => { self.mac.log.push(log); r }
            Err(e) => { self.mac.note_refusal(f.name(), e.0); 0 }
        };
        Ok(())
    }
}
