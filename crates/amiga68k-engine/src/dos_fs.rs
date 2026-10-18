//! Registration of the filesystem half of dos.library (dos_files, dos_io, dos_path, loadseg).
use crate::libtable::{Handler, LibraryBuilder};
use crate::{dos_misc as misc, dos_assign as asg, dos_files as f, dos_io as io, dos_path as p, loadseg as l};
use guestui_abi::GuestUi;

/// (LVO, name, handler). The LVOs are checked against the generated `lvo_names` table by a test.
pub fn table<U: GuestUi>() -> alloc::vec::Vec<(i32, &'static str, Handler<U>)> {
    alloc::vec![
        (-30, "Open", io::open::<U> as Handler<U>), (-36, "Close", io::close::<U>), (-66, "Seek", io::seek::<U>),
        (-72, "DeleteFile", f::delete_file::<U>), (-78, "Rename", f::rename::<U>), (-84, "Lock", f::lock::<U>),
        (-90, "UnLock", f::unlock::<U>), (-96, "DupLock", f::dup_lock::<U>), (-102, "Examine", f::examine::<U>),
        (-108, "ExNext", f::ex_next::<U>), (-120, "CreateDir", f::create_dir::<U>), (-126, "CurrentDir", f::current_dir::<U>),
        (-150, "LoadSeg", l::load_seg::<U>), (-156, "UnLoadSeg", l::unload_seg::<U>),
        (-180, "SetComment", f::set_comment::<U>), (-186, "SetProtection", f::set_protection::<U>),
        (-210, "ParentDir", f::parent_dir::<U>), (-228, "AllocDosObject", f::alloc_dos_object::<U>),
        (-234, "FreeDosObject", f::free_dos_object::<U>), (-372, "DupLockFromFH", io::dup_lock_from_fh::<U>),
        (-378, "OpenFromLock", io::open_from_lock::<U>), (-390, "ExamineFH", io::examine_fh::<U>),
        (-402, "NameFromLock", f::name_from_lock::<U>), (-408, "NameFromFH", io::name_from_fh::<U>),
        (-420, "SameLock", f::same_lock::<U>), (-456, "SetFileSize", io::set_file_size::<U>),
        (-564, "GetCurrentDirName", f::get_current_dir_name::<U>), (-612, "AssignLock", asg::assign_lock::<U>), (-618, "AssignLate", asg::assign_path::<U>), (-624, "AssignPath", asg::assign_path::<U>), (-630, "AssignAdd", asg::assign_add::<U>), (-636, "RemAssignList", asg::rem_assign_list::<U>),
        (-594, "SetProgramDir", f::set_program_dir::<U>), (-600, "GetProgramDir", f::get_program_dir::<U>), (-768, "NewLoadSeg", l::load_seg::<U>),
        (-114, "Info", misc::info::<U>), (-192, "DateStamp", misc::date_stamp::<U>), (-396, "SetFileDate", misc::set_file_date::<U>), (-738, "CompareDates", misc::compare_dates::<U>),
        (-306, "FGetC", misc::fgetc::<U>), (-312, "FPutC", misc::fputc::<U>), (-318, "UnGetC", misc::ungetc::<U>), (-324, "FRead", misc::fread::<U>), (-330, "FWrite", misc::fwrite::<U>), (-336, "FGets", misc::fgets::<U>),
        (-870, "FilePart", p::file_part_lvo::<U>), (-876, "PathPart", p::path_part_lvo::<U>), (-882, "AddPart", p::add_part_lvo::<U>),
    ]
}

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    table::<U>().into_iter().fold(b, |b, (lvo, name, h)| b.func(lvo, name, h))
}

#[cfg(test)]
mod tests {
    use super::*;
    use guestui_abi::RecordingGuestUi;

    #[test]
    fn every_registered_lvo_matches_the_generated_table() {
        let dos = crate::lvo_names::LIBRARIES.iter().find(|(n, _)| *n == "dos.library").unwrap().1;
        for (lvo, name, _) in table::<RecordingGuestUi>() {
            assert!(dos.contains(&(lvo, name)), "{name} at {lvo} is not in lvo_names");
        }
    }
}
