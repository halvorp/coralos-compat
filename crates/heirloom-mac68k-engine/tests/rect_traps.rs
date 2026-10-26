//! QuickDraw's rect/point utilities through real Pascal frames.
use heirloom_ui_abi::RecordingGuestUi;
use heirloom_mac68k_engine::fixture::RECTTEST;
use heirloom_mac68k_engine::Mac;

#[test]
fn rect_utilities_use_the_pascal_frame_correctly() {
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    let r = m.run_program(RECTTEST, 0, 10_000).unwrap();
    assert!(r.stats.halted && r.stats.bus_faults == 0 && m.refused.is_empty(), "{:?} {:#?}", m.refused, m.log);
    let rect = |a: u32| [0, 2, 4, 6].map(|o| m.mem.read_u16(a + o).unwrap() as i16);
    // SetRect(left 20, top 10, right 220, bottom 110) -> memory order top,left,bottom,right; InsetRect(5,7); OffsetRect(1,2)
    assert_eq!(rect(0x3000), [10 + 7 + 2, 20 + 5 + 1, 110 - 7 + 2, 220 - 5 + 1]);
    assert_eq!(rect(0x3010), [50, 100, 500, 500], "second SetRect: argument order is left, top, right, bottom");
    assert_eq!(rect(0x3020), [50, 100, 105, 216], "SectRect");
    assert_eq!(m.mem.read_u16(0x3030).unwrap(), 0x0100, "SectRect -> TRUE");
    assert_eq!(m.mem.read_u16(0x3032).unwrap(), 0x0100, "PtInRect inside");
    assert_eq!(m.mem.read_u16(0x3034).unwrap(), 0, "PtInRect outside (NEGATIVE)");
    assert_eq!(m.mem.read_u16(0x3036).unwrap(), 0x0100, "EmptyRect of an all-zero rect");
    assert_eq!(m.mem.read_u32(0x3050).unwrap(), (13 << 16) + 24, "AddPt: v and h both added");
}
