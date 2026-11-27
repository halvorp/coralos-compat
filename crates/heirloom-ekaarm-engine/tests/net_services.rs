//! Host oracle tests for REUSE U18 (Symbian `esock` stateless outbound TCP + DNS adapter over
//! `guestsvc_abi::GuestNet`, backed by CoralOS `coral-net-server` `EP_NET_LISTEN`).
use std::cell::RefCell;
use std::rc::Rc;

use guestsvc_abi::{FakeNet, GuestNet, GuestSvcError};
use heirloom_ekaarm_engine::cpu::Memory;
use heirloom_ekaarm_engine::loader::{self, Process};
use heirloom_ekaarm_engine::natives::{self, NativeOutcome};
use heirloom_ekaarm_engine::sym_net::{
    INET_ADDR_FAMILY_OFF, INET_ADDR_IPV4_OFF, INET_ADDR_PORT_OFF, KAF_INET, KERR_COULD_NOT_CONNECT,
    KERR_EOF, KERR_NONE, KERR_NOT_SUPPORTED, KERR_PERMISSION_DENIED, KPROTOCOL_INET_TCP,
    KSOCK_STREAM, NAME_RECORD_ADDR_OFF,
};

#[derive(Clone)]
struct SharedNet(Rc<RefCell<FakeNet>>);

impl GuestNet for SharedNet {
    fn resolve(&mut self, host: &str) -> Result<[u8; 4], GuestSvcError> {
        self.0.borrow_mut().resolve(host)
    }
    fn connect(&mut self, ip: [u8; 4], port: u16) -> Result<u32, GuestSvcError> {
        self.0.borrow_mut().connect(ip, port)
    }
    fn send(&mut self, conn: u32, data: &[u8]) -> Result<usize, GuestSvcError> {
        self.0.borrow_mut().send(conn, data)
    }
    fn recv(&mut self, conn: u32, buf: &mut [u8]) -> Result<usize, GuestSvcError> {
        self.0.borrow_mut().recv(conn, buf)
    }
    fn poll_events(&mut self, conn: u32) -> Result<u32, GuestSvcError> {
        self.0.borrow_mut().poll_events(conn)
    }
    fn shutdown_write(&mut self, conn: u32) -> Result<(), GuestSvcError> {
        self.0.borrow_mut().shutdown_write(conn)
    }
    fn close(&mut self, conn: u32) -> Result<(), GuestSvcError> {
        self.0.borrow_mut().close(conn)
    }
}

const STUBS: &[&str] = &[
    "_ZN4User6AllocZEi",
    "_ZN11RSocketServ7ConnectEv",
    "_ZN11RSocketServ5CloseEv",
    "_ZN13RHostResolver4OpenER11RSocketServjj",
    "_ZN13RHostResolver9GetByNameERK7TDesC16R5TPckgI9TNameEntryER14TRequestStatus",
    "_ZN13RHostResolver5CloseEv",
    "_ZN7RSocket4OpenER11RSocketServjjj",
    "_ZN7RSocket7ConnectER9TSockAddrR14TRequestStatus",
    "_ZN7RSocket5WriteERK6TDesC8R14TRequestStatus",
    "_ZN7RSocket4ReadER5TDes8R14TRequestStatus",
    "_ZN7RSocket5CloseEv",
    "_ZN7RSocket4BindER9TSockAddr",
    "_ZN7RSocket6ListenEj",
    "_ZN7RSocket6AcceptERS_R14TRequestStatus",
];

fn make_proc(net: Option<SharedNet>) -> Process {
    let mut p = loader::bare(STUBS).unwrap();
    if let Some(n) = net {
        p.net_host = Some(Box::new(n));
    }
    p
}

fn call(p: &mut Process, name: &str, args: &[u32]) -> u32 {
    let idx = p.stubs.iter().position(|s| s.name.as_deref() == Some(name)).unwrap_or_else(|| panic!("{name} missing"));
    for (i, &a) in args.iter().enumerate() {
        p.cpu.regs[i] = a;
    }
    match natives::call(p, idx) {
        NativeOutcome::Return => p.cpu.regs[0],
        other => panic!("{name}: unexpected {other:?}"),
    }
}

