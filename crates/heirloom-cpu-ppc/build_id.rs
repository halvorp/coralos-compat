// The core's BUILD identity, one definition for the two build scripts that compute it (heirloom-cpu-ppc's, which stamps the
// core, and heirloom-cpu-ppc-client's, which expects it): FNV-1a 64 over every file under the core's src/ in path order, each
// file's path (relative to src/) then its bytes - the same sources give the same number on any host.
fn build_id(src: &std::path::Path) -> u64 {
    fn files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let mut v: Vec<_> = std::fs::read_dir(dir).expect("src").map(|e| e.expect("entry").path()).collect();
        v.sort();
        for p in v { if p.is_dir() { files(&p, out) } else { out.push(p) } }
    }
    let mut all = Vec::new();
    files(src, &mut all);
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in &all {
        for b in p.strip_prefix(src).expect("under src").to_string_lossy().bytes().chain(std::fs::read(p).expect("read")) {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}
