//! The host side of the guest UI wire: decodes the engine's frames and makes each call on the real `GuestUi`.
//! `store_canvas` is crate-private: pixels never ride the ring, so the loopback hands them over out of band.
//! Audio frames are refused: audio does not cross this wire, the engine plays through its own Aura session
//! (23-ENGINE-SPLIT.md:256), and a PCM-less frame must never become played silence.

use crate::wire::{decode, encode_event, encode_refused, Frame, WireError};
use crate::recording::UiCall;
use crate::model::*;
use crate::event::*;
use crate::request::RequestRef;
use crate::GuestUi;
use alloc::vec::Vec;
use alloc::collections::BTreeMap;

pub struct WireHost<G: GuestUi> {
    ui: G,
    window_map: BTreeMap<u32, UiObjectRef>,
    control_map: BTreeMap<u32, UiObjectRef>,
    request_map: BTreeMap<u32, UiObjectRef>,
    window_counter: u32,
    control_counter: u32,
    request_counter: u32,
    call_seq: u32,
    refused: Vec<Vec<u8>>,
    pending_lease_region: Option<u32>,
    canvas_pixels: BTreeMap<u32, Vec<u32>>,
}

impl<G: GuestUi> WireHost<G> {
    pub fn new(ui: G) -> Self {
        Self {
            ui,
            window_map: BTreeMap::new(),
            control_map: BTreeMap::new(),
            request_map: BTreeMap::new(),
            window_counter: 0,
            control_counter: 0,
            request_counter: 0,
            call_seq: 0,
            refused: Vec::new(),
            pending_lease_region: None,
            canvas_pixels: BTreeMap::new(),
        }
    }

    pub fn ui(&self) -> &G { &self.ui }

    pub fn ui_mut(&mut self) -> &mut G { &mut self.ui }

    pub fn apply(&mut self, frame: &[u8]) -> Result<(), WireError> {
        let (decoded, _) = decode(frame)?;
        match decoded {
            Frame::Call(call) => {
                self.call_seq += 1;
                self.apply_call(call);
                Ok(())
            }
            Frame::Lease { window: _, region, generation: _ } => {
                self.pending_lease_region = Some(region);
                Ok(())
            }
            Frame::Event(_) => Err(WireError::UnknownKind(2)),
            Frame::Refused { .. } => Err(WireError::UnknownKind(3)),
        }
    }

