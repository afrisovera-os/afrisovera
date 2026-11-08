//! Asili Voice synthesizer v1 — **on-device source-filter speech, no model.**
//!
//! A proper (if still robotic) formant synthesizer built from integer DSP:
//!
//! * **Source:** a glottal pulse train at F0 (~120 Hz) — an impulse every
//!   `SR/F0` samples — provides the voiced excitation (pitch).
//! * **Filter:** two 2-pole **biquad formant resonators** in fixed point, tuned
//!   to each phoneme's F1/F2. Ringing the resonators with the pulse train yields
//!   a vowel-like timbre that actually differs per vowel (a/e/i/o/u are distinct).
//! * **Consonants** are voiced or unvoiced by class: **sonorants** (l r m n w y)
//!   use the voiced resonator path with their own formants; **fricatives**
//!   (s z f v h …) are shaped noise; **plosives** (p t k b d g) are a brief
//!   silence + a short noise burst. So different consonants sound different.
//! * Each segment is **peak-normalized** then shaped by an attack/release
//!   envelope, so nothing clips and edges don't click.
//!
//! Everything is integer math over a 256-entry sine table — no FP, no model, no
//! network. It streams through the virtio-sound driver ([`crate::virtio`]). This
//! is the honest stepping stone below the **Piper** neural model (docs/14), which
//! later replaces this synthesizer to produce natural words over the same path.

use crate::virtio;

const SR: u32 = 44100; // sample rate (matches the virtio-sound stream)
const F0: u32 = 120; // glottal fundamental (voice pitch), Hz
// 3.370 (design-3.367 (B)): this was a WHOLE-UTTERANCE buffer of 48000 samples,
// and because `say()` stopped at its end it was a ~1.09 s CEILING on every
// sentence (3.365). It is now a PER-CHARACTER staging buffer: one character's
// segments are synthesized here, handed to the device, and the buffer is reused.
// It only has to hold the largest thing one character can produce -- a voiced
// segment (MAXSEG) or a plosive's closure + burst (~2425) -- so 8192 is slack,
// the utterance length is unbounded, and .bss drops by 79616 B.
const MAXLEN: usize = 8192; // per-character staging buffer (was the ceiling)
const MAXSEG: usize = 6144; // longest single segment (~139 ms)

// Per-character staging (mono i16) and per-segment scratch (i32, pre-normalization). .bss.
static mut UTTER: [i16; MAXLEN] = [0; MAXLEN];
static mut VSEG: [i32; MAXSEG] = [0; MAXSEG];

// 256-entry full-period sine, amplitude ±10000 (generated).
#[rustfmt::skip]
static SINE: [i16; 256] = [
    0, 245, 491, 736, 980, 1224, 1467, 1710, 1951, 2191, 2430, 2667, 2903, 3137, 3369, 3599,
    3827, 4052, 4276, 4496, 4714, 4929, 5141, 5350, 5556, 5758, 5957, 6152, 6344, 6532, 6716, 6895,
    7071, 7242, 7410, 7572, 7730, 7883, 8032, 8176, 8315, 8449, 8577, 8701, 8819, 8932, 9040, 9142,
    9239, 9330, 9415, 9495, 9569, 9638, 9700, 9757, 9808, 9853, 9892, 9925, 9952, 9973, 9988, 9997,
    10000, 9997, 9988, 9973, 9952, 9925, 9892, 9853, 9808, 9757, 9700, 9638, 9569, 9495, 9415, 9330,
    9239, 9142, 9040, 8932, 8819, 8701, 8577, 8449, 8315, 8176, 8032, 7883, 7730, 7572, 7410, 7242,
    7071, 6895, 6716, 6532, 6344, 6152, 5957, 5758, 5556, 5350, 5141, 4929, 4714, 4496, 4276, 4052,
    3827, 3599, 3369, 3137, 2903, 2667, 2430, 2191, 1951, 1710, 1467, 1224, 980, 736, 491, 245,
    0, -245, -491, -736, -980, -1224, -1467, -1710, -1951, -2191, -2430, -2667, -2903, -3137, -3369, -3599,
    -3827, -4052, -4276, -4496, -4714, -4929, -5141, -5350, -5556, -5758, -5957, -6152, -6344, -6532, -6716, -6895,
    -7071, -7242, -7410, -7572, -7730, -7883, -8032, -8176, -8315, -8449, -8577, -8701, -8819, -8932, -9040, -9142,
    -9239, -9330, -9415, -9495, -9569, -9638, -9700, -9757, -9808, -9853, -9892, -9925, -9952, -9973, -9988, -9997,
    -10000, -9997, -9988, -9973, -9952, -9925, -9892, -9853, -9808, -9757, -9700, -9638, -9569, -9495, -9415, -9330,
    -9239, -9142, -9040, -8932, -8819, -8701, -8577, -8449, -8315, -8176, -8032, -7883, -7730, -7572, -7410, -7242,
    -7071, -6895, -6716, -6532, -6344, -6152, -5957, -5758, -5556, -5350, -5141, -4929, -4714, -4496, -4276, -4052,
    -3827, -3599, -3369, -3137, -2903, -2667, -2430, -2191, -1951, -1710, -1467, -1224, -980, -736, -491, -245,
];

