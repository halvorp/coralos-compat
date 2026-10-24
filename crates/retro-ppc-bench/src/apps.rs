//! Application workloads: real PowerPC programs from the corpus (outside git), run on macppc-engine exactly as
//! `macppc-run --census --run --idle 6` runs them (tools/corpus-census-mac.py). A file that is absent is SKIPPED
//! loudly: the corpus is test input only and is never committed.
use guestui_abi::RecordingGuestUi;
use macppc_engine::{MacPpc, Step};
use retro_pef::Pef;
use std::path::{Path, PathBuf};

pub struct App {
    pub name: String,
    /// Path relative to the corpus root (`MAC_CORPUS_ROOT`, default /data/build/corpus-bin).
    pub path: String,
    /// PEF libraries beside the program (relative to the corpus root), as `--libdir`.
    pub libdir: Option<String>,
    /// Instruction budget of the run.
    pub insns: u64,
}

/// Idle ticks given to a program that waits for an event (corpus-census-mac.py's MAC_IDLE default).
pub const IDLE: u32 = 6;

/// The workload list: tools/ppc-bench-apps.tsv (or `PPC_BENCH_APPS`). Which programs are measured is data, not code.
pub fn all() -> Vec<App> {
    let default = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tools/ppc-bench-apps.tsv");
    let path = std::env::var("PPC_BENCH_APPS").unwrap_or_else(|_| default.into());
    let Ok(text) = std::fs::read_to_string(&path) else { println!("SKIP every corpus workload: {path} is unreadable"); return Vec::new() };
    text.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()).filter_map(|l| {
        let f: Vec<&str> = l.split('\t').collect();
        let app = App { name: f.first()?.to_string(), path: f.get(1)?.to_string(), libdir: f.get(2).filter(|d| **d != "-").map(|d| d.to_string()), insns: f.get(3)?.parse().ok()? };
        Some(app)
    }).collect()
}

pub fn root() -> PathBuf { PathBuf::from(std::env::var("MAC_CORPUS_ROOT").unwrap_or_else(|_| "/data/build/corpus-bin".into())) }

pub fn present(a: &App) -> bool { root().join(&a.path).is_file() }

fn sibling_data_fork(p: &Path) -> Option<PathBuf> {
    let name = p.file_name()?.to_str()?;
    let stripped = name.strip_prefix('%').or_else(|| name.strip_prefix("._"))?;
    let q = p.with_file_name(stripped);
    q.is_file().then_some(q)
}

/// Load and start `a` on a fresh machine (what macppc-run does before `--run`), not timed.
pub fn load(a: &App) -> Result<MacPpc<RecordingGuestUi>, String> {
    let path = root().join(&a.path);
    let file = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let data = sibling_data_fork(&path).map(std::fs::read).transpose().map_err(|e| e.to_string())?;
    let (name, bytes) = macppc_engine::source::fragment_bytes(&file, data.as_deref()).map_err(|e| format!("{e}"))?;
    let pef = Pef::parse(&bytes).map_err(|e| format!("{e}"))?;
    let mut m = MacPpc::new(RecordingGuestUi::new()).map_err(|e| format!("{e:?}"))?;
    let mut fork = path.clone().into_os_string();
    fork.push(":rsrc");
    let fork = std::fs::read(PathBuf::from(fork)).ok();
    let _ = m.attach_resources(fork.as_deref().unwrap_or(&file));
    for (lname, b) in macppc_engine::source::library_fragments(&file, data.as_deref()) { m.provide_library(&lname, &b); }
    if let Some(dir) = &a.libdir {
        let mut names: Vec<_> = std::fs::read_dir(root().join(dir)).map_err(|e| e.to_string())?.filter_map(|e| e.ok()).collect();
        names.sort_by_key(|e| e.file_name());
        for e in names {
            let Ok(b) = std::fs::read(e.path()) else { continue };
            if !Pef::has_magic(&b) || e.file_name().to_string_lossy().ends_with(":rsrc") { continue; }
            m.provide_library(&e.file_name().to_string_lossy(), &b);
        }
    }
    let frag = m.instantiate(&name, &pef).map_err(|e| format!("instantiate: {e}"))?;
    m.start(&frag).map_err(|e| format!("start: {e}"))?;
    Ok(m)
}

/// Run a started machine for up to `insns` instructions, giving it IDLE ticks whenever it waits (as macppc-run
/// --idle does). Returns (instructions retired, the last step).
pub fn run(m: &mut MacPpc<RecordingGuestUi>, insns: u64) -> (u64, Step) {
    let (mut step, mut n) = m.run_loop(insns);
    let mut rounds = 0u32;
    while step == Step::Blocked && n < insns && rounds < 2_000_000 {
        m.mac.advance_ticks(IDLE);
        rounds += 1;
        let (st, k) = m.run_loop(insns - n);
        step = st;
        n += k;
    }
    (n, step)
}
