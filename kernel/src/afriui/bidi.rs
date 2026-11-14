//! Bidirectional text for one line (Unicode UAX 9), phase C1 of SHAPING_PLAN.md: the paragraph level (P2, P3), the
//! weak types (W1 to W7), bracket pairs (N0), the neutrals (N1, N2), the implicit levels (I1, I2), the line's
//! trailing white space (L1) and the visual order (L2). Mirrored glyphs (L4) are the shaper's: `mirror`.
//!
//! The explicit embedding and isolate controls (U+202A..U+202E, U+2066..U+2069) are not handled here: the shaper
//! drops them before this runs, as the console does (security F4, the architect's condition), so a string cannot
//! reorder the text around it. Boundary-neutral characters (class BN, X9) are skipped and take the level of the
//! character before them. One paragraph: a paragraph separator is treated as the end of the line.
//!
//! No heap, no statics: everything is in the caller's slices, at most MAX characters. Every table read is a
//! bounded search that cannot fail to give an answer. The host test (fuzz-host, tests/bidi.rs) runs it against
//! Unicode's BidiCharacterTest.txt.
#![allow(dead_code)]

use super::shape_data as sd;

/// The longest line, in characters (the shaper cuts and counts the rest).
pub const MAX: usize = 256;

// the classes, in shape_data's order
pub const L: u8 = 0;
pub const R: u8 = 1;
pub const AL: u8 = 2;
pub const EN: u8 = 3;
pub const ES: u8 = 4;
pub const ET: u8 = 5;
pub const AN: u8 = 6;
pub const CS: u8 = 7;
pub const NSM: u8 = 8;
pub const BN: u8 = 9;
pub const B: u8 = 10;
pub const S: u8 = 11;
pub const WS: u8 = 12;
pub const ON: u8 = 13;
pub const LRE: u8 = 14;
pub const PDI: u8 = 22;

/// A level the caller marks as removed (BN), shown as "x" in Unicode's test file.
pub const REMOVED: u8 = 0xFF;

/// The bidi class of a code point (a binary search of a range table that starts at 0, so it always answers).
pub fn class(cp: u32) -> u8 {
    let i = sd::BIDI.partition_point(|&(s, _)| s <= cp);
    match i.checked_sub(1).and_then(|i| sd::BIDI.get(i)) {
        Some(&(_, c)) => c,
        None => L,
    }
}

/// An explicit embedding, override or isolate control (dropped by the shaper).
pub fn is_explicit(cp: u32) -> bool {
    (0x202A..=0x202E).contains(&cp) || (0x2066..=0x2069).contains(&cp)
}

/// The mirrored form of a code point drawn right to left (L4), if it has one.
pub fn mirror(cp: u32) -> Option<u32> {
    sd::MIRROR.binary_search_by_key(&cp, |&(a, _)| a).ok().and_then(|i| sd::MIRROR.get(i)).map(|&(_, m)| m)
}

/// (pair, is_open) for a paired bracket, with the canonical equivalents U+2329/U+232A read as U+3008/U+3009.
fn bracket(cp: u32) -> Option<(u32, bool)> {
    let cp = match cp {
        0x2329 => 0x3008,
        0x232A => 0x3009,
        c => c,
    };
    let i = sd::BRACKET.binary_search_by_key(&cp, |&(a, _, _)| a).ok()?;
    sd::BRACKET.get(i).map(|&(_, p, t)| (p, t == 0))
}

fn canon(cp: u32) -> u32 {
    match cp {
        0x2329 => 0x3008,
        0x232A => 0x3009,
        c => c,
    }
}

/// Strong direction for N0 and N1: L stays L, R, AL, EN and AN count as R; anything else is neutral (None).
fn strong(t: u8) -> Option<u8> {
    match t {
        L => Some(L),
        R | AL | EN | AN => Some(R),
        _ => None,
    }
}

fn is_ni(t: u8) -> bool {
    matches!(t, B | S | WS | ON) || t >= 19
}

