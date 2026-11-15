//! Bidirectional text (Unicode UAX 9), SHAPING_PLAN.md C1 and C2b: paragraphs (P1 to P3), the explicit embeddings,
//! overrides and isolates (X1 to X10, a directional status stack of depth 125, isolating run sequences), the weak
//! types (W1 to W7), bracket pairs (N0), the neutrals (N1, N2), the implicit levels (I1, I2), the line's trailing
//! white space (L1) and the visual order (L2, one paragraph at a time). Mirrored glyphs (L4) are the shaper's:
//! `mirror`.
//!
//! What reaches this is the caller's choice (the architect's rule, C2b): the shaper drops the embeddings and
//! overrides (U+202A..U+202E) from text that names something a person must recognise (window titles, file names,
//! device names: `Text::Identity`), so an override cannot make "gpj.exe" read "exe.jpg"; isolates stay, since they
//! cannot reorder what is around them. Prose keeps everything. Either way it works inside one call only: a call is
//! its own text, so nothing reorders across the desk's own parts.
//!
//! No heap, no statics: everything is in the caller's slices or on the stack, at most MAX characters. Every table
//! read is a bounded search. The host test (fuzz-host, tests/bidi.rs) runs all of Unicode's BidiCharacterTest.txt.
#![allow(dead_code)]

use super::shape_data as sd;

/// The longest line, in characters (the shaper cuts and counts the rest).
pub const MAX: usize = 256;
/// The deepest embedding level (UAX 9 BD2).
pub const MAX_DEPTH: u8 = 125;

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
pub const LRO: u8 = 15;
pub const RLE: u8 = 16;
pub const RLO: u8 = 17;
pub const PDF: u8 = 18;
pub const LRI: u8 = 19;
pub const RLI: u8 = 20;
pub const FSI: u8 = 21;
pub const PDI: u8 = 22;

/// A level the caller marks as removed (X9: the embeddings, overrides, PDF and BN), "x" in Unicode's test file.
pub const REMOVED: u8 = 0xFF;

/// The bidi class of a code point (a binary search of a range table that starts at 0, so it always answers).
pub fn class(cp: u32) -> u8 {
    let i = sd::BIDI.partition_point(|&(s, _)| s <= cp);
    match i.checked_sub(1).and_then(|i| sd::BIDI.get(i)) {
        Some(&(_, c)) => c,
        None => L,
    }
}

/// An explicit embedding, override or isolate control.
pub fn is_explicit(cp: u32) -> bool {
    (0x202A..=0x202E).contains(&cp) || (0x2066..=0x2069).contains(&cp)
}

/// An embedding or override control or PDF (U+202A..U+202E): what `Text::Identity` drops.
pub fn is_embedding(cp: u32) -> bool {
    (0x202A..=0x202E).contains(&cp)
}

/// The mirrored form of a code point drawn right to left (L4), if it has one.
pub fn mirror(cp: u32) -> Option<u32> {
    sd::MIRROR.binary_search_by_key(&cp, |&(a, _)| a).ok().and_then(|i| sd::MIRROR.get(i)).map(|&(_, m)| m)
}

/// (pair, is_open) for a paired bracket, with the canonical equivalents U+2329/U+232A read as U+3008/U+3009.
fn bracket(cp: u32) -> Option<(u32, bool)> {
    let cp = canon(cp);
    let i = sd::BRACKET.binary_search_by_key(&cp, |&(a, _, _)| a).ok()?;
    sd::BRACKET.get(i).map(|&(_, p, t)| (canon(p), t == 0))
}

fn canon(cp: u32) -> u32 {
    match cp {
        0x2329 => 0x3008,
        0x232A => 0x3009,
        c => c,
    }
}

