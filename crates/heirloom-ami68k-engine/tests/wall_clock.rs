//! The host's wall clock reaches the guest: DateStamp "now" and the dates of the directories the machine made itself.
use heirloom_ami68k_engine::volume::AmigaDate;
use heirloom_ami68k_engine::{dos_misc, Amiga};
use heirloom_common::GuestMachine;
use heirloom_ui_abi::RecordingGuestUi;

/// 2026-10-26 07:33:20 (unix 1_793_000_000) as Amiga days since 1978-01-01.
const DAY_2026_10_26: i32 = 17_830;

fn ram_env_day(a: &Amiga<RecordingGuestUi>) -> i32 {
    let ram = a.fs.vol.device("RAM").unwrap();
    let env = a.fs.vol.find_child(ram, "Env").unwrap();
    a.fs.vol.get(env).date()[0]
}

#[test]
fn now_and_the_ram_directories_carry_the_hosts_date() {
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    // NEGATIVE CONTROL: the host's clock is what dates RAM:Env (before it, the directory carries no date of its own)
    assert_ne!(ram_env_day(&a), DAY_2026_10_26);
    a.set_wall_clock(1_793_000_000).unwrap();
    let now = dos_misc::now(&a);
    assert_eq!(now[0], DAY_2026_10_26);
    assert_eq!(now[1], 7 * 60 + 33, "minutes into the day");
    assert_eq!(ram_env_day(&a), DAY_2026_10_26);
}

#[test]
fn a_time_before_the_amiga_epoch_is_refused_by_name() {
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    let r = a.set_wall_clock(0).unwrap_err();
    assert!(r.0.contains("1978"), "{}", r.0);
}
