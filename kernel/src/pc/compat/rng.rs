//! PC stand-in for the ARM `rng.rs`: the SAME generator (the shared SP 800-90A HMAC_DRBG over SHA-256 in
//! `drbg.rs`, with its boot KAT, SP 800-90B health tests, reseeding and fail-closed `fill_secret`), fed by the
//! PC's entropy:
//!
//! - **strong sources** (counted, health-tested): `RDSEED` (CPUID.7.0:EBX bit 18), then `RDRAND`
//!   (CPUID.1:ECX bit 30);
//! - **uncounted context**: the TSC, its measured frequency and the CMOS clock; plus TSC jitter.
#![allow(dead_code)]

use core::ptr::addr_of_mut;

use crate::drbg::{Generator, Platform, RngError, Status};

static mut GEN: Generator = Generator::new();

/// True when CPUID.1:ECX bit 30 (RDRAND) is set.
pub fn has_rdrand() -> bool {
    #[allow(unused_unsafe)]
    // SAFETY: CPUID leaf 1 exists on every x86-64 CPU.
    let r = unsafe { core::arch::x86_64::__cpuid(1) };
    r.ecx & (1 << 30) != 0
}

/// True when CPUID.(EAX=7,ECX=0):EBX bit 18 (RDSEED) is set.
pub fn has_rdseed() -> bool {
    #[allow(unused_unsafe)]
    // SAFETY: CPUID leaf 0 exists on every x86-64 CPU.
    let max = unsafe { core::arch::x86_64::__cpuid(0) }.eax;
    if max < 7 {
        return false;
    }
    #[allow(unused_unsafe)]
    // SAFETY: leaf 7 exists (checked above).
    let r = unsafe { core::arch::x86_64::__cpuid_count(7, 0) };
    r.ebx & (1 << 18) != 0
}

/// One RDRAND draw (up to 10 retries, as Intel recommends); None if it never succeeds.
pub fn rdrand64() -> Option<u64> {
    if !has_rdrand() {
        return None;
    }
    for _ in 0..10 {
        let v: u64;
        let ok: u8;
        // SAFETY: RDRAND exists (checked above); it writes only its outputs and the flags.
        unsafe {
            core::arch::asm!("rdrand {v}", "setc {ok}", v = out(reg) v, ok = out(reg_byte) ok,
                             options(nomem, nostack));
        }
        if ok != 0 {
            return Some(v);
        }
    }
    None
}

/// One RDSEED draw (RDSEED may legitimately run dry for a moment: up to 100 retries); None if it never succeeds.
pub fn rdseed64() -> Option<u64> {
    if !has_rdseed() {
        return None;
    }
    for _ in 0..100 {
        let v: u64;
        let ok: u8;
        // SAFETY: RDSEED exists (checked above); it writes only its outputs and the flags.
        unsafe {
            core::arch::asm!("rdseed {v}", "setc {ok}", v = out(reg) v, ok = out(reg_byte) ok,
                             options(nomem, nostack));
        }
        if ok != 0 {
            return Some(v);
        }
        core::hint::spin_loop();
    }
    None
}

fn fill_from(buf: &mut [u8], draw: fn() -> Option<u64>) -> usize {
    let mut n = 0;
    while n < buf.len() {
        match draw() {
            Some(v) => {
                let b = v.to_le_bytes();
                let k = (buf.len() - n).min(8);
                buf[n..n + k].copy_from_slice(&b[..k]);
                n += k;
            }
            None => break,
        }
    }
    n
}

struct Pc;

impl Platform for Pc {
    /// The TPM last (F3 step 3): drawn only when RDSEED and RDRAND are absent or have failed.
    const STRONG: &'static [&'static str] = &["rdseed", "rdrand", "tpm"];
    /// 4 bits per byte for all three (F3 step 2a's claim; ADR 0008 1a: the TPM's output passed its own DRBG).
    const H64: &'static [u32] = &[crate::drbg::H64_DEFAULT, crate::drbg::H64_DEFAULT, crate::drbg::H64_DEFAULT];
    const ARCH: &'static [u8] = b"x86_64";

    fn strong(idx: usize, buf: &mut [u8]) -> usize {
        match idx {
            0 => fill_from(buf, rdseed64),
            1 => fill_from(buf, rdrand64),
            2 => crate::tpm::fill(buf),
            _ => 0,
        }
    }

    fn counter() -> u64 {
        crate::timer::rdtsc()
    }

    fn context(out: &mut [u8; 24]) {
        out[..8].copy_from_slice(&crate::timer::rdtsc().to_le_bytes());
        out[8..16].copy_from_slice(&crate::timer::hz().to_le_bytes());
        out[16..24].copy_from_slice(&crate::cmos::epoch().to_le_bytes());
    }
}