// Resonator pole: r = 0.98. a2 = -r^2 in Q14. a1 depends on the formant freq.
const A2_Q14: i64 = -15735; // -(0.98^2) * 16384

/// The biquad `a1 = 2·r·cos(2π·Fc/SR)` coefficient in Q14, from the sine table.
fn a1_q14(fc: u32) -> i64 {
    let k = ((fc * 256 / SR) & 255) as usize;
    let cosv = SINE[(k + 64) & 255] as i64; // cos·10000
    // a1 = 2 · 0.98 · (cosv/10000); scaled by 16384: (196·16384·cosv)/1_000_000
    (196 * 16384 * cosv) / 1_000_000
}

/// A phoneme class derived from a character.
enum Ph {
    Voiced(u32, u32), // formants F1,F2 (vowels + sonorants) — carries pitch
    Fricative(i32),   // amplitude of shaped noise
    Plosive,          // brief silence + burst
    Silence,          // inter-word gap
    Tone,             // digits/punctuation: a short mid beep
}

/// Map a character to its phoneme class + formants.
fn classify(c: u8) -> Ph {
    let c = c.to_ascii_lowercase();
    match c {
        b'a' => Ph::Voiced(730, 1090),
        b'e' => Ph::Voiced(530, 1840),
        b'i' | b'y' => Ph::Voiced(270, 2290),
        b'o' => Ph::Voiced(570, 840),
        b'u' => Ph::Voiced(300, 870),
        // sonorants (voiced consonants) — their own formants
        b'l' => Ph::Voiced(360, 1300),
        b'r' => Ph::Voiced(490, 1350),
        b'm' => Ph::Voiced(250, 1000), // nasal: low formants
        b'n' => Ph::Voiced(250, 1700),
        b'w' => Ph::Voiced(300, 610),
        // fricatives (unvoiced noise); s/sh brighter, so louder high content
        b's' | b'z' | b'c' | b'x' => Ph::Fricative(7000),
        b'f' | b'v' | b'h' | b'j' => Ph::Fricative(5000),
        // plosives (stop consonants)
        b'p' | b't' | b'k' | b'b' | b'd' | b'g' | b'q' => Ph::Plosive,
        b' ' | b'\r' | b'\n' | b'\t' => Ph::Silence,
        _ => Ph::Tone,
    }
}

/// Linear attack/release envelope in 0..=1000, to avoid clicks at edges.
fn envelope(p: usize, len: usize, ramp: usize) -> i64 {
    let r = ramp.min(len / 2).max(1);
    let up = if p < r { p * 1000 / r } else { 1000 };
    let down = if p >= len - r { (len - 1 - p) * 1000 / r } else { 1000 };
    up.min(down) as i64
}

