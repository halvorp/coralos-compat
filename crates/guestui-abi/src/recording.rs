//! A [`GuestUi`] that records every call. Test double and headless runner backend.
use alloc::vec::Vec;
use alloc::collections::VecDeque;
use alloc::collections::BTreeMap;
use crate::*;


#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiCall {
    OpenWindow(WindowModel),
    CloseWindow(UiObjectRef),
    SetWindowTitle(UiObjectRef, alloc::string::String),
    SetMenuStrip(UiObjectRef, MenuModel),
    ClearMenuStrip(UiObjectRef),
    AddControl(UiObjectRef, ControlModel),
    RemoveControl(UiObjectRef, UiObjectRef),
    SetControl(UiObjectRef, UiObjectRef, ControlValue),
    SetControlRect(UiObjectRef, UiObjectRef, GuestRect),
    /// `present_content`: the window, the damage rectangle, the canvas size, and an FNV-1a 64 hash over the damage pixels.
    PresentContent { win: UiObjectRef, damage: GuestRect, width: i32, height: i32, hash: u64 },
}

/// FNV-1a 64 over pixels as big-endian bytes: the stable fingerprint tests pin.
pub fn pixel_hash(pixels: &[u32]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for p in pixels { for b in p.to_be_bytes() { h = (h ^ b as u64).wrapping_mul(0x100000001b3); } }
    h
}

#[derive(Default)]
pub struct RecordingGuestUi {
    pub calls: Vec<UiCall>,
    /// Events queued by tests; handed out by `poll_event` in order.
    pub events: VecDeque<GuestUiEvent>,
    /// The canvas each window shows: (width, height, 0x00RRGGBB pixels), composed from every `present_content`.
    pub canvases: BTreeMap<u32, (i32, i32, Vec<u32>)>,
    /// The live native controls as the host would hold them now: control id -> (window, model with every `set_control` /
    /// `set_control_rect` applied). `calls` keeps the history; this is the current picture.
    pub controls: BTreeMap<u32, (UiObjectRef, ControlModel)>,
    next_id: u32,
    /// live object id -> generation
    live: BTreeMap<u32, u32>,
}

impl RecordingGuestUi {
    pub fn new() -> Self { Self::default() }
    fn alloc(&mut self) -> UiObjectRef {
        self.next_id += 1;
        self.live.insert(self.next_id, 1);
        UiObjectRef { id: self.next_id, generation: 1 }
    }
    fn check(&self, r: UiObjectRef) -> Result<(), UiRefusal> {
        match self.live.get(&r.id) {
            None => Err(UiRefusal::UnknownObject(r)),
            Some(g) if *g != r.generation => Err(UiRefusal::StaleGeneration(r)),
            Some(_) => Ok(()),
        }
    }
}

