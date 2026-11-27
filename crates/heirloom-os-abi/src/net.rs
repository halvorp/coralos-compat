//! Native CoralOS `coral-net-server` host bridge ([`crate::OsVTable::attach_net`], appended to OS ABI v3).
//!
//! Lets an engine process (`coral-heirloom-engine-rt::net_service::NetService`, backed by CoralOS
//! `EP_NET_LISTEN` outbound-only TCP + DNS) attach its networking session to a dynamically loaded
//! `os-*.library` without crossing the DLL boundary with Rust trait objects or cross-allocator frees.
//! Inside `os-*.library`, [`NetHost`] implements `guestsvc_abi::GuestNet`.
use core::ffi::c_void;

/// `attach_net` accepted the host.
pub const NET_ATTACH_OK: i32 = 0;
/// `attach_net` refused (null args or family does not consume host networking).
pub const NET_ATTACH_REFUSED: i32 = 1;

/// Call status codes returned by [`NetHost`] callbacks.
pub const NET_OK: i32 = 0;
pub const NET_NOT_GRANTED: i32 = -1;
pub const NET_BAD_NAME: i32 = -2;
pub const NET_TOO_LARGE: i32 = -3;
pub const NET_LOST: i32 = -4;
pub const NET_STALLED: i32 = -5;
pub const NET_UNREADABLE: i32 = -6;
pub const NET_REFUSED_DNS: i32 = -10;
pub const NET_REFUSED_CONN_REFUSED: i32 = -11;
pub const NET_REFUSED_TIMEOUT: i32 = -12;
pub const NET_REFUSED_UNREACHABLE: i32 = -13;
pub const NET_REFUSED_NO_RESOURCES: i32 = -14;
pub const NET_REFUSED_BAD_CONN: i32 = -15;
pub const NET_REFUSED_EOF: i32 = -16;
pub const NET_REFUSED_RESET: i32 = -17;
pub const NET_REFUSED_OTHER: i32 = -18;

/// Host-provided outbound-only TCP + DNS callbacks ([`crate::OsVTable::attach_net`], copied;
/// `ctx` and fn pointers must outlive the `OsInstance`).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NetHost {
    /// `size_of::<NetHost>()` as built by the host.
    pub size: u32,
    pub flags: u32,
    pub ctx: *mut c_void,
    /// Resolve `host[..host_len]` to an IPv4 address written into `*out_ip`.
    pub resolve: extern "C" fn(
        ctx: *mut c_void,
        host: *const u8,
        host_len: usize,
        out_ip: *mut [u8; 4],
    ) -> i32,
    /// Dial outbound TCP `(ip, port)`; writes non-zero handle into `*out_conn`.
    pub connect: extern "C" fn(
        ctx: *mut c_void,
        ip: *const [u8; 4],
        port: u16,
        out_conn: *mut u32,
    ) -> i32,
    /// Non-blocking TX ring write; writes accepted byte count into `*out_sent`.
    pub send: extern "C" fn(
        ctx: *mut c_void,
        conn: u32,
        data: *const u8,
        len: usize,
        out_sent: *mut usize,
    ) -> i32,
    /// Non-blocking RX ring read; writes byte count into `*out_recv` (0 = would-block).
    pub recv: extern "C" fn(
        ctx: *mut c_void,
        conn: u32,
        buf: *mut u8,
        cap: usize,
        out_recv: *mut usize,
    ) -> i32,
    /// Non-blocking readiness mask (`NET_EVT_*`) written into `*out_events`.
    pub poll_events: extern "C" fn(ctx: *mut c_void, conn: u32, out_events: *mut u32) -> i32,
    /// Half-close write side (`SHUT_WR`).
    pub shutdown_write: extern "C" fn(ctx: *mut c_void, conn: u32) -> i32,
    /// Close `conn` and free its slot.
    pub close: extern "C" fn(ctx: *mut c_void, conn: u32) -> i32,
}

// SAFETY: NetHost holds plain function pointers and an engine-owned ctx pointer accessed on the single engine thread.
unsafe impl Send for NetHost {}

#[cfg(any(feature = "serve", test))]
mod bridge {
    use super::*;
    use guestsvc_abi::{GuestNet, GuestSvcError};

    pub fn err_to_code(e: GuestSvcError) -> i32 {
        match e {
            GuestSvcError::NotGranted => NET_NOT_GRANTED,
            GuestSvcError::BadName => NET_BAD_NAME,
            GuestSvcError::TooLarge => NET_TOO_LARGE,
            GuestSvcError::Lost => NET_LOST,
            GuestSvcError::Stalled => NET_STALLED,
            GuestSvcError::Unreadable => NET_UNREADABLE,
            GuestSvcError::Refused("dns") => NET_REFUSED_DNS,
            GuestSvcError::Refused("conn-refused") => NET_REFUSED_CONN_REFUSED,
            GuestSvcError::Refused("timeout") => NET_REFUSED_TIMEOUT,
            GuestSvcError::Refused("unreachable") => NET_REFUSED_UNREACHABLE,
            GuestSvcError::Refused("no-resources") => NET_REFUSED_NO_RESOURCES,
            GuestSvcError::Refused("bad-conn") => NET_REFUSED_BAD_CONN,
            GuestSvcError::Refused("eof") => NET_REFUSED_EOF,
            GuestSvcError::Refused("reset") => NET_REFUSED_RESET,
            GuestSvcError::Refused(_) => NET_REFUSED_OTHER,
        }
    }