    pub fn pump_events(&mut self) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        while let Some(event) = self.ui.poll_event() {
            let translated = self.translate_event(event);
            let mut frame = Vec::new();
            let _ = encode_event(&translated, &mut frame);
            out.push(frame);
        }
        out
    }

    pub fn take_refused(&mut self) -> Vec<Vec<u8>> {
        core::mem::take(&mut self.refused)
    }

    pub(crate) fn store_canvas(&mut self, region: u32, pixels: Vec<u32>) {
        self.canvas_pixels.insert(region, pixels);
    }

    fn translate_window(&self, engine: UiObjectRef) -> UiObjectRef {
        self.window_map.get(&engine.id).copied().unwrap_or(engine)
    }
    fn translate_control(&self, engine: UiObjectRef) -> UiObjectRef {
        self.control_map.get(&engine.id).copied().unwrap_or(engine)
    }
    fn translate_request(&self, engine: RequestRef) -> RequestRef {
        self.request_map.get(&engine.id).copied().unwrap_or(engine)
    }

    fn rev_window(&self, host: UiObjectRef) -> UiObjectRef {
        for (engine_id, h) in &self.window_map {
            if h.id == host.id {
                return UiObjectRef { id: *engine_id, generation: 1 };
            }
        }
        host
    }
    fn rev_control(&self, host: UiObjectRef) -> UiObjectRef {
        for (engine_id, h) in &self.control_map {
            if h.id == host.id {
                return UiObjectRef { id: *engine_id, generation: 1 };
            }
        }
        host
    }
    fn rev_request(&self, host: RequestRef) -> RequestRef {
        for (engine_id, h) in &self.request_map {
            if h.id == host.id {
                return RequestRef { id: *engine_id, generation: 1 };
            }
        }
        host
    }

    fn encode_refused(&mut self, refusal: &UiRefusal) {
        let mut frame = Vec::new();
        let _ = encode_refused(CallbackToken(self.call_seq as u64), refusal, &mut frame);
        self.refused.push(frame);
    }

    fn take_canvas_pixels(&mut self) -> Vec<u32> {
        match self.pending_lease_region.take() {
            Some(region) => self.canvas_pixels.remove(&region).unwrap_or_default(),
            None => Vec::new(),
        }
    }

    fn apply_call(&mut self, call: UiCall) {
        match call {
            UiCall::OpenWindow(model) => {
                self.window_counter += 1;
                let engine_id = self.window_counter;
                match self.ui.open_window(model) {
                    Ok(host) => { self.window_map.insert(engine_id, host); }
                    Err(r) => self.encode_refused(&r),
                }
            }
            UiCall::CloseWindow(win) => {
                let host = self.translate_window(win);
                if let Err(r) = self.ui.close_window(host) { self.encode_refused(&r); }
            }
            UiCall::SetWindowTitle(win, title) => {
                let host = self.translate_window(win);
                if let Err(r) = self.ui.set_window_title(host, title) { self.encode_refused(&r); }
            }
            UiCall::HideWindow(win) => {
                let host = self.translate_window(win);
                if let Err(r) = self.ui.hide_window(host) { self.encode_refused(&r); }
            }
            UiCall::ShowWindow(win) => {
                let host = self.translate_window(win);
                if let Err(r) = self.ui.show_window(host) { self.encode_refused(&r); }
            }
            UiCall::SetMenuStrip(win, menu) => {
                let host = self.translate_window(win);
                if let Err(r) = self.ui.set_menu_strip(host, menu) { self.encode_refused(&r); }
            }
            UiCall::ClearMenuStrip(win) => {
                let host = self.translate_window(win);
                if let Err(r) = self.ui.clear_menu_strip(host) { self.encode_refused(&r); }
            }
            UiCall::AddControl(win, control) => {
                self.control_counter += 1;
                let engine_id = self.control_counter;
                let host_win = self.translate_window(win);
                match self.ui.add_control(host_win, control) {
                    Ok(host) => { self.control_map.insert(engine_id, host); }
                    Err(r) => self.encode_refused(&r),
                }
            }
            UiCall::RemoveControl(win, control) => {
                let host_win = self.translate_window(win);
                let host_ctrl = self.translate_control(control);
                if let Err(r) = self.ui.remove_control(host_win, host_ctrl) { self.encode_refused(&r); }
            }
            UiCall::SetControl(win, control, value) => {
                let host_win = self.translate_window(win);
                let host_ctrl = self.translate_control(control);
                if let Err(r) = self.ui.set_control(host_win, host_ctrl, value) { self.encode_refused(&r); }
            }
            UiCall::SetControlRect(win, control, rect) => {
                let host_win = self.translate_window(win);
                let host_ctrl = self.translate_control(control);
                if let Err(r) = self.ui.set_control_rect(host_win, host_ctrl, rect) { self.encode_refused(&r); }
            }
            UiCall::SetControlEnabled(win, control, enabled) => {
                let host_win = self.translate_window(win);
                let host_ctrl = self.translate_control(control);
                if let Err(r) = self.ui.set_control_enabled(host_win, host_ctrl, enabled) { self.encode_refused(&r); }
            }
            UiCall::PresentContent { win, damage, width, height, hash: _ } => {
                let host_win = self.translate_window(win);
                let pixels = self.take_canvas_pixels();
                if let Err(r) = self.ui.present_content(host_win, damage, width, height, &pixels) {
                    self.encode_refused(&r);
                }
            }
            UiCall::PresentPlane(win, plane) => {
                let host_win = self.translate_window(win);
                if let Err(r) = self.ui.present_plane(host_win, plane) { self.encode_refused(&r); }
            }
            UiCall::Request(win, model) => {
                self.request_counter += 1;
                let engine_id = self.request_counter;
                let host_win = win.map(|w| self.translate_window(w));
                match self.ui.request(host_win, model) {
                    Ok(host) => { self.request_map.insert(engine_id, host); }
                    Err(r) => self.encode_refused(&r),
                }
            }
            UiCall::CloseRequest(req) => {
                let host = self.translate_request(req);
                if let Err(r) = self.ui.close_request(host) { self.encode_refused(&r); }
            }
            UiCall::OpenAudio(_) | UiCall::QueueAudio(..) | UiCall::QueueAudioLooped(..) | UiCall::FlushAudio(_)
            | UiCall::PauseAudio(..) | UiCall::SetAudioVolume(..) | UiCall::CloseAudio(_) => {
                self.encode_refused(&UiRefusal::Unsupported("audio on the UI wire (the engine owns its Aura session)"));
            }
        }
    }

    fn translate_event(&self, event: GuestUiEvent) -> GuestUiEvent {
        match event {
            GuestUiEvent::CloseRequested { win, token } =>
                GuestUiEvent::CloseRequested { win: self.rev_window(win), token },
            GuestUiEvent::MenuCommand { win, token } =>
                GuestUiEvent::MenuCommand { win: self.rev_window(win), token },
            GuestUiEvent::Resized { win, rect } =>
                GuestUiEvent::Resized { win: self.rev_window(win), rect },
            GuestUiEvent::Intent { win, control, token, generation, intent } =>
                GuestUiEvent::Intent {
                    win: self.rev_window(win),
                    control: self.rev_control(control),
                    token, generation, intent,
                },
            GuestUiEvent::Pointer { win, x, y, button, phase, clicks } =>
                GuestUiEvent::Pointer { win: self.rev_window(win), x, y, button, phase, clicks },
            GuestUiEvent::Key { win, code, text, modifiers, down } =>
                GuestUiEvent::Key { win: self.rev_window(win), code, text, modifiers, down },
            GuestUiEvent::RequestAnswered { req, button } =>
                GuestUiEvent::RequestAnswered { req: self.rev_request(req), button },
        }
    }
}