fn dir_of(level: u8) -> u8 {
    if level & 1 == 1 {
        R
    } else {
        L
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

fn is_isolate_init(t: u8) -> bool {
    matches!(t, LRI | RLI | FSI)
}

fn is_ni(t: u8) -> bool {
    matches!(t, B | S | WS | ON | LRI | RLI | FSI | PDI)
}

fn removed_by_x9(t: u8) -> bool {
    matches!(t, LRE | RLE | LRO | RLO | PDF | BN)
}

/// P2 and P3 over orig[a..b]: the first strong character, skipping isolated text (from an isolate initiator to its
/// matching PDI, or to the end). None when there is no strong character.
fn first_strong(orig: &[u8], a: usize, b: usize, matching: &[u16]) -> Option<u8> {
    let mut k = a;
    while k < b {
        match orig[k] {
            L => return Some(0),
            R | AL => return Some(1),
            LRI | RLI | FSI => {
                let m = matching[k] as usize;
                if m >= b || m == NO_MATCH as usize {
                    return None;
                }
                k = m;
            }
            _ => {}
        }
        k += 1;
    }
    None
}

const NO_MATCH: u16 = u16::MAX;

/// Resolve the levels of `cps` (at most MAX; the rest is ignored). Paragraphs end after each paragraph separator
/// (class B) and are resolved one by one. `rtl`: the direction of every paragraph if the caller knows it, else P2 and
/// P3 for each (left to right when there is no strong character). Writes one level per character into `levels`
/// (REMOVED for what X9 removes) and returns the first paragraph's level.
pub fn resolve(cps: &[u32], rtl: Option<bool>, levels: &mut [u8]) -> u8 {
    let n = cps.len().min(MAX).min(levels.len());
    let cps = &cps[..n];
    let mut orig = [0u8; MAX];
    for (o, &c) in orig.iter_mut().zip(cps) {
        *o = class(c);
    }
    let mut first = None;
    let mut a = 0;
    while a < n {
        let mut b = a;
        while b < n && orig[b] != B {
            b += 1;
        }
        let end = (b + 1).min(n); // the separator belongs to the paragraph it ends
        let p = paragraph(cps, &orig, a, end, rtl, levels);
        first.get_or_insert(p);
        a = end;
    }
    first.unwrap_or(match rtl {
        Some(true) => 1,
        _ => 0,
    })
}

/// One paragraph, cps[a..b].
fn paragraph(cps: &[u32], orig: &[u8; MAX], a: usize, b: usize, rtl: Option<bool>, levels: &mut [u8]) -> u8 {
    // BD9: the matching PDI of every isolate initiator
    let mut matching = [NO_MATCH; MAX];
    {
        let mut stack = [0u16; MAX];
        let mut sp = 0;
        for k in a..b {
            match orig[k] {
                LRI | RLI | FSI => {
                    if sp < stack.len() {
                        stack[sp] = k as u16;
                        sp += 1;
                    }
                }
                PDI if sp > 0 => {
                    sp -= 1;
                    matching[stack[sp] as usize] = k as u16;
                }
                _ => {}
            }
        }
    }
    let para: u8 = match rtl {
        Some(r) => r as u8,
        None => first_strong(orig, a, b, &matching).unwrap_or(0),
    };
    // X1 to X8: the levels and the types after overrides
    let mut t = [0u8; MAX];
    {
        // (level, override: 0 none, L or R + 1, isolate)
        let mut stack = [(0u8, 0u8, false); MAX_DEPTH as usize + 2];
        let mut sp = 1;
        stack[0] = (para, 0, false);
        let (mut over_iso, mut over_emb, mut valid_iso) = (0u32, 0u32, 0u32);
        for k in a..b {
            let c = orig[k];
            let (lv, ov, _) = stack[sp - 1];
            let apply_override = |c: u8| match ov {
                1 => L,
                2 => R,
                _ => c,
            };
            match c {
                RLE | LRE | RLO | LRO => {
                    let odd = matches!(c, RLE | RLO);
                    let nl = if odd { (lv + 1) | 1 } else { (lv + 2) & !1 };
                    if nl <= MAX_DEPTH && over_iso == 0 && over_emb == 0 {
                        let o = match c {
                            RLO => 2,
                            LRO => 1,
                            _ => 0,
                        };
                        stack[sp] = (nl, o, false);
                        sp += 1;
                    } else if over_iso == 0 {
                        over_emb += 1;
                    }
                    levels[k] = lv;
                    t[k] = c;
                }
                RLI | LRI | FSI => {
                    levels[k] = lv;
                    t[k] = apply_override(c);
                    let odd = match c {
                        RLI => true,
                        LRI => false,
                        _ => {
                            let m = matching[k];
                            let e = if m == NO_MATCH { b } else { m as usize };
                            first_strong(orig, k + 1, e, &matching) == Some(1)
                        }
                    };
                    let nl = if odd { (lv + 1) | 1 } else { (lv + 2) & !1 };
                    if nl <= MAX_DEPTH && over_iso == 0 && over_emb == 0 {
                        valid_iso += 1;
                        stack[sp] = (nl, 0, true);
                        sp += 1;
                    } else {
                        over_iso += 1;
                    }
                }
                PDI => {
                    if over_iso > 0 {
                        over_iso -= 1;
                    } else if valid_iso > 0 {
                        over_emb = 0;
                        while sp > 1 && !stack[sp - 1].2 {
                            sp -= 1;
                        }
                        if sp > 1 {
                            sp -= 1;
                        }
                        valid_iso -= 1;
                    }
                    let (lv2, ov2, _) = stack[sp - 1];
                    levels[k] = lv2;
                    t[k] = match ov2 {
                        1 => L,
                        2 => R,
                        _ => c,
                    };
                }
                PDF => {
                    if over_iso > 0 {
                    } else if over_emb > 0 {
                        over_emb -= 1;
                    } else if !stack[sp - 1].2 && sp >= 2 {
                        sp -= 1;
                    }
                    levels[k] = lv;
                    t[k] = c;
                }
                B => {
                    levels[k] = para;
                    t[k] = c;
                }
                BN => {
                    levels[k] = lv;
                    t[k] = c;
                }
                _ => {
                    levels[k] = lv;
                    t[k] = apply_override(c);
                }
            }
        }
    }
    // X9: the removed characters
    for k in a..b {
        if removed_by_x9(orig[k]) {
            levels[k] = REMOVED;
        }
    }
    // X10: level runs, chained into isolating run sequences
    let mut seq_done = [false; MAX];
    let mut seq = [0u16; MAX];
    for k0 in a..b {
        if levels[k0] == REMOVED || seq_done[k0] {
            continue;
        }
        // a sequence starts at a level run that does not start with a PDI matching an initiator
        let starts_run = (a..k0).rev().find(|&j| levels[j] != REMOVED).map_or(true, |j| levels[j] != levels[k0]);
        if !starts_run {
            continue;
        }
        if orig[k0] == PDI && (a..k0).any(|j| matching[j] as usize == k0) {
            continue; // reached from its initiator's run
        }
        // gather the runs
        let mut m = 0;
        let mut k = k0;
        loop {
            let lv = levels[k];
            let mut last = k;
            let mut j = k;
            while j < b {
                if levels[j] == REMOVED {
                    j += 1;
                    continue;
                }
                if levels[j] != lv {
                    break;
                }
                if m < seq.len() {
                    seq[m] = j as u16;
                    m += 1;
                }
                seq_done[j] = true;
                last = j;
                j += 1;
            }
            if is_isolate_init(orig[last]) && matching[last] != NO_MATCH {
                k = matching[last] as usize;
                continue;
            }
            break;
        }
        let s = &seq[..m];
        let lv = levels[s[0] as usize];
        let first = s[0] as usize;
        let lastc = s[m - 1] as usize;
        let before = (a..first).rev().find(|&j| levels[j] != REMOVED).map_or(para, |j| levels[j]);
        let after = if is_isolate_init(orig[lastc]) {
            para
        } else {
            (lastc + 1..b).find(|&j| levels[j] != REMOVED).map_or(para, |j| levels[j])
        };
        let sos = dir_of(lv.max(before));
        let eos = dir_of(lv.max(after));
        resolve_sequence(cps, orig, &mut t, s, lv, sos, eos);
    }
    // I1, I2
    for k in a..b {
        if levels[k] == REMOVED {
            continue;
        }
        let lv = levels[k];
        levels[k] = if lv & 1 == 0 {
            match t[k] {
                R => lv + 1,
                AN | EN => lv + 2,
                _ => lv,
            }
        } else {
            match t[k] {
                L | EN | AN => lv + 1,
                _ => lv,
            }
        };
    }
    // L1: segment and paragraph separators, and the white space and isolate controls before them and at the end,
    // to the paragraph level (removed characters within such a run are left removed)
    let mut tail = true;
    for k in (a..b).rev() {
        match orig[k] {
            S | B => {
                levels[k] = para;
                tail = true;
            }
            WS | LRI | RLI | FSI | PDI => {
                if tail {
                    levels[k] = para;
                }
            }
            c if removed_by_x9(c) => {}
            _ => tail = false,
        }
    }
    para
}

/// W1 to W7, N0, N1 and N2 over one isolating run sequence (indices into the paragraph, in order).
fn resolve_sequence(cps: &[u32], orig: &[u8; MAX], t: &mut [u8; MAX], s: &[u16], lv: u8, sos: u8, eos: u8) {
    let m = s.len();
    let at = |j: usize| s[j] as usize;
    let e_dir = dir_of(lv);
    // W1
    let mut prev = sos;
    for j in 0..m {
        let k = at(j);
        if t[k] == NSM {
            t[k] = if is_isolate_init(prev) || prev == PDI { ON } else { prev };
        }
        prev = t[k];
    }
    // W2
    let mut last = sos;
    for j in 0..m {
        let k = at(j);
        match t[k] {
            L | R | AL => last = t[k],
            EN if last == AL => t[k] = AN,
            _ => {}
        }
    }
    // W3
    for j in 0..m {
        let k = at(j);
        if t[k] == AL {
            t[k] = R;
        }
    }
    // W4
    for j in 1..m.saturating_sub(1) {
        let (p, k, q) = (at(j - 1), at(j), at(j + 1));
        let (a, c) = (t[p], t[q]);
        if (t[k] == ES || t[k] == CS) && a == EN && c == EN {
            t[k] = EN;
        } else if t[k] == CS && a == AN && c == AN {
            t[k] = AN;
        }
    }
    // W5
    let mut j = 0;
    while j < m {
        if t[at(j)] == ET {
            let st = j;
            while j < m && t[at(j)] == ET {
                j += 1;
            }
            let near = (st > 0 && t[at(st - 1)] == EN) || (j < m && t[at(j)] == EN);
            if near {
                for q in st..j {
                    t[at(q)] = EN;
                }
            }
        } else {
            j += 1;
        }
    }
    // W6
    for j in 0..m {
        let k = at(j);
        if matches!(t[k], ES | ET | CS) {
            t[k] = ON;
        }
    }
    // W7
    let mut last = sos;
    for j in 0..m {
        let k = at(j);
        match t[k] {
            L | R => last = t[k],
            EN if last == L => t[k] = L,
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
            let k = at(j);
            if t[k] != ON {
                continue;
            }
            match bracket(cps[k]) {
                Some((pair, true)) => {
                    if sp == stack.len() {
                        break 'scan;
                    }
                    stack[sp] = (pair, j as u16);
                    sp += 1;
                }
                Some((_, false)) => {
                    let me = canon(cps[k]);
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
        let (mut found_e, mut found_other) = (false, false);
        for q in o + 1..c {
            match strong(t[at(q)]) {
                Some(d) if d == e_dir => found_e = true,
                Some(_) => found_other = true,
                None => {}
            }
        }
        let set = if found_e {
            Some(e_dir)
        } else if found_other {
            let mut ctx = sos;
            for q in (0..o).rev() {
                if let Some(d) = strong(t[at(q)]) {
                    ctx = d;
                    break;
                }
            }
            Some(if ctx != e_dir { ctx } else { e_dir })
        } else {
            None
        };
        if let Some(d) = set {
            t[at(o)] = d;
            t[at(c)] = d;
            for b0 in [o, c] {
                let mut q = b0 + 1;
                while q < m && orig[at(q)] == NSM {
                    t[at(q)] = d;
                    q += 1;
                }
            }
        }
    }
    // N1, N2
    let mut j = 0;
    while j < m {
        if is_ni(t[at(j)]) {
            let st = j;
            while j < m && is_ni(t[at(j)]) {
                j += 1;
            }
            let before = if st == 0 { sos } else { strong(t[at(st - 1)]).unwrap_or(e_dir) };
            let after = if j == m { eos } else { strong(t[at(j)]).unwrap_or(e_dir) };
            let d = if before == after { before } else { e_dir };
            for q in st..j {
                t[at(q)] = d;
            }
        } else {
            j += 1;
        }
    }
}

/// The visual order (L2) of one line: `order` gets the indices of the characters that are not REMOVED, left to
/// right. Returns how many.
pub fn reorder(levels: &[u8], order: &mut [u16]) -> usize {
    reorder_by(levels, order, &|_| false)
}

/// The same, with the line cut into paragraphs: `brk(k)` true for a paragraph separator, which ends its paragraph
/// (each paragraph is reordered on its own, as lines never cross paragraphs).
pub fn reorder_by(levels: &[u8], order: &mut [u16], brk: &dyn Fn(usize) -> bool) -> usize {
    let mut m = 0;
    for (k, &lv) in levels.iter().enumerate() {
        if lv == REMOVED || m >= order.len() {
            continue;
        }
        order[m] = k as u16;
        m += 1;
    }
    let mut s = 0;
    while s < m {
        let mut e = s;
        while e < m && !brk(order[e] as usize) {
            e += 1;
        }
        let e = (e + 1).min(m);
        reverse_levels(levels, &mut order[s..e]);
        s = e;
    }
    m
}

fn reverse_levels(levels: &[u8], order: &mut [u16]) {
    let m = order.len();
    if m == 0 {
        return;
    }
    let (mut hi, mut lo) = (0u8, u8::MAX);
    for &k in order.iter() {
        let lv = levels[k as usize];
        hi = hi.max(lv);
        lo = lo.min(lv);
    }
    // from the highest level down to the lowest odd level at or above the line's lowest (L2): a line of levels 0
    // and 2 only is reversed at 2 and again at 1
    let lo_odd = lo | 1;
    if hi < lo_odd {
        return;
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
}
