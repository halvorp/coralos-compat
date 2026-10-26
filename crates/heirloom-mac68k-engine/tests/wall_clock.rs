//! The host's wall clock reaches the guest: the `Time` global is the host's local time in Mac seconds (1904 epoch).
use heirloom_common::GuestMachine;
use heirloom_mac68k_engine::{lowmem, Mac};
use heirloom_ui_abi::RecordingGuestUi;

#[test]
fn time_is_the_hosts_local_time_in_mac_seconds() {
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    let before = m.mem.read_u32(lowmem::TIME).unwrap();
    m.set_wall_clock(1_793_000_000).unwrap();
    let t = m.mem.read_u32(lowmem::TIME).unwrap();
    assert_eq!(i64::from(t), 1_793_000_000 + 2_082_844_800);
    assert_ne!(before, t, "NEGATIVE CONTROL: the engine's default date is not the host's");
}

#[test]
fn a_time_past_2040_is_refused_by_name() {
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    let r = m.set_wall_clock(2_300_000_000).unwrap_err();
    assert!(r.0.contains("1904-2040"), "{}", r.0);
}
