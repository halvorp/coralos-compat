//! UTCUtils (Mac OS 9 DateTimeUtils.h) by name: local <-> UTC seconds and DateTimes move by the machine location's gmtDelta.
use heirloom_macppc_engine::symbols::Binding;
use heirloom_macppc_engine::{GlueOutcome, MacPpc, PpcHost};
use heirloom_ui_abi::RecordingGuestUi;

type M = MacPpc<RecordingGuestUi>;
fn call(m: &mut M, name: &str, args: &[u32]) -> i32 {
    assert!(!matches!(m.bind_from("UTCUtils", name), Binding::Unbound(_)), "{name} must be bound");
    let mut regs = [0u32; 32];
    regs[3..3 + args.len()].copy_from_slice(args);
    assert_eq!(m.glue_call(name, &mut regs), GlueOutcome::Return, "{name}: {:?}", m.mac.refused);
    regs[3] as i32
}

#[test]
fn utc_conversions_follow_the_machine_location() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    // a fresh machine location is gmtDelta 0: local time IS UTC
    assert_eq!(call(&mut m, "ConvertUTCToLocalTime", &[1000, 0x3000]), 0);
    assert_eq!(m.mac.mem.read_u32(0x3000).unwrap(), 1000);
    // WriteLocation gmtDelta +3600 (British Summer Time): UTC 1000 is local 4600, and back
    let mut loc = [0u8; 12];
    loc[8..12].copy_from_slice(&3600u32.to_be_bytes());
    m.mac.mem.write_bytes(0x3100, &loc).unwrap();
    let mut regs = [0u32; 32];
    regs[3] = 0x3100;
    m.bind("WriteLocation");
    assert_eq!(m.glue_call("WriteLocation", &mut regs), GlueOutcome::Return);
    assert_eq!(call(&mut m, "ConvertUTCToLocalTime", &[1000, 0x3000]), 0);
    assert_eq!(m.mac.mem.read_u32(0x3000).unwrap(), 4600);
    assert_eq!(call(&mut m, "ConvertLocalTimeToUTC", &[4600, 0x3000]), 0);
    assert_eq!(m.mac.mem.read_u32(0x3000).unwrap(), 1000);
    // a negative delta (west of UTC) sign-extends from 24 bits: -18000 (New York winter)
    loc[8..12].copy_from_slice(&((-18000i32 as u32) & 0x00FF_FFFF).to_be_bytes());
    m.mac.mem.write_bytes(0x3100, &loc).unwrap();
    regs[3] = 0x3100;
    assert_eq!(m.glue_call("WriteLocation", &mut regs), GlueOutcome::Return);
    // UTCDateTime highSeconds 1, lowSeconds 0, fraction 0x8000 -> local = 2^32 - 18000, fraction kept
    m.mac.mem.write_bytes(0x3200, &[0, 1, 0, 0, 0, 0, 0x80, 0]).unwrap();
    assert_eq!(call(&mut m, "ConvertUTCToLocalDateTime", &[0x3200, 0x3300]), 0);
    let t = (1u64 << 32) - 18000;
    assert_eq!(m.mac.mem.read_u16(0x3300).unwrap(), (t >> 32) as u16);
    assert_eq!(m.mac.mem.read_u32(0x3302).unwrap(), t as u32);
    assert_eq!(m.mac.mem.read_u16(0x3306).unwrap(), 0x8000);
    // NEGATIVE: a NULL result pointer is paramErr
    assert_eq!(call(&mut m, "ConvertUTCToLocalTime", &[1, 0]), -50);
    assert_eq!(call(&mut m, "ConvertLocalToUTCDateTime", &[0x3200, 0]), -50);
}
