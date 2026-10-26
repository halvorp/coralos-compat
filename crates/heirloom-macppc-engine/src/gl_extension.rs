//! OpenGLEngine Extensions file — truthfully advertises GL version via 'vers' 1 resource.
use alloc::format;
use alloc::vec::Vec;
use heirloom_fmt_resfork::writer::ForkBuilder;
use heirloom_gl_abi::caps::Capabilities;
use heirloom_gl_abi::version::Version;
use heirloom_mac68k_engine::fm::FileMgr;

/// Build a Mac 'vers' resource (Inside Macintosh: Toolbox Essentials layout).
/// Header: majorRev(u8 BCD), minorAndBugRev(u8, minor high nibble BCD, bug low nibble 0),
/// stage(0x80 final), nonRelRev(0), countryCode(u16 BE 0).
/// Then Pascal string "<major>.<minor>", then Pascal string "<major>.<minor>, CoralOS Heirloom GL".
pub fn vers_resource(v: Version) -> Vec<u8> {
    let (major, minor) = v.pair();
    let mut out = Vec::with_capacity(40);
    out.push(major);              // majorRev (BCD)
    // BCD limitation: the high nibble represents minor only while minor < 10.
    out.push(minor << 4);         // minorAndBugRev (minor high nibble, bug=0 low)
    out.push(0x80);               // stage = final
    out.push(0x00);               // nonRelRev
    out.extend_from_slice(&[0x00, 0x00]); // countryCode (BE 0)
    let short = format!("{}.{}", major, minor);
    push_pascal(&mut out, &short);
    let long = format!("{}, CoralOS Heirloom GL", short);
    push_pascal(&mut out, &long);
    out
}

fn push_pascal(out: &mut Vec<u8>, s: &str) {
    let b = s.as_bytes();
    let len = b.len().min(255) as u8;
    out.push(len);
    out.extend_from_slice(&b[..len as usize]);
}

/// Install OpenGLEngine into the boot volume's Extensions folder.
/// Returns Ok(true) if installed, Ok(false) if no version is attested (file NOT created).
///
/// This is called once for a fresh boot volume. Replacing an existing installation would
/// require a read-only file-by-name lookup before `put_file` creates another node.
pub fn install_opengl_engine(fm: &mut FileMgr, caps: &Capabilities) -> Result<bool, i16> {
    let v = match caps.version {
        Some(v) => v,
        None => return Ok(false),
    };
    let (vref, dir_id) = fm.find_folder(0, *b"extn")?;
    let vol = fm.volume_of_vref(vref)?;
    let dir = fm.dir_node(vol, dir_id)?;
    let file_id = fm.tree
        .put_file(dir, "OpenGLEngine", Vec::new())
        .map_err(|_| -1i16)?;
    let node = fm.tree.get_mut(file_id);
    node.finfo.file_type = *b"shlb";
    node.finfo.creator = *b"????";
    node.rsrc = ForkBuilder::new()
        .add(b"vers", 1, None, &vers_resource(v))
        .build();
    Ok(true)
}