    pub fn code_to_err(rc: i32) -> GuestSvcError {
        match rc {
            NET_NOT_GRANTED => GuestSvcError::NotGranted,
            NET_BAD_NAME => GuestSvcError::BadName,
            NET_TOO_LARGE => GuestSvcError::TooLarge,
            NET_LOST => GuestSvcError::Lost,
            NET_STALLED => GuestSvcError::Stalled,
            NET_UNREADABLE => GuestSvcError::Unreadable,
            NET_REFUSED_DNS => GuestSvcError::Refused("dns"),
            NET_REFUSED_CONN_REFUSED => GuestSvcError::Refused("conn-refused"),
            NET_REFUSED_TIMEOUT => GuestSvcError::Refused("timeout"),
            NET_REFUSED_UNREACHABLE => GuestSvcError::Refused("unreachable"),
            NET_REFUSED_NO_RESOURCES => GuestSvcError::Refused("no-resources"),
            NET_REFUSED_BAD_CONN => GuestSvcError::Refused("bad-conn"),
            NET_REFUSED_EOF => GuestSvcError::Refused("eof"),
            NET_REFUSED_RESET => GuestSvcError::Refused("reset"),
            _ => GuestSvcError::Refused("refused"),
        }
    }

    impl GuestNet for NetHost {
        fn resolve(&mut self, host: &str) -> Result<[u8; 4], GuestSvcError> {
            let mut ip = [0u8; 4];
            let rc = (self.resolve)(self.ctx, host.as_ptr(), host.len(), &mut ip);
            if rc == NET_OK { Ok(ip) } else { Err(code_to_err(rc)) }
        }

        fn connect(&mut self, ip: [u8; 4], port: u16) -> Result<u32, GuestSvcError> {
            let mut conn = 0u32;
            let rc = (self.connect)(self.ctx, &ip, port, &mut conn);
            if rc == NET_OK { Ok(conn) } else { Err(code_to_err(rc)) }
        }

        fn send(&mut self, conn: u32, data: &[u8]) -> Result<usize, GuestSvcError> {
            let mut sent = 0usize;
            let rc = (self.send)(self.ctx, conn, data.as_ptr(), data.len(), &mut sent);
            if rc == NET_OK { Ok(sent) } else { Err(code_to_err(rc)) }
        }

        fn recv(&mut self, conn: u32, buf: &mut [u8]) -> Result<usize, GuestSvcError> {
            let mut got = 0usize;
            let rc = (self.recv)(self.ctx, conn, buf.as_mut_ptr(), buf.len(), &mut got);
            if rc == NET_OK { Ok(got) } else { Err(code_to_err(rc)) }
        }

        fn poll_events(&mut self, conn: u32) -> Result<u32, GuestSvcError> {
            let mut ev = 0u32;
            let rc = (self.poll_events)(self.ctx, conn, &mut ev);
            if rc == NET_OK { Ok(ev) } else { Err(code_to_err(rc)) }
        }

        fn shutdown_write(&mut self, conn: u32) -> Result<(), GuestSvcError> {
            let rc = (self.shutdown_write)(self.ctx, conn);
            if rc == NET_OK { Ok(()) } else { Err(code_to_err(rc)) }
        }

        fn close(&mut self, conn: u32) -> Result<(), GuestSvcError> {
            let rc = (self.close)(self.ctx, conn);
            if rc == NET_OK { Ok(()) } else { Err(code_to_err(rc)) }
        }
    }

