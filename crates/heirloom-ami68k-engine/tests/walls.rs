//! The walls Directory Opus hit after CreateNewProc, one program (tests/guest/c/walls.c), every call with its negative control.
mod common;
use common::*;

const PROG: &[u8] = include_bytes!("guest/walls.bin");

#[test]
fn ports_commodities_asl_workbench_notify_dopkt_modes_flood_and_small_intuition_calls() {
    let mut a = machine();
    let r = run_refusing(&mut a, PROG);
    assert_eq!((r[0], r[1], r[2]), (1, 0, 0), "AddPort/FindPort/RemPort");
    assert_eq!((r[3], r[4], r[5], r[6], r[7]), (1, 6, 0, 2, 3), "CxBroker: object of type CX_BROKER; a unique duplicate is NULL with CBERR_DUP; a bad version CBERR_VERSION");
    assert_eq!((r[8], r[9]), (0, 1), "ActivateCxObj returns the previous state (brokers start inactive)");
    assert_eq!((r[10], r[11], r[12], r[13]), (0, 0x50, 0x38, u32::MAX), "ParseIX \"ctrl alt f1\" -> f1 with control and the alt synonym; trailing tokens are -1");
    assert_eq!((r[14], r[15], r[16]), (1, 0, 0), "AllocAslRequest takes the initial geometry from the tags; AslRequest answers FALSE (no UI); a font requester is refused -> NULL");
    assert_eq!((r[17], r[18]), (1, 0), "AddAppIconA registers; a NULL message port gives NULL");
    assert_eq!((r[19], r[20]), (u32::MAX, 1), "StartNotify registers and fills nr_FullName");
    assert_eq!((r[21], r[22]), (u32::MAX, 0x444F_5300), "DoPkt(ACTION_DISK_INFO) fills the InfoData (ID_DOS_DISK)");
    assert_eq!((r[23], r[24], r[25], r[26], r[27]), (0x29000, u32::MAX, 56, 0x8000_0000, 0), "GetVPModeID, INVALID_ID for NULL, DTAG_DISP record of 56 bytes, an unknown mode gives 0");
    assert_eq!((r[28], r[29], r[30], r[31], r[32]), (0, 1, 5, 0, 2), "Flood needs a TmpRas (FALSE without); inside filled, outside and the outline untouched");
    assert_eq!((r[33], r[34], r[35]), (8, 20505, 9), "ReadPixelLine8 / WritePixelLine8");
    assert_eq!((r[36], r[37]), (1, 1), "LockIBase, LockPubScreenList's first node is the Workbench screen");
    assert_eq!((r[38], r[39], r[40]), (1, 50_402_001, 50), "WindowLimits stores the limits; zeros keep them");
    assert_eq!(r[41], 30_401_201, "ChangeWindowBox moves and resizes");
    let want = [
        "RemPort", "RemMemHandler", "CxObjType", "FreeAslRequest", "AllocAslRequest", "AddAppWindowA", "RemoveAppIcon", "StartNotify", "EndNotify",
        "DoPkt", "DoPkt", "GetDisplayInfoData", "UnlockIBase", "ScreenToFront", "ChangeWindowBox",
    ];
    let got: Vec<&str> = a.refused.iter().map(|r| r.split(['/', ':']).nth(1).unwrap_or("")).collect();
    assert_eq!(got, want, "{:#?}", a.refused);
}
