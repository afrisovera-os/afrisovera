//! Architect's TPM merge follow-up 2 (13 Nov 2026): the Startup path had no test, because OVMF always starts the
//! TPM before the kernel runs. Here kernel/src/pc/tpm_tis.rs (the code the PC kernel builds, by `#[path]`) drives a
//! scripted TPM behind the `Bus` trait: a TIS FIFO model that answers like a TPM 1.2 or 2.0 that was, or was not,
//! started by firmware. The rule under test: Startup only on "not started", once per boot, then the command once
//! more; any other error sends nothing more. Also the 1.2 flag probe, bytes left over, and the start-up ceiling
//! with the numbers the "too slow" line prints (follow-up 4).
use drbg_host::drbg::STARTUP_SAMPLES;
use drbg_host::tpm_tis::*;
use drbg_host::tpm_wire::{Ver, WireErr};
use std::collections::VecDeque;

const HZ: u64 = 1_000_000; // one tick is one microsecond

#[derive(PartialEq, Debug)]
enum Phase {
    Idle,
    Ready,
    Resp,
}

/// A TPM behind a TIS FIFO, with a clock that moves one tick per read and `us_per_cmd` per command.
struct Fake {
    ver: Ver,
    /// The TPM's own state: has a Startup run on it.
    started: bool,
    /// Startup's return code (0 = success).
    startup_rc: u32,
    /// Startup "succeeds" but the TPM still answers "not started" (a broken TPM: must not loop).
    stays_unstarted: bool,
    /// GetRandom and GetCapability answer this code instead.
    other_rc: u32,
    /// Bytes sent after a whole response.
    extra: usize,
    /// TPM 1.2: the permanent `disable` and volatile `deactivated` flag bytes.
    disable: u8,
    deactivated: u8,
    burst: usize,
    us_per_cmd: u64,
    // TIS state
    active: bool,
    phase: Phase,
    cmd: Vec<u8>,
    rsp: VecDeque<u8>,
    clock: u64,
    /// Every command executed, whole.
    log: Vec<Vec<u8>>,
}

impl Fake {
    fn new(ver: Ver, started: bool) -> Fake {
        Fake {
            ver,
            started,
            startup_rc: 0,
            stays_unstarted: false,
            other_rc: 0,
            extra: 0,
            disable: 0,
            deactivated: 0,
            burst: 8,
            us_per_cmd: 50,
            active: false,
            phase: Phase::Idle,
            cmd: vec![],
            rsp: VecDeque::new(),
            clock: 0,
            log: vec![],
        }
    }

    fn codes(&self) -> Vec<u32> {
        self.log.iter().map(|c| be32(&c[6..10])).collect()
    }

    fn not_started_rc(&self) -> u32 {
        match self.ver {
            Ver::V12 => 0x26,
            Ver::V20 => 0x100,
        }
    }

    fn answer(&self, rc: u32, body: &[u8]) -> Vec<u8> {
        let tag: u16 = match self.ver {
            Ver::V12 => 0x00C4,
            Ver::V20 => 0x8001,
        };
        let mut v = tag.to_be_bytes().to_vec();
        v.extend_from_slice(&((10 + body.len()) as u32).to_be_bytes());
        v.extend_from_slice(&rc.to_be_bytes());
        v.extend_from_slice(body);
        v.extend(std::iter::repeat(0).take(self.extra));
        v
    }