/// Synthesize a voiced segment, **gliding** the formants from `(f1s,f2s)` at the
/// start to `(f1e,f2e)` at the end — coarticulation, so adjacent phonemes flow
/// into each other instead of stepping. Peak-normalizes and appends to UTTER.
unsafe fn voiced(n: usize, f1s: u32, f2s: u32, f1e: u32, f2e: u32, len: usize) -> usize {
    let seg = len.min(MAXSEG).min(MAXLEN - n);
    if seg == 0 {
        return n;
    }
    let (mut ya1, mut ya2) = (0i64, 0i64); // resonator A (F1) state
    let (mut yb1, mut yb2) = (0i64, 0i64); // resonator B (F2) state
    let t0 = (SR / F0) as usize;
    let mut since = t0; // fire a pulse on the first sample
    let mut maxa: i64 = 1;
    let span = if seg > 1 { seg - 1 } else { 1 };
    for p in 0..seg {
        // linearly interpolate each formant across the segment
        let f1 = (f1s as i64 + (f1e as i64 - f1s as i64) * p as i64 / span as i64) as u32;
        let f2 = (f2s as i64 + (f2e as i64 - f2s as i64) * p as i64 / span as i64) as u32;
        let a1a = a1_q14(f1);
        let a1b = a1_q14(f2);
        let x: i64 = if since >= t0 {
            since = 0;
            10000
        } else {
            0
        };
        since += 1;
        let oa = x + ((a1a * ya1 + A2_Q14 * ya2) >> 14);
        ya2 = ya1;
        ya1 = oa;
        let ob = x + ((a1b * yb1 + A2_Q14 * yb2) >> 14);
        yb2 = yb1;
        yb1 = ob;
        let s = oa + ob;
        VSEG[p] = s as i32;
        let a = if s < 0 { -s } else { s };
        if a > maxa {
            maxa = a;
        }
    }
    for p in 0..seg {
        let v = (VSEG[p] as i64) * 9000 / maxa; // normalize to ±9000
        UTTER[n + p] = ((v * envelope(p, seg, 300)) / 1000) as i16;
    }
    n + seg
}

/// Append `len` samples of shaped noise (fricative) at amplitude `amp`.
unsafe fn noise(n: usize, len: usize, amp: i32, lcg: &mut u32) -> usize {
    let seg = len.min(MAXLEN - n);
    for p in 0..seg {
        *lcg = lcg.wrapping_mul(1103515245).wrapping_add(12345);
        let nz = ((*lcg >> 16) as i32 & 0x7fff) - 16384; // ±16384
        let v = (nz as i64) * (amp as i64) / 16384; // scale to ±amp
        UTTER[n + p] = ((v * envelope(p, seg, 120)) / 1000) as i16;
    }
    n + seg
}

/// Append `len` samples of silence.
unsafe fn silence(n: usize, len: usize) -> usize {
    let seg = len.min(MAXLEN - n);
    for p in 0..seg {
        UTTER[n + p] = 0;
    }
    n + seg
}

/// Append a short mid tone (for digits/punctuation).
unsafe fn tone(n: usize, len: usize) -> usize {
    let seg = len.min(MAXLEN - n);
    let mut ph = 0u32;
    let incr = (500u64 * 256 * 65536 / SR as u64) as u32;
    for p in 0..seg {
        let idx = ((ph >> 16) & 255) as usize;
        ph = ph.wrapping_add(incr);
        UTTER[n + p] = (((SINE[idx] as i64) * envelope(p, seg, 200)) / 1000) as i16;
    }
    n + seg
}