fn gen() -> &'static mut Generator {
    // SAFETY: single accessor (the shell's core), as with the old generator.
    unsafe { &mut *addr_of_mut!(GEN) }
}

pub fn next_u32() -> u32 {
    let mut b = [0u8; 4];
    gen().fill::<Pc>(&mut b);
    u32::from_le_bytes(b)
}

pub fn next_u64() -> u64 {
    let mut b = [0u8; 8];
    gen().fill::<Pc>(&mut b);
    u64::from_le_bytes(b)
}

pub fn fill(buf: &mut [u8]) {
    gen().fill::<Pc>(buf);
}

pub fn fill_secret(buf: &mut [u8]) -> Result<(), RngError> {
    gen().fill_secret::<Pc>(buf)
}

pub fn status() -> Status {
    gen().status::<Pc>()
}

pub fn source_names() -> &'static [&'static str] {
    Pc::STRONG
}

// ---- ADR 0008 step 1: diagnostics only. Nothing below is counted as entropy, and drbg.rs is unchanged. ----

fn cpuid(leaf: u32) -> core::arch::x86_64::CpuidResult {
    #[allow(unused_unsafe)]
    // SAFETY: CPUID exists on every x86-64 CPU; a leaf past the maximum returns stale data, so callers check it.
    unsafe { core::arch::x86_64::__cpuid(leaf) }
}

/// CPUID.1:ECX bit 31: a hypervisor says it is there. Then TSC timing is the VM's, not the CPU's (ADR 0008 point 4).
pub fn has_hypervisor() -> bool {
    cpuid(1).ecx & (1 << 31) != 0
}

/// CPUID.80000007:EDX bit 8: the TSC runs at a constant rate in every power state (what a jitter source needs).
pub fn has_invariant_tsc() -> bool {
    cpuid(0x8000_0000).eax >= 0x8000_0007 && cpuid(0x8000_0007).edx & (1 << 8) != 0
}

/// The CPU class, for matching an assessment to a machine: vendor, family, model, stepping (Intel/AMD rules).
fn cpu_class() -> ([u8; 12], u32, u32, u32) {
    let v = cpuid(0);
    let mut vendor = [0u8; 12];
    vendor[..4].copy_from_slice(&v.ebx.to_le_bytes());
    vendor[4..8].copy_from_slice(&v.edx.to_le_bytes());
    vendor[8..].copy_from_slice(&v.ecx.to_le_bytes());
    let a = cpuid(1).eax;
    let (base_fam, base_model) = ((a >> 8) & 0xf, (a >> 4) & 0xf);
    let fam = if base_fam == 0xf { base_fam + ((a >> 20) & 0xff) } else { base_fam };
    let model = if base_fam == 0x6 || base_fam == 0xf { base_model | (((a >> 16) & 0xf) << 4) } else { base_model };
    (vendor, fam, model, a & 0xf)
}

/// The jitter loop ADR 0008 will assess: one sample is the TSC delta around a short walk over a 4 KiB buffer, the
/// number of touches (1 to 8) taken from the TSC's low bits. Step 2 must count samples from THIS function, so the
/// `ea_non_iid` result on `rng jitter dump` output applies to what is counted. Single accessor (the shell's core).
static mut JMEM: [u8; 4096] = [0; 4096];
static mut JIDX: usize = 0;

pub fn jitter_sample() -> u64 {
    let t0 = crate::timer::rdtsc();
    let rounds = (t0 & 7) + 1;
    // SAFETY: single accessor (the shell's core); the pointer stays inside JMEM (index taken modulo its length).
    unsafe {
        let m = addr_of_mut!(JMEM) as *mut u8;
        let mut i = JIDX;
        for _ in 0..rounds {
            let p = m.add(i);
            core::ptr::write_volatile(p, core::ptr::read_volatile(p).wrapping_add(1));
            i = (i + 67) % 4096; // 67: a prime stride, so successive touches land on different cache lines
        }
        JIDX = i;
    }
    crate::timer::rdtsc().wrapping_sub(t0)
}

