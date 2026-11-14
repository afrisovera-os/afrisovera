//! The shaper's layout engine (SHAPING_PLAN.md C1): the font's own substitution lookups (GSUB types 1 to 6) and its
//! mark attachment (GPOS mark-to-base, mark-to-ligature, mark-to-mark), applied to one run of face 1 glyphs in
//! HarfBuzz's order for Arabic. The lookups are not read from a font: tools/face/shape_src.py compiled them offline
//! into the flat tables of shape_data.rs (contextual rules of every format became rules over glyph sets).
//!
//! The run comes from untrusted text, so: every table read is a checked `get` or a bounded search; the buffer has a
//! fixed size and a substitution that would overflow it is not made (counted); nested lookups stop at a fixed depth;
//! and the whole run has an operations budget, after which no further lookup is tried (counted). No heap.
#![allow(dead_code)]

use super::shape_data as sd;

/// The most glyphs a run may hold (a multiple substitution can grow it; the shaper's line is at most 256 characters).
pub const MAXG: usize = 512;
/// Nested lookups deeper than this are not applied.
const DEPTH: u32 = 6;
/// Lookups tried at a position, per run, before the run is left as it is.
const BUDGET: u32 = 40_000;

#[derive(Clone, Copy)]
pub struct Gi {
    pub g: u16,
    /// the character (index in the line) the glyph came from
    pub cl: u16,
    /// the feature mask bits (shape_data::M_*)
    pub mask: u16,
}

pub const EMPTY: Gi = Gi { g: 0, cl: 0, mask: 0 };

/// (buffer full, budget spent, too deep) since boot.
pub static mut STATS: (u64, u64, u64) = (0, 0, 0);

pub struct Run<'a> {
    pub b: &'a mut [Gi; MAXG],
    pub n: usize,
    ops: u32,
}

impl<'a> Run<'a> {
    pub fn new(b: &'a mut [Gi; MAXG], n: usize) -> Run<'a> {
        Run { b, n: n.min(MAXG), ops: 0 }
    }
}

pub fn class(g: u16) -> u8 {
    sd::GCLASS.get(g as usize).copied().unwrap_or(0)
}

/// Glyph set `s` holds `g` (a bounded binary search; a set index past the table holds nothing).
pub fn in_set(s: u16, g: u16) -> bool {
    match sd::SETS.get(s as usize) {
        Some(&(a, n)) => sd::SETG.get(a as usize..a as usize + n as usize).map_or(false, |v| v.binary_search(&g).is_ok()),
        None => false,
    }
}

/// The lookup flag tells this glyph to be skipped.
fn ignored(g: u16, flag: u16, mfs: u16) -> bool {
    let c = class(g);
    (flag & 2 != 0 && c == 1)
        || (flag & 4 != 0 && c == 2)
        || (flag & 8 != 0 && c == 3)
        || (c == 3 && flag & 0x10 != 0 && !in_set(mfs, g))
}

impl Run<'_> {
    fn next(&self, from: usize, flag: u16, mfs: u16) -> Option<usize> {
        (from + 1..self.n).find(|&j| !ignored(self.b[j].g, flag, mfs))
    }
    fn prev(&self, from: usize, flag: u16, mfs: u16) -> Option<usize> {
        (0..from).rev().find(|&j| !ignored(self.b[j].g, flag, mfs))
    }
    /// Put `seq` in place of the glyph at `i` (each a copy of it with a new glyph). False if it would not fit.
    fn replace_seq(&mut self, i: usize, seq: &[u16]) -> bool {
        let k = seq.len();
        if self.n - 1 + k > MAXG {
            // SAFETY: a scalar counter, mainline only.
            unsafe { STATS.0 += 1 };
            return false;
        }
        let tmpl = self.b[i];
        if k != 1 {
            // move the tail
            let tail = i + 1..self.n;
            if k > 1 {
                self.b.copy_within(tail, i + k);
            } else {
                self.b.copy_within(tail, i);
            }
            self.n = self.n + k - 1;
        }
        for (j, &g) in seq.iter().enumerate() {
            self.b[i + j] = Gi { g, ..tmpl };
        }
        true
    }
    fn delete(&mut self, i: usize) {
        if i < self.n {
            self.b.copy_within(i + 1..self.n, i);
            self.n -= 1;
        }
    }

