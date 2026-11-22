//! F3 TPM over CRB (PLAN_OS_MESH_F3_TPM_CRB.md, architect's OK 14 Nov 2026): kernel/src/pc/tpm_crb.rs (the code the PC
//! kernel builds, by `#[path]`) against a scripted CRB TPM 2.0 behind the `Bus` trait, laid out as QEMU's tpm-crb and
//! Intel PTT are (control area at +0x40, one 0xF80-byte buffer at +0x80). The shared rules (tpm_rules.rs) run
//! through `CrbLink`, so "Startup only on not started, once" is checked on this link as tests/tpm_tis.rs checks it
//! on TIS. Every `check` failure is asserted to have written nothing at all.
use drbg_host::tpm_crb::*;
use drbg_host::tpm_rules::{self as rules, Bus, IoErr, Session};
use drbg_host::tpm_wire::{Ver, WireErr};

const HZ: u64 = 1_000_000; // one tick is one microsecond
const BASE: u64 = 0xFED4_0000;
const CTRL: u64 = 0x40;
const BUF: u64 = 0x80;

struct Fake {
    mem: Vec<u8>,
    active: bool,
    grant: bool,
    started: bool,
    stays_unstarted: bool,
    fatal_before: bool,
    fatal_after: bool,
    ready_hangs: bool,
    start_hangs: bool,
    /// Overrides the response's size field (a lying TPM).
    size_lie: Option<u32>,
    us_per_cmd: u64,
    start: u32,
    cancel_log: Vec<u32>,
    idle_requests: usize,
    writes: usize,
    max_rsp_read: u64,
    clock: u64,
    log: Vec<Vec<u8>>,
}

impl Fake {
    fn new(started: bool) -> Fake {
        let mut f = Fake {
            mem: vec![0; 0x1000],
            active: false,
            grant: true,
            started,
            stays_unstarted: false,
            fatal_before: false,
            fatal_after: false,
            ready_hangs: false,
            start_hangs: false,
            size_lie: None,
            us_per_cmd: 50,
            start: 0,
            cancel_log: vec![],
            idle_requests: 0,
            writes: 0,
            max_rsp_read: 0,
            clock: 0,
            log: vec![],
        };
        f.set32(INTF_ID, INTF_TYPE_CRB | 0x10 << 4); // type CRB, version 1
        f.set32(INTF_ID_HI, 0x0001_8086); // vendor 0x8086, device 0x0001
        f.set32(CTRL + CMD_SIZE, 0xF80);
        f.set32(CTRL + CMD_LADDR, (BASE + BUF) as u32);
        f.set32(CTRL + CMD_HADDR, 0);
        f.set32(CTRL + RSP_SIZE, 0xF80);
        f.set32(CTRL + RSP_ADDR, (BASE + BUF) as u32);
        f.set32(CTRL + RSP_ADDR + 4, 0);
        f
    }

    fn set32(&mut self, off: u64, v: u32) {
        self.mem[off as usize..off as usize + 4].copy_from_slice(&v.to_le_bytes());
    }

    fn codes(&self) -> Vec<u32> {
        self.log.iter().map(|c| be32(&c[6..10])).collect()
    }

    fn answer(rc: u32, body: &[u8]) -> Vec<u8> {
        let mut v = vec![0x80, 0x01];
        v.extend_from_slice(&((10 + body.len()) as u32).to_be_bytes());
        v.extend_from_slice(&rc.to_be_bytes());
        v.extend_from_slice(body);
        v
    }

