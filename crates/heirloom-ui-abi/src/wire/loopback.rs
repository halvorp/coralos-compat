//! A `WireTransport` that feeds frames straight into a `WireHost` in the same process: the phase-0 equivalence harness.
use crate::wire::{decode, Frame};
use crate::wire::host::WireHost;
use crate::wire::proxy::WireTransport;
use crate::model::UiObjectRef;
use crate::GuestUi;
use alloc::vec::Vec;
use alloc::collections::VecDeque;

pub struct Loopback<G: GuestUi> {
    host: WireHost<G>,
    pending: VecDeque<Vec<u8>>,
    canvases: Vec<(u32, Vec<u32>)>,
    next_region: u32,
}

impl<G: GuestUi> Loopback<G> {
    pub fn new(ui: G) -> Self {
        Self {
            host: WireHost::new(ui),
            pending: VecDeque::new(),
            canvases: Vec::new(),
            next_region: 0,
        }
    }

    pub fn ui(&self) -> &G {
        self.host.ui()
    }

    pub fn ui_mut(&mut self) -> &mut G {
        self.host.ui_mut()
    }

    fn drain_host(&mut self) {
        self.pending.extend(self.host.take_refused());
        self.pending.extend(self.host.pump_events());
    }
}

impl<G: GuestUi> WireTransport for Loopback<G> {
    fn send(&mut self, frame: &[u8]) {
        // Pixels travel out-of-band; hand them over before the lease makes
        // its region pending, so the following PresentContent can take them.
        if let Ok((Frame::Lease { region, .. }, _)) = decode(frame) {
            if let Some(index) = self.canvases.iter().position(|(id, _)| *id == region) {
                let (_, pixels) = self.canvases.remove(index);
                self.host.store_canvas(region, pixels);
            }
        }
        let _ = self.host.apply(frame);
        self.drain_host();
    }

    fn ask(&mut self, frame: &[u8]) -> Vec<u8> {
        self.send(frame);
        self.recv().unwrap_or_default()
    }

    fn recv(&mut self) -> Option<Vec<u8>> {
        self.drain_host();
        self.pending.pop_front()
    }

    fn lease_canvas(&mut self, _win: UiObjectRef, _width: i32, _height: i32, pixels: &[u32]) -> u32 {
        self.next_region += 1;
        let region = self.next_region;
        self.canvases.push((region, pixels.to_vec()));
        region
    }
}