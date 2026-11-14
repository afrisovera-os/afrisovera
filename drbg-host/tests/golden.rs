//! F3 step 2a: "no behaviour change" as bytes. Two deterministic sources at the default claim (4 bits per byte),
//! a deterministic counter, and a fixed sequence of fills: the output must be byte for byte what the generator
//! gave before per-source entropy existed. This file uses only the API that existed before step 2a, so it builds
//! against the old drbg.rs too; GOLDEN below was printed by that old drbg.rs (master dd31b40a) on the cloud.
use drbg_host::drbg::{Generator, Platform};
use std::sync::atomic::{AtomicU64, Ordering};

static A: AtomicU64 = AtomicU64::new(0x0f1e_2d3c_4b5a_6978);
static B: AtomicU64 = AtomicU64::new(0x8796_a5b4_c3d2_e1f0);
static C: AtomicU64 = AtomicU64::new(1);
static A_CALLS: AtomicU64 = AtomicU64::new(0);

fn step(s: &AtomicU64) -> u64 {
    let mut x = s.load(Ordering::Relaxed);
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    s.store(x, Ordering::Relaxed);
    x
}

struct Fixed;
impl Platform for Fixed {
    const STRONG: &'static [&'static str] = &["a", "b"];
    const ARCH: &'static [u8] = b"golden";
    fn strong(idx: usize, buf: &mut [u8]) -> usize {
        // source a gives at most 40 bytes per call and, after its start-up test, nothing on every 5th call, so
        // some seeds need both sources: the order and the split of the draw are part of what is pinned
        if idx == 0 {
            let k = A_CALLS.fetch_add(1, Ordering::Relaxed);
            if k >= 20 && k % 5 == 4 {
                return 0;
            }
        }
        let (s, max) = if idx == 0 { (&A, 40) } else { (&B, 64) };
        let n = buf.len().min(max);
        for b in buf[..n].iter_mut() {
            *b = (step(s) >> 24) as u8;
        }
        n
    }
    fn counter() -> u64 {
        C.fetch_add(977, Ordering::Relaxed)
    }
    fn context(o: &mut [u8; 24]) {
        o[0] = 0x24;
    }
}

const GOLDEN: &str =
    "last_strong=64 samples=960,1344 b2c49ca6fbc44183e3b77ea4ec8f1e5ca4996c9cfb26bbe2a5b0ec9079509446fc38b6d5b1d9a2d2b016dba9acc2e6c2402722f9a345ab89914dabeb9077f4b5ed33c45763247a34cf181e527a664ec4702b6b5e8f56f49b5f139d58eb60c423391c73110f9665f028b8e3f0d5759a745c4c0d7074f1de8d76e3b1a112713885cd4695cdd572df607b1ed329c0ba0ed5afec39e403e52e008d792904436288aafbeaa6b123c9e8171b9ff9b7c7033627aa27823484ba68c5c34000121a44b1893bb2f936d2725abb49a34f7417def7d09d925ecbeddcc3a8798565c286bab23d228d";

#[test]
fn same_bytes_as_before_per_source_entropy() {
    let mut g = Generator::new();
    let mut out = Vec::new();
    for len in [16usize, 33, 64] {
        let mut b = vec![0u8; len];
        g.fill::<Fixed>(&mut b);
        out.extend_from_slice(&b);
        let mut s = vec![0u8; len];
        g.fill_secret::<Fixed>(&mut s).unwrap();
        out.extend_from_slice(&s);
    }
    let st = g.status::<Fixed>();
    let hex: String = out.iter().map(|b| format!("{:02x}", b)).collect();
    println!("GOLDEN last_strong={} samples={},{} {}", st.last_strong, st.samples[0], st.samples[1], hex);
    assert_eq!(format!("last_strong={} samples={},{} {}", st.last_strong, st.samples[0], st.samples[1], hex), GOLDEN);
}