fn alloc_z(p: &mut Process, len: u32) -> u32 {
    call(p, "_ZN4User6AllocZEi", &[len])
}

fn mk_tbufc16(p: &mut Process, text: &str) -> u32 {
    let u16s: Vec<u16> = text.encode_utf16().collect();
    let addr = alloc_z(p, 4 + u16s.len() as u32 * 2);
    p.mem.write_u32(addr, u16s.len() as u32).unwrap();
    for (i, &c) in u16s.iter().enumerate() {
        p.mem.write_u16(addr + 4 + i as u32 * 2, c).unwrap();
    }
    addr
}

fn mk_tbufc8(p: &mut Process, bytes: &[u8]) -> u32 {
    let addr = alloc_z(p, 4 + bytes.len() as u32);
    p.mem.write_u32(addr, bytes.len() as u32).unwrap();
    for (i, &b) in bytes.iter().enumerate() {
        p.mem.write_u8(addr + 4 + i as u32, b).unwrap();
    }
    addr
}

fn mk_tbuf8(p: &mut Process, cap: u32) -> u32 {
    let addr = alloc_z(p, 8 + cap);
    // EBuf = type 3 << 28 | len(0)
    p.mem.write_u32(addr, 3u32 << 28).unwrap();
    p.mem.write_u32(addr + 4, cap).unwrap();
    addr
}

#[test]
fn esock_dns_connect_send_recv_eof_close_round_trip() {
    let fake = Rc::new(RefCell::new(FakeNet::new()));
    fake.borrow_mut().add_host("shell.coral.lan", [10, 0, 2, 22]);
    let mut p = make_proc(Some(SharedNet(fake.clone())));

    let serv = alloc_z(&mut p, 16);
    let resv = alloc_z(&mut p, 16);
    let sock = alloc_z(&mut p, 16);
    let st = alloc_z(&mut p, 4);
    let name_rec = alloc_z(&mut p, 600);
    let pckg = alloc_z(&mut p, 16);
    p.mem.write_u32(pckg + 8, name_rec).unwrap();

    // 1. RSocketServ::Connect + RHostResolver::Open + GetByName
    assert_eq!(call(&mut p, "_ZN11RSocketServ7ConnectEv", &[serv]) as i32, KERR_NONE);
    assert_eq!(call(&mut p, "_ZN13RHostResolver4OpenER11RSocketServjj", &[resv, serv, KAF_INET, KPROTOCOL_INET_TCP]) as i32, KERR_NONE);

    let host_des = mk_tbufc16(&mut p, "shell.coral.lan");
    call(&mut p, "_ZN13RHostResolver9GetByNameERK7TDesC16R5TPckgI9TNameEntryER14TRequestStatus", &[resv, host_des, pckg, st]);
    assert_eq!(p.mem.read_u32(st).unwrap() as i32, KERR_NONE);
    let resolved_ip = p.mem.read_u32(name_rec + NAME_RECORD_ADDR_OFF + INET_ADDR_IPV4_OFF).unwrap().to_be_bytes();
    assert_eq!(resolved_ip, [10, 0, 2, 22]);

    // 2. RSocket::Open + Connect + Send + Recv + EOF + Close
    let sp = alloc_z(&mut p, 16);
    p.mem.write_u32(sp, KPROTOCOL_INET_TCP).unwrap();
    p.cpu.regs[13] = sp;
    assert_eq!(call(&mut p, "_ZN7RSocket4OpenER11RSocketServjjj", &[sock, serv, KAF_INET, KSOCK_STREAM]) as i32, KERR_NONE);

    let addr = alloc_z(&mut p, 16);
    p.mem.write_u32(addr + INET_ADDR_FAMILY_OFF, KAF_INET).unwrap();
    p.mem.write_u32(addr + INET_ADDR_PORT_OFF, 22).unwrap();
    p.mem.write_u32(addr + INET_ADDR_IPV4_OFF, u32::from_be_bytes(resolved_ip)).unwrap();
    call(&mut p, "_ZN7RSocket7ConnectER9TSockAddrR14TRequestStatus", &[sock, addr, st]);
    assert_eq!(p.mem.read_u32(st).unwrap() as i32, KERR_NONE);
    let conn_id = *fake.borrow().conns().keys().next().unwrap();

    let tx_des = mk_tbufc8(&mut p, b"SSH-2.0-PuTTY\r\n");
    call(&mut p, "_ZN7RSocket5WriteERK6TDesC8R14TRequestStatus", &[sock, tx_des, st]);
    assert_eq!(p.mem.read_u32(st).unwrap() as i32, KERR_NONE);
    assert_eq!(fake.borrow().conns()[&conn_id].tx.as_slice(), b"SSH-2.0-PuTTY\r\n");

    fake.borrow_mut().inject_rx(conn_id, b"SSH-2.0-Coral\r\n");
    let rx_des = mk_tbuf8(&mut p, 64);
    call(&mut p, "_ZN7RSocket4ReadER5TDes8R14TRequestStatus", &[sock, rx_des, st]);
    assert_eq!(p.mem.read_u32(st).unwrap() as i32, KERR_NONE);
    assert_eq!(heirloom_ekaarm_engine::desc::read8(&p, rx_des).unwrap(), b"SSH-2.0-Coral\r\n");

    fake.borrow_mut().inject_eof(conn_id);
    call(&mut p, "_ZN7RSocket4ReadER5TDes8R14TRequestStatus", &[sock, rx_des, st]);
    assert_eq!(p.mem.read_u32(st).unwrap() as i32, KERR_EOF);

    call(&mut p, "_ZN7RSocket5CloseEv", &[sock]);
    assert!(fake.borrow().conns().is_empty());
}

