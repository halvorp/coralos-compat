//! The DOS device list driven by a compiled 68k program (tests/guest/c/doslist.c).
mod common;
use common::*;

const PROG: &[u8] = include_bytes!("guest/doslist.bin");

#[test]
fn the_dos_list_mirrors_the_volume_and_assigns_can_be_added_and_removed() {
    let mut a = machine();
    a.fs.vol.add_device("DOpus5", false);
    a.fs.vol.put_file("DOpus5:Libs/x.library", b"x".to_vec()).unwrap();
    a.fs.vol.assign_add("LIBS", "DOpus5:Libs");
    let r = run(&mut a, PROG);
    assert_eq!((r[0], r[1], r[2]), (9, 3, 3), "9 assigns, and 3 devices each with its volume node: SYS RAM DOpus5");
    assert_eq!(r[3], 1, "RAM: device node: DLT_DEVICE, BSTR name, a handler port, no lock");
    assert_eq!(r[4], 1, "RAM volume node: DLT_VOLUME, 'DOS\\0' disk type, a date, and the device's own port");
    assert_eq!((r[5], r[6]), (1, 1), "LIBS assign: DLT_DIRECTORY with a real lock on SYS:Libs");
    assert_eq!((r[7], r[8]), (1, 1), "the second directory hangs off dol_List as an AssignList node");
    assert_eq!((r[9], r[10], r[11]), (0, 0, 205), "RAM is not an assign; an unknown name is NULL with 205");
    assert_eq!((r[13], r[14]), (0, 232), "past the end: NULL with ERROR_NO_MORE_ENTRIES");
    assert_eq!((r[15], r[16]), (0, 1), "a write lock while read-locked is refused (NULL); after unlocking it is granted");
    assert_eq!(r[17], 1, "DosLibrary.dl_Root -> rn_Info -> di_DevInfo leads to the first entry, with no lock taken");
    assert_eq!((r[18], r[19], r[20]), (3, 1, 1), "GetDeviceProc on an assign: DVPF_ASSIGN with the first lock; the second call advances the same DevProc");
    assert_eq!((r[21], r[22]), (0, 232), "no third directory: NULL, ERROR_NO_MORE_ENTRIES");
    assert_eq!(r[23], 1, "GetDeviceProc on a device: its port, no lock, no flags");
    assert_eq!((r[24], r[25]), (0, 218), "an unmounted device: ERROR_DEVICE_NOT_MOUNTED");
    assert_eq!((r[26], r[27], r[28]), (u32::MAX, 0, 0), "IsFileSystem: RAM: yes; NIL: and an unmounted name no");
    assert_eq!((r[29], r[30]), (u32::MAX, 1), "AddDosEntry of an assign node makes TESTAS: resolve");
    assert_eq!((r[31], r[32]), (0, 203), "a second TESTAS (case-insensitive) is ERROR_OBJECT_EXISTS");
    assert_eq!((r[33], r[34]), (u32::MAX, 0), "RemDosEntry removes it: Lock(TESTAS:) is NULL again");
    assert_eq!((r[35], r[36]), (u32::MAX, 1), "Info's id_VolumeNode is the SYS volume node of the DOS list");
    assert!(a.fs.vol.assign_targets("TESTAS").is_none());
    assert_eq!(a.fs.dos.lock_counts(), (0, false), "every LockDosList was unlocked");
}