    fn execute(&mut self) {
        let size = be32(&self.mem[BUF as usize + 2..BUF as usize + 6]) as usize;
        let c = self.mem[BUF as usize..BUF as usize + size].to_vec();
        self.log.push(c.clone());
        self.clock += self.us_per_cmd;
        let code = be32(&c[6..10]);
        let mut r = if code == 0x144 {
            self.started = !self.stays_unstarted;
            Fake::answer(0, &[])
        } else if !self.started {
            Fake::answer(0x100, &[])
        } else if code == 0x17B {
            let k = u16::from_be_bytes([c[10], c[11]]) as usize;
            let mut b = (k as u16).to_be_bytes().to_vec();
            b.extend(std::iter::repeat(0xA5).take(k));
            Fake::answer(0, &b)
        } else {
            panic!("the driver sent a command outside the closed list: {code:#x}")
        };
        if let Some(n) = self.size_lie {
            r[2..6].copy_from_slice(&n.to_be_bytes());
        }
        self.mem[BUF as usize..BUF as usize + r.len()].copy_from_slice(&r);
    }
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

impl Bus for Fake {
    fn r8(&mut self, off: u64) -> u8 {
        self.clock += 1;
        assert!(off < 0x1000, "a read outside the page");
        if off >= BUF {
            self.max_rsp_read = self.max_rsp_read.max(off);
        }
        self.mem[off as usize]
    }
    fn r32(&mut self, off: u64) -> u32 {
        self.clock += 1;
        assert!(off + 4 <= 0x1000, "a read outside the page");
        match off {
            LOC_STATE => {
                let v = be_le(&self.mem, LOC_STATE);
                if v != 0 {
                    return v; // a test's override (no answer, invalid)
                }
                LOC_STATE_VALID | if self.active { LOC_STATE_ASSIGNED } else { 0 }
            }
            o if o == CTRL + REQ => {
                if self.ready_hangs {
                    REQ_CMD_READY
                } else {
                    0
                }
            }
            o if o == CTRL + STS => {
                if self.fatal_before || (self.fatal_after && !self.log.is_empty()) {
                    STS_FATAL
                } else {
                    0
                }
            }
            o if o == CTRL + START => self.start,
            _ => be_le(&self.mem, off),
        }
    }
    fn w8(&mut self, off: u64, v: u8) {
        self.writes += 1;
        assert!(self.active, "a buffer write without locality");
        assert!((BUF..BUF + 0xF80).contains(&off), "a byte write outside the command buffer: {off:#x}");
        self.mem[off as usize] = v;
    }
    fn w32(&mut self, off: u64, v: u32) {
        self.writes += 1;
        match off {
            LOC_CTRL => {
                if v & LOC_CTRL_REQUEST != 0 && self.grant {
                    self.active = true;
                }
                if v & LOC_CTRL_RELINQUISH != 0 {
                    self.active = false;
                }
            }
            o if o == CTRL + REQ => {
                if v & REQ_GO_IDLE != 0 {
                    self.idle_requests += 1;
                }
            }
            o if o == CTRL + CANCEL => {
                self.cancel_log.push(v);
                if v == CANCEL_INVOKE {
                    self.start = 0;
                }
            }
            o if o == CTRL + START => {
                assert!(self.active, "START without locality");
                if self.start_hangs {
                    self.start = START_INVOKE;
                } else {
                    self.execute();
                    self.start = 0;
                }
            }
            _ => panic!("write to an unexpected register {off:#x}"),
        }
    }
    fn now(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }
    fn hz(&self) -> u64 {
        HZ
    }
}

fn be_le(mem: &[u8], off: u64) -> u32 {
    let o = off as usize;
    u32::from_le_bytes([mem[o], mem[o + 1], mem[o + 2], mem[o + 3]])
}

fn layout(f: &mut Fake) -> Layout {
    check(f, BASE, BASE + CTRL).expect("the QEMU/PTT layout passes")
}

fn random(f: &mut Fake, s: &mut Session) -> Result<usize, IoErr> {
    let l = layout(f);
    assert!(request_locality(f));
    let mut b = [0u8; 32];
    let r = rules::get_random_once(&mut CrbLink { bus: &mut *f, layout: l }, s, &mut b);
    release_locality(f);
    if let Ok(n) = r {
        assert!(b[..n].iter().all(|&x| x == 0xA5), "the bytes are the TPM's");
    }
    r
}

fn v20() -> Session {
    Session { ver: Ver::V20, started_by_us: false }
}

#[test]
fn the_qemu_and_ptt_layout_passes_and_a_draw_works() {
    let mut f = Fake::new(true);
    let l = layout(&mut f);
    assert_eq!(l, Layout { ctrl: 0x40, cmd: 0x80, cmd_size: 0xF80, rsp: 0x80, rsp_size: 0xF80, vid_did: 0x0001_8086 });
    assert_eq!(f.writes, 0, "check only reads");
    let mut s = v20();
    assert_eq!(random(&mut f, &mut s), Ok(32));
    assert_eq!(f.codes(), vec![0x17B]);
    assert_eq!(f.idle_requests, 1, "goIdle after the command");
    assert!(!f.active, "locality given up");
    assert!(f.max_rsp_read < BUF + 128, "reads stay inside 128 bytes of the response");
}

#[test]
fn an_unstarted_tpm_is_started_once_over_crb_too() {
    let mut f = Fake::new(false);
    let mut s = v20();
    assert_eq!(random(&mut f, &mut s), Ok(32));
    assert_eq!(f.codes(), vec![0x17B, 0x144, 0x17B]);
    assert_eq!(f.log[1], vec![0x80, 0x01, 0, 0, 0, 0x0C, 0, 0, 0x01, 0x44, 0, 0], "TPM2_Startup(SU_CLEAR)");
    assert!(s.started_by_us);
    assert_eq!(f.idle_requests, 3, "goIdle after every command");
    let mut f = Fake::new(false);
    f.stays_unstarted = true;
    let mut s = v20();
    assert_eq!(random(&mut f, &mut s), Err(IoErr::Wire(WireErr::Rc(0x100))));
    assert_eq!(random(&mut f, &mut s), Err(IoErr::Wire(WireErr::Rc(0x100))));
    assert_eq!(f.codes(), vec![0x17B, 0x144, 0x17B, 0x17B], "Startup at most once per boot");
}

#[test]
fn every_check_failure_writes_nothing() {
    type Setup = fn(&mut Fake);
    let cases: Vec<(&str, u64, Setup, CrbOff)> = vec![
        ("control area 0 (the synthetic table)", 0, |_| {}, CrbOff::Ctrl(0)),
        ("control area past the page end", BASE + 0xFE0, |_| {}, CrbOff::Ctrl(BASE + 0xFE0)),
        ("control area on the locality registers", BASE + 0x10, |_| {}, CrbOff::Ctrl(BASE + 0x10)),
        ("control area that wraps", u64::MAX, |_| {}, CrbOff::Ctrl(u64::MAX)),
        ("nothing answers", BASE + CTRL, |f| f.set32(LOC_STATE, u32::MAX), CrbOff::NoAnswer(u32::MAX)),
        ("not valid", BASE + CTRL, |f| f.set32(LOC_STATE, 0x02), CrbOff::NoAnswer(0x02)),
        ("interface id says FIFO", BASE + CTRL, |f| f.set32(INTF_ID, 0), CrbOff::NotCrb(0)),
        ("interface id all ones", BASE + CTRL, |f| f.set32(INTF_ID, u32::MAX), CrbOff::NotCrb(u32::MAX)),
        (
            "command buffer with a high half",
            BASE + CTRL,
            |f| f.set32(CTRL + CMD_HADDR, 1),
            CrbOff::Outside(Buf::Command, 1 << 32 | (BASE + BUF), 0xF80),
        ),
        (
            "command buffer in RAM (AMD fTPM style)",
            BASE + CTRL,
            |f| f.set32(CTRL + CMD_LADDR, 0x7000_0000),
            CrbOff::Outside(Buf::Command, 0x7000_0000, 0xF80),
        ),
        (
            "command buffer one byte past the page",
            BASE + CTRL,
            |f| f.set32(CTRL + CMD_SIZE, 0xF81),
            CrbOff::Outside(Buf::Command, BASE + BUF, 0xF81),
        ),
        (
            // passed before the floor: in the page, clear of the control area, big enough; the command bytes would
            // then have been written over LOC_STATE, LOC_CTRL and the interface id
            "command buffer on the locality registers",
            BASE + CTRL,
            |f| {
                f.set32(CTRL + CMD_LADDR, BASE as u32);
                f.set32(CTRL + CMD_SIZE, 0x20);
            },
            CrbOff::Outside(Buf::Command, BASE, 0x20),
        ),
        (
            // refused before the floor only as too small; the floor names the real fault first
            "response buffer on the locality registers",
            BASE + CTRL,
            |f| {
                f.set32(CTRL + RSP_ADDR, BASE as u32);
                f.set32(CTRL + RSP_SIZE, 0x20);
            },
            CrbOff::Outside(Buf::Response, BASE, 0x20),
        ),
        (
            // the case the floor exists for (the architect, 22 Nov 2026): the control area high in the page, the
            // command buffer at the page's first byte with exactly CMD_MAX bytes. In the page, clear of the control
            // area and big enough, so every older check passed it; only the floor refuses it
            "command buffer at the page start, control area at +0x800",
            BASE + 0x800,
            |f| {
                f.set32(0x800 + CMD_SIZE, drbg_host::tpm_wire::CMD_MAX as u32);
                f.set32(0x800 + CMD_LADDR, BASE as u32);
                f.set32(0x800 + CMD_HADDR, 0);
                f.set32(0x800 + RSP_SIZE, 0x7D0);
                f.set32(0x800 + RSP_ADDR, (BASE + 0x830) as u32);
                f.set32(0x800 + RSP_ADDR + 4, 0);
            },
            CrbOff::Outside(Buf::Command, BASE, drbg_host::tpm_wire::CMD_MAX as u64),
        ),
        (
            // the same for the response buffer: RSP_MAX bytes at the page start, the command buffer after the control
            // area
            "response buffer at the page start, control area at +0x800",
            BASE + 0x800,
            |f| {
                f.set32(0x800 + CMD_SIZE, 0x7D0);
                f.set32(0x800 + CMD_LADDR, (BASE + 0x830) as u32);
                f.set32(0x800 + CMD_HADDR, 0);
                f.set32(0x800 + RSP_SIZE, drbg_host::tpm_wire::RSP_MAX as u32);
                f.set32(0x800 + RSP_ADDR, BASE as u32);
                f.set32(0x800 + RSP_ADDR + 4, 0);
            },
            CrbOff::Outside(Buf::Response, BASE, drbg_host::tpm_wire::RSP_MAX as u64),
        ),
        (
            "command buffer over the control area",
            BASE + CTRL,
            |f| f.set32(CTRL + CMD_LADDR, (BASE + CTRL) as u32),
            CrbOff::Outside(Buf::Command, BASE + CTRL, 0xF80),
        ),
        (
            "response size that would wrap",
            BASE + CTRL,
            |f| f.set32(CTRL + RSP_SIZE, u32::MAX),
            CrbOff::Outside(Buf::Response, BASE + BUF, u32::MAX as u64),
        ),
        (
            "response buffer with a high half",
            BASE + CTRL,
            |f| f.set32(CTRL + RSP_ADDR + 4, 2),
            CrbOff::Outside(Buf::Response, 2 << 32 | (BASE + BUF), 0xF80),
        ),
        ("command buffer too small", BASE + CTRL, |f| f.set32(CTRL + CMD_SIZE, 16), CrbOff::Small(Buf::Command, 16)),
        ("response buffer too small", BASE + CTRL, |f| f.set32(CTRL + RSP_SIZE, 64), CrbOff::Small(Buf::Response, 64)),
    ];
    for (what, ctrl, setup, want) in cases {
        let mut f = Fake::new(true);
        setup(&mut f);
        assert_eq!(check(&mut f, BASE, ctrl), Err(want), "{what}");
        assert_eq!(f.writes, 0, "{what}: nothing may be written before every check passes");
    }
}

#[test]
fn the_reasons_name_their_values() {
    assert_eq!(CrbOff::Ctrl(0).to_string(), "CRB control area 0x0 is not inside the TPM's page");
    assert_eq!(
        CrbOff::NotCrb(0).to_string(),
        "the TPM2 table says CRB (start method 7) but the chip's interface id 0x0 says type 0"
    );
    assert_eq!(
        CrbOff::Outside(Buf::Command, 0x7000_0000, 0xF80).to_string(),
        "CRB command buffer 0x70000000 (3968 bytes) is not inside the TPM's page, clear of the control area"
    );
    assert_eq!(CrbOff::Small(Buf::Response, 64).to_string(), "CRB response buffer of 64 bytes is too small");
    assert_eq!(CrbOff::NoAnswer(0xffffffff).to_string(), "table but no CRB answer (loc_state 0xffffffff)");
}

#[test]
fn a_command_that_never_finishes_is_cancelled_then_the_tpm_goes_idle() {
    let mut f = Fake::new(true);
    f.start_hangs = true;
    let mut s = v20();
    let t0 = f.clock;
    assert_eq!(random(&mut f, &mut s), Err(IoErr::Timeout));
    assert_eq!(f.cancel_log, vec![0, CANCEL_INVOKE, 0], "cleared before START, invoked on the timeout, then cleared");
    assert_eq!(f.idle_requests, 1);
    let ms = (f.clock - t0) / 1000;
    assert!(ms <= TIMEOUT_CMD_MS + 2 * TIMEOUT_C_MS + 10, "bounded: {ms} ms");
}

#[test]
fn a_fatal_error_stops_before_start_or_before_the_response() {
    let mut f = Fake::new(true);
    f.fatal_before = true;
    let mut s = v20();
    assert_eq!(random(&mut f, &mut s), Err(IoErr::Fatal));
    assert!(f.log.is_empty(), "no START");
    assert_eq!(f.idle_requests, 1);
    let mut f = Fake::new(true);
    f.fatal_after = true;
    let mut s = v20();
    assert_eq!(random(&mut f, &mut s), Err(IoErr::Fatal));
    assert_eq!(f.max_rsp_read, 0, "the response is not read");
}

#[test]
fn a_response_size_above_128_is_refused_after_the_header() {
    for lie in [9u32, 129, 200, 0xF80, u32::MAX] {
        let mut f = Fake::new(true);
        f.size_lie = Some(lie);
        let mut s = v20();
        assert_eq!(random(&mut f, &mut s), Err(IoErr::Wire(WireErr::BadSize)), "size {lie}");
        assert!(f.max_rsp_read < BUF + 10, "size {lie}: only the header was read");
    }
}

#[test]
fn cmd_ready_or_locality_that_never_comes_is_bounded() {
    let mut f = Fake::new(true);
    f.ready_hangs = true;
    let mut s = v20();
    assert_eq!(random(&mut f, &mut s), Err(IoErr::NotReady));
    assert!(f.log.is_empty());
    let mut f = Fake::new(true);
    f.grant = false;
    let t0 = f.clock;
    assert!(!request_locality(&mut f));
    assert!((f.clock - t0) / 1000 <= TIMEOUT_C_MS + 10);
}

#[test]
fn the_probe_and_a_batch_run_over_crb() {
    let mut f = Fake::new(true);
    let l = layout(&mut f);
    let mut s = v20();
    assert!(request_locality(&mut f));
    let us = rules::probe(&mut CrbLink { bus: &mut f, layout: l }, &mut s).expect("counted");
    assert!(us > 0 && us < 10_000, "{us} us");
    let mut buf = [0u8; 100];
    let (n, calls, stopped) = rules::fill_batch(&mut CrbLink { bus: &mut f, layout: l }, &mut s, &mut buf);
    release_locality(&mut f);
    assert_eq!((n, calls, stopped), (100, 4, false));
    assert!(buf.iter().all(|&x| x == 0xA5));
    assert_eq!(f.idle_requests, 5, "goIdle after each of the 5 commands");
}
