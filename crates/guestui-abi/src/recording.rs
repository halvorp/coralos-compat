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
    HideWindow(UiObjectRef),
    ShowWindow(UiObjectRef),
    SetMenuStrip(UiObjectRef, MenuModel),
    ClearMenuStrip(UiObjectRef),
    AddControl(UiObjectRef, ControlModel),
    RemoveControl(UiObjectRef, UiObjectRef),
    SetControl(UiObjectRef, UiObjectRef, ControlValue),
    SetControlRect(UiObjectRef, UiObjectRef, GuestRect),
    SetControlEnabled(UiObjectRef, UiObjectRef, bool),
    /// `present_content`: the window, the damage rectangle, the canvas size, and an FNV-1a 64 hash over the damage pixels.
    PresentContent { win: UiObjectRef, damage: GuestRect, width: i32, height: i32, hash: u64 },
    /// `present_plane`: the window and the plane frame as given.
    PresentPlane(UiObjectRef, Plane),
    /// `request`: the window it belongs to (if any) and the question as asked.
    Request(Option<UiObjectRef>, RequestModel),
    /// `close_request`: an unanswered request the guest withdrew.
    CloseRequest(RequestRef),
    /// `open_audio_stream`: the stream as asked.
    OpenAudio(AudioStreamModel),
    /// `queue_audio`: the stream and the number of bytes (the bytes themselves are in [`RecordingGuestUi::audio`]).
    QueueAudio(AudioStreamRef, usize),
    FlushAudio(AudioStreamRef),
    PauseAudio(AudioStreamRef, bool),
    SetAudioVolume(AudioStreamRef, u16, u16),
    CloseAudio(AudioStreamRef),
}

/// What the recording host holds of one audio stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedAudio {
    pub model: AudioStreamModel,
    /// Every byte ever queued and not flushed, in order.
    pub queued: Vec<u8>,
    /// How many of `queued` the pretend device has played (see [`RecordingGuestUi::play_audio`]).
    pub played: usize,
    pub paused: bool,
    pub volume: (u16, u16),
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
    /// Test artwork `icon_image` hands out per role (width, height, pixels); a role not in the map has none.
    pub icon_art: BTreeMap<crate::IconRole, (u32, u32, Vec<u32>)>,
    /// Every `icon_image` request seen, in order.
    pub icon_requests: Vec<(crate::IconRole, u32, u32)>,
    /// The live native controls as the host would hold them now: control id -> (window, model with every `set_control` /
    /// `set_control_rect` applied). `calls` keeps the history; this is the current picture.
    pub controls: BTreeMap<u32, (UiObjectRef, ControlModel)>,
    /// The current content planes: (window id, plane id) -> the latest frame (a later frame replaces the earlier).
    pub planes: BTreeMap<(u32, u32), Plane>,
    /// What the host still shows of each plane: every frame not fully repainted by a later one (a frame replaces the frames its rect
    /// covers entirely). A hole that a later PARTIAL frame did not repaint is still on screen, so the census reads this, not `planes`.
    pub plane_frames: BTreeMap<(u32, u32), Vec<Plane>>,
    /// Control kind names (`"Gauge"`) this host REFUSES to draw (`UiRefusal::Unsupported`): a test of a host that lacks a part.
    pub refuse_kinds: Vec<&'static str>,
    /// Test answers (button indexes) handed out to the next requests in order: a request made while this has an entry is answered
    /// at once (the event is queued for `poll_event`); with none, the request stays in `requests` until `answer_request`.
    pub request_answers: VecDeque<u32>,
    /// The live audio streams: stream id -> what was queued and played.
    pub audio: BTreeMap<u32, RecordedAudio>,
    /// Make every audio call refuse by name (a host with no audio service).
    pub refuse_audio: bool,
    /// A headless run with nobody listening: queued audio counts as played at once (so a program that waits for its sound to end, with a
    /// completion routine, is not left waiting for a clock that never ticks). Off by default: tests keep the clock with `play_audio`.
    pub drain_audio: bool,
    /// Make `request` refuse by name (a host with no alert part).
    pub refuse_requests: bool,
    /// A headless run with nobody to answer: every request not covered by `request_answers` is answered at once with its Escape
    /// button (`cancel`, else the last one), as a user dismissing the alert would.
    pub auto_cancel_requests: bool,
    /// The requests asked and not yet answered or closed: request id -> the question.
    pub requests: BTreeMap<u32, RequestModel>,
    next_id: u32,
    /// live object id -> generation
    live: BTreeMap<u32, u32>,
    /// Ids of removed controls: a ref to one is `StaleGeneration`, not `UnknownObject` (it WAS valid).
    retired: alloc::collections::BTreeSet<u32>,
    /// (scope window id, token) -> incarnations so far: a token reused after its control was removed gets the next generation.
    incarnations: BTreeMap<(u32, u64), u32>,
}

