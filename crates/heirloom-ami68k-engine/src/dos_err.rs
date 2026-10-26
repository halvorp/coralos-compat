//! AmigaDOS error codes (values from AROS dos/dos.h, interface facts) and the secondary-error store (pr_Result2).
use heirloom_mem_flat32::GuestMem;

/// An AmigaDOS error code as IoErr() reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DosErr(pub u32);

pub const ERROR_NO_FREE_STORE: DosErr = DosErr(103);
pub const ERROR_LINE_TOO_LONG: DosErr = DosErr(120);
pub const ERROR_FILE_NOT_OBJECT: DosErr = DosErr(121);
pub const ERROR_OBJECT_IN_USE: DosErr = DosErr(202);
pub const ERROR_OBJECT_EXISTS: DosErr = DosErr(203);
pub const ERROR_OBJECT_NOT_FOUND: DosErr = DosErr(205);
pub const ERROR_ACTION_NOT_KNOWN: DosErr = DosErr(209);
pub const ERROR_INVALID_COMPONENT_NAME: DosErr = DosErr(210);
pub const ERROR_INVALID_LOCK: DosErr = DosErr(211);
pub const ERROR_OBJECT_WRONG_TYPE: DosErr = DosErr(212);
pub const ERROR_DISK_WRITE_PROTECTED: DosErr = DosErr(214);
pub const ERROR_RENAME_ACROSS_DEVICES: DosErr = DosErr(215);
pub const ERROR_DIRECTORY_NOT_EMPTY: DosErr = DosErr(216);
pub const ERROR_TOO_MANY_LEVELS: DosErr = DosErr(217);
pub const ERROR_DEVICE_NOT_MOUNTED: DosErr = DosErr(218);
pub const ERROR_SEEK_ERROR: DosErr = DosErr(219);
pub const ERROR_WRITE_PROTECTED: DosErr = DosErr(223);
pub const ERROR_NO_MORE_ENTRIES: DosErr = DosErr(232);
pub const ERROR_BAD_HUNK: DosErr = DosErr(235);

/// `struct Process.pr_Result2` (offset 148, see process.rs).
pub(crate) const PR_RESULT2: u32 = 148;

/// Store `e` as the program's secondary error (what IoErr() returns).
pub fn set_ioerr(mem: &mut GuestMem, e: DosErr) {
    // The running task's Process: ExecBase.ThisTask, as FindTask(0) would say.
    let task = mem.read_u32(crate::machine::EXEC_BASE + crate::sched::EXECBASE_THIS_TASK).unwrap_or(crate::process::PROCESS);
    let _ = mem.write_u32(task + PR_RESULT2, e.0);
}
pub const ERROR_DELETE_PROTECTED: DosErr = DosErr(222);
pub const ERROR_COMMENT_TOO_BIG: DosErr = DosErr(220);
pub const ERROR_READ_PROTECTED: DosErr = DosErr(224);
