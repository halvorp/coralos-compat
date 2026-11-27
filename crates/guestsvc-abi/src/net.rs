//! Host-trait ABI for guest outbound TCP stream networking + DNS over CoralOS `coral-net-server`
//! (`EP_NET_LISTEN`, `(net)` grant, REUSE-AUDIT U18).
//!
//! Security & least-privilege policy:
//! * **Outbound-only:** `GuestNet` exposes DNS lookup + outbound TCP dial/send/recv/shutdown/close.
//!   Inbound `bind`/`listen`/`accept` do NOT exist on this seam (CoralOS `EP_NET_LISTEN` refuses
//!   inbound listener ops by endpoint; `EP_NET_SERVER_LISTEN` is never granted to guest engines).
//! * **Bounded slots:** At most [`NET_MAX_CONNS`] (4) concurrent outbound connections per engine,
//!   matching `coral_net_abi::srv::NET_CLIENT_SLOTS`.
//! * **Non-blocking data path:** `send` and `recv` read/write the shared `SockRing` without blocking;
//!   `poll_events` reports level-triggered readiness (`NET_EVT_READABLE | NET_EVT_WRITABLE | NET_EVT_EOF | NET_EVT_RESET`).
use alloc::collections::BTreeMap;
use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;

use crate::GuestSvcError;

/// Maximum concurrent outbound connections per engine (matches `coral_net_abi::srv::NET_CLIENT_SLOTS`).
pub const NET_MAX_CONNS: usize = 4;

/// Maximum hostname length for DNS lookup (matches `coral_net_abi` op22 `hlen: 1..=127`).
pub const NET_HOST_MAX: usize = 127;

/// Maximum bytes transferred in a single `send`/`recv` call across the C ABI (64 KiB).
pub const NET_IO_MAX: usize = 64 * 1024;

/// Readiness bitmask flags returned by [`GuestNet::poll_events`].
pub const NET_EVT_READABLE: u32 = 1 << 0;
pub const NET_EVT_WRITABLE: u32 = 1 << 1;
pub const NET_EVT_EOF: u32 = 1 << 2;
pub const NET_EVT_RESET: u32 = 1 << 3;

/// Whether `host` is a valid DNS hostname or numeric IPv4 literal (`1..=127` ASCII graphic bytes, no whitespace or `:`).
pub fn host_ok(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= NET_HOST_MAX
        && host.bytes().all(|b| b.is_ascii_graphic() && b != b':' && b != b'/')
}

/// Outbound-only TCP + DNS session owned by the engine process (`coral-net-server` on `EP_NET_LISTEN`).
pub trait GuestNet {
    /// Resolve `host` (DNS name or dotted-quad IPv4) to an IPv4 address.
    fn resolve(&mut self, host: &str) -> Result<[u8; 4], GuestSvcError>;
    /// Dial outbound TCP `(ip, port)`; returns a non-zero connection handle (`1..=NET_MAX_CONNS`).
    fn connect(&mut self, ip: [u8; 4], port: u16) -> Result<u32, GuestSvcError>;
    /// Queue up to `data.len()` bytes onto `conn`'s TX ring; returns bytes accepted (`0` = TX full / would-block).
    fn send(&mut self, conn: u32, data: &[u8]) -> Result<usize, GuestSvcError>;
    /// Read up to `buf.len()` bytes from `conn`'s RX ring; returns `Ok(0)` when no bytes are ready yet,
    /// `Err(GuestSvcError::Refused("eof"))` once peer FIN is drained, or `Err(GuestSvcError::Refused("reset"))`.
    fn recv(&mut self, conn: u32, buf: &mut [u8]) -> Result<usize, GuestSvcError>;
    /// Query non-blocking readiness flags (`NET_EVT_*`) for `conn`.
    fn poll_events(&mut self, conn: u32) -> Result<u32, GuestSvcError>;
    /// Half-close the write side of `conn` (TCP FIN).
    fn shutdown_write(&mut self, conn: u32) -> Result<(), GuestSvcError>;
    /// Close `conn` and release its slot.
    fn close(&mut self, conn: u32) -> Result<(), GuestSvcError>;
}

/// One simulated TCP connection inside [`FakeNet`].
#[derive(Clone, Debug)]
pub struct FakeConn {
    pub ip: [u8; 4],
    pub port: u16,
    pub rx: VecDeque<u8>,
    pub tx: Vec<u8>,
    pub eof: bool,
    pub reset: bool,
    pub write_closed: bool,
}

/// Deterministic in-memory [`GuestNet`] implementor for host oracle tests.
#[derive(Clone, Debug, Default)]
pub struct FakeNet {
    dns: BTreeMap<String, [u8; 4]>,
    conns: BTreeMap<u32, FakeConn>,
    next_id: u32,
    not_granted: bool,
    injected_error: Option<GuestSvcError>,
}

impl FakeNet {
    pub fn new() -> Self {
        Self::default()
    }