impl RecordingGuestUi {
    pub fn new() -> Self { Self::default() }
    /// A new object under `scope` (0 for a window, else its window's id) for `token`; its generation counts the token's incarnations.
    fn alloc(&mut self, scope: u32, token: CallbackToken) -> UiObjectRef {
        self.next_id += 1;
        let g = self.incarnations.entry((scope, token.0)).or_insert(0);
        *g += 1;
        self.live.insert(self.next_id, *g);
        UiObjectRef { id: self.next_id, generation: *g }
    }
    fn check(&self, r: UiObjectRef) -> Result<(), UiRefusal> {
        match self.live.get(&r.id) {
            None if self.retired.contains(&r.id) => Err(UiRefusal::StaleGeneration(r)),
            None => Err(UiRefusal::UnknownObject(r)),
            Some(g) if *g != r.generation => Err(UiRefusal::StaleGeneration(r)),
            Some(_) => Ok(()),
        }
    }
    fn retire(&mut self, id: u32) {
        self.live.remove(&id);
        self.controls.remove(&id);
        self.retired.insert(id);
    }
    /// The pretend device plays up to `bytes` of the stream's queue (it is a test clock: nothing plays by itself). Returns how many played.
    pub fn play_audio(&mut self, stream: AudioStreamRef, bytes: usize) -> Result<usize, UiRefusal> {
        self.check(stream)?;
        let a = self.audio.get_mut(&stream.id).ok_or(UiRefusal::UnknownObject(stream))?;
        if a.paused { return Ok(0); }
        let n = bytes.min(a.queued.len() - a.played);
        a.played += n;
        Ok(n)
    }
    /// Queue the intent a user's gesture on `control` means, the way a host would: the control must be live (a removed or
    /// stale ref is refused by name) and the event carries its token and generation as `add_control` recorded them.
    pub fn push_intent(&mut self, win: UiObjectRef, control: UiObjectRef, intent: Intent) -> Result<(), UiRefusal> {
        self.check(win)?;
        self.check(control)?;
        let token = match self.controls.get(&control.id) {
            Some((w, m)) if *w == win => m.token,
            _ => return Err(UiRefusal::UnknownObject(control)),
        };
        self.events.push_back(GuestUiEvent::intent(win, control, token, intent));
        Ok(())
    }
    /// The live control whose model satisfies `pred`, as the refs a host would address it by: (window, control, model). Tests drive
    /// a program's controls through this (the first match in creation order).
    pub fn find_control(&self, pred: impl Fn(&ControlModel) -> bool) -> Option<(UiObjectRef, UiObjectRef, ControlModel)> {
        self.controls.iter().find(|(_, (_, m))| pred(m)).map(|(&id, (w, m))| (*w, UiObjectRef { id, generation: self.live[&id] }, m.clone()))
    }
    /// The user answers request `req` with button `button` (an index into its buttons): the event is queued like a host's. A closed,
    /// answered or unknown request is refused by name; a button past the question's buttons is `InvalidGeometry`.
    pub fn answer_request(&mut self, req: RequestRef, button: u32) -> Result<(), UiRefusal> {
        self.check(req)?;
        let m = self.requests.get(&req.id).ok_or(UiRefusal::UnknownObject(req))?;
        if button as usize >= m.buttons.len() { return Err(UiRefusal::InvalidGeometry("answer past the request's buttons")); }
        self.requests.remove(&req.id);
        self.retire(req.id);
        self.events.push_back(GuestUiEvent::RequestAnswered { req, button });
        Ok(())
    }
    /// The requests asked and not yet answered or closed, as the refs the host would answer them by (oldest first).
    pub fn open_requests(&self) -> Vec<RequestRef> {
        self.requests.keys().map(|&id| UiObjectRef { id, generation: self.live[&id] }).collect()
    }
    /// How the window's content reaches the host right now (see [`RouteCensus`]), from what this host holds. A closed or
    /// unknown window holds nothing and counts zero.
    pub fn route_census(&self, win: UiObjectRef) -> RouteCensus {
        let native = self.controls.values().filter(|(w, _)| *w == win).count() as u32;
        let (mut replayed, mut raw) = (0, self.canvases.contains_key(&win.id) as u32);
        for frames in self.plane_frames.iter().filter(|((w, _), _)| *w == win.id).map(|(_, f)| f) {
            if frames.iter().any(Plane::is_raw) { raw += 1 } else { replayed += 1 }
        }
        RouteCensus { native, replayed, raw }
    }
}