    /// Build a [`NetHost`] whose callbacks dispatch to `*mut T` (any `GuestNet` implementor).
    ///
    /// # Safety
    /// `ctx` must remain valid, unmoved, and exclusively borrowed by the engine thread for the entire
    /// lifetime of the `OsInstance` that receives this `NetHost`.
    pub unsafe fn host_for<T: GuestNet>(ctx: *mut T) -> NetHost {
        extern "C" fn tramp_resolve<T: GuestNet>(
            ctx: *mut c_void,
            host: *const u8,
            host_len: usize,
            out_ip: *mut [u8; 4],
        ) -> i32 {
            if ctx.is_null() || out_ip.is_null() || (host.is_null() && host_len != 0) {
                return NET_UNREADABLE;
            }
            let bytes = if host_len == 0 { &[][..] } else { unsafe { core::slice::from_raw_parts(host, host_len) } };
            let Ok(s) = core::str::from_utf8(bytes) else { return NET_BAD_NAME };
            let net = unsafe { &mut *(ctx as *mut T) };
            match net.resolve(s) {
                Ok(ip) => {
                    unsafe { *out_ip = ip };
                    NET_OK
                }
                Err(e) => err_to_code(e),
            }
        }

        extern "C" fn tramp_connect<T: GuestNet>(
            ctx: *mut c_void,
            ip: *const [u8; 4],
            port: u16,
            out_conn: *mut u32,
        ) -> i32 {
            if ctx.is_null() || ip.is_null() || out_conn.is_null() {
                return NET_UNREADABLE;
            }
            let net = unsafe { &mut *(ctx as *mut T) };
            match net.connect(unsafe { *ip }, port) {
                Ok(id) => {
                    unsafe { *out_conn = id };
                    NET_OK
                }
                Err(e) => err_to_code(e),
            }
        }

        extern "C" fn tramp_send<T: GuestNet>(
            ctx: *mut c_void,
            conn: u32,
            data: *const u8,
            len: usize,
            out_sent: *mut usize,
        ) -> i32 {
            if ctx.is_null() || out_sent.is_null() || (data.is_null() && len != 0) {
                return NET_UNREADABLE;
            }
            let bytes = if len == 0 { &[][..] } else { unsafe { core::slice::from_raw_parts(data, len) } };
            let net = unsafe { &mut *(ctx as *mut T) };
            match net.send(conn, bytes) {
                Ok(n) => {
                    unsafe { *out_sent = n };
                    NET_OK
                }
                Err(e) => err_to_code(e),
            }
        }

        extern "C" fn tramp_recv<T: GuestNet>(
            ctx: *mut c_void,
            conn: u32,
            buf: *mut u8,
            cap: usize,
            out_recv: *mut usize,
        ) -> i32 {
            if ctx.is_null() || out_recv.is_null() || (buf.is_null() && cap != 0) {
                return NET_UNREADABLE;
            }
            let out = if cap == 0 { &mut [][..] } else { unsafe { core::slice::from_raw_parts_mut(buf, cap) } };
            let net = unsafe { &mut *(ctx as *mut T) };
            match net.recv(conn, out) {
                Ok(n) => {
                    unsafe { *out_recv = n };
                    NET_OK
                }
                Err(e) => err_to_code(e),
            }
        }

        extern "C" fn tramp_poll<T: GuestNet>(ctx: *mut c_void, conn: u32, out_ev: *mut u32) -> i32 {
            if ctx.is_null() || out_ev.is_null() { return NET_UNREADABLE; }
            let net = unsafe { &mut *(ctx as *mut T) };
            match net.poll_events(conn) {
                Ok(ev) => {
                    unsafe { *out_ev = ev };
                    NET_OK
                }
                Err(e) => err_to_code(e),
            }
        }

        extern "C" fn tramp_shutdown<T: GuestNet>(ctx: *mut c_void, conn: u32) -> i32 {
            if ctx.is_null() { return NET_UNREADABLE; }
            let net = unsafe { &mut *(ctx as *mut T) };
            match net.shutdown_write(conn) { Ok(()) => NET_OK, Err(e) => err_to_code(e) }
        }

        extern "C" fn tramp_close<T: GuestNet>(ctx: *mut c_void, conn: u32) -> i32 {
            if ctx.is_null() { return NET_UNREADABLE; }
            let net = unsafe { &mut *(ctx as *mut T) };
            match net.close(conn) { Ok(()) => NET_OK, Err(e) => err_to_code(e) }
        }

        NetHost {
            size: core::mem::size_of::<NetHost>() as u32,
            flags: 0,
            ctx: ctx.cast(),
            resolve: tramp_resolve::<T>,
            connect: tramp_connect::<T>,
            send: tramp_send::<T>,
            recv: tramp_recv::<T>,
            poll_events: tramp_poll::<T>,
            shutdown_write: tramp_shutdown::<T>,
            close: tramp_close::<T>,
        }
    }
}

#[cfg(any(feature = "serve", test))]
pub use bridge::{code_to_err, err_to_code, host_for};

#[cfg(test)]
mod tests {
    use super::*;
    use guestsvc_abi::{FakeNet, GuestNet, GuestSvcError, NET_EVT_READABLE};

    #[test]
    fn net_host_round_trips_tcp_dns_and_not_granted() {
        let mut fake = FakeNet::new();
        fake.add_host("example.coral", [10, 0, 2, 9]);
        // SAFETY: `fake` outlives `host` in this test.
        let mut host = unsafe { host_for(&mut fake) };
        assert_eq!(host.resolve("example.coral"), Ok([10, 0, 2, 9]));
        let c = host.connect([10, 0, 2, 9], 22).unwrap();
        assert_eq!(host.send(c, b"hello"), Ok(5));
        let mut buf = [0u8; 8];
        assert_eq!(host.recv(c, &mut buf), Ok(0));
        assert_eq!(host.poll_events(c).unwrap() & NET_EVT_READABLE, 0);
        host.shutdown_write(c).unwrap();
        host.close(c).unwrap();

        let mut denied = FakeNet::not_granted();
        let mut dhost = unsafe { host_for(&mut denied) };
        assert_eq!(dhost.resolve("example.coral"), Err(GuestSvcError::NotGranted));
        assert_eq!(dhost.connect([10, 0, 2, 9], 22), Err(GuestSvcError::NotGranted));
    }
}
