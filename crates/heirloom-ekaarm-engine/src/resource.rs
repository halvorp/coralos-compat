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
    pub fn app_resource(&self, id: u32) -> Result<Vec<u8>, String> {
        let drive = self.host.drive.as_ref().ok_or_else(|| String::from("no drive: the application's resource file cannot be read (run with --drive)"))?;
        let path = app_resource_path(drive, self.uid3)?;
        let bytes = drive.read(&path).map_err(|e| format!("{path}: {e:?}"))?;
        let file = RscFile::parse(bytes).map_err(|e| format!("{path}: {e:?}"))?;
        file.resource(id).map_err(|e| format!("{path}: resource {id:#x}: {e:?}"))
    }
}

/// The language the device runs in: British English (`.r01`), the project's default locale. A ROM resource file `name.rsc` has its
/// localised twin `name.r01`; the twin is the one a device in this language loads.
const ROM_LANGUAGE_EXT: &str = "r01";

impl Process {
    /// Resource `id` wherever it lives: the application's own file when `id`'s offset (its high 20 bits) is that file's, else the
    /// ROM file under `\resource` that owns the offset. Each failure says what was missing.
    pub fn resource(&mut self, id: u32) -> Result<Vec<u8>, String> {
        let offset = id & 0xFFFF_F000;
        if offset == 0 {
            return self.app_resource(id);
        }
        if let Ok(r) = self.app_resource(id) {
            return Ok(r);
        }
        let path = self.rom_resource_for(offset)?;
        let drive = self.host.drive.as_ref().ok_or_else(|| String::from("no drive"))?;
        let bytes = drive.read_owned(&path).map_err(|e| format!("{path}: {e:?}"))?;
        let file = RscFile::parse(&bytes).map_err(|e| format!("{path}: {e:?}"))?;
        file.resource(id).map_err(|e| format!("{path}: resource {id:#x}: {e:?}"))
    }

    /// The ROM resource file whose id offset is `offset`, found by reading the signature record of each `\resource\*.rsc` once.
    fn rom_resource_for(&mut self, offset: u32) -> Result<String, String> {
        if self.nat.rom_resources.is_none() {
            let drive = self.host.drive.as_ref().ok_or_else(|| String::from("no drive"))?;
            let mut index = alloc::collections::BTreeMap::new();
            index_rom_dir(drive, "\\resource", &mut index);
            self.nat.rom_resources = Some(index);
        }
        match self.nat.rom_resources.as_ref().and_then(|i| i.get(&offset)) {
            Some(p) => Ok(p.clone()),
            None => Err(format!("no resource file on the drive or in the ROM owns resource offset {offset:#x}")),
        }
    }
}

/// Index every `.rsc` under the ROM directory `dir` and its subdirectories (`\resource\uiklaf\eikpriv.rsc` owns the Eikon private
/// resources, offset 0x19bd6000): id offset -> the file a device in [`ROM_LANGUAGE_EXT`] loads. The first file seen keeps an offset.
fn index_rom_dir(drive: &Drive, dir: &str, index: &mut alloc::collections::BTreeMap<u32, String>) {
    for name in drive.list_rom(dir).unwrap_or_default() {
        let full = format!("{dir}\\{name}");
        if drive.list_rom(&full).is_some() {
            index_rom_dir(drive, &full, index);
            continue;
        }
        let lower = name.to_ascii_lowercase();
        let Some(stem) = lower.strip_suffix(".rsc") else { continue };
        let twin = format!("{dir}\\{stem}.{ROM_LANGUAGE_EXT}");
        let path = if drive.exists_anywhere(&twin) { twin } else { full };
        if let Ok(bytes) = drive.read_owned(&path) {
            if let Ok(f) = RscFile::parse(&bytes) {
                index.entry(f.id_offset).or_insert(path);
            }
        }
    }
}