    /// Apply lookup `li` at position `i`: the first subtable that applies. `check`: the top level, where the glyph
    /// must not be one the lookup's flag skips. Returns where the next position is when something was done.
    fn apply_at(&mut self, li: u16, i: usize, depth: u32, check: bool) -> Option<usize> {
        if depth > DEPTH {
            // SAFETY: a scalar counter, mainline only.
            unsafe { STATS.2 += 1 };
            return None;
        }
        if self.ops >= BUDGET || i >= self.n {
            return None;
        }
        self.ops += 1;
        let &(_, flag, mfs, s0, ns) = sd::LOOKUPS.get(li as usize)?;
        let g = self.b[i].g;
        if check && ignored(g, flag, mfs) {
            return None;
        }
        for si in s0 as usize..s0 as usize + ns as usize {
            let &(kind, a, b, c) = sd::SUBS.get(si)?;
            let (a, b, c) = (a as usize, b as usize, c as usize);
            match kind {
                1 => {
                    let rows = sd::SINGLE.get(a..a + b)?;
                    if let Ok(k) = rows.binary_search_by_key(&g, |r| r.0) {
                        self.b[i].g = rows[k].1;
                        return Some(i + 1);
                    }
                }
                2 => {
                    let rows = sd::MULT.get(a..a + b)?;
                    if let Ok(k) = rows.binary_search_by_key(&g, |r| r.0) {
                        let (_, s, l) = rows[k];
                        let seq = sd::MULTG.get(s as usize..s as usize + l as usize)?;
                        if self.replace_seq(i, seq) {
                            return Some(i + seq.len());
                        }
                        return None;
                    }
                }
                4 => {
                    let rows = sd::LIGS.get(a..a + b)?;
                    let lo = rows.partition_point(|r| r.0 < g);
                    for &(_, cs, cn, lig) in rows[lo..].iter().take_while(|r| r.0 == g) {
                        let comps = sd::LIGG.get(cs as usize..cs as usize + cn as usize)?;
                        let mut at = [0usize; 16];
                        if comps.len() > at.len() {
                            continue;
                        }
                        let mut j = i;
                        let mut ok = true;
                        for (k, &cg) in comps.iter().enumerate() {
                            match self.next(j, flag, mfs) {
                                Some(nj) if self.b[nj].g == cg => {
                                    at[k] = nj;
                                    j = nj;
                                }
                                _ => {
                                    ok = false;
                                    break;
                                }
                            }
                        }
                        if ok {
                            self.b[i].g = lig;
                            for k in (0..comps.len()).rev() {
                                self.delete(at[k]);
                            }
                            return Some(i + 1);
                        }
                    }
                }
                5 => {
                    if !in_set(a as u16, g) {
                        continue;
                    }
                    let rules = sd::RULES.get(b..b + c)?;
                    for &(m0, nb, ni, na, r0, nr) in rules {
                        let (m0, nb, ni, na) = (m0 as usize, nb as usize, ni as usize, na as usize);
                        let sets = match sd::MATCH.get(m0..m0 + nb + ni + na) {
                            Some(s) => s,
                            None => continue,
                        };
                        let mut pos = [0usize; 32];
                        if ni == 0 || ni > pos.len() {
                            continue;
                        }
                        // the input: the first glyph is at i, the rest found by skipping what the flag skips
                        pos[0] = i;
                        let mut ok = in_set(sets[nb], g);
                        let mut j = i;
                        for k in 1..ni {
                            if !ok {
                                break;
                            }
                            match self.next(j, flag, mfs) {
                                Some(nj) if in_set(sets[nb + k], self.b[nj].g) => {
                                    pos[k] = nj;
                                    j = nj;
                                }
                                _ => ok = false,
                            }
                        }
                        // the backtrack, nearest first
                        let mut j = i;
                        for k in 0..nb {
                            if !ok {
                                break;
                            }
                            match self.prev(j, flag, mfs) {
                                Some(pj) if in_set(sets[k], self.b[pj].g) => j = pj,
                                _ => ok = false,
                            }
                        }
                        // the lookahead, after the last input glyph
                        let mut j = pos[ni - 1];
                        for k in 0..na {
                            if !ok {
                                break;
                            }
                            match self.next(j, flag, mfs) {
                                Some(nj) if in_set(sets[nb + ni + k], self.b[nj].g) => j = nj,
                                _ => ok = false,
                            }
                        }
                        if !ok {
                            continue;
                        }
                        // matched: the nested lookups, in order; a length change moves the later positions
                        let recs = sd::RECS.get(r0 as usize..r0 as usize + nr as usize).unwrap_or(&[]);
                        for &(seq, nl) in recs {
                            let s = seq as usize;
                            if s >= ni {
                                continue;
                            }
                            let before = self.n;
                            let _ = self.apply_at(nl, pos[s], depth + 1, false);
                            let d = self.n as isize - before as isize;
                            if d != 0 {
                                for p in pos.iter_mut().take(ni).skip(s + 1) {
                                    *p = (*p as isize + d).max(0) as usize;
                                }
                            }
                        }
                        return Some((pos[ni - 1] + 1).min(self.n).max(i + 1));
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Every GSUB stage, in order: each lookup of a stage over the whole run, at the glyphs whose mask it has.
    pub fn substitute(&mut self) {
        for &(f, c) in sd::STAGES.iter() {
            let ls = match sd::STAGEL.get(f as usize..f as usize + c as usize) {
                Some(v) => v,
                None => continue,
            };
            for &(li, mask) in ls {
                let mut i = 0;
                while i < self.n {
                    if self.ops >= BUDGET {
                        // SAFETY: a scalar counter, mainline only.
                        unsafe { STATS.1 += 1 };
                        return;
                    }
                    if self.b[i].mask & mask != 0 {
                        if let Some(nx) = self.apply_at(li, i, 0, true) {
                            i = nx.max(i + 1).min(self.n.max(i + 1));
                            continue;
                        }
                    }
                    i += 1;
                }
            }
        }
    }

    /// Kerning (GPOS pair positioning, SHAPING_C2_PLAN C2a): for each glyph, (the change to where it is drawn, the
    /// change to its advance), font units. Both values of a pair move its first glyph.
    /// Lookups in order, each over the whole run; within a lookup the first subtable that applies wins, as in
    /// OpenType. Format 1 applies only when its pair is listed; format 2 applies whenever the first glyph is covered
    /// (a missing cell is 0). After a pair, the next try starts at its second glyph, as HarfBuzz does.
    pub fn kern(&self, out: &mut [(i32, i32); MAXG]) {
        for o in out.iter_mut().take(self.n) {
            *o = (0, 0);
        }
        let ks = &sd::KSUB;
        let mut s0 = 0;
        while s0 < ks.len() {
            let lk = ks[s0].0;
            let mut s1 = s0;
            while s1 < ks.len() && ks[s1].0 == lk {
                s1 += 1;
            }
            let subs = &ks[s0..s1];
            let (flag, mfs) = (subs[0].2, subs[0].3);
            let mut i = 0;
            while i < self.n {
                let g = self.b[i].g;
                if ignored(g, flag, mfs) {
                    i += 1;
                    continue;
                }
                let j = match self.next(i, flag, mfs) {
                    Some(j) => j,
                    None => break,
                };
                let g2 = self.b[j].g;
                let mut applied = false;
                for &(_, fmt, _, _, cov, a, b, c, d, e, f) in subs {
                    if !in_set(cov, g) {
                        continue;
                    }
                    let (a, b, c, d, e, f) = (a as usize, b as usize, c as usize, d as usize, e as usize, f as usize);
                    if fmt == 1 {
                        if let Some(rows) = sd::KPAIR.get(a..a + b) {
                            if let Ok(k) = rows.binary_search_by(|r| (r.0, r.1).cmp(&(g, g2))) {
                                out[i].0 = out[i].0.saturating_add(rows[k].2 as i32);
                                out[i].1 = out[i].1.saturating_add(rows[k].3 as i32);
                                applied = true;
                            }
                        }
                    } else {
                        let class = |rows: Option<&[(u16, u16)]>, x: u16| {
                            rows.and_then(|r| r.binary_search_by_key(&x, |q| q.0).ok().map(|k| r[k].1)).unwrap_or(0)
                        };
                        let c1 = class(sd::KCD.get(a..a + b), g);
                        let c2 = class(sd::KCD.get(c..c + d), g2);
                        let (xp, xa) = sd::KCELL
                            .get(e..e + f)
                            .and_then(|r| r.binary_search_by(|q| (q.0, q.1).cmp(&(c1, c2))).ok().map(|k| (r[k].2, r[k].3)))
                            .unwrap_or((0, 0));
                        out[i].0 = out[i].0.saturating_add(xp as i32);
                        out[i].1 = out[i].1.saturating_add(xa as i32);
                        applied = true;
                    }
                    if applied {
                        break;
                    }
                }
                i = if applied { j } else { i + 1 };
            }
            s0 = s1;
        }
    }

    /// Mark attachment: for each glyph, None or (the glyph it hangs on, dx, dy) in font units. Lookups in order; a
    /// later one that matches the same mark replaces the earlier.
    pub fn attach(&self, out: &mut [Option<(u16, i16, i16)>; MAXG]) {
        for o in out.iter_mut().take(self.n) {
            *o = None;
        }
        for &(typ, flag, mfs, ma, mn, ba, bn) in sd::PLK.iter() {
            let marks = match sd::PMARK.get(ma as usize..ma as usize + mn as usize) {
                Some(v) => v,
                None => continue,
            };
            let bases = match sd::PBASE.get(ba as usize..ba as usize + bn as usize) {
                Some(v) => v,
                None => continue,
            };
            for j in 0..self.n {
                let g = self.b[j].g;
                if class(g) != 3 || ignored(g, flag, mfs) {
                    continue;
                }
                let m = match marks.binary_search_by_key(&g, |r| r.0) {
                    Ok(k) => marks[k],
                    Err(_) => continue,
                };
                let target = if typ == 6 {
                    // mark to mark: the glyph before, skipping what the flag skips, must be a mark
                    self.prev(j, flag, mfs).filter(|&k| class(self.b[k].g) == 3)
                } else {
                    // mark to base or ligature: the nearest glyph before that is not a mark
                    (0..j).rev().find(|&k| class(self.b[k].g) != 3)
                };
                let k = match target {
                    Some(k) => k,
                    None => continue,
                };
                let tg = self.b[k].g;
                if let Ok(r) = bases.binary_search_by(|r| (r.0, r.1).cmp(&(tg, m.1))) {
                    let b = bases[r];
                    out[j] = Some((k as u16, b.2.wrapping_sub(m.2), b.3.wrapping_sub(m.3)));
                }
            }
        }
    }
}
