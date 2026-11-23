//! Symbian `CMdaAudioOutputStream` PCM audio output bridged to `GuestUi` audio streams (`fbs.rs` split).
use alloc::format;
use alloc::string::String;
use crate::loader::Process;
use crate::natives::NativeOutcome;
use crate::objects::HostObj;
use crate::enginestub::slots_with_gaps;
use heirloom_ui_abi::{AudioFormat, AudioStreamModel, UiObjectRef};

#[derive(Debug, Clone)]
pub struct AudioOutState {
    pub callback: u32,
    pub stream: Option<UiObjectRef>,
    pub rate_hz: u32,
    pub channels: u8,
    pub volume: u32,
    pub pos_addr: u32,
    pub bytes_played: u64,
}

const AUDIO_OUT_SLOTS: usize = 20;
const AUDIO_OUT_VTABLE: &[(usize, &str)] = &[
    (0, "_ZN21CMdaAudioOutputStreamD1Ev"),
    (1, "_ZN21CMdaAudioOutputStreamD0Ev"),
    (2, "_ZN5CBase10Extension_EjRPvS0_"),
    (3, "_ZN21CMdaAudioOutputStream19SetAudioPropertiesLEii"),
    (4, "_ZN21CMdaAudioOutputStream4OpenEP11TMdaPackage"),
    (5, "_ZN21CMdaAudioOutputStream9MaxVolumeEv"),
    (6, "_ZN21CMdaAudioOutputStream6VolumeEv"),
    (7, "_ZN21CMdaAudioOutputStream9SetVolumeEi"),
    (8, "_ZN21CMdaAudioOutputStream11SetPriorityEi22TMdaPriorityPreference"),
    (9, "_ZN21CMdaAudioOutputStream6WriteLERK6TDesC8"),
    (10, "_ZN21CMdaAudioOutputStream4StopEv"),
    (11, "_ZN21CMdaAudioOutputStream8PositionEv"),
    (12, "_ZN21CMdaAudioOutputStream8GetBytesEv"),
    (13, "_ZN21CMdaAudioOutputStream11SetBalanceLEi"),
    (14, "_ZN21CMdaAudioOutputStream10GetBalanceEv"),
];

fn symbian_sample_rate(code: u32) -> u32 {
    match code {
        0x0001 => 8000,
        0x0002 => 11025,
        0x0004 => 16000,
        0x0008 => 22050,
        0x0010 => 32000,
        0x0020 => 44100,
        0x0040 => 48000,
        0x0080 => 88200,
        0x0100 => 96000,
        0x0200 => 12000,
        0x0400 => 24000,
        0x0800 => 64000,
        _ => 16000,
    }
}


pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let [r0, r1, r2, _r3] = [p.cpu.regs[0], p.cpu.regs[1], p.cpu.regs[2], p.cpu.regs[3]];
    let refuse = |m: String| Some(NativeOutcome::Refused(format!("NEXT-NEEDED audio {name}: {m}")));
    Some(match name {
        // CMdaAudioOutputStream::NewL(MMdaAudioOutputStreamCallback& aCallBack, CMdaServer* aServer)
        "_ZN21CMdaAudioOutputStream4NewLER29MMdaAudioOutputStreamCallbackP10CMdaServer" => {
            let callback = r0;
            let Some(obj) = p.alloc_zeroed(0x30) else { return refuse("no heap for CMdaAudioOutputStream".into()) };
            let Ok(vptr) = p.build_vtable("CMdaAudioOutputStream", &slots_with_gaps(AUDIO_OUT_SLOTS, AUDIO_OUT_VTABLE)) else { return refuse("vtable".into()) };
            let _ = p.wr(obj, vptr);
            let pos_addr = p.alloc_zeroed(8).unwrap_or(0);
            let _ = p.obj_set(obj, HostObj::AudioOut(AudioOutState {
                callback, stream: None, rate_hz: 16000, channels: 1, volume: 10, pos_addr, bytes_played: 0,
            }));
            p.cpu.regs[0] = obj;
            NativeOutcome::Return
        }
        "_ZN21CMdaAudioOutputStreamD1Ev" | "_ZN21CMdaAudioOutputStreamD0Ev" => {
            let stream = match p.obj_get(r0) { Some(HostObj::AudioOut(a)) => a.stream.take(), _ => None };
            if let (Some(s), Some(ui)) = (stream, p.host.ui.as_mut()) { let _ = ui.close_audio_stream(s); }
            p.obj_remove(r0);
            if name.ends_with("D0Ev") { if let Some(heap) = p.heap.as_mut() { let _ = heap.free(r0); } }
            NativeOutcome::Return
        }
        "_ZN21CMdaAudioOutputStream19SetAudioPropertiesLEii" => {
            if let Some(HostObj::AudioOut(a)) = p.obj_get(r0) {
                a.rate_hz = symbian_sample_rate(r1);
                a.channels = if r2 == 2 { 2 } else { 1 };
            }
            NativeOutcome::Return
        }
        "_ZN21CMdaAudioOutputStream4OpenEP11TMdaPackage" => {
            let info = match p.obj_get(r0) {
                Some(HostObj::AudioOut(a)) => Some((a.callback, a.rate_hz, a.channels)),
                _ => None,
            };
            if let Some((cb, rate_hz, channels)) = info {
                let opened = p.host.ui.as_mut().and_then(|ui| {
                    ui.open_audio_stream(AudioStreamModel {
                        rate_hz,
                        channels,
                        format: AudioFormat::S16Le,
                        label: String::from("Symbian CMdaAudioOutputStream"),
                    }).ok()
                });
                if let Some(HostObj::AudioOut(a)) = p.obj_get(r0) { a.stream = opened; }
                // Invoke callback->MaoscOpenComplete(KErrNone) (slot 0 of MMdaAudioOutputStreamCallback)
                if cb != 0 {
                    if let Ok(vt) = p.rd(cb) {
                        if let Ok(fn_ptr) = p.rd(vt) {
                            if fn_ptr != 0 {
                                let _ = p.call_guest(fn_ptr, &[cb, 0], crate::callback::Continuation::Resume);
                            }
                        }
                    }
                }
            }
            NativeOutcome::Return
        }
        "_ZN21CMdaAudioOutputStream9MaxVolumeEv" => { p.cpu.regs[0] = 10; NativeOutcome::Return }
        "_ZN21CMdaAudioOutputStream6VolumeEv" => {
            p.cpu.regs[0] = match p.obj_get(r0) { Some(HostObj::AudioOut(a)) => a.volume, _ => 10 };
            NativeOutcome::Return
        }
        "_ZN21CMdaAudioOutputStream9SetVolumeEi" => {
            if let Some(HostObj::AudioOut(a)) = p.obj_get(r0) { a.volume = r1.min(10); }
            NativeOutcome::Return
        }
        "_ZN21CMdaAudioOutputStream11SetPriorityEi22TMdaPriorityPreference"
        | "_ZN21CMdaAudioOutputStream11SetBalanceLEi" => NativeOutcome::Return,
        "_ZN21CMdaAudioOutputStream10GetBalanceEv" => { p.cpu.regs[0] = 0; NativeOutcome::Return }
        "_ZN21CMdaAudioOutputStream6WriteLERK6TDesC8" => {
            let pcm = crate::desc::read8(p, r1).unwrap_or_default();
            let info = match p.obj_get(r0) {
                Some(HostObj::AudioOut(a)) => {
                    a.bytes_played += pcm.len() as u64;
                    a.stream
                }
                _ => None,
            };
            if let (Some(s), Some(ui)) = (info, p.host.ui.as_mut()) {
                if !pcm.is_empty() { let _ = ui.queue_audio(s, &pcm); }
            }
            NativeOutcome::Return
        }
        "_ZN21CMdaAudioOutputStream4StopEv" => {
            let s = match p.obj_get(r0) { Some(HostObj::AudioOut(a)) => a.stream, _ => None };
            if let (Some(s), Some(ui)) = (s, p.host.ui.as_mut()) { let _ = ui.flush_audio(s); }
            NativeOutcome::Return
        }
        "_ZN21CMdaAudioOutputStream8PositionEv" => {
            let pos_addr = match p.obj_get(r0) { Some(HostObj::AudioOut(a)) => a.pos_addr, _ => 0 };
            p.cpu.regs[0] = pos_addr;
            NativeOutcome::Return
        }
        "_ZN21CMdaAudioOutputStream8GetBytesEv" => {
            p.cpu.regs[0] = match p.obj_get(r0) { Some(HostObj::AudioOut(a)) => a.bytes_played as u32, _ => 0 };
            NativeOutcome::Return
        }
        _ => return None,
    })
}