fn yn(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

/// `rng sources`: what this PC could count, read-only.
pub fn cmd_sources() {
    let (vendor, fam, model, step) = cpu_class();
    crate::kprintln!(
        "rng sources: cpu {} family 0x{:x} model 0x{:x} stepping {}",
        core::str::from_utf8(&vendor).unwrap_or("?"), fam, model, step
    );
    crate::kprintln!(
        "rng sources: rdrand {}  rdseed {}  invariant-tsc {}  hypervisor {}",
        yn(has_rdrand()), yn(has_rdseed()), yn(has_invariant_tsc()), yn(has_hypervisor())
    );
    let t = crate::acpi::tpm_tables();
    match t.tcpa {
        (0, _) => crate::kprintln!("rng sources: acpi TCPA (TPM 1.2) absent"),
        (l, r) => crate::kprintln!("rng sources: acpi TCPA (TPM 1.2) present, length {} rev {}", l, r),
    }
    match t.tpm2 {
        (0, _) => crate::kprintln!("rng sources: acpi TPM2 (TPM 2.0) absent"),
        (l, r) => crate::kprintln!(
            "rng sources: acpi TPM2 (TPM 2.0) present, length {} rev {}, start method {} ({})",
            l, r, t.tpm2_start,
            match t.tpm2_start {
                2 => "ACPI start",
                6 => "memory-mapped FIFO",
                7 => "CRB",
                8 => "CRB with ACPI start",
                _ => "other",
            }
        ),
    }
    crate::tpm::sources_line();
    // F3 step 3: the TPM is counted now, so this line names what is counted on this PC (pc-rngsources grades it).
    let cpu = has_rdseed() || has_rdrand();
    let tpm = crate::tpm::counted();
    crate::kprintln!(
        "rng sources: counted now: {}; jitter is not counted yet (ADR 0008 step 2b)",
        match (cpu, tpm) {
            (true, true) => "rdseed/rdrand, tpm",
            (true, false) => "rdseed/rdrand",
            (false, true) => "tpm",
            (false, false) => "none, so fill_secret refuses on this PC",
        }
    );
}

/// `rng secret <n>` (PC, F3 step 3): one fail-closed draw of n bytes (1 to 64) that prints only ok or refused and
/// the counters, never a byte of the secret (architect's condition 7). The shared bare `rng secret` prints no byte either.
pub fn cmd_secret(n: usize) {
    let mut b = [0u8; 64];
    let r = fill_secret(&mut b[..n]);
    b.fill(0);
    let s = status();
    match r {
        Ok(()) => crate::kprintln!(
            "rng secret: ok, {} bytes, not shown (secret_ok={} secret_refused={} counted bits={})",
            n, s.secret_ok, s.secret_refused, s.last_bits
        ),
        Err(_) => crate::kprintln!(
            "rng secret: refused, no counted source (secret_ok={} secret_refused={} counted bits={})",
            s.secret_ok, s.secret_refused, s.last_bits
        ),
    }
}

/// `rng jitter dump <n>`: n raw jitter_sample() deltas in hex, 16 per line, collected in blocks of 1024 so the
/// serial output never runs inside a sample. scripts/lib/jitter-log.py turns the log into ea_non_iid input.
pub fn cmd_jitter_dump(n: u64) {
    const BLOCK: usize = 1024;
    static mut BUF: [u64; BLOCK] = [0; BLOCK];
    crate::kprintln!(
        "rng jitter dump: n={} loop=mem4096-stride67 block={} tsc_hz={} hypervisor={} (TSC ticks, hex, 16 per line)",
        n, BLOCK, crate::timer::hz(), yn(has_hypervisor())
    );
    if has_hypervisor() {
        crate::kprintln!("rng jitter dump: a hypervisor is present, so these are the VM's timings, not the CPU's");
    }
    let (mut done, mut min, mut max, mut sum) = (0u64, u64::MAX, 0u64, 0u128);
    let mut low = [false; 256];
    let mut h = crate::sha256::Sha256::new();
    // SAFETY: single accessor (the shell's core).
    let buf = unsafe { &mut *addr_of_mut!(BUF) };
    while done < n {
        let k = (n - done).min(BLOCK as u64) as usize;
        for s in buf[..k].iter_mut() {
            *s = jitter_sample();
        }
        for row in buf[..k].chunks(16) {
            crate::kprint!("jd");
            for &d in row {
                crate::kprint!(" {:x}", d);
                h.update(&d.to_le_bytes());
                min = min.min(d);
                max = max.max(d);
                sum += d as u128;
                low[(d & 0xff) as usize] = true;
            }
            crate::kprintln!("");
        }
        done += k as u64;
    }
    let d = h.finish();
    crate::kprint!(
        "rng jitter end: n={} min={:x} max={:x} mean={} distinct-low-bytes={} sha256=",
        n, if n == 0 { 0 } else { min }, max, if n == 0 { 0 } else { (sum / n as u128) as u64 },
        low.iter().filter(|&&b| b).count()
    );
    for b in d {
        crate::kprint!("{:02x}", b);
    }
    crate::kprintln!("");
}
