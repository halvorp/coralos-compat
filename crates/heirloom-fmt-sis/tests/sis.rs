use heirloom_fmt_sis::{Package, SisError};

fn sized(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    while out.len() % 4 != 0 {
        out.push(0);
    }
    out
}

fn field(kind: u32, data: &[u8]) -> Vec<u8> {
    let mut out = (kind as u32).to_le_bytes().to_vec();
    out.extend_from_slice(&sized(data));
    out
}

fn array(kind: u32, elements: &[Vec<u8>]) -> Vec<u8> {
    let mut body = kind.to_le_bytes().to_vec();
    for element in elements {
        body.extend_from_slice(&sized(element));
    }
    field(2, &body)
}

fn utf16(value: &str) -> Vec<u8> {
    value
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>()
}

fn string_field(value: &str) -> Vec<u8> {
    field(1, &utf16(value))
}

fn compressed(bytes: &[u8]) -> Vec<u8> {
    let mut body = 0u32.to_le_bytes().to_vec();
    body.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    body.extend_from_slice(bytes);
    field(3, &body)
}

/// SISFileDescription in the package's order: Target, MIMEType, Hash, Operation, OperationOptions, Length,
/// UncompressedLength, FileIndex (Capabilities, optional, is left out as real packages do).
fn file_description(target: &str, index: u32) -> Vec<u8> {
    let mut body = string_field(target);
    body.extend_from_slice(&string_field("application/octet-stream"));
    body.extend_from_slice(&field(25, &[]));
    body.extend_from_slice(&0u32.to_le_bytes());
    body.extend_from_slice(&0u32.to_le_bytes());
    body.extend_from_slice(&0u64.to_le_bytes());
    body.extend_from_slice(&0u64.to_le_bytes());
    body.extend_from_slice(&index.to_le_bytes());
    body
}

fn package() -> Vec<u8> {
    // SISData = Array<SISDataUnit>; unit 0 = Array<SISFileData>; FileIndex indexes unit 0.
    let first_data = compressed(b"first file");
    let second_data = compressed(b"second file");
    let unit = array(32, &[first_data, second_data]);
    let data = field(30, &array(31, &[unit]));

    // InstallBlock = Array<FileDescription>, Array<Controller>, Array<If>; If = Expression, InstallBlock, Array<ElseIf>.
    let mut conditional = field(29, &[]);
    let mut inner = array(24, &[file_description("sys\\bin\\second.exe", 1)]);
    inner.extend_from_slice(&array(13, &[]));
    inner.extend_from_slice(&array(26, &[]));
    conditional.extend_from_slice(&field(28, &inner));
    conditional.extend_from_slice(&array(27, &[]));
    let mut install = array(24, &[file_description("sys\\bin\\first.exe", 0)]);
    install.extend_from_slice(&array(13, &[]));
    install.extend_from_slice(&array(26, &[conditional]));

    // SISInfo = Uid, UniqueVendorName, Names, VendorNames, Version, CreationTime, InstallType u8, InstallFlags u8.
    let mut info = field(9, &0x1234_5678u32.to_le_bytes());
    info.extend_from_slice(&string_field("Unique Vendor"));
    info.extend_from_slice(&array(1, &[utf16("Example")]));
    info.extend_from_slice(&array(1, &[utf16("Example Vendor")]));
    let mut version = Vec::new();
    version.extend_from_slice(&0u32.to_le_bytes());
    version.extend_from_slice(&1u32.to_le_bytes());
    version.extend_from_slice(&0u32.to_le_bytes());
    info.extend_from_slice(&field(4, &version));
    info.extend_from_slice(&field(8, &[0; 12]));
    info.extend_from_slice(&[0, 0]);

    let mut controller = field(14, &info);
    controller.extend_from_slice(&field(
        15,
        &array(11, &[1u32.to_le_bytes().to_vec()]),
    ));
    controller.extend_from_slice(&field(28, &install));
    let controller = compressed(&field(13, &controller));

    let mut contents = field(34, &0u32.to_le_bytes());
    contents.extend_from_slice(&field(35, &0u32.to_le_bytes()));
    contents.extend_from_slice(&controller);
    contents.extend_from_slice(&data);

    let mut package = Vec::new();
    package.extend_from_slice(&0x1020_1a7au32.to_le_bytes());
    package.extend_from_slice(&0x1000_0000u32.to_le_bytes());
    package.extend_from_slice(&0x1234_5678u32.to_le_bytes());
    package.extend_from_slice(&0u32.to_le_bytes());
    package.extend_from_slice(&field(12, &contents));
    package
}

