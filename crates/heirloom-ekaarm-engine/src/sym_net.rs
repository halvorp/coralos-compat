//! Stateless Symbian `esock` (`RSocketServ`, `RSocket`, `RHostResolver`) adapter over
//! `guestsvc_abi::GuestNet` (CoralOS `coral-net-server` `EP_NET_LISTEN`, REUSE-AUDIT U18).
//!
//! Security & least-privilege policy:
//! * **Outbound-only TCP + DNS:** `RSocket::Open` accepts `KAfInet` + `KSockStream` + `KProtocolInetTcp`
//!   and dials through `GuestNet::connect`. Inbound `Bind`, `Listen`, `Accept`, and non-TCP / radio
//!   socket families (UDP, raw, Bluetooth, IrDA) are **refused by name** (`KErrNotSupported`, `-5`).
//! * **No grant -> `KErrPermissionDenied` (`-46`):** when `(net)` was not granted to the engine or
//!   no `GuestNet` host is attached, every network call refuses cleanly with zero silent fallback.
use alloc::format;
use alloc::string::String;
use alloc::vec;

use guestsvc_abi::{GuestNet, GuestSvcError, NET_IO_MAX};

use crate::loader::Process;
use crate::natives::NativeOutcome;

pub const KERR_NONE: i32 = 0;
pub const KERR_NO_MEMORY: i32 = -4;
pub const KERR_NOT_SUPPORTED: i32 = -5;
pub const KERR_ARGUMENT: i32 = -6;
pub const KERR_BAD_HANDLE: i32 = -8;
pub const KERR_SERVER_TERMINATED: i32 = -15;
pub const KERR_EOF: i32 = -25;
pub const KERR_TIMED_OUT: i32 = -33;
pub const KERR_COULD_NOT_CONNECT: i32 = -34;
pub const KERR_DISCONNECTED: i32 = -36;
pub const KERR_PERMISSION_DENIED: i32 = -46;

pub const KAF_INET: u32 = 0x0800;
pub const KSOCK_STREAM: u32 = 1;
pub const KPROTOCOL_INET_TCP: u32 = 6;

/// Minimal `TInetAddr` byte layout read/written by `RHostResolver::GetByName` & `RSocket::Connect`:
/// offset 0: `family: u32 LE` (`KAF_INET`), offset 4: `port: u32 LE`, offset 8: `ipv4: u32 BE`.
pub const INET_ADDR_FAMILY_OFF: u32 = 0;
pub const INET_ADDR_PORT_OFF: u32 = 4;
pub const INET_ADDR_IPV4_OFF: u32 = 8;

/// Offset of `iAddr` inside a Symbian `TNameRecord` (`iName` TBuf<256> = 4 + 512 = 516 bytes).
pub const NAME_RECORD_ADDR_OFF: u32 = 516;

fn map_net_err(e: GuestSvcError) -> i32 {
    match e {
        GuestSvcError::NotGranted => KERR_PERMISSION_DENIED,
        GuestSvcError::BadName => KERR_ARGUMENT,
        GuestSvcError::TooLarge => KERR_ARGUMENT,
        GuestSvcError::Lost => KERR_SERVER_TERMINATED,
        GuestSvcError::Stalled => KERR_TIMED_OUT,
        GuestSvcError::Unreadable => KERR_ARGUMENT,
        GuestSvcError::Refused("dns") => KERR_COULD_NOT_CONNECT,
        GuestSvcError::Refused("conn-refused") => KERR_COULD_NOT_CONNECT,
        GuestSvcError::Refused("timeout") => KERR_TIMED_OUT,
        GuestSvcError::Refused("unreachable") => KERR_COULD_NOT_CONNECT,
        GuestSvcError::Refused("no-resources") => KERR_NO_MEMORY,
        GuestSvcError::Refused("bad-conn") => KERR_BAD_HANDLE,
        GuestSvcError::Refused("eof") => KERR_EOF,
        GuestSvcError::Refused("reset") => KERR_DISCONNECTED,
        GuestSvcError::Refused(_) => KERR_COULD_NOT_CONNECT,
    }
}