    fn execute(&mut self) -> Vec<u8> {
        let c = self.cmd.clone();
        self.log.push(c.clone());
        self.clock += self.us_per_cmd;
        let code = be32(&c[6..10]);
        if code == 0x99 || code == 0x144 {
            if self.startup_rc != 0 {
                return self.answer(self.startup_rc, &[]);
            }
            self.started = !self.stays_unstarted;
            return self.answer(0, &[]);
        }
        if !self.started {
            return self.answer(self.not_started_rc(), &[]);
        }
        if self.other_rc != 0 {
            return self.answer(self.other_rc, &[]);
        }
        match code {
            0x46 => {
                let k = be32(&c[10..14]) as usize;
                let mut b = (k as u32).to_be_bytes().to_vec();
                b.extend(std::iter::repeat(0xA5).take(k));
                self.answer(0, &b)
            }
            0x17B => {
                let k = u16::from_be_bytes([c[10], c[11]]) as usize;
                let mut b = (k as u16).to_be_bytes().to_vec();
                b.extend(std::iter::repeat(0xA5).take(k));
                self.answer(0, &b)
            }
            0x65 => {
                let (tag, flag): (u16, u8) = match be32(&c[18..22]) {
                    0x108 => (0x1F, self.disable),
                    0x109 => (0x20, self.deactivated),
                    _ => panic!("the driver asked for a capability outside the closed list"),
                };
                let mut b = 4u32.to_be_bytes().to_vec();
                b.extend_from_slice(&tag.to_be_bytes());
                b.extend_from_slice(&[flag, 0]);
                self.answer(0, &b)
            }
            other => panic!("the driver sent a command outside the closed list: {other:#x}"),
        }
    }