/// Synthesize `text` to PCM and play it through the sound device. Returns false
/// if there is no audio device or nothing to say.
pub fn say(text: &[u8]) -> bool {
    if !virtio::sound_present() {
        return false;
    }
    let ms = |m: u32| (SR * m / 1000) as usize;
    let mut n = 0usize;
    let mut lcg = 0x1234_5678u32;
    // Previous voiced phoneme's formants, for coarticulation glides.
    let mut pf1 = 0u32;
    let mut pf2 = 0u32;
    let mut have_prev = false;

    // SAFETY: single-threaded kernel context; UTTER/VSEG only touched here.
    unsafe {
        // 3.370: MEASURED, not assumed -- an early `return false` here cost the
        // `[asili voice] synthesized ...` line on any boot where the device is
        // present but the stream will not start, and scripts/smoke-test.sh is
        // exactly such a boot (it red-ed on the first run of this rewrite). The
        // pre-rewrite code synthesized and PRINTED first and only then found out
        // playback was impossible; keep that order -- synthesis is unconditional,
        // only the submission is gated.
        let live = virtio::sound_stream_begin();
        let mut said = 0usize; // characters actually handed to the device
        let mut total = 0usize; // samples synthesized across the whole utterance
        let mut syn_cycles = 0u64; // SYNTHESIS cycles only -- submission excluded
        let mut aborted = false;
        for &c in text {
            // 3.370: one character at a time. `n` restarts at 0 for every
            // character and the staging buffer is drained below, so there is no
            // whole-utterance buffer left to run out of -- the 3.365 ceiling is
            // gone by construction, not by a larger constant.
            n = 0;
            let t0 = crate::timer::cycles();
            match classify(c) {
                Ph::Voiced(f1, f2) => {
                    // glide from the previous voiced formants into this phoneme
                    let (s1, s2) = if have_prev { (pf1, pf2) } else { (f1, f2) };
                    n = voiced(n, s1, s2, f1, f2, ms(140));
                    pf1 = f1;
                    pf2 = f2;
                    have_prev = true;
                }
                Ph::Fricative(amp) => n = noise(n, ms(100), amp, &mut lcg),
                Ph::Plosive => {
                    n = silence(n, ms(25)); // the stop closure
                    n = noise(n, ms(30), 6000, &mut lcg); // the release burst
                }
                Ph::Silence => n = silence(n, ms(90)),
                Ph::Tone => n = tone(n, ms(70)),
            }
            // 3.367/3.368: RTF observability -- the pre-registered discriminator
            // that JUSTIFIED this rewrite (0.104 measured). It now accumulates
            // per character so submission time cannot leak into it.
            syn_cycles = syn_cycles.wrapping_add(crate::timer::cycles().wrapping_sub(t0));
            if live && n > 0 && !virtio::sound_stream_write(&UTTER[..n]) {
                aborted = true;
                break;
            }
            said += 1;
            total += n;
        }
        if live && !virtio::sound_stream_end() {
            aborted = true;
        }
        let t_syn = syn_cycles;
        let n = total;
        if n == 0 {
            return false;
        }
        crate::kprintln!(
            "[asili voice] synthesized {} ms for {} chars (source-filter v2, coarticulated)",
            n * 1000 / SR as usize,
            text.len()
        );
        // Real-time factor: wall time spent in the synthesis loop divided by the
        // duration of audio it produced. Reported in per-mille to stay integer.
        // <= 300 justifies streaming period-by-period (design-3.367 (B));
        // >= 800 falsifies it before it is written. Observability only.
        {
            let hz = crate::timer::hz().max(1);
            let syn_us = t_syn.saturating_mul(1_000_000) / hz;
            let audio_us = (n as u64) * 1_000_000 / SR as u64;
            let rtf_ppt = if audio_us > 0 { syn_us * 1000 / audio_us } else { 0 };
            crate::kprintln!(
                "[asili voice] RTF: synthesis {} us for {} us of audio = {}/1000 real-time",
                syn_us,
                audio_us,
                rtf_ppt
            );
        }
        if said < text.len() {
            // 3.366 kept, with its CAUSE replaced: the buffer ceiling that used
            // to trip this is gone (3.370), so the only way to lose a tail now
            // is the device refusing a period mid-stream (the 3.344 abort class,
            // which prints its own line from virtio). Silent when the whole text
            // is spoken -- so on a healthy boot this marker is ABSENT, which is
            // exactly what scripts/longspeak-probe-3369.sh grades.
            crate::kprintln!(
                "[asili voice] TRUNCATED: said {} of {} chars, dropped {}; playback aborted after {} samples ({} ms)",
                said,
                text.len(),
                text.len() - said,
                n,
                n * 1000 / SR as usize
            );
        }
        live && !aborted
    }
}
