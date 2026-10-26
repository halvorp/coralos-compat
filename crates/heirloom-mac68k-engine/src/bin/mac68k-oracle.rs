//! Oracle helper: `mac68k-oracle app OUT` writes the QuickDraw oracle application as a MacBinary II file (for an external
//! implementation to run); `mac68k-oracle dump OUT` runs it on this engine and writes the window's content as text
//! (`W H` then one line of `0`/`1` per row, `1` = black), the format `tests/oracle/systemless/compare.py` reads.
use heirloom_mac68k_engine::fixture::oracle_app_macbinary;
use heirloom_mac68k_engine::Mac;
use heirloom_ui_abi::RecordingGuestUi;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (mode, out) = match a.as_slice() { [m, o] => (m.as_str(), o.as_str()), _ => { eprintln!("usage: mac68k-oracle app|dump OUT"); std::process::exit(2) } };
    let file = oracle_app_macbinary();
    match mode {
        "app" => std::fs::write(out, &file).expect("write"),
        "dump" => {
            let mut m = Mac::new(RecordingGuestUi::new()).expect("machine");
            m.launch(&file).expect("launch");
            let r = m.run_loop(50_000_000).expect("run");
            assert!(r.stats.blocked, "the app should wait for events: {:?}", r.stats);
            for x in &m.refused { eprintln!("refused: {x}"); }
            let canvas = m.qd.canvases.values().next().expect("the app's window canvas");
            let mut s = format!("{} {}\n", canvas.width(), canvas.pixels().len() as i32 / canvas.width());
            for row in canvas.pixels().chunks(canvas.width() as usize) { s.extend(row.iter().map(|&p| if p == 1 { '1' } else { '0' })); s.push('\n'); }
            std::fs::write(out, s).expect("write");
        }
        _ => { eprintln!("unknown mode {mode}"); std::process::exit(2) }
    }
}