    fn expect(&self) -> bool {
        if self.cmd.len() < 6 {
            return !self.cmd.is_empty();
        }
        self.cmd.len() < be32(&self.cmd[2..6]) as usize
    }
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

impl Bus for Fake {
    fn r8(&mut self, off: u64) -> u8 {
        self.clock += 1;
        match off {
            ACCESS => ACCESS_VALID | if self.active { ACCESS_ACTIVE } else { 0 },
            FIFO => self.rsp.pop_front().unwrap_or(0xFF),
            _ => 0xFF,
        }
    }
    fn r32(&mut self, off: u64) -> u32 {
        self.clock += 1;
        assert_eq!(off, STS, "only STS is read as a word after the probe");
        match self.phase {
            Phase::Idle => STS_VALID,
            Phase::Ready => {
                STS_VALID
                    | STS_COMMAND_READY
                    | ((self.burst as u32) << 8)
                    | if self.expect() { STS_EXPECT } else { 0 }
            }
            Phase::Resp => {
                let k = self.rsp.len().min(self.burst) as u32;
                STS_VALID | if k > 0 { STS_DATA_AVAIL } else { 0 } | (k << 8)
            }
        }
    }
    fn w8(&mut self, off: u64, v: u8) {
        match off {
            ACCESS => {
                if v & ACCESS_REQUEST_USE != 0 {
                    self.active = true;
                }
                if v & ACCESS_ACTIVE != 0 {
                    self.active = false;
                }
            }
            FIFO => {
                assert!(self.active, "a FIFO write without locality");
                assert_eq!(self.phase, Phase::Ready, "a FIFO write outside commandReady");
                self.cmd.push(v);
            }
            _ => panic!("write to an unknown register {off:#x}"),
        }
    }
    fn w32(&mut self, off: u64, v: u32) {
        assert_eq!(off, STS);
        if v == STS_COMMAND_READY {
            self.phase = Phase::Ready;
            self.cmd.clear();
            self.rsp.clear();
        } else if v == STS_GO {
            assert!(self.active, "GO without locality");
            let r = self.execute();
            self.rsp = r.into();
            self.phase = Phase::Resp;
        } else {
            panic!("unexpected STS write {v:#x}");
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

fn session(ver: Ver) -> Session {
    Session { ver, started_by_us: false }
}

fn random(f: &mut Fake, s: &mut Session) -> Result<usize, IoErr> {
    assert!(request_locality(f));
    let mut b = [0u8; 32];
    let r = get_random_once(f, s, &mut b);
    release_locality(f);
    if let Ok(n) = r {
        assert!(b[..n].iter().all(|&x| x == 0xA5), "the bytes are the TPM's");
    }
    r
}

#[test]
fn an_unstarted_tpm_2_0_is_started_once_and_the_command_sent_again() {
    let mut f = Fake::new(Ver::V20, false);
    let mut s = session(Ver::V20);
    assert_eq!(random(&mut f, &mut s), Ok(32));
    assert_eq!(f.codes(), vec![0x17B, 0x144, 0x17B], "GetRandom, Startup, GetRandom");
    assert_eq!(f.log[1], vec![0x80, 0x01, 0, 0, 0, 0x0C, 0, 0, 0x01, 0x44, 0, 0], "TPM2_Startup(SU_CLEAR)");
    assert!(s.started_by_us);
    // Started now: the next call is one command.
    assert_eq!(random(&mut f, &mut s), Ok(32));
    assert_eq!(f.codes(), vec![0x17B, 0x144, 0x17B, 0x17B]);
}

#[test]
fn an_unstarted_tpm_1_2_is_started_once_with_st_clear() {
    let mut f = Fake::new(Ver::V12, false);
    let mut s = session(Ver::V12);
    assert_eq!(random(&mut f, &mut s), Ok(32));
    assert_eq!(f.codes(), vec![0x46, 0x99, 0x46]);
    assert_eq!(f.log[1], vec![0x00, 0xC1, 0, 0, 0, 0x0C, 0, 0, 0, 0x99, 0, 0x01], "TPM_Startup(ST_CLEAR)");
    assert!(s.started_by_us);
}

#[test]
fn a_started_tpm_is_never_sent_startup() {
    for ver in [Ver::V12, Ver::V20] {
        let mut f = Fake::new(ver, true);
        let mut s = session(ver);
        for _ in 0..3 {
            assert_eq!(random(&mut f, &mut s), Ok(32));
        }
        assert!(f.codes().iter().all(|&c| c != 0x99 && c != 0x144), "{ver:?}: {:x?}", f.codes());
        assert!(!s.started_by_us);
    }
}

#[test]
fn startup_is_sent_at_most_once_per_boot() {
    // A TPM that says "not started" even after a good Startup: one Startup, one retry, then the error stands.
    for ver in [Ver::V12, Ver::V20] {
        let mut f = Fake::new(ver, false);
        f.stays_unstarted = true;
        let mut s = session(ver);
        let ns = f.not_started_rc();
        assert_eq!(random(&mut f, &mut s), Err(IoErr::Wire(WireErr::Rc(ns))));
        assert_eq!(f.log.len(), 3, "{ver:?}: command, Startup, command");
        assert_eq!(random(&mut f, &mut s), Err(IoErr::Wire(WireErr::Rc(ns))));
        assert_eq!(f.log.len(), 4, "{ver:?}: the second call sends no second Startup");
        assert_eq!(f.codes().iter().filter(|&&c| c == 0x99 || c == 0x144).count(), 1);
    }
}

#[test]
fn any_other_error_sends_nothing_more() {
    // TPM_RC_FAILURE (2.0), TPM_FAIL (1.2), TPM_RC_RETRY and TPM_RC_TESTING (2.0, treated as errors: design gap).
    for (ver, rc) in [(Ver::V20, 0x101), (Ver::V12, 0x09), (Ver::V20, 0x922), (Ver::V20, 0x90A)] {
        let mut f = Fake::new(ver, true);
        f.other_rc = rc;
        let mut s = session(ver);
        assert_eq!(random(&mut f, &mut s), Err(IoErr::Wire(WireErr::Rc(rc))));
        assert_eq!(f.log.len(), 1, "{ver:?} rc {rc:#x}: one command, no Startup, no retry");
        assert!(!s.started_by_us);
    }
}

#[test]
fn a_failed_startup_is_not_followed_by_the_command() {
    let mut f = Fake::new(Ver::V20, false);
    f.startup_rc = 0x101;
    let mut s = session(Ver::V20);
    assert_eq!(random(&mut f, &mut s), Err(IoErr::Wire(WireErr::Rc(0x101))));
    assert_eq!(f.codes(), vec![0x17B, 0x144]);
}

#[test]
fn the_1_2_probe_starts_once_reads_both_flags_then_draws() {
    let mut f = Fake::new(Ver::V12, false);
    let mut s = session(Ver::V12);
    assert!(request_locality(&mut f));
    let r = probe(&mut f, &mut s);
    release_locality(&mut f);
    let us = r.expect("a 1.2 switched on is counted");
    assert!(us > 0 && us < 10_000, "one call's time in microseconds: {us}");
    assert_eq!(f.codes(), vec![0x65, 0x99, 0x65, 0x65, 0x46], "GetCap, Startup, GetCap(perm), GetCap(vol), GetRandom");
    assert!(!f.active, "locality given up");
}

#[test]
fn a_1_2_switched_off_or_unreadable_is_not_counted() {
    for (dis, deact, want) in [
        (1, 0, "switched off in the firmware setup; switch it on there to use it"),
        (0, 1, "switched off in the firmware setup; switch it on there to use it"),
        (2, 0, "its on/off flags could not be read, so it is not counted"),
        (0, 7, "its on/off flags could not be read, so it is not counted"),
    ] {
        let mut f = Fake::new(Ver::V12, true);
        f.disable = dis;
        f.deactivated = deact;
        let mut s = session(Ver::V12);
        assert!(request_locality(&mut f));
        assert_eq!(probe(&mut f, &mut s), Err(want), "disable={dis} deactivated={deact}");
        assert!(!f.codes().contains(&0x46), "no GetRandom from a TPM that is not counted");
    }
}

#[test]
fn bytes_left_over_after_a_response_are_refused() {
    let mut f = Fake::new(Ver::V20, true);
    f.extra = 1;
    let mut s = session(Ver::V20);
    assert_eq!(random(&mut f, &mut s), Err(IoErr::Extra));
}

#[test]
fn a_tpm_that_never_offers_a_burst_is_given_up_under_the_ceiling() {
    let mut f = Fake::new(Ver::V20, true);
    f.burst = 0;
    let mut s = session(Ver::V20);
    let t0 = f.clock;
    assert_eq!(random(&mut f, &mut s), Err(IoErr::NoBurst));
    let spent_ms = (f.clock - t0) / 1000;
    assert!(spent_ms <= 2 * TIMEOUT_A_MS + 10, "bounded by timeout A: {spent_ms} ms");
    assert!(f.log.is_empty(), "nothing was executed");
}

#[test]
fn a_batch_fills_the_buffer_in_32_byte_calls() {
    let mut f = Fake::new(Ver::V20, true);
    let mut s = session(Ver::V20);
    let mut buf = [0u8; 100];
    assert!(request_locality(&mut f));
    assert_eq!(fill_batch(&mut f, &mut s, &mut buf), (100, 4, false));
    release_locality(&mut f);
    assert!(buf.iter().all(|&x| x == 0xA5));
    assert!(!f.active);
}

#[test]
fn the_start_up_ceiling_and_the_too_slow_numbers() {
    // A fast TPM: 1024 bytes in 32 calls well under 2 s, and nothing is charged after the start-up samples.
    let mut b = Budget::default();
    assert_eq!(b.charge(1024, 32 * 900, 32, HZ, STARTUP_SAMPLES, STARTUP_CEILING_MS), Ok(()));
    assert_eq!(b.charge(64, 10 * HZ, 2, HZ, STARTUP_SAMPLES, STARTUP_CEILING_MS), Ok(()), "after the test: free");
    // A slow one, end to end through the scripted TPM: 70 ms per GetRandom, 32 calls for 1024 bytes = 2.24 s.
    let mut f = Fake::new(Ver::V20, true);
    f.us_per_cmd = 70_000;
    let mut s = session(Ver::V20);
    let mut buf = [0u8; 1024];
    assert!(request_locality(&mut f));
    let t0 = f.now();
    let (n, calls, stopped) = fill_batch(&mut f, &mut s, &mut buf);
    release_locality(&mut f);
    let dt = f.now() - t0;
    assert_eq!((n, calls, stopped), (1024, 32, false));
    let mut b = Budget::default();
    let slow = b.charge(n, dt, calls, HZ, STARTUP_SAMPLES, STARTUP_CEILING_MS).expect_err("over 2 s");
    assert_eq!(slow.calls, 32);
    assert!((2240..2300).contains(&slow.total_ms), "total {} ms", slow.total_ms);
    assert!((70_000..72_000).contains(&slow.us_per_call), "per call {} us", slow.us_per_call);
    // The ceiling is a total: two batches that are each under it but together over it fail on the second.
    let mut b = Budget::default();
    assert_eq!(b.charge(512, HZ, 16, HZ, STARTUP_SAMPLES, STARTUP_CEILING_MS), Ok(()));
    assert_eq!(
        b.charge(512, HZ + 1, 16, HZ, STARTUP_SAMPLES, STARTUP_CEILING_MS),
        Err(Slow { total_ms: 2000, calls: 32, us_per_call: 62_500 })
    );
}
