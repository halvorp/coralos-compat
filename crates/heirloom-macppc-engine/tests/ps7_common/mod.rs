//! What the Photoshop 7 host harnesses (ps7_installer.rs, ps7_launch.rs) share: writing a volume's tree out in the installed-tree form.

/// The tree under `top`, written for staging (corpus lane, never committed): each file's data fork at its Mac path (`:` -> `/`), its
/// resource fork beside it as `<name>.rsrc`, and `manifest.tsv` (path, type, creator, finder flags, data bytes, resource bytes).
pub fn export(vol: &heirloom_volume::Tree, top: heirloom_volume::NodeId, dir: &std::path::Path) {
    let mut manifest = String::from("# path\ttype\tcreator\tflags\tdata\trsrc\n");
    let mut stack = vec![(top, dir.to_path_buf(), String::new())];
    while let Some((d, host, rel)) = stack.pop() {
        std::fs::create_dir_all(&host).unwrap();
        for &c in vol.children(d) {
            let name = vol.name(c).replace('/', ":");
            let (h, r) = (host.join(&name), if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") });
            if vol.is_dir(c) { stack.push((c, h, r)); continue }
            let n = vol.get(c);
            // an in-memory file's bytes, or a stream's read through its source (a staged tree the program did not rewrite)
            let streamed;
            let data = match vol.file_data(c) {
                Some(d) => d,
                None => {
                    let mut v = vec![0u8; vol.file_len(c).unwrap_or(0) as usize];
                    let got = vol.read_at(c, 0, &mut v).unwrap_or(0);
                    assert_eq!(got, v.len(), "the stream {name} read back whole");
                    streamed = v;
                    &streamed[..]
                }
            };
            std::fs::write(&h, data).unwrap();
            if !n.rsrc.is_empty() { std::fs::write(host.join(format!("{name}.rsrc")), &n.rsrc).unwrap(); }
            let fourcc = |b: [u8; 4]| b.iter().map(|&x| if x.is_ascii_graphic() || x == b' ' { char::from(x) } else { '?' }).collect::<String>();
            manifest += &format!("{r}\t{}\t{}\t{:#06x}\t{}\t{}\n", fourcc(n.finfo.file_type), fourcc(n.finfo.creator), n.finfo.flags, data.len(), n.rsrc.len());
        }
    }
    std::fs::write(dir.join("manifest.tsv"), manifest).unwrap();
    eprintln!("EXPORTED -> {}", dir.display());
}