#[test]
fn parses_controller_condition_and_data() {
    let package = Package::parse(&package()).unwrap();
    assert_eq!(package.uids, (0x1020_1a7a, 0x1000_0000, 0x1234_5678));
    assert_eq!(package.package_name, "Example");
    assert_eq!(package.vendor, "Example Vendor");
    assert_eq!(package.version, (0, 1, 0));
    assert_eq!(package.supported_languages, [1]);
    assert_eq!(package.files.len(), 2);
    assert_eq!(package.files[0].target, "sys\\bin\\first.exe");
    assert_eq!(package.files[0].bytes, b"first file");
    assert!(!package.files[0].is_conditional);
    assert_eq!(package.files[1].target, "sys\\bin\\second.exe");
    assert_eq!(package.files[1].bytes, b"second file");
    assert!(package.files[1].is_conditional);
}

#[test]
fn every_proper_prefix_is_truncated() {
    let bytes = package();
    for length in 0..bytes.len() {
        assert!(
            matches!(Package::parse(&bytes[..length]), Err(SisError::Truncated(_))),
            "prefix {length} was not rejected as truncated"
        );
    }
}

#[test]
fn legacy_uid_is_refused_by_name() {
    let mut bytes = package();
    bytes[0..4].copy_from_slice(&0x1000_007au32.to_le_bytes());
    assert!(matches!(
        Package::parse(&bytes),
        Err(SisError::Unsupported(_))
    ));
}

#[test]
fn declared_field_length_cannot_escape_the_input() {
    let mut bytes = package();
    bytes[20..24].copy_from_slice(&0x7fff_fff0u32.to_le_bytes());
    assert!(matches!(
        Package::parse(&bytes),
        Err(SisError::Truncated(_))
    ));
}

/// HEIRLOOM_SIS_CORPUS names a directory holding the owner's test packages (never part of the sources):
/// nokiafilebrowser/nokiafilebrowser.sis and hero-of-sparta/hero-of-sparta.sis. Unset = skipped, said so.
fn corpus(rel: &str) -> Option<Vec<u8>> {
    let Ok(dir) = std::env::var("HEIRLOOM_SIS_CORPUS") else {
        eprintln!("SKIP: HEIRLOOM_SIS_CORPUS is not set; no real SIS package read");
        return None;
    };
    Some(std::fs::read(std::path::Path::new(&dir).join(rel)).unwrap())
}

/// An E32 image carries "EPOC" at offset 16.
fn is_e32(bytes: &[u8]) -> bool {
    bytes.get(16..20) == Some(b"EPOC")
}

#[test]
fn real_package_nokia_file_browser() {
    let Some(bytes) = corpus("nokiafilebrowser/nokiafilebrowser.sis") else { return };
    let package = Package::parse(&bytes).unwrap();
    assert_eq!(package.package_name, "FileBrowser");
    assert_eq!(package.version, (1, 0, 0));
    assert_eq!(package.files.len(), 7);
    let exe = package.files.iter().find(|f| f.target.ends_with("\\FileBrowser.exe")).unwrap();
    assert_eq!(exe.bytes.len(), 98092);
    assert!(is_e32(&exe.bytes));
}

#[test]
fn real_package_hero_of_sparta() {
    let Some(bytes) = corpus("hero-of-sparta/hero-of-sparta.sis") else { return };
    let package = Package::parse(&bytes).unwrap();
    assert_eq!(package.package_name, "Hero of Sparta - POPDA");
    assert_eq!(package.vendor, "Gameloft");
    assert_eq!(package.files.len(), 136);
    let exe = package.files.iter().find(|f| f.target.ends_with("\\HeroOfSparta.exe")).unwrap();
    assert!(is_e32(&exe.bytes));
}
