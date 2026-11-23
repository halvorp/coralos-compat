//! The telephony client (`etel`): when no S60 device ROM is mounted, reports `KErrNotSupported` and keeps handles zero.
//! When an S60 device ROM is mounted, provides S60's standard Offline/Airplane-mode ETel server (1 idle phone, 1 idle voice
//! line, no incoming cellular calls) so apps that monitor call state during initialization succeed cleanly.
use crate::loader::Process;
use crate::natives::NativeOutcome;

const KERR_NOT_SUPPORTED: u32 = -5i32 as u32; // e32err.h

fn has_offline_etel(p: &Process) -> bool {
    p.host.drive.as_ref().is_some_and(|d| d.has_rom())
}

fn write_tname(p: &mut Process, addr: u32, s: &str) {
    let units: alloc::vec::Vec<u16> = s.encode_utf16().collect();
    let _ = p.wr(addr, (crate::desc::DES_BUF << crate::desc::KSHIFT_DES_TYPE) | (units.len() as u32));
    let _ = p.wr(addr.wrapping_add(4), 128);
    for (i, u) in units.iter().enumerate() {
        let _ = p.mem.vm.write_u16(addr.wrapping_add(8 + (i as u32) * 2), *u);
    }
}

pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let this = p.cpu.regs[0];
    let r1 = p.cpu.regs[1];
    let r2 = p.cpu.regs[2];
    let offline = has_offline_etel(p);
    let words = match name {
        "_ZN10RTelServerC1Ev" => 1,
        "_ZN6RPhoneC1Ev" | "_ZN5RLineC1Ev" | "_ZN5RCallC1Ev" => 2,
        "_ZN10RTelServer7ConnectEi" => {
            if offline {
                let _ = p.wr(this, 1);
                p.cpu.regs[0] = 0;
            } else {
                p.cpu.regs[0] = KERR_NOT_SUPPORTED;
            }
            return Some(NativeOutcome::Return);
        }
        "_ZN6RPhone4OpenER10RTelServerRK7TDesC16" | "_ZN5RLine4OpenER6RPhoneRK7TDesC16" => {
            if offline {
                let _ = p.wr(this, 1);
                let _ = p.wr(this.wrapping_add(4), 1);
                p.cpu.regs[0] = 0;
            } else {
                p.cpu.regs[0] = KERR_NOT_SUPPORTED;
            }
            return Some(NativeOutcome::Return);
        }
        "_ZNK10RTelServer15LoadPhoneModuleERK7TDesC16"
        | "_ZNK10RTelServer17UnloadPhoneModuleERK7TDesC16" => {
            p.cpu.regs[0] = if offline { 0 } else { KERR_NOT_SUPPORTED };
            return Some(NativeOutcome::Return);
        }
        "_ZNK10RTelServer15EnumeratePhonesERi" => {
            let _ = p.wr(r1, if offline { 1 } else { 0 });
            p.cpu.regs[0] = if offline { 0 } else { KERR_NOT_SUPPORTED };
            return Some(NativeOutcome::Return);
        }
        "_ZNK10RTelServer12GetPhoneInfoEiRNS_10TPhoneInfoE" => {
            if offline {
                // RTelServer::TPhoneInfo: iNetworkType (+0), TName iName (+4, 264B), iNumberOfLines (+268), iExtensions (+272)
                let _ = p.wr(r2, 1); // ENetworkTypeMobileDigital
                write_tname(p, r2.wrapping_add(4), "Phone");
                let _ = p.wr(r2.wrapping_add(268), 1);
                let _ = p.wr(r2.wrapping_add(272), 0);
                p.cpu.regs[0] = 0;
            } else {
                p.cpu.regs[0] = KERR_NOT_SUPPORTED;
            }
            return Some(NativeOutcome::Return);
        }
        "_ZNK6RPhone9GetStatusERNS_7TStatusE" => {
            if offline {
                // RPhone::TStatus: iModemDetected (+0 = EDetectedNotPresent 2), iMode (+4 = EModeIdle 1)
                let _ = p.wr(r1, 2);
                let _ = p.wr(r1.wrapping_add(4), 1);
                p.cpu.regs[0] = 0;
            } else {
                p.cpu.regs[0] = KERR_NOT_SUPPORTED;
            }
            return Some(NativeOutcome::Return);
        }
        "_ZNK6RPhone11GetLineInfoEiRNS_9TLineInfoE" => {
            if offline {
                // RPhone::TLineInfo: iStatus (+0 = EStatusIdle 1), iLineCapsFlags (+4), TName iName (+8)
                let _ = p.wr(r2, 1);
                let _ = p.wr(r2.wrapping_add(4), 0);
                write_tname(p, r2.wrapping_add(8), "Voice1");
                p.cpu.regs[0] = 0;
            } else {
                p.cpu.regs[0] = KERR_NOT_SUPPORTED;
            }
            return Some(NativeOutcome::Return);
        }
        "_ZNK5RLine13EnumerateCallERi" => {
            // Offline mode: 0 active calls on the line.
            let _ = p.wr(r1, 0);
            p.cpu.regs[0] = if offline { 0 } else { KERR_NOT_SUPPORTED };
            return Some(NativeOutcome::Return);
        }
        "_ZNK5RLine11GetCallInfoEiRNS_9TCallInfoE" | "_ZNK5RLine9GetStatusERN5RCall7TStatusE" => {
            if name.contains("GetStatus") { let _ = p.wr(r1, 1); } // EStatusIdle
            p.cpu.regs[0] = if offline { 0 } else { KERR_NOT_SUPPORTED };
            return Some(NativeOutcome::Return);
        }
        "_ZNK5RLine18NotifyIncomingCallER14TRequestStatusR4TBufILi128EE"
        | "_ZNK5RLine18NotifyStatusChangeER14TRequestStatusRN5RCall7TStatusE" => {
            // Offline mode: stays KRequestPending (no incoming calls).
            let _ = p.wr(r1, crate::request::KREQUEST_PENDING);
            return Some(NativeOutcome::Return);
        }
        "_ZNK5RLine24NotifyIncomingCallCancelEv"
        | "_ZNK5RLine24NotifyStatusChangeCancelEv" => {
            return Some(NativeOutcome::Return);
        }
        "_ZN10RTelServer5CloseEv" | "_ZN6RPhone5CloseEv" | "_ZN5RLine5CloseEv" | "_ZN5RCall5CloseEv" => {
            if offline {
                let _ = p.wr(this, 0);
                return Some(NativeOutcome::Return);
            }
            return Some(match p.mem.vm.read_u32(this) {
                Ok(0) => NativeOutcome::Return,
                _ => NativeOutcome::Refused(alloc::format!("NEXT-NEEDED etel {name}: closing an open handle (no etel handle is ever opened)")),
            });
        }
        // CTelephony (etel3rdparty.h): device has no cellular modem; NewL returns an object whose async queries complete with KErrNotSupported.
        "_ZN10CTelephony4NewLEv" | "_ZN10CTelephony5NewLCEv" => {
            match p.plain_object("CTelephony", 16) {
                Ok(obj) => {
                    if name.contains("NewLC") {
                        p.nat.cleanup.push(crate::cleanup::CleanupItem::Base(obj));
                    }
                    p.cpu.regs[0] = obj;
                    return Some(NativeOutcome::Return);
                }
                Err(m) => return Some(NativeOutcome::Refused(alloc::format!("NEXT-NEEDED etel3rdparty {name}: {m}"))),
            }
        }
        "_ZN10CTelephony10TPhoneIdV1C1Ev" | "_ZN10CTelephony10TPhoneIdV1C2Ev"
        | "_ZN10CTelephony13TCallStatusV1C1Ev" | "_ZN10CTelephony13TCallStatusV1C2Ev" => {
            // TPhoneIdV1 / TCallStatusV1 inherit TTelephonyInfoV1 (iExtensionId=0).
            let _ = p.wr(this, 0);
            p.cpu.regs[0] = this;
            return Some(NativeOutcome::Return);
        }
        "_ZNK10CTelephony12NotifyChangeER14TRequestStatusRKNS_18TNotificationEventER5TDes8" => {
            // Leave status KRequestPending: no cellular telephony event fires on a desktop host.
            let status_addr = p.cpu.regs[1];
            let _ = p.wr(status_addr, crate::request::KREQUEST_PENDING);
            return Some(NativeOutcome::Return);
        }
        "_ZNK10CTelephony10GetPhoneIdER14TRequestStatusR5TDes8" => {
            let status_addr = p.cpu.regs[1];
            let _ = p.wr(status_addr, KERR_NOT_SUPPORTED);
            p.nat.requests.signals += 1;
            return Some(NativeOutcome::Return);
        }
        "_ZNK10CTelephony11CancelAsyncENS_19TCancellationRequestE" => {
            p.cpu.regs[0] = 0;
            return Some(NativeOutcome::Return);
        }
        "_ZN10CTelephonyD1Ev" | "_ZN10CTelephonyD2Ev" | "_ZN10CTelephonyD0Ev" => {
            if name.ends_with("D0Ev") {
                if let Some(heap) = p.heap.as_mut() { let _ = heap.free(this); }
            }
            p.cpu.regs[0] = this;
            return Some(NativeOutcome::Return);
        }
        _ => return None,
    };
    for w in 0..words {
        if p.mem.vm.write_u32(this + w * 4, 0).is_err() {
            return Some(NativeOutcome::Refused(alloc::format!("NEXT-NEEDED etel {name}: object at {this:#x} unwritable")));
        }
    }
    Some(NativeOutcome::Return)
}
