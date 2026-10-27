//! Resource files off the drive: the application registration, and the application's own resource file (the one its
//! `CEikonEnv` resource reads address, `CCoeEnv::CreateResourceReaderLC` and friends).
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_fmt_rsc::{Reader, RscFile};
use crate::drive::Drive;
use crate::loader::Process;

const REG_DIR: &str = "\\private\\10003a3f\\import\\apps";

/// The fields of an `APP_REGISTRATION_INFO` (appinfo.rh:83-92: reserved LONG, LLINK, `app_file`, `attributes`,
/// `localisable_resource_file`, `localisable_resource_id`) the engine uses.
pub struct Registration {
    pub path: String,
    pub app_file: String,
    pub localisable_file: String,
    pub localisable_id: u32,
}

/// The registration resource of the application whose UID3 is `uid3`.
pub fn registration(drive: &Drive, uid3: u32) -> Result<Registration, String> {
    let names = drive.list(REG_DIR).map_err(|e| format!("no registration directory {REG_DIR} on the drive ({e:?})"))?;
    for name in names.iter().filter(|n| n.to_ascii_lowercase().ends_with("_reg.rsc")) {
        let path = format!("{REG_DIR}\\{name}");
        let bytes = drive.read(&path).map_err(|e| format!("{path}: {e:?}"))?;
        let reg = RscFile::parse(bytes).map_err(|e| format!("{path}: {e:?}"))?;
        if reg.uid3 != uid3 {
            continue;
        }
        let res = reg.resource(1).map_err(|e| format!("{path}: resource 1: {e:?}"))?;
        let mut r = Reader::new(&res);
        let field = |e| format!("{path}: APP_REGISTRATION_INFO: {e:?}");
        r.long().map_err(field)?;
        r.long().map_err(field)?;
        let app_file = r.ltext().map_err(field)?;
        r.long().map_err(field)?; // attributes
        let localisable_file = r.ltext().map_err(field)?;
        let localisable_id = r.long().map_err(field)?;
        return Ok(Registration { path, app_file, localisable_file, localisable_id });
    }
    Err(format!("no *_reg.rsc under {REG_DIR} has UID3 {uid3:#010x}"))
}

/// Where the application's own resource file is: `CEikApplication::ResourceFileName()` (eikapp.h:51) defaults to
/// `\resource\apps\<application file name>.rsc`. DELETE WHEN a corpus application overrides `ResourceFileName` (then call the
/// virtual through the callback machine instead of deriving the default).
pub fn app_resource_path(drive: &Drive, uid3: u32) -> Result<String, String> {
    let reg = registration(drive, uid3)?;
    let file = reg.app_file.rsplit(['\\', '/']).next().unwrap_or("");
    let stem = file.rsplit_once('.').map_or(file, |(s, _)| s);
    if stem.is_empty() {
        return Err(format!("{}: the registration names no application file", reg.path));
    }
    Ok(format!("\\resource\\apps\\{stem}.rsc"))
}

impl Process {
    /// Resource `id` of the application's own resource file, decoded.
    pub(crate) fn app_resource(&self, id: u32) -> Result<Vec<u8>, String> {
        let drive = self.host.drive.as_ref().ok_or_else(|| String::from("no drive: the application's resource file cannot be read (run with --drive)"))?;
        let path = app_resource_path(drive, self.uid3)?;
        let bytes = drive.read(&path).map_err(|e| format!("{path}: {e:?}"))?;
        let file = RscFile::parse(bytes).map_err(|e| format!("{path}: {e:?}"))?;
        file.resource(id).map_err(|e| format!("{path}: resource {id:#x}: {e:?}"))
    }
}