fn complete_status(p: &mut Process, status_addr: u32, code: i32) {
    if status_addr != 0 {
        let _ = p.request_issue(status_addr);
        let _ = p.request_complete(status_addr, code);
    }
}

fn host_mut(p: &mut Process) -> Result<&mut (dyn GuestNet + 'static), i32> {
    match p.net_host.as_mut() {
        Some(b) => Ok(&mut **b),
        None => Err(KERR_PERMISSION_DENIED),
    }
}

pub fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    if !name.starts_with("_ZN11RSocketServ")
        && !name.starts_with("_ZN7RSocket")
        && !name.starts_with("_ZN13RHostResolver")
    {
        return None;
    }

    let r0 = p.cpu.regs[0];
    let r1 = p.cpu.regs[1];
    let r2 = p.cpu.regs[2];
    let r3 = p.cpu.regs[3];

    // ---- RSocketServ ----
    if name.starts_with("_ZN11RSocketServ7Connect") {
        let code = if p.net_host.is_some() {
            let h = p.nat.objs.next_net_handle.wrapping_add(1).max(1);
            p.nat.objs.next_net_handle = h;
            let _ = p.wr(r0, h);
            p.nat.objs.net_sessions.insert(h, ());
            p.log.push(format!("[net] RSocketServ::Connect -> handle={h}"));
            KERR_NONE
        } else {
            p.log.push(String::from("[net] RSocketServ::Connect REFUSED (no net_host) -> KErrPermissionDenied"));
            KERR_PERMISSION_DENIED
        };
        p.cpu.regs[0] = code as u32;
        return Some(NativeOutcome::Return);
    }
    if name == "_ZN11RSocketServ5CloseEv" {
        if let Ok(h) = p.rd(r0) {
            p.nat.objs.net_sessions.remove(&h);
            let _ = p.wr(r0, 0);
        }
        return Some(NativeOutcome::Return);
    }

    // ---- RHostResolver ----
    if name.starts_with("_ZN13RHostResolver4Open") {
        let family = r2;
        if family != KAF_INET && family != 0 {
            p.log.push(format!("[net] RHostResolver::Open REFUSED family={family:#x} -> KErrNotSupported"));
            p.cpu.regs[0] = KERR_NOT_SUPPORTED as u32;
            return Some(NativeOutcome::Return);
        }
        if p.net_host.is_none() {
            p.cpu.regs[0] = KERR_PERMISSION_DENIED as u32;
            return Some(NativeOutcome::Return);
        }
        let h = p.nat.objs.next_net_handle.wrapping_add(1).max(1);
        p.nat.objs.next_net_handle = h;
        let _ = p.wr(r0, h);
        p.nat.objs.net_resolvers.insert(h, ());
        p.cpu.regs[0] = KERR_NONE as u32;
        return Some(NativeOutcome::Return);
    }
    if name.starts_with("_ZN13RHostResolver9GetByName") {
        let host = match crate::desc::read16(p, r1) {
            Ok(u) => crate::desc::to_utf8(&u),
            Err(_) => {
                complete_status(p, r3, KERR_ARGUMENT);
                return Some(NativeOutcome::Return);
            }
        };
        let res = match host_mut(p) {
            Ok(h) => h.resolve(&host).map_err(map_net_err),
            Err(e) => Err(e),
        };
        match res {
            Ok(ip) => {
                // `r2` points to `TNameEntry` (`TPckg<TNameRecord>`): write TInetAddr at +NAME_RECORD_ADDR_OFF
                // or directly if buffer is a raw TInetAddr; for TPckg<TNameRecord>, payload starts after 8-byte TPtr8 header.
                let base = p.rd(r2 + 8).unwrap_or(r2);
                let addr_off = base.wrapping_add(NAME_RECORD_ADDR_OFF);
                let _ = p.wr(addr_off + INET_ADDR_FAMILY_OFF, KAF_INET);
                let _ = p.wr(addr_off + INET_ADDR_PORT_OFF, 0);
                let _ = p.wr(addr_off + INET_ADDR_IPV4_OFF, u32::from_be_bytes(ip));
                p.log.push(format!("[net] RHostResolver::GetByName {host:?} -> {}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]));
                complete_status(p, r3, KERR_NONE);
            }
            Err(code) => {
                p.log.push(format!("[net] RHostResolver::GetByName {host:?} -> {code}"));
                complete_status(p, r3, code);
            }
        }
        return Some(NativeOutcome::Return);
    }
    if name == "_ZN13RHostResolver5CloseEv" {
        if let Ok(h) = p.rd(r0) {
            p.nat.objs.net_resolvers.remove(&h);
            let _ = p.wr(r0, 0);
        }
        return Some(NativeOutcome::Return);
    }

    // ---- RSocket: explicit outbound-only refusals ----
    if name.starts_with("_ZN7RSocket4Bind")
        || name.starts_with("_ZN7RSocket6Listen")
        || name.starts_with("_ZN7RSocket6Accept")
        || name.starts_with("_ZN7RSocket12SetLocalPort")
    {
        p.log.push(format!("[net] REFUSED inbound socket op {name} (outbound-only policy) -> KErrNotSupported"));
        if name.starts_with("_ZN7RSocket6Accept") {
            complete_status(p, r2, KERR_NOT_SUPPORTED);
        }
        p.cpu.regs[0] = KERR_NOT_SUPPORTED as u32;
        return Some(NativeOutcome::Return);
    }

    // ---- RSocket: outbound TCP lifecycle ----
    if name.starts_with("_ZN7RSocket4Open") {
        let family = r2;
        let sock_type = r3;
        let proto = p.rd(p.cpu.regs[13]).unwrap_or(KPROTOCOL_INET_TCP);
        if family != KAF_INET || sock_type != KSOCK_STREAM || (proto != KPROTOCOL_INET_TCP && proto != 0) {
            p.log.push(format!("[net] RSocket::Open REFUSED family={family:#x} type={sock_type} proto={proto} -> KErrNotSupported"));
            p.cpu.regs[0] = KERR_NOT_SUPPORTED as u32;
            return Some(NativeOutcome::Return);
        }
        if p.net_host.is_none() {
            p.cpu.regs[0] = KERR_PERMISSION_DENIED as u32;
            return Some(NativeOutcome::Return);
        }
        let h = p.nat.objs.next_net_handle.wrapping_add(1).max(1);
        p.nat.objs.next_net_handle = h;
        let _ = p.wr(r0, h);
        p.nat.objs.net_sockets.insert(h, None);
        p.cpu.regs[0] = KERR_NONE as u32;
        return Some(NativeOutcome::Return);
    }

    if name.starts_with("_ZN7RSocket7Connect") {
        let Ok(h) = p.rd(r0) else {
            complete_status(p, r2, KERR_BAD_HANDLE);
            return Some(NativeOutcome::Return);
        };
        if !p.nat.objs.net_sockets.contains_key(&h) {
            complete_status(p, r2, KERR_BAD_HANDLE);
            return Some(NativeOutcome::Return);
        }
        let port = p.rd(r1 + INET_ADDR_PORT_OFF).unwrap_or(0) as u16;
        let ipv4_be = p.rd(r1 + INET_ADDR_IPV4_OFF).unwrap_or(0);
        let ip = ipv4_be.to_be_bytes();
        let res = match host_mut(p) {
            Ok(host) => host.connect(ip, port).map_err(map_net_err),
            Err(e) => Err(e),
        };
        match res {
            Ok(conn_id) => {
                p.nat.objs.net_sockets.insert(h, Some(conn_id));
                p.log.push(format!("[net] RSocket::Connect handle={h} -> {}.{}.{}.{}:{port} conn={conn_id}", ip[0], ip[1], ip[2], ip[3]));
                complete_status(p, r2, KERR_NONE);
            }
            Err(code) => {
                p.log.push(format!("[net] RSocket::Connect handle={h} {}.{}.{}.{}:{port} -> {code}", ip[0], ip[1], ip[2], ip[3]));
                complete_status(p, r2, code);
            }
        }
        return Some(NativeOutcome::Return);
    }

    if name.starts_with("_ZN7RSocket5Write") || name.starts_with("_ZN7RSocket4Send") {
        let status_addr = if name.starts_with("_ZN7RSocket5Write") { r2 } else { r3 };
        let Ok(h) = p.rd(r0) else {
            complete_status(p, status_addr, KERR_BAD_HANDLE);
            return Some(NativeOutcome::Return);
        };
        let Some(Some(conn)) = p.nat.objs.net_sockets.get(&h).copied() else {
            complete_status(p, status_addr, KERR_DISCONNECTED);
            return Some(NativeOutcome::Return);
        };
        let Ok(bytes) = crate::desc::read8(p, r1) else {
            complete_status(p, status_addr, KERR_ARGUMENT);
            return Some(NativeOutcome::Return);
        };
        let res = match host_mut(p) {
            Ok(host) => host.send(conn, &bytes).map_err(map_net_err),
            Err(e) => Err(e),
        };
        match res {
            Ok(_) => complete_status(p, status_addr, KERR_NONE),
            Err(code) => complete_status(p, status_addr, code),
        }
        return Some(NativeOutcome::Return);
    }

    if name.starts_with("_ZN7RSocket4Read")
        || name.starts_with("_ZN7RSocket4Recv")
        || name.starts_with("_ZN7RSocket13RecvOneOrMore")
    {
        let status_addr = if name.starts_with("_ZN7RSocket4Read") { r2 } else { r3 };
        let Ok(h) = p.rd(r0) else {
            complete_status(p, status_addr, KERR_BAD_HANDLE);
            return Some(NativeOutcome::Return);
        };
        let Some(Some(conn)) = p.nat.objs.net_sockets.get(&h).copied() else {
            complete_status(p, status_addr, KERR_DISCONNECTED);
            return Some(NativeOutcome::Return);
        };
        let mut buf = vec![0u8; 4096.min(NET_IO_MAX)];
        let res = match host_mut(p) {
            Ok(host) => host.recv(conn, &mut buf).map_err(map_net_err),
            Err(e) => Err(e),
        };
        match res {
            Ok(n) => {
                let _ = crate::desc::write8(p, r1, &buf[..n]);
                complete_status(p, status_addr, KERR_NONE);
            }
            Err(code) => complete_status(p, status_addr, code),
        }
        return Some(NativeOutcome::Return);
    }

    if name.starts_with("_ZN7RSocket8Shutdown") {
        let Ok(h) = p.rd(r0) else {
            complete_status(p, r2, KERR_BAD_HANDLE);
            return Some(NativeOutcome::Return);
        };
        let Some(Some(conn)) = p.nat.objs.net_sockets.get(&h).copied() else {
            complete_status(p, r2, KERR_NONE);
            return Some(NativeOutcome::Return);
        };
        let res = match host_mut(p) {
            Ok(host) => host.shutdown_write(conn).map_err(map_net_err),
            Err(e) => Err(e),
        };
        complete_status(p, r2, res.err().unwrap_or(KERR_NONE));
        return Some(NativeOutcome::Return);
    }

    if name == "_ZN7RSocket5CloseEv" {
        if let Ok(h) = p.rd(r0) {
            if let Some(Some(conn)) = p.nat.objs.net_sockets.remove(&h) {
                if let Some(host) = p.net_host.as_deref_mut() {
                    let _ = host.close(conn);
                }
            }
            let _ = p.wr(r0, 0);
        }
        return Some(NativeOutcome::Return);
    }

    None
}