impl GuestUi for RecordingGuestUi {
    fn icon_image(&mut self, role: crate::IconRole, width: u32, height: u32) -> Option<Vec<u32>> {
        self.icon_requests.push((role, width, height));
        self.icon_art.get(&role).filter(|(w, h, _)| (*w, *h) == (width, height)).map(|a| a.2.clone())
    }
    fn open_window(&mut self, model: WindowModel) -> Result<UiObjectRef, UiRefusal> {
        if model.rect.width <= 0 || model.rect.height <= 0 {
            return Err(UiRefusal::InvalidGeometry("window width/height must be positive"));
        }
        let token = model.token;
        self.calls.push(UiCall::OpenWindow(model));
        Ok(self.alloc(0, token))
    }
    fn hide_window(&mut self, win: UiObjectRef) -> Result<(), UiRefusal> { self.check(win)?; self.calls.push(UiCall::HideWindow(win)); Ok(()) }
    fn show_window(&mut self, win: UiObjectRef) -> Result<(), UiRefusal> { self.check(win)?; self.calls.push(UiCall::ShowWindow(win)); Ok(()) }
    fn close_window(&mut self, win: UiObjectRef) -> Result<(), UiRefusal> {
        self.check(win)?;
        self.live.remove(&win.id);
        let gone: Vec<u32> = self.controls.iter().filter(|(_, (w, _))| *w == win).map(|(&id, _)| id).collect();
        for id in gone { self.retire(id); }
        self.planes.retain(|(w, _), _| *w != win.id);
        self.plane_frames.retain(|(w, _), _| *w != win.id);
        self.canvases.remove(&win.id);
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
        control.kind.validate()?;
        let name = alloc::format!("{:?}", control.kind);
        if self.refuse_kinds.iter().any(|k| name.starts_with(k)) {
            return Err(UiRefusal::Unsupported("control: this host draws no such kind"));
        }
        self.calls.push(UiCall::AddControl(win, control.clone()));
        let id = self.alloc(win.id, control.token);
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
    fn set_control_enabled(&mut self, win: UiObjectRef, control: UiObjectRef, enabled: bool) -> Result<(), UiRefusal> {
        self.check(win)?;
        self.check(control)?;
        if let Some((_, m)) = self.controls.get_mut(&control.id) { m.enabled = enabled; }
        self.calls.push(UiCall::SetControlEnabled(win, control, enabled));
        Ok(())
    }
    fn remove_control(&mut self, win: UiObjectRef, control: UiObjectRef) -> Result<(), UiRefusal> {
        self.check(win)?;
        self.check(control)?;
        self.retire(control.id);
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
    fn present_plane(&mut self, win: UiObjectRef, plane_damage: Plane) -> Result<(), UiRefusal> {
        self.check(win)?;
        plane_damage.validate()?;
        let (new, old) = (plane_damage.rect, self.plane_frames.entry((win.id, plane_damage.id)).or_default());
        old.retain(|f| !(f.rect.left >= new.left && f.rect.top >= new.top && f.rect.left + f.rect.width <= new.left + new.width && f.rect.top + f.rect.height <= new.top + new.height));
        old.push(plane_damage.clone());
        self.planes.insert((win.id, plane_damage.id), plane_damage.clone());
        self.calls.push(UiCall::PresentPlane(win, plane_damage));
        Ok(())
    }
    fn request(&mut self, win: Option<UiObjectRef>, model: RequestModel) -> Result<RequestRef, UiRefusal> {
        if self.refuse_requests { return Err(UiRefusal::Unsupported("requests (no alert part in this host)")); }
        if let Some(w) = win { self.check(w)?; }
        model.validate()?;
        self.calls.push(UiCall::Request(win, model.clone()));
        let req = self.alloc(0, CallbackToken(0));
        self.requests.insert(req.id, model);
        let queued = self.request_answers.pop_front();
        let cancel = self.requests.get(&req.id).map(|m| m.cancel.unwrap_or(m.buttons.len() as u32 - 1));
        if let Some(b) = queued.or(cancel.filter(|_| self.auto_cancel_requests)) { self.answer_request(req, b)?; }
        Ok(req)
    }
    fn close_request(&mut self, req: RequestRef) -> Result<(), UiRefusal> {
        self.check(req)?;
        self.requests.remove(&req.id).ok_or(UiRefusal::UnknownObject(req))?;
        self.retire(req.id);
        self.calls.push(UiCall::CloseRequest(req));
        Ok(())
    }
    fn open_audio_stream(&mut self, model: AudioStreamModel) -> Result<AudioStreamRef, UiRefusal> {
        if self.refuse_audio { return Err(UiRefusal::Unsupported("audio out (no audio service in this host)")); }
        model.validate()?;
        self.calls.push(UiCall::OpenAudio(model.clone()));
        let s = self.alloc(0, CallbackToken(0));
        self.audio.insert(s.id, RecordedAudio { model, queued: Vec::new(), played: 0, paused: false, volume: (AUDIO_UNITY, AUDIO_UNITY) });
        Ok(s)
    }
    fn queue_audio(&mut self, stream: AudioStreamRef, pcm: &[u8]) -> Result<(), UiRefusal> {
        if self.refuse_audio { return Err(UiRefusal::Unsupported("audio out (no audio service in this host)")); }
        self.check(stream)?;
        let a = self.audio.get_mut(&stream.id).ok_or(UiRefusal::UnknownObject(stream))?;
        if pcm.len() as u32 % a.model.frame_bytes() != 0 { return Err(UiRefusal::InvalidGeometry("queued audio is not a whole number of frames")); }
        a.queued.extend_from_slice(pcm);
        if self.drain_audio { a.played = a.queued.len(); }
        self.calls.push(UiCall::QueueAudio(stream, pcm.len()));
        Ok(())
    }
    fn audio_queued_bytes(&mut self, stream: AudioStreamRef) -> Result<u32, UiRefusal> {
        if self.refuse_audio { return Err(UiRefusal::Unsupported("audio out (no audio service in this host)")); }
        self.check(stream)?;
        let a = self.audio.get(&stream.id).ok_or(UiRefusal::UnknownObject(stream))?;
        Ok((a.queued.len() - a.played) as u32)
    }
    fn flush_audio(&mut self, stream: AudioStreamRef) -> Result<(), UiRefusal> {
        if self.refuse_audio { return Err(UiRefusal::Unsupported("audio out (no audio service in this host)")); }
        self.check(stream)?;
        let a = self.audio.get_mut(&stream.id).ok_or(UiRefusal::UnknownObject(stream))?;
        a.queued.truncate(a.played);
        self.calls.push(UiCall::FlushAudio(stream));
        Ok(())
    }
    fn pause_audio(&mut self, stream: AudioStreamRef, paused: bool) -> Result<(), UiRefusal> {
        if self.refuse_audio { return Err(UiRefusal::Unsupported("audio out (no audio service in this host)")); }
        self.check(stream)?;
        self.audio.get_mut(&stream.id).ok_or(UiRefusal::UnknownObject(stream))?.paused = paused;
        self.calls.push(UiCall::PauseAudio(stream, paused));
        Ok(())
    }
    fn set_audio_volume(&mut self, stream: AudioStreamRef, left: u16, right: u16) -> Result<(), UiRefusal> {
        if self.refuse_audio { return Err(UiRefusal::Unsupported("audio out (no audio service in this host)")); }
        self.check(stream)?;
        if left > AUDIO_UNITY || right > AUDIO_UNITY { return Err(UiRefusal::InvalidGeometry("audio gain above unity")); }
        self.audio.get_mut(&stream.id).ok_or(UiRefusal::UnknownObject(stream))?.volume = (left, right);
        self.calls.push(UiCall::SetAudioVolume(stream, left, right));
        Ok(())
    }
    fn close_audio_stream(&mut self, stream: AudioStreamRef) -> Result<(), UiRefusal> {
        if self.refuse_audio { return Err(UiRefusal::Unsupported("audio out (no audio service in this host)")); }
        self.check(stream)?;
        self.audio.remove(&stream.id).ok_or(UiRefusal::UnknownObject(stream))?;
        self.retire(stream.id);
        self.calls.push(UiCall::CloseAudio(stream));
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
        let bar = ui.add_control(w, ctl(ControlKind::ScrollBar { vertical: true, total: 100, visible: 10, top: 0 })).unwrap();
        ui.set_control(w, bar, ControlValue::Scroll { total: 100, visible: 10, top: 500 }).unwrap();
        assert!(matches!(ui.controls[&bar.id].1.kind, ControlKind::ScrollBar { top: 90, .. }), "top clamps to total - visible");
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

    #[test]
    fn the_census_keeps_a_hole_a_later_partial_frame_did_not_repaint() {
        let mut ui = RecordingGuestUi::new();
        let w = ui.open_window(win()).unwrap();
        let r = |left, width| GuestRect { left, top: 0, width, height: 2 };
        let fill = |rect| DrawOp::Fill { rect, pen_role: PenRole::Background };
        let hole = DrawOp::Pixels { rect: r(0, 1), rgb: alloc::vec![0; 2] };
        ui.present_plane(w, Plane { id: 0, rect: r(0, 8), ops: alloc::vec![fill(r(0, 8)), hole] }).unwrap();
        assert_eq!(ui.route_census(w), RouteCensus { native: 0, replayed: 0, raw: 1 });
        // A later frame over another part, with no hole, does not clear the hole still on screen.
        ui.present_plane(w, Plane { id: 0, rect: r(4, 2), ops: alloc::vec![fill(r(4, 2))] }).unwrap();
        assert_eq!(ui.route_census(w), RouteCensus { native: 0, replayed: 0, raw: 1 }, "the hole at x=0 is still shown");
        // CONTROL: a frame that repaints the hole's frame entirely replaces it, and the plane is replayed.
        ui.present_plane(w, Plane { id: 0, rect: r(0, 8), ops: alloc::vec![fill(r(0, 8))] }).unwrap();
        assert_eq!(ui.route_census(w), RouteCensus { native: 0, replayed: 1, raw: 0 });
    }

    fn ask(buttons: &[&str]) -> RequestModel {
        RequestModel { title: "T".into(), text: "Q?".into(), buttons: buttons.iter().map(|b| (*b).into()).collect(), default: Some(0), cancel: Some(buttons.len() as u32 - 1), kind: RequestKind::default() }
    }

    #[test]
    fn a_request_is_answered_once_by_a_queued_or_manual_answer() {
        let mut ui = RecordingGuestUi::new();
        ui.request_answers.push_back(1);
        let r1 = ui.request(None, ask(&["Yes", "No"])).unwrap();
        assert_eq!(ui.poll_event(), Some(GuestUiEvent::RequestAnswered { req: r1, button: 1 }), "the queued answer comes back as the host's event");
        // Negative controls: an answered request is gone, so a second answer is stale and nothing more is queued.
        assert_eq!(ui.answer_request(r1, 0), Err(UiRefusal::StaleGeneration(r1)));
        assert_eq!(ui.poll_event(), None);
        // No queued answer: the request stays pending until the test answers it.
        let r2 = ui.request(None, ask(&["OK"])).unwrap();
        assert_eq!(ui.poll_event(), None);
        assert_eq!(ui.answer_request(r2, 3), Err(UiRefusal::InvalidGeometry("answer past the request's buttons")));
        assert!(ui.requests.contains_key(&r2.id), "a refused answer leaves the request open");
        ui.answer_request(r2, 0).unwrap();
        assert_eq!(ui.poll_event(), Some(GuestUiEvent::RequestAnswered { req: r2, button: 0 }));
    }

    #[test]
    fn a_headless_run_dismisses_requests_with_their_escape_button() {
        let mut ui = RecordingGuestUi::new();
        ui.auto_cancel_requests = true;
        let r = ui.request(None, RequestModel { cancel: Some(1), ..ask(&["A", "B", "C"]) }).unwrap();
        assert_eq!(ui.poll_event(), Some(GuestUiEvent::RequestAnswered { req: r, button: 1 }), "its cancel button");
        let r = ui.request(None, RequestModel { cancel: None, ..ask(&["A", "B", "C"]) }).unwrap();
        assert_eq!(ui.poll_event(), Some(GuestUiEvent::RequestAnswered { req: r, button: 2 }), "no cancel: the last button");
        ui.request_answers.push_back(0);
        let r = ui.request(None, ask(&["A", "B"])).unwrap();
        assert_eq!(ui.poll_event(), Some(GuestUiEvent::RequestAnswered { req: r, button: 0 }), "a queued answer wins");
        ui.auto_cancel_requests = false;
        ui.request(None, ask(&["A", "B"])).unwrap();
        assert_eq!(ui.poll_event(), None, "negative control: off, a request just waits");
    }

    #[test]
    fn a_withdrawn_request_gets_no_answer_and_bad_questions_are_refused() {
        let mut ui = RecordingGuestUi::new();
        let r = ui.request(None, ask(&["A", "B"])).unwrap();
        ui.close_request(r).unwrap();
        assert!(matches!(ui.calls.last(), Some(UiCall::CloseRequest(x)) if *x == r));
        assert_eq!(ui.answer_request(r, 0), Err(UiRefusal::StaleGeneration(r)), "closed means stale");
        assert_eq!(ui.close_request(r), Err(UiRefusal::StaleGeneration(r)));
        assert!(ui.request(None, RequestModel { buttons: vec![], ..ask(&["x"]) }).is_err(), "no buttons");
        assert!(ui.request(None, RequestModel { cancel: Some(5), ..ask(&["x"]) }).is_err(), "cancel past the buttons");
        let ghost = UiObjectRef { id: 99, generation: 1 };
        assert_eq!(ui.request(Some(ghost), ask(&["x"])), Err(UiRefusal::UnknownObject(ghost)), "a window that is not open");
    }
}

#[cfg(test)]
mod audio_tests {
    use super::*;
    fn model() -> AudioStreamModel { AudioStreamModel { rate_hz: 22_254, channels: 1, format: AudioFormat::U8, label: "t".into() } }

    #[test]
    fn a_stream_queues_plays_flushes_and_closes_and_a_stale_ref_is_refused() {
        let mut ui = RecordingGuestUi::new();
        let s = ui.open_audio_stream(model()).unwrap();
        ui.queue_audio(s, &[1, 2, 3, 4]).unwrap();
        assert_eq!(ui.audio_queued_bytes(s).unwrap(), 4);
        assert_eq!(ui.play_audio(s, 3).unwrap(), 3);
        assert_eq!(ui.audio_queued_bytes(s).unwrap(), 1, "the host clock decides how much is left");
        ui.pause_audio(s, true).unwrap();
        assert_eq!(ui.play_audio(s, 9).unwrap(), 0, "a paused stream plays nothing");
        ui.flush_audio(s).unwrap();
        assert_eq!(ui.audio_queued_bytes(s).unwrap(), 0);
        ui.close_audio_stream(s).unwrap();
        assert!(matches!(ui.queue_audio(s, &[0]), Err(UiRefusal::StaleGeneration(_))), "a closed stream is refused by name");
    }

    #[test]
    fn the_defaults_and_bad_models_refuse_by_name() {
        struct Mute;
        impl GuestUi for Mute {
            fn open_window(&mut self, _: WindowModel) -> Result<UiObjectRef, UiRefusal> { unreachable!() }
            fn close_window(&mut self, _: UiObjectRef) -> Result<(), UiRefusal> { unreachable!() }
            fn set_window_title(&mut self, _: UiObjectRef, _: alloc::string::String) -> Result<(), UiRefusal> { unreachable!() }
            fn set_menu_strip(&mut self, _: UiObjectRef, _: MenuModel) -> Result<(), UiRefusal> { unreachable!() }
            fn clear_menu_strip(&mut self, _: UiObjectRef) -> Result<(), UiRefusal> { unreachable!() }
            fn add_control(&mut self, _: UiObjectRef, _: ControlModel) -> Result<UiObjectRef, UiRefusal> { unreachable!() }
            fn remove_control(&mut self, _: UiObjectRef, _: UiObjectRef) -> Result<(), UiRefusal> { unreachable!() }
            fn set_control(&mut self, _: UiObjectRef, _: UiObjectRef, _: ControlValue) -> Result<(), UiRefusal> { unreachable!() }
            fn poll_event(&mut self) -> Option<GuestUiEvent> { None }
        }
        assert!(matches!(Mute.open_audio_stream(model()), Err(UiRefusal::Unsupported(_))), "a host without an audio part says so");
        let mut ui = RecordingGuestUi::new();
        for bad in [AudioStreamModel { rate_hz: 0, ..model() }, AudioStreamModel { channels: 3, ..model() }, AudioStreamModel { channels: 0, ..model() }] {
            assert!(matches!(ui.open_audio_stream(bad), Err(UiRefusal::InvalidGeometry(_))));
        }
        let s = ui.open_audio_stream(AudioStreamModel { channels: 2, format: AudioFormat::S16Le, ..model() }).unwrap();
        assert!(matches!(ui.queue_audio(s, &[0; 6]), Err(UiRefusal::InvalidGeometry(_))), "6 bytes is not whole 4-byte frames");
        assert!(matches!(ui.set_audio_volume(s, AUDIO_UNITY + 1, 0), Err(UiRefusal::InvalidGeometry(_))));
        ui.refuse_audio = true;
        assert!(matches!(ui.queue_audio(s, &[0; 4]), Err(UiRefusal::Unsupported(_))));
    }
}

#[cfg(test)]
mod drain_tests {
    use super::*;
    #[test]
    fn a_draining_headless_host_plays_what_is_queued_at_once_and_the_default_does_not() {
        let m = AudioStreamModel { rate_hz: 8000, channels: 1, format: AudioFormat::U8, label: "t".into() };
        let mut ui = RecordingGuestUi::new();
        let s = ui.open_audio_stream(m.clone()).unwrap();
        ui.queue_audio(s, &[1, 2, 3]).unwrap();
        assert_eq!(ui.audio_queued_bytes(s).unwrap(), 3, "by default nothing plays until the test says so");
        ui.drain_audio = true;
        ui.queue_audio(s, &[4, 5]).unwrap();
        assert_eq!(ui.audio_queued_bytes(s).unwrap(), 0, "draining: everything queued has played");
    }
}
