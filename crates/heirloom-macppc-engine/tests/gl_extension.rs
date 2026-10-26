//! The Extensions folder's OpenGLEngine file states the GL version the engine truthfully claims, and only then (Warcraft III
//! checks this file before it starts; measured under real Mac OS 9.2.2, 2026-10-26).

use heirloom_fmt_resfork::writer::ForkBuilder;
use heirloom_gl_abi::version::Version;
use heirloom_macppc_engine::gl_extension::vers_resource;

#[test]
fn vers_resource_v1_2_bytes() {
    let b = vers_resource(Version::V1_2);
    assert_eq!(&b[..10], &[0x01, 0x20, 0x80, 0x00, 0x00, 0x00, 0x03, b'1', b'.', b'2']);
    assert_eq!(b[10], 24);
    assert_eq!(&b[11..], b"1.2, CoralOS Heirloom GL");
}

#[test]
fn vers_resource_round_trips_through_resource_fork() {
    let expected = vers_resource(Version::V1_2);
    let bytes = ForkBuilder::new()
        .add(b"vers", 1, None, &expected)
        .build();
    let fork = heirloom_fmt_resfork::ResFork::parse(bytes).unwrap();
    let index = fork.find(*b"vers", 1).expect("vers 1 should exist");
    let actual = fork.data(index).expect("vers 1 should have data");
    assert_eq!(actual, expected);
}

fn extensions_file(m: &mut heirloom_macppc_engine::MacPpc<heirloom_ui_abi::RecordingGuestUi>) -> Option<Vec<u8>> {
    let (vref, dir) = m.mac.fm.find_folder(0, *b"extn").unwrap();
    let vol = m.mac.fm.volume_of_vref(vref).unwrap();
    let dir = m.mac.fm.dir_node(vol, dir).unwrap();
    let f = m.mac.fm.tree.find_child(dir, "OpenGLEngine")?;
    Some(m.mac.fm.tree.get_mut(f).rsrc.clone())
}

#[test]
fn an_attested_version_installs_the_file_with_its_vers() {
    use heirloom_gl_abi::caps::Capabilities;
    let mut m = heirloom_macppc_engine::MacPpc::new(heirloom_ui_abi::RecordingGuestUi::new()).unwrap();
    let caps = Capabilities { version: Some(Version::V1_2), extensions: Vec::new() };
    assert_eq!(heirloom_macppc_engine::gl_extension::install_opengl_engine(&mut m.mac.fm, &caps), Ok(true));
    let rsrc = extensions_file(&mut m).expect("Extensions:OpenGLEngine exists");
    let fork = heirloom_fmt_resfork::ResFork::parse(rsrc).unwrap();
    let i = fork.find(*b"vers", 1).expect("'vers' 1");
    assert_eq!(&fork.data(i).unwrap()[..2], &[0x01, 0x20], "vers 1.2: what Warcraft III compares against 0x0120");
}

#[test]
fn no_attested_version_creates_nothing() {
    use heirloom_gl_abi::caps::Capabilities;
    let mut m = heirloom_macppc_engine::MacPpc::new(heirloom_ui_abi::RecordingGuestUi::new()).unwrap();
    let caps = Capabilities { version: None, extensions: Vec::new() };
    let before = extensions_file(&mut m).is_some();
    assert_eq!(heirloom_macppc_engine::gl_extension::install_opengl_engine(&mut m.mac.fm, &caps), Ok(false));
    assert_eq!(extensions_file(&mut m).is_some(), before, "nothing is created when nothing is claimed");
}

/// THE HONESTY TEST: a machine built the normal way carries the file IF AND ONLY IF the GL table attests a version — the
/// file never says more than glGetString(GL_VERSION). Today (ATTESTED_THROUGH = None) it has none.
#[test]
fn the_file_exists_exactly_when_the_gl_layer_claims_a_version() {
    let claimed = heirloom_gl_abi::caps::derive(heirloom_gl_abi::ENTRIES, heirloom_gl_abi::caps::ATTESTED_THROUGH).version;
    let mut m = heirloom_macppc_engine::MacPpc::new(heirloom_ui_abi::RecordingGuestUi::new()).unwrap();
    assert_eq!(extensions_file(&mut m).is_some(), claimed.is_some());
}