/// Resolve the levels of `cps` (at most MAX; the rest is ignored). `rtl`: the paragraph direction if the caller
/// knows it, else P2 and P3 (the first strong character; left to right when there is none). Writes one level per
/// character into `levels` (REMOVED for BN) and returns the paragraph level.
pub fn resolve(cps: &[u32], rtl: Option<bool>, levels: &mut [u8]) -> u8 {
    let n = cps.len().min(MAX).min(levels.len());
    let cps = &cps[..n];
    let mut orig = [0u8; MAX];
    for (o, &c) in orig.iter_mut().zip(cps) {
        *o = class(c);
    }
    // P2, P3
    let para = match rtl {
        Some(r) => r as u8,
        None => orig[..n]
            .iter()
            .find_map(|&t| match t {
                L => Some(0),
                R | AL => Some(1),
                _ => None,
            })
            .unwrap_or(0),
    };
    let e_dir = if para & 1 == 1 { R } else { L };
    // X9: the characters that take part (BN skipped; explicit controls were dropped by the caller, and any left
    // are skipped the same way)
    let mut idx = [0u16; MAX];
    let mut m = 0;
    for k in 0..n {
        if orig[k] != BN && !(LRE..=PDI).contains(&orig[k]) {
            idx[m] = k as u16;
            m += 1;
        }
    }
    let idx = &idx[..m];
    let mut t = [0u8; MAX];
    for (j, &k) in idx.iter().enumerate() {
        t[j] = orig[k as usize];
    }
    let t = &mut t[..m];
    let sos = e_dir;
    // W1
    let mut prev = sos;
    for x in t.iter_mut() {
        if *x == NSM {
            *x = prev;
        }
        prev = *x;
    }
    // W2
    let mut last = sos;
    for x in t.iter_mut() {
        match *x {
            L | R | AL => last = *x,
            EN if last == AL => *x = AN,
            _ => {}
        }
    }
    // W3
    for x in t.iter_mut() {
        if *x == AL {
            *x = R;
        }
    }
    // W4
    for j in 1..m.saturating_sub(1) {
        let (a, c) = (t[j - 1], t[j + 1]);
        if t[j] == ES && a == EN && c == EN {
            t[j] = EN;
        } else if t[j] == CS && a == EN && c == EN {
            t[j] = EN;
        } else if t[j] == CS && a == AN && c == AN {
            t[j] = AN;
        }
    }
    // W5
    let mut j = 0;
    while j < m {
        if t[j] == ET {
            let s = j;
            while j < m && t[j] == ET {
                j += 1;
            }
            let before = s > 0 && t[s - 1] == EN;
            let after = j < m && t[j] == EN;
            if before || after {
                for x in &mut t[s..j] {
                    *x = EN;
                }
            }
        } else {
            j += 1;
        }
    }
    // W6
    for x in t.iter_mut() {
        if matches!(*x, ES | ET | CS) {
            *x = ON;
        }
    }
    // W7
    let mut last = sos;
    for x in t.iter_mut() {
        match *x {
            L | R => last = *x,
            EN if last == L => *x = L,
            _ => {}
        }
    }
    // N0: bracket pairs (BD16, a stack of 63), then each pair in the order of its opening bracket
    let mut pairs = [(0u16, 0u16); MAX / 2];
    let mut np = 0;
    {
        let mut stack = [(0u32, 0u16); 63];
        let mut sp = 0;
        'scan: for j in 0..m {
            if t[j] != ON {
                continue;
            }
            let cp = cps[idx[j] as usize];
            match bracket(cp) {
                Some((pair, true)) => {
                    if sp == stack.len() {
                        break 'scan;
                    }
                    stack[sp] = (canon(pair), j as u16);
                    sp += 1;
                }
                Some((_, false)) => {
                    let me = canon(cp);
                    let mut q = sp;
                    while q > 0 {
                        q -= 1;
                        if stack[q].0 == me {
                            if np < pairs.len() {
                                pairs[np] = (stack[q].1, j as u16);
                                np += 1;
                            }
                            sp = q;
                            break;
                        }
                    }
                }
                None => {}
            }
        }
    }
    let pairs = &mut pairs[..np];
    pairs.sort_unstable_by_key(|p| p.0);
    for &(o, c) in pairs.iter() {
        let (o, c) = (o as usize, c as usize);
        let mut found_e = false;
        let mut found_other = false;
        for &x in &t[o + 1..c] {
            match strong(x) {
                Some(d) if d == e_dir => found_e = true,
                Some(_) => found_other = true,
                None => {}
            }
        }
        let set = if found_e {
            Some(e_dir)
        } else if found_other {
            let mut ctx = sos;
            for &x in t[..o].iter().rev() {
                if let Some(d) = strong(x) {
                    ctx = d;
                    break;
                }
            }
            Some(if ctx != e_dir { ctx } else { e_dir })
        } else {
            None
        };
        if let Some(d) = set {
            t[o] = d;
            t[c] = d;
            // NSMs (original class) right after a changed bracket take its direction
            for b in [o, c] {
                let mut q = b + 1;
                while q < m && orig[idx[q] as usize] == NSM {
                    t[q] = d;
                    q += 1;
                }
            }
        }
    }
    // N1, N2
    let mut j = 0;
    while j < m {
        if is_ni(t[j]) {
            let s = j;
            while j < m && is_ni(t[j]) {
                j += 1;
            }
            let before = if s == 0 { sos } else { strong(t[s - 1]).unwrap_or(e_dir) };
            let after = if j == m { e_dir } else { strong(t[j]).unwrap_or(e_dir) };
            let d = if before == after { before } else { e_dir };
            for x in &mut t[s..j] {
                *x = d;
            }
        } else {
            j += 1;
        }
    }
    // I1, I2
    for k in 0..n {
        levels[k] = REMOVED;
    }
    for (j, &k) in idx.iter().enumerate() {
        let lv = if para & 1 == 0 {
            match t[j] {
                R => para + 1,
                AN | EN => para + 2,
                _ => para,
            }
        } else {
            match t[j] {
                L | EN | AN => para + 1,
                _ => para,
            }
        };
        levels[k as usize] = lv;
    }
    // L1: segment and paragraph separators, and the white space before them and at the line's end, to the
    // paragraph level (BN within such a run too)
    let mut tail = true;
    for k in (0..n).rev() {
        match orig[k] {
            S | B => {
                levels[k] = para;
                tail = true;
            }
            WS | BN => {
                if tail && orig[k] == WS {
                    levels[k] = para;
                }
            }
            c if c >= 19 => {
                if tail {
                    levels[k] = para;
                }
            }
            _ => tail = false,
        }
    }
    para
}

/// The visual order (L2): `order` gets the indices of the characters that are not REMOVED, left to right. Returns
/// how many.
pub fn reorder(levels: &[u8], order: &mut [u16]) -> usize {
    let mut m = 0;
    let (mut hi, mut lo_odd) = (0u8, u8::MAX);
    for (k, &lv) in levels.iter().enumerate() {
        if lv == REMOVED || m >= order.len() {
            continue;
        }
        order[m] = k as u16;
        m += 1;
        hi = hi.max(lv);
        if lv & 1 == 1 {
            lo_odd = lo_odd.min(lv);
        }
    }
    if lo_odd == u8::MAX {
        return m;
    }
    let mut lv = hi;
    while lv >= lo_odd {
        let mut j = 0;
        while j < m {
            if levels[order[j] as usize] >= lv {
                let s = j;
                while j < m && levels[order[j] as usize] >= lv {
                    j += 1;
                }
                order[s..j].reverse();
            } else {
                j += 1;
            }
        }
        if lv == 0 {
            break;
        }
        lv -= 1;
    }
    m
}