    /// A fake net session whose engine has no `(net)` grant: every call returns `Err(GuestSvcError::NotGranted)`.
    pub fn not_granted() -> Self {
        Self { not_granted: true, ..Self::default() }
    }

    pub fn set_granted(&mut self, granted: bool) {
        self.not_granted = !granted;
    }

    pub fn set_injected_error(&mut self, err: Option<GuestSvcError>) {
        self.injected_error = err;
    }

    pub fn add_host(&mut self, host: &str, ip: [u8; 4]) {
        self.dns.insert(String::from(host), ip);
    }

    pub fn inject_rx(&mut self, conn: u32, bytes: &[u8]) {
        if let Some(c) = self.conns.get_mut(&conn) {
            c.rx.extend(bytes);
        }
    }

    pub fn inject_eof(&mut self, conn: u32) {
        if let Some(c) = self.conns.get_mut(&conn) {
            c.eof = true;
        }
    }

    pub fn inject_reset(&mut self, conn: u32) {
        if let Some(c) = self.conns.get_mut(&conn) {
            c.reset = true;
        }
    }

    pub fn conns(&self) -> &BTreeMap<u32, FakeConn> {
        &self.conns
    }

    fn check_access(&self) -> Result<(), GuestSvcError> {
        if self.not_granted {
            return Err(GuestSvcError::NotGranted);
        }
        if let Some(err) = self.injected_error {
            return Err(err);
        }
        Ok(())
    }
}

fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut parts = s.split('.');
    for slot in &mut out {
        *slot = parts.next()?.parse::<u8>().ok()?;
    }
    parts.next().is_none().then_some(out)
}

impl GuestNet for FakeNet {
    fn resolve(&mut self, host: &str) -> Result<[u8; 4], GuestSvcError> {
        self.check_access()?;
        if !host_ok(host) {
            return Err(GuestSvcError::BadName);
        }
        if let Some(ip) = parse_ipv4(host) {
            return Ok(ip);
        }
        self.dns.get(host).copied().ok_or(GuestSvcError::Refused("dns"))
    }

    fn connect(&mut self, ip: [u8; 4], port: u16) -> Result<u32, GuestSvcError> {
        self.check_access()?;
        if port == 0 {
            return Err(GuestSvcError::Refused("conn-refused"));
        }
        if self.conns.len() >= NET_MAX_CONNS {
            return Err(GuestSvcError::Refused("no-resources"));
        }
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let id = self.next_id;
        self.conns.insert(
            id,
            FakeConn {
                ip,
                port,
                rx: VecDeque::new(),
                tx: Vec::new(),
                eof: false,
                reset: false,
                write_closed: false,
            },
        );
        Ok(id)
    }

    fn send(&mut self, conn: u32, data: &[u8]) -> Result<usize, GuestSvcError> {
        self.check_access()?;
        let c = self.conns.get_mut(&conn).ok_or(GuestSvcError::Refused("bad-conn"))?;
        if c.reset {
            return Err(GuestSvcError::Refused("reset"));
        }
        if c.write_closed {
            return Err(GuestSvcError::Refused("eof"));
        }
        let n = data.len().min(NET_IO_MAX);
        c.tx.extend_from_slice(&data[..n]);
        Ok(n)
    }

    fn recv(&mut self, conn: u32, buf: &mut [u8]) -> Result<usize, GuestSvcError> {
        self.check_access()?;
        let c = self.conns.get_mut(&conn).ok_or(GuestSvcError::Refused("bad-conn"))?;
        if c.reset {
            return Err(GuestSvcError::Refused("reset"));
        }
        let n = buf.len().min(NET_IO_MAX).min(c.rx.len());
        if n > 0 {
            for slot in &mut buf[..n] {
                *slot = c.rx.pop_front().unwrap_or(0);
            }
            return Ok(n);
        }
        if c.eof {
            return Err(GuestSvcError::Refused("eof"));
        }
        Ok(0)
    }

    fn poll_events(&mut self, conn: u32) -> Result<u32, GuestSvcError> {
        self.check_access()?;
        let c = self.conns.get(&conn).ok_or(GuestSvcError::Refused("bad-conn"))?;
        let mut mask = 0u32;
        if !c.rx.is_empty() {
            mask |= NET_EVT_READABLE;
        }
        if !c.write_closed && !c.reset {
            mask |= NET_EVT_WRITABLE;
        }
        if c.eof {
            mask |= NET_EVT_EOF;
        }
        if c.reset {
            mask |= NET_EVT_RESET;
        }
        Ok(mask)
    }

    fn shutdown_write(&mut self, conn: u32) -> Result<(), GuestSvcError> {
        self.check_access()?;
        let c = self.conns.get_mut(&conn).ok_or(GuestSvcError::Refused("bad-conn"))?;
        c.write_closed = true;
        Ok(())
    }

    fn close(&mut self, conn: u32) -> Result<(), GuestSvcError> {
        self.check_access()?;
        self.conns.remove(&conn).ok_or(GuestSvcError::Refused("bad-conn"))?;
        Ok(())
    }
}