impl GuestUi for RecordingGuestUi {
    fn open_window(&mut self, model: WindowModel) -> Result<UiObjectRef, UiRefusal> {
        if model.rect.width <= 0 || model.rect.height <= 0 {
            return Err(UiRefusal::InvalidGeometry("window width/height must be positive"));
        }
        self.calls.push(UiCall::OpenWindow(model));
        Ok(self.alloc())
    }
    fn close_window(&mut self, win: UiObjectRef) -> Result<(), UiRefusal> {
        self.check(win)?;
        self.live.remove(&win.id);
        let gone: Vec<u32> = self.controls.iter().filter(|(_, (w, _))| *w == win).map(|(&id, _)| id).collect();
        for id in gone { self.controls.remove(&id); self.live.remove(&id); }
        self.calls.push(UiCall::CloseWindow(win));
        Ok(())
    }
    fn set_window_title(&mut self, win: UiObjectRef, title: alloc::string::String) -> Result<(), UiRefusal> {
        self.check(win)?;
        self.calls.push(UiCall::SetWindowTitle(win, title));
        Ok(())
    }
    fn set_menu_strip(&mut self, win: UiObjectRef, menu: MenuModel) -> Result<(), UiRefusal> {
        self.check(win)?;
        self.calls.push(UiCall::SetMenuStrip(win, menu));
        Ok(())
    }
    fn clear_menu_strip(&mut self, win: UiObjectRef) -> Result<(), UiRefusal> {
        self.check(win)?;
        self.calls.push(UiCall::ClearMenuStrip(win));
        Ok(())
    }
    fn add_control(&mut self, win: UiObjectRef, control: ControlModel) -> Result<UiObjectRef, UiRefusal> {
        self.check(win)?;
        if let ControlKind::Slider { min, max, value } = &control.kind {
            if min > max || value < min || value > max {
                return Err(UiRefusal::InvalidGeometry("slider range"));
            }
        }
        self.calls.push(UiCall::AddControl(win, control.clone()));
        let id = self.alloc();
        self.controls.insert(id.id, (win, control));
        Ok(id)
    }
    fn set_control(&mut self, win: UiObjectRef, control: UiObjectRef, value: ControlValue) -> Result<(), UiRefusal> {
        self.check(win)?;
        self.check(control)?;
        if let Some((_, m)) = self.controls.get_mut(&control.id) { m.apply(&value)?; }
        self.calls.push(UiCall::SetControl(win, control, value));
        Ok(())
    }
    fn set_control_rect(&mut self, win: UiObjectRef, control: UiObjectRef, rect: GuestRect) -> Result<(), UiRefusal> {
        self.check(win)?;
        self.check(control)?;
        if rect.width < 0 || rect.height < 0 { return Err(UiRefusal::InvalidGeometry("control rect with a negative size")); }
        if let Some((_, m)) = self.controls.get_mut(&control.id) { m.rect = rect; }
        self.calls.push(UiCall::SetControlRect(win, control, rect));
        Ok(())
    }
    fn remove_control(&mut self, win: UiObjectRef, control: UiObjectRef) -> Result<(), UiRefusal> {
        self.check(win)?;
        self.check(control)?;
        self.live.remove(&control.id);
        self.controls.remove(&control.id);
        self.calls.push(UiCall::RemoveControl(win, control));
        Ok(())
    }
    fn present_content(&mut self, win: UiObjectRef, damage: GuestRect, width: i32, height: i32, pixels: &[u32]) -> Result<(), UiRefusal> {
        self.check(win)?;
        let inside = damage.left >= 0 && damage.top >= 0 && damage.width > 0 && damage.height > 0
            && damage.left + damage.width <= width && damage.top + damage.height <= height;
        if !inside { return Err(UiRefusal::InvalidGeometry("content damage outside the canvas")); }
        if pixels.len() != (damage.width * damage.height) as usize { return Err(UiRefusal::InvalidGeometry("content pixel count != damage area")); }
        let c = self.canvases.entry(win.id).or_insert_with(|| (width, height, alloc::vec![0; (width * height) as usize]));
        if (c.0, c.1) != (width, height) { *c = (width, height, alloc::vec![0; (width * height) as usize]); }
        for row in 0..damage.height {
            let d = ((damage.top + row) * width + damage.left) as usize;
            c.2[d..d + damage.width as usize].copy_from_slice(&pixels[(row * damage.width) as usize..((row + 1) * damage.width) as usize]);
        }
        self.calls.push(UiCall::PresentContent { win, damage, width, height, hash: pixel_hash(pixels) });
        Ok(())
    }
    fn poll_event(&mut self) -> Option<GuestUiEvent> { self.events.pop_front() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win() -> WindowModel {
        WindowModel { title: "T".into(), rect: GuestRect { left: 0, top: 0, width: 10, height: 10 },
            role: WindowRole::Document, closable: true, resizable: true, token: CallbackToken(1) }
    }
    fn ctl(kind: ControlKind) -> ControlModel {
        ControlModel { kind, label: "x".into(), rect: GuestRect { left: 1, top: 1, width: 5, height: 5 },
            enabled: true, token: CallbackToken(2) }
    }

    #[test]
    fn records_in_order_and_hands_back_events() {
        let mut ui = RecordingGuestUi::new();
        let w = ui.open_window(win()).unwrap();
        let kinds = [ControlKind::Button, ControlKind::Checkbox { checked: true },
            ControlKind::String { text: "a".into(), max_len: 8 }, ControlKind::Slider { min: 0, max: 10, value: 3 }];
        for k in kinds { ui.add_control(w, ctl(k)).unwrap(); }
        ui.set_menu_strip(w, MenuModel::default()).unwrap();
        ui.set_window_title(w, "New".into()).unwrap();
        ui.close_window(w).unwrap();
        assert_eq!(ui.calls.len(), 8);
        assert!(matches!(ui.calls[0], UiCall::OpenWindow(_)));
        assert_eq!(ui.calls[6], UiCall::SetWindowTitle(w, "New".into()));
        assert!(matches!(ui.calls[7], UiCall::CloseWindow(_)));
        ui.events.push_back(GuestUiEvent::CloseRequested { win: w, token: CallbackToken(1) });
        assert!(ui.poll_event().is_some() && ui.poll_event().is_none());
    }

    #[test]
    fn stale_and_unknown_objects_refused_by_name() {
        let mut ui = RecordingGuestUi::new();
        let w = ui.open_window(win()).unwrap();
        ui.close_window(w).unwrap();
        assert_eq!(ui.close_window(w), Err(UiRefusal::UnknownObject(w)));
        assert_eq!(ui.set_window_title(w, "x".into()), Err(UiRefusal::UnknownObject(w)));
        assert_eq!(ui.clear_menu_strip(UiObjectRef { id: 99, generation: 1 }), Err(UiRefusal::UnknownObject(UiObjectRef { id: 99, generation: 1 })));
    }

    #[test]
    fn content_frames_compose_into_a_canvas_and_bad_frames_are_refused() {
        let mut ui = RecordingGuestUi::new();
        let w = ui.open_window(win()).unwrap();
        let r = |l, t, w_, h| GuestRect { left: l, top: t, width: w_, height: h };
        ui.present_content(w, r(0, 0, 4, 2), 4, 2, &[1; 8]).unwrap();
        ui.present_content(w, r(1, 1, 2, 1), 4, 2, &[7, 8]).unwrap();
        assert_eq!(ui.canvases[&w.id].2, vec![1, 1, 1, 1, 1, 7, 8, 1]);
        assert_eq!(ui.calls.last(), Some(&UiCall::PresentContent { win: w, damage: r(1, 1, 2, 1), width: 4, height: 2, hash: pixel_hash(&[7, 8]) }));
        assert_ne!(pixel_hash(&[7, 8]), pixel_hash(&[8, 7]), "the hash depends on order");
        let n = ui.calls.len();
        assert!(matches!(ui.present_content(w, r(3, 0, 2, 1), 4, 2, &[1, 1]), Err(UiRefusal::InvalidGeometry(_))), "outside the canvas");
        assert!(matches!(ui.present_content(w, r(0, 0, 2, 2), 4, 2, &[1]), Err(UiRefusal::InvalidGeometry(_))), "wrong pixel count");
        assert_eq!(ui.calls.len(), n, "refused frames are not recorded");
        ui.close_window(w).unwrap();
        assert!(ui.present_content(w, r(0, 0, 1, 1), 4, 2, &[1]).is_err());
    }

    #[test]
    fn bad_geometry_and_slider_refused() {
        let mut ui = RecordingGuestUi::new();
        let mut m = win();
        m.rect.width = 0;
        assert!(matches!(ui.open_window(m), Err(UiRefusal::InvalidGeometry(_))));
        let w = ui.open_window(win()).unwrap();
        assert!(ui.add_control(w, ctl(ControlKind::Slider { min: 5, max: 1, value: 3 })).is_err());
        assert_eq!(ui.calls.len(), 1, "refused calls are not recorded");
    }

    #[test]
    fn native_gadget_controls_are_tracked_valued_and_moved() {
        let mut ui = RecordingGuestUi::new();
        let w = ui.open_window(win()).unwrap();
        let bar = ui.add_control(w, ctl(ControlKind::Scrollbar { orientation: Orientation::Vertical, total: 100, visible: 10, top: 0 })).unwrap();
        ui.set_control(w, bar, ControlValue::Scroll { total: 100, visible: 10, top: 500 }).unwrap();
        assert!(matches!(ui.controls[&bar.id].1.kind, ControlKind::Scrollbar { top: 90, .. }), "top clamps to total - visible");
        ui.set_control_rect(w, bar, GuestRect { left: 1, top: 2, width: 3, height: 4 }).unwrap();
        assert_eq!(ui.controls[&bar.id].1.rect, GuestRect { left: 1, top: 2, width: 3, height: 4 });
        // Negative controls: a value the kind cannot hold, and a negative rect, are refused by name and change nothing.
        assert_eq!(ui.set_control(w, bar, ControlValue::Checked(true)), Err(UiRefusal::Unsupported("a value the control's kind cannot hold")));
        assert!(ui.set_control_rect(w, bar, GuestRect { left: 0, top: 0, width: -1, height: 4 }).is_err());
        assert_eq!(ui.controls[&bar.id].1.rect.width, 3);
        let t = ui.add_control(w, ctl(ControlKind::ToggleButton { on: false })).unwrap();
        ui.set_control(w, t, ControlValue::Checked(true)).unwrap();
        assert!(matches!(ui.controls[&t.id].1.kind, ControlKind::ToggleButton { on: true }));
        ui.close_window(w).unwrap();
        assert!(ui.controls.is_empty());
    }
}