#[test]
fn negative_controls_refuse_inbound_sockets_ungranted_and_dns_miss() {
    // 1. Outbound-only policy refuses Bind / Listen / Accept / UDP by name with KErrNotSupported (-5).
    let fake = Rc::new(RefCell::new(FakeNet::new()));
    let mut p = make_proc(Some(SharedNet(fake.clone())));
    let sock = alloc_z(&mut p, 16);
    let st = alloc_z(&mut p, 4);

    assert_eq!(call(&mut p, "_ZN7RSocket4BindER9TSockAddr", &[sock]) as i32, KERR_NOT_SUPPORTED);
    assert_eq!(call(&mut p, "_ZN7RSocket6ListenEj", &[sock, 5]) as i32, KERR_NOT_SUPPORTED);
    call(&mut p, "_ZN7RSocket6AcceptERS_R14TRequestStatus", &[sock, sock, st]);
    assert_eq!(p.mem.read_u32(st).unwrap() as i32, KERR_NOT_SUPPORTED);

    // UDP socket refused by name.
    assert_eq!(call(&mut p, "_ZN7RSocket4OpenER11RSocketServjjj", &[sock, 0, KAF_INET, 2]) as i32, KERR_NOT_SUPPORTED);

    // DNS miss -> KErrCouldNotConnect (-34).
    let missing = mk_tbufc16(&mut p, "nope.invalid");
    call(&mut p, "_ZN13RHostResolver9GetByNameERK7TDesC16R5TPckgI9TNameEntryER14TRequestStatus", &[sock, missing, sock, st]);
    assert_eq!(p.mem.read_u32(st).unwrap() as i32, KERR_COULD_NOT_CONNECT);

    // 2. Missing (net) grant -> KErrPermissionDenied (-46).
    let denied = Rc::new(RefCell::new(FakeNet::not_granted()));
    let mut pd = make_proc(Some(SharedNet(denied)));
    let st2 = alloc_z(&mut pd, 4);
    let host2 = mk_tbufc16(&mut pd, "shell.coral.lan");
    call(&mut pd, "_ZN13RHostResolver9GetByNameERK7TDesC16R5TPckgI9TNameEntryER14TRequestStatus", &[0, host2, 0, st2]);
    assert_eq!(pd.mem.read_u32(st2).unwrap() as i32, KERR_PERMISSION_DENIED);

    // 3. Unattached host -> RSocketServ::Connect returns KErrPermissionDenied (-46).
    let mut pu = make_proc(None);
    let s3 = alloc_z(&mut pu, 16);
    assert_eq!(call(&mut pu, "_ZN11RSocketServ7ConnectEv", &[s3]) as i32, KERR_PERMISSION_DENIED);
}
