//! The glBegin / glEnd state machine of the ENGINE (GL 1.2 §2.6.1): errors are raised by the engine and the offending call is
//! never sent to the worker. REAL PowerPC programs make the calls; the worker mock records what it was sent.
mod ppc_gl;
use ppc_gl::*;
use std::cell::Cell;

fn keep(name: &'static str, at: i32) -> Call { Call { name, ints: vec![], fl: vec![], keep: Some(at) } }

fn error_after(steps: Vec<Call>, slots: usize, setup: impl Fn(&mut Data) -> Vec<Call>) -> (Vec<u32>, Vec<String>) {
    let at = Cell::new(0);
    let (m, seen, d) = run(|data| {
        at.set(data.put(&vec![0xEE; 4 * slots]));
        let mut v = setup(data);
        v.extend(steps.iter().map(|c| Call { name: c.name, ints: c.ints.clone(), fl: c.fl.clone(), keep: c.keep.map(|k| at.get() + 4 * k) }));
        v
    }, true);
    let errs = (0..slots as u32).map(|k| m.mac.mem.read_u32(d + at.get() as u32 + 4 * k).unwrap()).collect();
    let sent = seen.lock().unwrap().calls.iter().map(|c| c.0.clone()).filter(|n| !n.starts_with("agl") && n != "glGetError").collect();
    (errs, sent)
}

#[test]
fn a_misplaced_begin_or_end_is_an_error_that_is_never_sent() {
    let f = |x: f32| (x.to_bits() as i32, x);
    let _ = f;
    // glEnd with no glBegin; glBegin(99); a valid glBegin/glEnd; a nested glBegin; glEnable inside; glVertex2f inside is fine
    let (errs, sent) = error_after(
        vec![
            call("glEnd", vec![], vec![]), keep("glGetError", 0),
            call("glBegin", ints(&[99]), vec![]), keep("glGetError", 1),
            call("glBegin", ints(&[4]), vec![]),
            call("glBegin", ints(&[4]), vec![]), keep("glGetError", 2),
            call("glEnable", ints(&[0x0B50]), vec![]), keep("glGetError", 3),
            call("glDepthFunc", ints(&[0x0203]), vec![]), keep("glGetError", 4),
            call("glEnd", vec![], vec![]), keep("glGetError", 5),
            call("glEnable", ints(&[0x0B50]), vec![]), keep("glGetError", 6),
        ],
        7,
        |_| vec![],
    );
    assert_eq!(errs, [0x0502, 0x0500, 0x0502, 0x0502, 0x0502, 0, 0], "glEnd with no glBegin; an invalid mode; a nested glBegin; glEnable and glDepthFunc inside; the matching glEnd; glEnable after it");
    assert_eq!(sent, ["glBegin", "glEnd", "glEnable"], "only the valid Begin, its End and the Enable after it reached the worker");
}

#[test]
fn vertex_data_material_and_edge_flags_are_allowed_inside_begin_and_nothing_else() {
    let (errs, sent) = error_after(
        vec![
            call("glBegin", ints(&[7]), vec![]),
            call("glVertex2s", ints(&[1, 2]), vec![]),
            call("glColor3ub", ints(&[1, 2, 3]), vec![]),
            call("glNormal3b", ints(&[1, 2, 3]), vec![]),
            call("glTexCoord1s", ints(&[1]), vec![]),
            call("glEdgeFlag", ints(&[0]), vec![]),
            call("glIndexs", ints(&[3]), vec![]),
            keep("glGetError", 0),
            call("glPushMatrix", vec![], vec![]), keep("glGetError", 1),
            call("glColorMaterial", ints(&[0x0408, 0x1602]), vec![]), keep("glGetError", 2),
            call("glEnd", vec![], vec![]), keep("glGetError", 3),
        ],
        4,
        |_| vec![],
    );
    assert_eq!(errs, [0, 0x0502, 0x0502, 0], "vertex, colour, normal, texcoord, edge-flag and index calls are allowed; glPushMatrix and glColorMaterial (not Color*) are not");
    assert_eq!(sent, ["glBegin", "glVertex2s", "glColor3ub", "glNormal3b", "glTexCoord1s", "glEdgeFlag", "glIndexs", "glEnd"]);
}

#[test]
fn the_display_list_state_machine_is_the_engines() {
    // glEndList with no glNewList; glNewList(0); a valid glNewList; a nested glNewList; the matching glEndList
    let (errs, sent) = error_after(
        vec![
            call("glEndList", vec![], vec![]), keep("glGetError", 0),
            call("glNewList", ints(&[0, 0x1300]), vec![]), keep("glGetError", 1),
            call("glNewList", ints(&[7, 0x1234]), vec![]), keep("glGetError", 2),
            call("glNewList", ints(&[7, 0x1300]), vec![]), keep("glGetError", 3),
            call("glNewList", ints(&[8, 0x1300]), vec![]), keep("glGetError", 4),
            call("glEndList", vec![], vec![]), keep("glGetError", 5),
            call("glEndList", vec![], vec![]), keep("glGetError", 6),
        ],
        7,
        |_| vec![],
    );
    assert_eq!(errs, [0x0502, 0x0501, 0x0500, 0, 0x0502, 0, 0x0502], "EndList alone; list 0; an unknown mode; a valid NewList; a nested NewList; its EndList; a second EndList");
    assert_eq!(sent, ["glNewList", "glEndList"], "only the valid pair reached the worker");
}

#[test]
fn a_negative_size_is_invalid_value_and_is_never_sent() {
    let (errs, sent) = error_after(
        vec![
            call("glDeleteLists", ints(&[1, 0xFFFF_FFFF]), vec![]), keep("glGetError", 0),
            call("glGenLists", ints(&[0x8000_0000]), vec![]), keep("glGetError", 1),
            call("glDeleteLists", ints(&[1, 3]), vec![]), keep("glGetError", 2),
        ],
        3,
        |_| vec![],
    );
    assert_eq!(errs, [0x0501, 0x0501, 0]);
    assert_eq!(sent, ["glDeleteLists"], "only the valid call reached the worker");
}

#[test]
fn a_call_the_worker_keeps_itself_is_refused_by_name_inside_a_display_list_and_served_outside_one() {
    for (name, ints_) in [("glDrawPixels", vec![1u32, 1, 0x1908, 0x1401, 0x4000]), ("glPixelMapfv", vec![0x0C76, 1, 0x4000]), ("glPrioritizeTextures", vec![1, 0x4000, 0x4000]), ("glPolygonStipple", vec![0x4000])] {
        let build = |data: &mut Data| { let _ = data.put(&[0u8; 256]); vec![call("glNewList", ints(&[7, 0x1300]), vec![]), call(name, ints(&ints_), vec![])] };
        let (calls, d) = layout(&build);
        let (bytes, _) = assemble(&calls, &d, true);
        let mut m = ppc_gl::M::new(heirloom_ui_abi::RecordingGuestUi::new()).unwrap();
        m.gl_attach(Box::new(Mock(heirloom_gl_abi::TokenTable::default(), Default::default())));
        let frag = m.instantiate("il", &heirloom_fmt_pef::Pef::parse(&bytes).unwrap()).unwrap();
        m.start(&frag).unwrap();
        assert_ne!(m.step(5_000_000).0, heirloom_macppc_engine::Step::Exited, "{name}");
        assert!(m.fault().expect("a named fault").to_string().contains("inside-a-display-list"), "{name}");
    }
}
