//! The Wi-Fi test image's access point (kernel/src/pc/wifisim.rs, unchanged by `#[path]`, as `air`) against the
//! kernel's station (kernel/src/wlan/mlme.rs), driven the way pc/wifinet.rs drives them (tick, pop every frame into
//! the station, poll), with a pretend e1000 here. What suite pc-wifi-sim runs in QEMU, on the host, step by step.
#[path = "../../kernel/src/wlan/mod.rs"]
#[allow(dead_code, unused_imports)]
mod wlan;

macro_rules! kprintln {
    ($($t:tt)*) => { println!($($t)*) };
}
#[allow(unused_imports)]
pub(crate) use kprintln;

/// The pretend e1000: what the access point sends up, and what the "network" sends down.
mod e1000 {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    thread_local! {
        pub static UP: RefCell<Vec<Vec<u8>>> = RefCell::new(Vec::new());
        pub static DOWN: RefCell<VecDeque<Vec<u8>>> = RefCell::new(VecDeque::new());
    }
    pub const MAC: [u8; 6] = [0x52, 0x54, 0x00, 0x12, 0x34, 0x56];
    pub fn init() -> bool {
        true
    }
    pub fn mac() -> [u8; 6] {
        MAC
    }
    pub fn send(f: &[u8]) -> bool {
        UP.with(|u| u.borrow_mut().push(f.to_vec()));
        true
    }
    pub fn rx_ready() -> bool {
        DOWN.with(|d| !d.borrow().is_empty())
    }
    pub fn recv(dst: &mut [u8]) -> Option<usize> {
        let f = DOWN.with(|d| d.borrow_mut().pop_front())?;
        dst[..f.len()].copy_from_slice(&f);
        Some(f.len())
    }
}

#[path = "../../kernel/src/pc/wifisim.rs"]
#[allow(dead_code)]
mod air;

use wlan::mlme::{Event, NoConnect, Phase, Radio, Station};

struct Pc {
    st: Station,
    air: air::Air,
    now: u64,
    events: Vec<Event>,
}

impl Pc {
    fn new() -> Pc {
        wlan::mlme::set_secret_rng(|b| {
            for (i, x) in b.iter_mut().enumerate() {
                *x = 0x77 ^ i as u8;
            }
            true
        });
        let air = air::Air::probe().expect("the pretend e1000 is there");
        Pc { st: Station::new(air.mac()), air, now: 1000, events: Vec::new() }
    }
    /// pc/wifinet.rs pump, with `ms` of time passing first
    fn pump(&mut self, ms: u64) {
        self.now += ms;
        let mut buf = [0u8; 2400];
        self.air.tick(self.now);
        for _ in 0..16 {
            let Some(len) = self.air.pop(&mut buf) else { break };
            if let Some(e) = self.st.on_frame(&mut self.air, &buf[..len], -40, 6, self.now) {
                self.events.push(e);
            }
        }
        if let Some(e) = self.st.poll(&mut self.air, self.now) {
            self.events.push(e);
        }
    }
    fn run(&mut self, ms: u64) {
        for _ in 0..ms / 10 {
            self.pump(10);
        }
    }
}

#[test]
fn scan_hears_the_three_networks() {
    let mut p = Pc::new();
    assert!(p.st.scan(&mut p.air, p.now));
    p.run(300);
    let mut out = [None; wlan::bss::MAX_BSS];
    let n = p.st.table.sorted(&mut out);
    assert_eq!(n, 3);
}

#[test]
fn wpa2_join_through_the_test_access_point() {
    let mut p = Pc::new();
    p.st.scan(&mut p.air, p.now);
    p.run(300);
    assert_eq!(p.st.connect(&mut p.air, b"AfriSim", None, p.now), Err(NoConnect::NeedsPassphrase));
    let mut typed = *b"sim-passphrase-1";
    let pmk = wlan::crypto::pmk_from_typed(&mut typed, b"AfriSim").unwrap();
    p.st.connect(&mut p.air, b"AfriSim", Some(pmk.0), p.now).expect("joining starts");
    for step in 0..1200 {
        p.pump(10);
        if !p.events.is_empty() {
            println!("event after {} ms: {:?}", step * 10, p.events);
            break;
        }
    }
    println!("phase {:?}, counters {:?}", p.st.phase, p.st.counters);
    assert_eq!(p.events.first(), Some(&Event::Connected), "the join did not finish");
    assert_eq!(p.st.phase, Phase::Connected);
    // data up: an Ethernet frame from the stack leaves by the e1000, decrypted by the access point
    let mut eth = vec![0xFFu8; 6];
    eth.extend_from_slice(&e1000::MAC);
    eth.extend_from_slice(&0x0806u16.to_be_bytes());
    eth.extend_from_slice(&[0x11; 28]);
    assert!(p.st.send_eth(&mut p.air, &eth));
    p.run(50); // the access point answers what it heard, one frame per tick (message 4 first)
    let up = e1000::UP.with(|u| u.borrow().clone());
    assert_eq!(up.last(), Some(&eth), "the e1000 did not get the frame");
    // data down: a frame for the station from the network reaches the stack
    let mut down = e1000::MAC.to_vec();
    down.extend_from_slice(&[0x52, 0x55, 0x0A, 0x00, 0x02, 0x02]);
    down.extend_from_slice(&0x0800u16.to_be_bytes());
    down.extend_from_slice(&[0x45; 40]);
    e1000::DOWN.with(|d| d.borrow_mut().push_back(down.clone()));
    p.pump(10);
    let mut got = [0u8; 1600];
    let n = p.st.recv_eth(&mut got).expect("the frame from the network");
    assert_eq!(&got[..n], &down[..]);
    // beacons keep it up for longer than the beacon-loss time
    p.run(wlan::mlme::BEACON_LOSS_MS * 2);
    assert_eq!(p.st.phase, Phase::Connected, "lost: {:?}", p.events);
    assert_eq!(p.st.counters.rx_mic_fail + p.st.counters.rx_replay + p.st.counters.rx_plain_dropped, 0);
}

#[test]
fn wrong_passphrase_times_out() {
    let mut p = Pc::new();
    p.st.scan(&mut p.air, p.now);
    p.run(300);
    let mut typed = *b"not-the-passphrase";
    let pmk = wlan::crypto::pmk_from_typed(&mut typed, b"AfriSim").unwrap();
    p.st.connect(&mut p.air, b"AfriSim", Some(pmk.0), p.now).unwrap();
    p.run(wlan::supp::TIMEOUT_MS + 500);
    assert!(matches!(p.events.first(), Some(Event::Ended(_))), "{:?}", p.events);
    assert!(!p.st.holds_secrets());
}
