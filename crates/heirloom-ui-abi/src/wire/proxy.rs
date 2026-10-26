//! The engine side of the guest UI wire: a `GuestUi` that encodes every call as a frame for the host.
//! Named limit: `icon_image` and `audio_queued_bytes` answer None / Err(Unsupported) over the wire, because `UiCall`
//! has no variant carrying their replies. DELETE WHEN `HostProfile` lands (23-ENGINE-SPLIT.md, phase 3).
//! Audio is not on this wire at all: the engine plays through its own Aura session (23-ENGINE-SPLIT.md:256), so the
//! proxy keeps `GuestUi`'s default audio refusals.

use crate::wire::{encode_call, encode_lease, decode, Frame, WireRefusal};
use crate::recording::{UiCall, pixel_hash};
use crate::model::*;
use crate::event::*;
use crate::request::{RequestModel, RequestRef};
use crate::plane::*;
use crate::{GuestUi, IconRole};
use alloc::vec::Vec;
use alloc::string::String;

pub trait WireTransport {
    fn send(&mut self, frame: &[u8]);
    fn ask(&mut self, frame: &[u8]) -> Vec<u8>;
    fn recv(&mut self) -> Option<Vec<u8>>;
    fn lease_canvas(&mut self, win: UiObjectRef, width: i32, height: i32, pixels: &[u32]) -> u32;
}

pub struct ProxyUi<T: WireTransport> {
    transport: T,
    next_window: u32,
    next_control: u32,
    next_request: u32,
    call_seq: u32,
    refusals: Vec<(u32, WireRefusal)>,
    bad_frame_count: u32,
}

impl<T: WireTransport> ProxyUi<T> {
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            next_window: 0,
            next_control: 0,
            next_request: 0,
            call_seq: 0,
            refusals: Vec::new(),
            bad_frame_count: 0,
        }
    }

    pub fn take_refusals(&mut self) -> Vec<(u32, WireRefusal)> {
        core::mem::take(&mut self.refusals)
    }

    pub fn bad_frames(&self) -> u32 {
        self.bad_frame_count
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }

    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    fn mint_window(&mut self) -> UiObjectRef {
        self.next_window += 1;
        UiObjectRef { id: self.next_window, generation: 1 }
    }
    fn mint_control(&mut self) -> UiObjectRef {
        self.next_control += 1;
        UiObjectRef { id: self.next_control, generation: 1 }
    }
    fn mint_request(&mut self) -> RequestRef {
        self.next_request += 1;
        RequestRef { id: self.next_request, generation: 1 }
    }
    /// A call that cannot be encoded (longer than the wire's frame limit) is refused here and never sent, so the host's
    /// call tokens and object ids stay in step with this side's.
    fn send_call(&mut self, call: &UiCall) -> Result<(), UiRefusal> {
        let mut frame = Vec::new();
        encode_call(call, &mut frame).map_err(|_| UiRefusal::Unsupported("a UI call longer than the wire's frame limit"))?;
        self.call_seq += 1;
        self.transport.send(&frame);
        Ok(())
    }
}

impl<T: WireTransport + 'static> GuestUi for ProxyUi<T> {
    fn open_window(&mut self, model: WindowModel) -> Result<UiObjectRef, UiRefusal> {
        self.send_call(&UiCall::OpenWindow(model))?;
        Ok(self.mint_window())
    }
    fn close_window(&mut self, win: UiObjectRef) -> Result<(), UiRefusal> {
        self.send_call(&UiCall::CloseWindow(win))?;
        Ok(())
    }
    fn set_window_title(&mut self, win: UiObjectRef, title: String) -> Result<(), UiRefusal> {
        self.send_call(&UiCall::SetWindowTitle(win, title))?;
        Ok(())
    }
    fn set_menu_strip(&mut self, win: UiObjectRef, menu: MenuModel) -> Result<(), UiRefusal> {
        self.send_call(&UiCall::SetMenuStrip(win, menu))?;
        Ok(())
    }
    fn clear_menu_strip(&mut self, win: UiObjectRef) -> Result<(), UiRefusal> {
        self.send_call(&UiCall::ClearMenuStrip(win))?;
        Ok(())
    }
    fn hide_window(&mut self, win: UiObjectRef) -> Result<(), UiRefusal> {
        self.send_call(&UiCall::HideWindow(win))?;
        Ok(())
    }
    fn show_window(&mut self, win: UiObjectRef) -> Result<(), UiRefusal> {
        self.send_call(&UiCall::ShowWindow(win))?;
        Ok(())
    }
    fn add_control(&mut self, win: UiObjectRef, control: ControlModel) -> Result<UiObjectRef, UiRefusal> {
        self.send_call(&UiCall::AddControl(win, control))?;
        Ok(self.mint_control())
    }
    fn remove_control(&mut self, win: UiObjectRef, control: UiObjectRef) -> Result<(), UiRefusal> {
        self.send_call(&UiCall::RemoveControl(win, control))?;
        Ok(())
    }
    fn set_control(&mut self, win: UiObjectRef, control: UiObjectRef, value: ControlValue) -> Result<(), UiRefusal> {
        self.send_call(&UiCall::SetControl(win, control, value))?;
        Ok(())
    }
    fn set_control_rect(&mut self, win: UiObjectRef, control: UiObjectRef, rect: GuestRect) -> Result<(), UiRefusal> {
        self.send_call(&UiCall::SetControlRect(win, control, rect))?;
        Ok(())
    }
    fn set_control_enabled(&mut self, win: UiObjectRef, control: UiObjectRef, enabled: bool) -> Result<(), UiRefusal> {
        self.send_call(&UiCall::SetControlEnabled(win, control, enabled))?;
        Ok(())
    }
    fn present_content(&mut self, win: UiObjectRef, damage: GuestRect, width: i32, height: i32, pixels: &[u32]) -> Result<(), UiRefusal> {
        let region = self.transport.lease_canvas(win, width, height, pixels);
        let mut lease_frame = Vec::new();
        let _ = encode_lease(win, region, 1, &mut lease_frame);
        self.transport.send(&lease_frame);
        let hash = pixel_hash(pixels);
        self.send_call(&UiCall::PresentContent { win, damage, width, height, hash })?;
        Ok(())
    }
    fn present_plane(&mut self, win: UiObjectRef, plane: Plane) -> Result<(), UiRefusal> {
        self.send_call(&UiCall::PresentPlane(win, plane))?;
        Ok(())
    }
    fn icon_image(&mut self, _role: IconRole, _width: u32, _height: u32) -> Option<Vec<u32>> {
        None // the named limit in the module doc
    }
    fn request(&mut self, win: Option<UiObjectRef>, model: RequestModel) -> Result<RequestRef, UiRefusal> {
        self.send_call(&UiCall::Request(win, model))?;
        Ok(self.mint_request())
    }
    fn close_request(&mut self, req: RequestRef) -> Result<(), UiRefusal> {
        self.send_call(&UiCall::CloseRequest(req))?;
        Ok(())
    }
    fn poll_event(&mut self) -> Option<GuestUiEvent> {
        loop {
            let frame = self.transport.recv()?;
            match decode(&frame) {
                Ok((Frame::Event(event), _)) => return Some(event),
                Ok((Frame::Refused { token, why }, _)) => {
                    self.refusals.push((token.0 as u32, why));
                    continue;
                }
                Ok(_) => {
                    // Unexpected frame kind from host (Call or Lease).
                    self.bad_frame_count += 1;
                    continue;
                }
                Err(_) => {
                    self.bad_frame_count += 1;
                    continue;
                }
            }
        }
    }
}
