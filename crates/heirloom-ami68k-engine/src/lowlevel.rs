//! lowlevel.library (game-controller and keyboard polling, the Amiga's CD32 library; iGame opens it unconditionally and
//! polls `ReadJoyPort` from its main loop). The host machine has no game ports, so the honest answers are the
//! library's own "nothing here" values; the interrupt and timer-interrupt functions stay named refusals.
//!
//! Interface facts: vector numbers come from AROS workbench/libs/lowlevel/lowlevel.conf (generated into
//! `lvo_names`); `JP_TYPE_*` and `LANG_*` from libraries/lowlevel.h; `struct KeyQuery` is { UWORD kq_KeyCode; BOOL
//! kq_Pressed } with BOOL a 16-bit short in AROS's exec/types.h, so it is 4 bytes (m68k gcc, -mno-align-int).
use alloc::format;
use alloc::string::String;
use crate::libtable::LibraryBuilder;
use crate::{machine::Amiga, Refusal};
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_68k::Cpu;

const JP_TYPE_NOTAVAIL: u32 = 0;
const LANG_AMERICAN: u32 = 1;
const KEY_QUERY_SIZE: u32 = 4;
const KQ_PRESSED: u32 = 2;

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("ReadJoyPort", read_joy_port::<U>)
        .named("GetLanguageSelection", get_language_selection::<U>)
        .named("GetKey", get_key::<U>)
        .named("QueryKeys", query_keys::<U>)
}

/// ReadJoyPort(d0 = port) -> JP_TYPE_NOTAVAIL: no controller is attached to any port.
fn read_joy_port<U: GuestUi>(_: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let port = cpu.dreg(0);
    cpu.set_d0(JP_TYPE_NOTAVAIL);
    Ok(format!("[lowlevel] ReadJoyPort {port} -> JP_TYPE_NOTAVAIL (no game ports)"))
}

/// GetLanguageSelection() -> the user's language: the engine has no locale preferences, so American English.
fn get_language_selection<U: GuestUi>(_: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    cpu.set_d0(LANG_AMERICAN);
    Ok("[lowlevel] GetLanguageSelection -> LANG_AMERICAN".into())
}

/// GetKey() -> 0: no key event is waiting (the raw keyboard queue is not bridged to native key events).
fn get_key<U: GuestUi>(_: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    cpu.set_d0(0);
    Ok("[lowlevel] GetKey -> 0 (no key waiting)".into())
}

/// QueryKeys(a0 = KeyQuery array, d1 = count): every kq_Pressed is cleared, as no key is held.
fn query_keys<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (arr, n) = (cpu.areg(0), cpu.dreg(1) & 0xFF);
    for i in 0..n { a.mem.write_u16(arr + i * KEY_QUERY_SIZE + KQ_PRESSED, 0).map_err(Refusal::mem("QueryKeys.array"))?; }
    Ok(format!("[lowlevel] QueryKeys {n} keys -> none pressed"))
}
