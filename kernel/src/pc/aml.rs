//! The pure part of the read-only AML subset (ACPI AML option C, rung C1; AML_OPTIONS.md, ARCHITECT_REVIEWS
//! 14 Nov): the DSDT/SSDT namespace walk, the `_STA` folding, the embedded controller's map, the controller's
//! read handshake over any `EcIo`, and the read-only evaluator. No `crate::` paths: fuzz-host compiles this file
//! unchanged (target `aml_eval`). battery.rs holds the commands, the real ports and the printing.

use alloc::vec::Vec;


pub const MAX_DEPTH: usize = 24;
pub const MAX_KEPT: usize = 32;
pub const MAX_METHODS: usize = 16;
pub const MAX_INTS: usize = 2048;
pub const MAX_REFS: usize = 512;
pub const MAX_REGIONS: usize = 512;
pub const MAX_FIELDS: usize = 1024;
pub const MAX_SHOWN: usize = 16;
pub const MAX_BODY: usize = 1024;
pub const MAX_PKGS: usize = 256;
/// the most elements a named Package may have to be kept (a _BIX has 21)
pub const MAX_PKG_ELS: usize = 32;
pub const EVAL_DEPTH: usize = 24;
pub const EVAL_READS: u32 = 64;
/// the most methods kept for calls, and the most bytes of their bodies kept in all
pub const MAX_CALLEES: usize = 1024;
pub const MAX_CALLEE_BYTES: usize = 256 * 1024;
/// how deep one method may call another
pub const EVAL_CALLS: u32 = 8;
/// the most times one run may go round its While loops in all (ACPICA stops a loop after as many)
pub const EVAL_LOOPS: u32 = 0xFFFF;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Other,
    Battery,
    Ac,
    Lid,
    Ec,
    Thermal,
}

pub struct Dev {
    pub path: Vec<[u8; 4]>,
    pub kind: Kind,
    pub methods: Vec<[u8; 4]>,
    pub ec_regions: u32,
    pub cut: bool,
    pub sta: Sta,
    /// its _GLK: whether the firmware asks for the ACPI global lock around the controller's use
    pub glk: Sta,
    pub crs: Crs,
    /// the body of its _PSR method (after the flags byte), when it has one of at most MAX_BODY bytes
    pub psr: Option<Vec<u8>>,
    /// the body of its _BST method (after the flags byte), when it has one of at most MAX_BODY bytes
    pub bst: Option<Vec<u8>>,
    /// the body of its _BIX method (true) or, when it has none, its _BIF method (false), at most MAX_BODY bytes
    pub bif: Option<(bool, Vec<u8>)>,
}

/// A device's _CRS as the tables give it.
#[derive(Clone, Copy)]
pub enum Crs {
    Absent,
    /// the first two I/O ports of a constant resource template, and how many it gave
    Ports([u16; 2], usize),
    /// a method: would have to be run
    Run,
}

/// One named unit of a Field: its name, its first bit in the region, its width in bits.
#[derive(Clone, Copy)]
pub struct Unit {
    pub name: [u8; 4],
    pub bit: u32,
    pub bits: u32,
}

/// A device's _STA as the tables give it.
#[derive(Clone, Copy)]
pub enum Sta {
    /// no _STA: present, enabled, shown and working (0x0F) by the ACPI rules
    Absent,
    Value(u64),
    /// `Return(NAME)`: settled after every table is read, since NAME may be declared later
    Ref(Name),
    /// anything that would have to be run
    Run,
}

#[derive(Default)]
pub struct Walk {
    pub devs: Vec<Dev>,
    pub objects: u32,
    pub cut: u32,
    pub conditional: u32,
    pub dropped: u32,
    /// every `Name(X, integer)` outside methods: its full path and value
    pub ints: Vec<(Vec<[u8; 4]>, u64)>,
    /// every `Name(X, Package)` of constant integers and strings outside methods: its full path and elements
    pub pkgs: Vec<(Vec<[u8; 4]>, Vec<El>)>,
    /// `Return(NAME)` statuses to settle: the device's path, the name, the kept device's index
    pub refs: Vec<(Vec<[u8; 4]>, Name, Option<usize>)>,
    pub sta_known: u32,
    pub sta_run: u32,
    /// every OperationRegion: its full path and address space (3 is EmbeddedControl)
    pub regions: Vec<(Vec<[u8; 4]>, u8)>,
    /// units of Fields over an EmbeddedControl region: the region's index, the unit, the scope the Field is in
    pub ec_fields: Vec<(usize, Unit, Vec<[u8; 4]>)>,
    /// units of Fields whose region was not declared yet: the Field's scope, the region's name, the unit
    pub pending: Vec<(Vec<[u8; 4]>, Name, Unit)>,
    pub fields_dropped: u32,
    /// every Method outside other methods, so one may call another: its full path, its argument count and its
    /// body after the flags byte (None when longer than MAX_BODY or past MAX_CALLEE_BYTES in all)
    pub callees: Vec<(Vec<[u8; 4]>, u8, Option<Vec<u8>>)>,
    pub callee_bytes: usize,
}

/// An AML PkgLength at `i`: (the length, the bytes it takes).
pub fn pkg_len(t: &[u8], i: usize) -> Option<(usize, usize)> {
    let lead = *t.get(i)?;
    let extra = (lead >> 6) as usize;
    if extra == 0 {
        return Some(((lead & 0x3F) as usize, 1));
    }
    let mut len = (lead & 0x0F) as usize;
    for k in 0..extra {
        len |= (*t.get(i + 1 + k)? as usize) << (4 + 8 * k);
    }
    Some((len, 1 + extra))
}

/// The package that starts with the PkgLength at `i`: (where its contents start, where it ends), when it ends
/// inside `limit`.
pub fn body_of(t: &[u8], i: usize, limit: usize) -> Option<(usize, usize)> {
    let (len, n) = pkg_len(t, i)?;
    let e = i.checked_add(len)?;
    (len >= n && e <= limit && e <= t.len()).then_some((i + n, e))
}

#[derive(Clone, Copy)]
pub struct Name {
    pub root: bool,
    pub up: usize,
    pub segs: [[u8; 4]; 8],
    pub n: usize,
}

pub fn seg_ok(s: &[u8]) -> bool {
    s.len() == 4
        && (s[0].is_ascii_uppercase() || s[0] == b'_')
        && s[1..].iter().all(|&c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_')
}

/// An AML NameString at `i` (root or parent prefixes, then one segment, a dual or a multi name, or the null name):
/// the name and where it ends.
pub fn name_string(t: &[u8], mut i: usize, end: usize) -> Option<(Name, usize)> {
    let mut nm = Name { root: false, up: 0, segs: [[0; 4]; 8], n: 0 };
    if t.get(i) == Some(&b'\\') {
        nm.root = true;
        i += 1;
    } else {
        while t.get(i) == Some(&b'^') {
            nm.up += 1;
            i += 1;
        }
    }
    let count = match *t.get(i)? {
        0x00 => {
            i += 1;
            0
        }
        0x2E => {
            i += 1;
            2
        }
        0x2F => {
            let c = *t.get(i + 1)? as usize;
            i += 2;
            c
        }
        _ => 1,
    };
    if count > nm.segs.len() {
        return None;
    }
    for k in 0..count {
        let s = t.get(i..i + 4)?;
        if !seg_ok(s) {
            return None;
        }
        nm.segs[k].copy_from_slice(s);
        i += 4;
    }
    nm.n = count;
    (i <= end).then_some((nm, i))
}

/// Where a name declared in `scope` lives.
pub fn resolve(scope: &[[u8; 4]], nm: &Name) -> Vec<[u8; 4]> {
    let mut p = Vec::new();
    if !nm.root {
        p.extend_from_slice(&scope[..scope.len().saturating_sub(nm.up)]);
    }
    p.extend_from_slice(&nm.segs[..nm.n]);
    p
}

/// A single plain segment (no prefix), as a device's own _HID or method is named.
pub fn plain(nm: &Name) -> Option<[u8; 4]> {
    (!nm.root && nm.up == 0 && nm.n == 1).then_some(nm.segs[0])
}

/// The end of a constant data object at `j` (integer, string, buffer, package, Revision), or None for anything
/// that would have to be run.
pub fn data_end(t: &[u8], j: usize, end: usize) -> Option<usize> {
    let e = match *t.get(j)? {
        0x00 | 0x01 | 0xFF => j + 1,
        0x0A => j + 2,
        0x0B => j + 3,
        0x0C => j + 5,
        0x0E => j + 9,
        0x0D => j + 2 + t.get(j + 1..end)?.iter().position(|&c| c == 0)?,
        0x11..=0x13 => body_of(t, j + 1, end)?.1,
        0x5B if t.get(j + 1) == Some(&0x30) => j + 2,
        _ => return None,
    };
    (e <= end).then_some(e)
}

/// A string literal at `j`: its bytes (without the closing zero) and where it ends.
pub fn str_const(t: &[u8], j: usize, end: usize) -> Option<(Vec<u8>, usize)> {
    if *t.get(j)? != 0x0D || j + 1 >= end {
        return None;
    }
    let n = t.get(j + 1..end)?.iter().position(|&c| c == 0)?;
    Some((Vec::from(&t[j + 1..j + 1 + n]), j + 2 + n))
}

/// A `Package (count) {elements}` at `j` whose elements are all constant integers or strings and exactly `count` of
/// them: the elements. Anything else (a buffer, a nested Package, a name, a short Package) is None.
pub fn const_pkg(t: &[u8], j: usize, end: usize) -> Option<Vec<El>> {
    if *t.get(j)? != 0x12 {
        return None;
    }
    let (b, pe) = body_of(t, j + 1, end)?;
    let count = usize::from(*t.get(b).filter(|_| b < pe)?);
    if count > MAX_PKG_ELS {
        return None;
    }
    let mut v = Vec::new();
    let mut k = b + 1;
    while k < pe {
        let (x, n) = match int_const(t, k, pe) {
            Some((x, n)) => (El::Int(x), n),
            None => str_const(t, k, pe).map(|(s, n)| (El::Str(s), n))?,
        };
        v.push(x);
        k = n;
    }
    (v.len() == count).then_some(v)
}

/// A constant integer at `j` (Zero, One, Ones, a byte, word, dword or qword): its value and where it ends.
pub fn int_const(t: &[u8], j: usize, end: usize) -> Option<(u64, usize)> {
    let le = |n: usize| -> Option<u64> {
        let b = t.get(j + 1..j + 1 + n)?;
        Some(b.iter().rev().fold(0u64, |v, &x| v << 8 | x as u64))
    };
    let (v, e) = match *t.get(j)? {
        0x00 => (0, j + 1),
        0x01 => (1, j + 1),
        0xFF => (u64::MAX, j + 1),
        0x0A => (le(1)?, j + 2),
        0x0B => (le(2)?, j + 3),
        0x0C => (le(4)?, j + 5),
        0x0E => (le(8)?, j + 9),
        _ => return None,
    };
    (e <= end).then_some((v, e))
}

/// What a `Method(_STA)` body `t[b..e]` (flags byte first) gives without being run.
pub fn sta_of_method(t: &[u8], b: usize, e: usize) -> Sta {
    let r = b + 1;
    if b >= e || t.get(r) != Some(&0xA4) {
        return Sta::Run;
    }
    if let Some((v, n)) = int_const(t, r + 1, e) {
        if n == e {
            return Sta::Value(v);
        }
    }
    match name_string(t, r + 1, e) {
        Some((nm, n)) if n == e && nm.n > 0 => Sta::Ref(nm),
        _ => Sta::Run,
    }
}

/// The name `nm` referred to from inside `scope`, by the ACPI search rules: a single plain segment is searched for
/// in the scope and then in each parent up to the root; a prefixed or multi-part name is looked up as written.
pub fn search<T>(scope: &[[u8; 4]], nm: &Name, find: impl Fn(&[[u8; 4]]) -> Option<T>) -> Option<T> {
    if let Some(s) = plain(nm) {
        let mut p = Vec::from(scope);
        loop {
            p.push(s);
            if let Some(v) = find(&p) {
                return Some(v);
            }
            p.pop();
            if p.pop().is_none() {
                return None;
            }
        }
    }
    find(&resolve(scope, nm))
}

/// The value of the integer Name `nm` referred to from inside `scope`.
pub fn lookup(ints: &[(Vec<[u8; 4]>, u64)], scope: &[[u8; 4]], nm: &Name) -> Option<u64> {
    search(scope, nm, |p| ints.iter().find(|(q, _)| q.as_slice() == p).map(|(_, v)| *v))
}

/// The index of the OperationRegion `nm` referred to from inside `scope`.
pub fn find_region(regions: &[(Vec<[u8; 4]>, u8)], scope: &[[u8; 4]], nm: &Name) -> Option<usize> {
    search(scope, nm, |p| regions.iter().position(|(q, _)| q.as_slice() == p))
}

/// A Field's list of units in `t[j..e]` over region `rn`, declared in `scope`: the units of an EmbeddedControl
/// region are kept (a region not declared yet is settled later); any other region's are passed over. Reserved
/// units (Offset) move the bit along; access and connection entries take no bits.
pub fn field_list(t: &[u8], mut j: usize, e: usize, scope: &[[u8; 4]], rn: &Name, w: &mut Walk) {
    let region = find_region(&w.regions, scope, rn);
    if let Some(k) = region {
        if w.regions[k].1 != 3 {
            return;
        }
    }
    let mut bit: u32 = 0;
    while j < e {
        match t[j] {
            0x00 => {
                let Some((len, n)) = pkg_len(t, j + 1) else { return };
                bit = bit.saturating_add(len as u32);
                j += 1 + n;
            }
            0x01 => j += 3,
            0x03 => j += 4,
            0x02 => {
                let next = if t.get(j + 1) == Some(&0x11) {
                    body_of(t, j + 2, e).map(|(_, x)| x)
                } else {
                    name_string(t, j + 1, e).map(|(_, x)| x)
                };
                let Some(x) = next else { return };
                j = x;
            }
            _ => {
                let Some(s) = t.get(j..j + 4) else { return };
                if !seg_ok(s) {
                    return;
                }
                let Some((len, n)) = pkg_len(t, j + 4) else { return };
                let mut name = [0u8; 4];
                name.copy_from_slice(s);
                let u = Unit { name, bit, bits: len as u32 };
                match region {
                    Some(k) if w.ec_fields.len() < MAX_FIELDS => w.ec_fields.push((k, u, Vec::from(scope))),
                    None if w.pending.len() < MAX_FIELDS => w.pending.push((Vec::from(scope), *rn, u)),
                    _ => w.fields_dropped += 1,
                }
                bit = bit.saturating_add(len as u32);
                j += 4 + n;
            }
        }
    }
}

/// The I/O ports of a `Name(_CRS, ResourceTemplate)` value at `p`: the first two I/O (0x47) or fixed I/O (0x4B)
/// descriptors, in order.
pub fn crs_of(t: &[u8], p: usize, end: usize) -> Crs {
    let mut ports = [0u16; 2];
    let mut n = 0;
    if t.get(p) != Some(&0x11) {
        return Crs::Ports(ports, 0);
    }
    let Some((b, e)) = body_of(t, p + 1, end) else { return Crs::Ports(ports, 0) };
    let Some((_, mut k)) = int_const(t, b, e) else { return Crs::Ports(ports, 0) };
    while k < e && n < 2 {
        let tag = t[k];
        if tag & 0x80 == 0 {
            let len = (tag & 7) as usize;
            let typ = (tag >> 3) & 0x0F;
            if typ == 0x0F || k + 1 + len > e {
                break;
            }
            let w16 = |o: usize| t[k + o] as u16 | (t[k + o + 1] as u16) << 8;
            if typ == 0x08 && len >= 7 {
                ports[n] = w16(2);
                n += 1;
            } else if typ == 0x09 && len >= 3 {
                ports[n] = w16(1) & 0x3FF;
                n += 1;
            }
            k += 1 + len;
        } else {
            let Some(l) = t.get(k + 1..k + 3) else { break };
            k += 3 + (l[0] as usize | (l[1] as usize) << 8);
        }
    }
    Crs::Ports(ports, n)
}

/// Settle the `Return(NAME)` statuses once every table is read.
pub fn settle(w: &mut Walk) {
    let refs = core::mem::take(&mut w.refs);
    for (path, nm, kept) in refs {
        let sta = match lookup(&w.ints, &path, &nm) {
            Some(v) => {
                w.sta_known += 1;
                Sta::Value(v)
            }
            None => {
                w.sta_run += 1;
                Sta::Run
            }
        };
        if let Some(d) = kept.and_then(|k| w.devs.get_mut(k)) {
            d.sta = sta;
        }
    }
    let pending = core::mem::take(&mut w.pending);
    for (scope, rn, u) in pending {
        if let Some(k) = find_region(&w.regions, &scope, &rn) {
            if w.regions[k].1 == 3 {
                if w.ec_fields.len() < MAX_FIELDS {
                    w.ec_fields.push((k, u, scope));
                } else {
                    w.fields_dropped += 1;
                }
            }
        }
    }
}

/// An EISA id (`PNP0C0A` from 0x0A0CD041 as it sits in memory).
pub fn eisa(b: &[u8]) -> [u8; 7] {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let l = |v: u8| b'@' + (v & 0x1F);
    [
        l(b[0] >> 2),
        l((b[0] & 3) << 3 | b[1] >> 5),
        l(b[1]),
        HEX[(b[2] >> 4) as usize],
        HEX[(b[2] & 15) as usize],
        HEX[(b[3] >> 4) as usize],
        HEX[(b[3] & 15) as usize],
    ]
}

/// What a `Name(_HID, ...)` value at `p` makes a device.
pub fn hid_kind(t: &[u8], p: usize, end: usize) -> Kind {
    let mut id = [0u8; 8];
    let n = match t.get(p) {
        Some(0x0C) => match t.get(p + 1..p + 5) {
            Some(b) if p + 5 <= end => {
                id[..7].copy_from_slice(&eisa(b));
                7
            }
            _ => return Kind::Other,
        },
        Some(0x0D) => {
            let Some(s) = t.get(p + 1..end) else { return Kind::Other };
            match s.iter().position(|&c| c == 0) {
                Some(z) if z <= 8 => {
                    id[..z].copy_from_slice(&s[..z]);
                    z
                }
                _ => return Kind::Other,
            }
        }
        _ => return Kind::Other,
    };
    match &id[..n] {
        b"PNP0C0A" => Kind::Battery,
        b"ACPI0003" => Kind::Ac,
        b"PNP0C0D" => Kind::Lid,
        b"PNP0C09" => Kind::Ec,
        _ => Kind::Other,
    }
}

/// A list of AML objects in `t[i..end]`, declared in `scope`, inside device `dev` when there is one.
pub fn block(t: &[u8], mut i: usize, end: usize, scope: &[[u8; 4]], mut dev: Option<&mut Dev>, w: &mut Walk, depth: usize) {
    if depth > MAX_DEPTH {
        w.cut += 1;
        return;
    }
    while i < end {
        match one(t, i, end, scope, dev.as_deref_mut(), w, depth) {
            Some(n) if n > i => i = n,
            _ => {
                w.cut += 1;
                if let Some(d) = dev.as_deref_mut() {
                    d.cut = true;
                }
                return;
            }
        }
    }
}

/// One AML object at `i`: where the next one starts, or None when it is not a form this reader knows.
pub fn one(t: &[u8], i: usize, end: usize, scope: &[[u8; 4]], dev: Option<&mut Dev>, w: &mut Walk, depth: usize) -> Option<usize> {
    let op = *t.get(i)?;
    let ext = if op == 0x5B { *t.get(i + 1)? } else { 0 };
    match (op, ext) {
        // Scope(name) { objects }
        (0x10, _) => {
            let (b, e) = body_of(t, i + 1, end)?;
            let (nm, p) = name_string(t, b, e)?;
            block(t, p, e, &resolve(scope, &nm), None, w, depth + 1);
            Some(e)
        }
        // Device(name) { objects }, ThermalZone(name) { objects }
        (0x5B, 0x82) | (0x5B, 0x85) => {
            let (b, e) = body_of(t, i + 2, end)?;
            let (nm, p) = name_string(t, b, e)?;
            let kind = if ext == 0x85 { Kind::Thermal } else { Kind::Other };
            let mut d = Dev { path: resolve(scope, &nm), kind, methods: Vec::new(), ec_regions: 0, cut: false, sta: Sta::Absent, glk: Sta::Absent, crs: Crs::Absent, psr: None, bst: None, bif: None };
            w.objects += 1;
            let inner = d.path.clone();
            block(t, p, e, &inner, Some(&mut d), w, depth + 1);
            let kept = (d.kind != Kind::Other && w.devs.len() < MAX_KEPT).then_some(w.devs.len());
            let sta = d.sta;
            match sta {
                Sta::Absent => {}
                Sta::Value(_) => w.sta_known += 1,
                Sta::Run => w.sta_run += 1,
                Sta::Ref(r) if w.refs.len() < MAX_REFS => w.refs.push((d.path.clone(), r, kept)),
                Sta::Ref(_) => {
                    w.sta_run += 1;
                    d.sta = Sta::Run;
                }
            }
            if kept.is_some() {
                w.devs.push(d);
            } else if d.kind != Kind::Other {
                w.dropped += 1;
            }
            Some(e)
        }
        // Processor(name, id, PBLK address, PBLK length) { objects }, PowerResource(name, level, order) { objects }
        (0x5B, 0x83) | (0x5B, 0x84) => {
            let (b, e) = body_of(t, i + 2, end)?;
            let (nm, p) = name_string(t, b, e)?;
            let skip = if ext == 0x83 { 6 } else { 3 };
            if p + skip > e {
                return None;
            }
            w.objects += 1;
            block(t, p + skip, e, &resolve(scope, &nm), None, w, depth + 1);
            Some(e)
        }
        // Method(name, flags) { body }: never run, skipped whole
        (0x14, _) => {
            let (b, e) = body_of(t, i + 1, end)?;
            let (nm, p) = name_string(t, b, e)?;
            w.objects += 1;
            if p < e && w.callees.len() < MAX_CALLEES {
                let keep = e - p <= MAX_BODY && w.callee_bytes + (e - p) <= MAX_CALLEE_BYTES;
                if keep {
                    w.callee_bytes += e - p;
                }
                w.callees.push((resolve(scope, &nm), t[p] & 7, keep.then(|| Vec::from(&t[p + 1..e]))));
            }
            if let (Some(d), Some(s)) = (dev, plain(&nm)) {
                if d.methods.len() < MAX_METHODS {
                    d.methods.push(s);
                }
                if &s == b"_STA" {
                    d.sta = sta_of_method(t, p, e);
                }
                if &s == b"_GLK" {
                    // a constant Return is used; anything else counts as "asks for the lock" (Sta::Run)
                    d.glk = match sta_of_method(t, p, e) {
                        Sta::Value(v) => Sta::Value(v),
                        _ => Sta::Run,
                    };
                }
                if &s == b"_CRS" {
                    d.crs = Crs::Run;
                }
                if &s == b"_PSR" && p < e && e - p <= MAX_BODY {
                    d.psr = Some(Vec::from(&t[p + 1..e]));
                }
                if &s == b"_BST" && p < e && e - p <= MAX_BODY {
                    d.bst = Some(Vec::from(&t[p + 1..e]));
                }
                // _BIX wins over _BIF, whichever comes first (ACPI spec 10.2.2.2)
                let bix = &s == b"_BIX";
                if (bix || &s == b"_BIF") && p < e && e - p <= MAX_BODY && !d.bif.as_ref().is_some_and(|(x, _)| *x) {
                    d.bif = Some((bix, Vec::from(&t[p + 1..e])));
                }
            }
            Some(e)
        }
        // Name(name, constant)
        (0x08, _) => {
            let (nm, p) = name_string(t, i + 1, end)?;
            w.objects += 1;
            let int = int_const(t, p, end).map(|(v, _)| v);
            if let Some(v) = int {
                if w.ints.len() < MAX_INTS {
                    w.ints.push((resolve(scope, &nm), v));
                }
            }
            if let Some(v) = const_pkg(t, p, end) {
                if w.pkgs.len() < MAX_PKGS {
                    w.pkgs.push((resolve(scope, &nm), v));
                }
            }
            if let (Some(d), Some(s)) = (dev, plain(&nm)) {
                if &s == b"_HID" && d.kind == Kind::Other {
                    d.kind = hid_kind(t, p, end);
                }
                if &s == b"_STA" {
                    d.sta = int.map_or(Sta::Run, Sta::Value);
                }
                if &s == b"_GLK" {
                    d.glk = int.map_or(Sta::Run, Sta::Value);
                }
                if &s == b"_CRS" {
                    d.crs = crs_of(t, p, end);
                }
            }
            data_end(t, p, end)
        }
        // OperationRegion(name, space, offset, length); space 3 is the embedded controller's own
        (0x5B, 0x80) => {
            let (nm, p) = name_string(t, i + 2, end)?;
            let space = *t.get(p)?;
            let q = data_end(t, p + 1, end)?;
            let r = data_end(t, q, end)?;
            w.objects += 1;
            if w.regions.len() < MAX_REGIONS {
                w.regions.push((resolve(scope, &nm), space));
            }
            if let (Some(d), 3) = (dev, space) {
                d.ec_regions += 1;
            }
            Some(r)
        }
        // Field(region, flags) { units }: an EmbeddedControl region's units are kept
        (0x5B, 0x81) => {
            let (b, e) = body_of(t, i + 2, end)?;
            if let Some((rn, p)) = name_string(t, b, e) {
                field_list(t, p + 1, e, scope, &rn, w);
            }
            Some(e)
        }
        // IndexField, BankField: skipped whole
        (0x5B, 0x86) | (0x5B, 0x87) => Some(body_of(t, i + 2, end)?.1),
        // If, Else, While at definition level: the condition would have to be run, so not entered
        (0xA0..=0xA2, _) => {
            let e = body_of(t, i + 1, end)?.1;
            w.conditional += 1;
            Some(e)
        }
        // External(name, type, argument count)
        (0x15, _) => {
            let (_, p) = name_string(t, i + 1, end)?;
            (p + 2 <= end).then_some(p + 2)
        }
        // Alias(name, name)
        (0x06, _) => {
            let (_, p) = name_string(t, i + 1, end)?;
            Some(name_string(t, p, end)?.1)
        }
        // Mutex(name, level), Event(name)
        (0x5B, 0x01) => {
            let (_, p) = name_string(t, i + 2, end)?;
            (p + 1 <= end).then_some(p + 1)
        }
        (0x5B, 0x02) => Some(name_string(t, i + 2, end)?.1),
        _ => None,
    }
}

/// Walk one table (DSDT or SSDT, header included).
pub fn walk_table(t: &[u8], w: &mut Walk) {
    if t.len() > 36 {
        block(t, 36, t.len(), &[], None, w, 0);
    }
}

// ---- `battery ec`: the controller's named fields read through its two ports (ACPI spec 12.2 and 12.3) ----
//
// One read is: wait until the controller's input buffer is empty (status bit 1 clear), send the read command 0x80
// to the command port, wait again, send the byte's address to the data port, wait until its output buffer is full
// (status bit 0 set), take the byte from the data port. Every wait is limited (EC_WAIT polls of the status port).
// Nothing is ever written to the controller's memory: 0x80 is the only command sent. Before anything is sent the
// status port is read once: 0xFF means nothing answers there (no controller), and a full output buffer means a
// byte is waiting for some other reader (the firmware), so the controller is left alone in both cases. The ACPI
// global lock is taken around each read by the EcIo's begin and end, when the controller needs it (battery.rs).

pub const EC_WAIT: u32 = 100_000;
pub const EC_READ: u8 = 0x80;

pub trait EcIo {
    fn inb(&mut self, port: u16) -> u8;
    fn outb(&mut self, port: u16, v: u8);
    /// The controller about to be used, by its _GLK (Sta::Absent when it has none): false when it may not be used
    /// (the global lock is needed and cannot be had), so nothing is sent.
    fn device(&mut self, _glk: Sta) -> Result<(), EcErr> {
        Ok(())
    }
    /// One transaction begins (the global lock taken when the device needs it); false: it could not be had in time.
    fn begin(&mut self) -> bool {
        true
    }
    /// The transaction ended (the global lock given back).
    fn end(&mut self) {}
}

// ---- the ACPI global lock (ACPI spec 5.2.10.1), over any 32-bit word: the FACS's in the kernel, a pretend one in
// the selftest and fuzz-host. Bit 0 pending, bit 1 owned. Every compare-and-swap loop is bounded. ----

/// The global lock's 32-bit word.
pub trait LockWord {
    fn load(&self) -> u32;
    /// Write `new` if the word still holds `old`.
    fn cas(&self, old: u32, new: u32) -> bool;
}

/// Compare-and-swap tries before an acquire or release gives up (another agent changing the word that often is
/// a broken firmware).
pub const GLK_CAS_TRIES: u32 = 1000;

/// One try at the lock: Some(true) owned now, Some(false) the firmware holds it (our pending bit is set, so it
/// tells us when it lets go), None the word kept changing.
pub fn glk_try(w: &dyn LockWord) -> Option<bool> {
    for _ in 0..GLK_CAS_TRIES {
        let old = w.load();
        let mut new = (old & !1) | 2;
        if old & 2 != 0 {
            new |= 1;
        }
        if w.cas(old, new) {
            return Some(new & 1 == 0);
        }
    }
    None
}

/// Give the lock back: Some(true) when the firmware was waiting for it (GBL_RLS must then be set), None the word
/// kept changing.
pub fn glk_release(w: &dyn LockWord) -> Option<bool> {
    for _ in 0..GLK_CAS_TRIES {
        let old = w.load();
        if w.cas(old, old & !3) {
            return Some(old & 1 != 0);
        }
    }
    None
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EcErr {
    /// the status port reads 0xFF: nothing answers there
    Dead,
    /// a byte was already waiting for another reader
    Waiting,
    /// the input buffer stayed full before the command (at this address)
    Command(u8),
    /// the input buffer stayed full before the address
    Address(u8),
    /// the output buffer never filled
    Answer(u8),
    /// the ACPI global lock was not free in time (at this address)
    Lock(u8),
    /// the controller asks for the ACPI global lock, and there is no FACS to take it in
    NoLock,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EcVal {
    Value(u64),
    /// wider than 32 bits
    Wide,
    /// past the controller's 256 bytes
    Past,
    /// not reached: the reads stopped before it
    Skipped,
}

pub fn ec_wait(io: &mut dyn EcIo, cmd: u16, full: bool) -> bool {
    (0..EC_WAIT).any(|_| {
        let s = io.inb(cmd);
        if full { s & 1 != 0 } else { s & 2 == 0 }
    })
}

pub fn ec_byte(io: &mut dyn EcIo, data: u16, cmd: u16, a: u8) -> Result<u8, EcErr> {
    if !io.begin() {
        return Err(EcErr::Lock(a));
    }
    let r = ec_byte_locked(io, data, cmd, a);
    io.end();
    r
}

/// One read, the global lock (when needed) already held. Before the command: wait for the input buffer to empty,
/// and stop with nothing sent if any status read shows the output buffer full (a byte waiting for another reader:
/// the firmware's SMM code or an EC query, which would otherwise be taken as our answer; architect, 14 Nov).
fn ec_byte_locked(io: &mut dyn EcIo, data: u16, cmd: u16, a: u8) -> Result<u8, EcErr> {
    let mut ready = false;
    for _ in 0..EC_WAIT {
        let s = io.inb(cmd);
        if s & 1 != 0 {
            return Err(EcErr::Waiting);
        }
        if s & 2 == 0 {
            ready = true;
            break;
        }
    }
    if !ready {
        return Err(EcErr::Command(a));
    }
    io.outb(cmd, EC_READ);
    if !ec_wait(io, cmd, false) {
        return Err(EcErr::Address(a));
    }
    io.outb(data, a);
    if !ec_wait(io, cmd, true) {
        return Err(EcErr::Answer(a));
    }
    Ok(io.inb(data))
}

/// Ports an embedded controller's _CRS must never name: the keyboard controller, the interrupt controllers, PCI
/// configuration and the first serial port (this kernel's console).
pub fn ec_port_refused(p: u16) -> bool {
    matches!(p, 0 | 0x20 | 0x21 | 0x60 | 0x64 | 0xA0 | 0xA1 | 0xCF8..=0xCFF | 0x3F8..=0x3FF)
}

/// Read every unit's bytes (each byte once), in order. Returns the values, the bytes read and why it stopped early.
pub fn ec_read_units(io: &mut dyn EcIo, data: u16, cmd: u16, us: &[Unit]) -> (Vec<EcVal>, u32, Option<EcErr>) {
    let mut vals = Vec::new();
    let st = io.inb(cmd);
    let first = if st == 0xFF {
        Some(EcErr::Dead)
    } else if st & 1 != 0 {
        Some(EcErr::Waiting)
    } else {
        None
    };
    if let Some(e) = first {
        vals.resize(us.len(), EcVal::Skipped);
        return (vals, 0, Some(e));
    }
    let mut seen = [None::<u8>; 256];
    let mut bytes = 0;
    let mut stop = None;
    for u in us {
        let lo = u.bit / 8;
        let hi = (u.bit + u.bits.max(1) - 1) / 8;
        let v = if stop.is_some() {
            EcVal::Skipped
        } else if u.bits > 32 {
            EcVal::Wide
        } else if hi > 0xFF {
            EcVal::Past
        } else {
            let mut raw = 0u64;
            for b in lo..=hi {
                let x = match seen[b as usize] {
                    Some(x) => x,
                    None => match ec_byte(io, data, cmd, b as u8) {
                        Ok(x) => {
                            bytes += 1;
                            seen[b as usize] = Some(x);
                            x
                        }
                        Err(e) => {
                            stop = Some(e);
                            break;
                        }
                    },
                };
                raw |= (x as u64) << (8 * (b - lo));
            }
            if stop.is_some() {
                EcVal::Skipped
            } else {
                EcVal::Value((raw >> (u.bit % 8)) & ((1u64 << u.bits.max(1)) - 1))
            }
        };
        vals.push(v);
    }
    (vals, bytes, stop)
}

// ---- the interpreter's second slice: an AC adapter's _PSR, run read only ----

/// Why a method could not be worked out.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Why {
    /// an AML form this evaluator does not run (its first byte)
    Op(u8),
    /// a name that is neither a constant integer Name nor an embedded controller field
    Name([u8; 4]),
    /// the controller read of this field failed
    Read([u8; 4], EcErr),
    /// this field's controller has no plain pair of ports
    NoController([u8; 4]),
    /// this field is wider than 32 bits or past the controller's 256 bytes
    Wide([u8; 4]),
    Deep,
    NoReturn,
    /// the body ends in the middle of a form
    Bad,
    /// more than EVAL_READS field reads
    Many,
    /// a store into something other than a Local or the Debug object (its first byte)
    Write(u8),
    /// this Local (0 to 7) is read before anything was stored in it
    Unset(u8),
    /// a Package or a string where a number is needed
    Pkg,
    /// an Index of something other than a Package held in a Local or a named Package, or past its end
    Index,
    /// a Package that declares more elements than it gives
    Short,
    /// this Arg (0 to 6) is read, but the method was not given it
    NoArg(u8),
    /// a call of this method, whose body is longer than this evaluator keeps
    Big([u8; 4]),
    /// methods calling methods more than EVAL_CALLS deep
    Calls,
    /// a Divide or Mod by Zero
    Zero,
    /// While loops gone round more than EVAL_LOOPS times
    Loops,
    /// a Break or Continue outside a While
    NoWhile,
}

/// What a Break or Continue asked of the While loop around it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Go,
    Break,
    Continue,
}

/// A value the evaluator works with: an integer, a string, or a Package (as a battery's _BST and _BIF give).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Val {
    Int(u64),
    Str(Vec<u8>),
    Pkg(Vec<El>),
}

/// One element of a Package: an integer or a string.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum El {
    Int(u64),
    Str(Vec<u8>),
}

pub type FieldRead<'r> = dyn FnMut(usize, &Unit) -> Result<u64, Why> + 'r;

pub struct Eval<'a, 'r> {
    pub w: &'a Walk,
    pub scope: &'a [[u8; 4]],
    pub t: &'a [u8],
    pub read: &'a mut FieldRead<'r>,
    pub reads: u32,
    /// Local0 to Local7: they live only while the method runs
    pub locals: [Option<Val>; 8],
    /// Arg0 to Arg6: what the caller gave (copies; a Package changed here is seen by the caller only when returned)
    pub args: [Option<Val>; 7],
    /// how many calls deep this run is
    pub calls: u32,
    /// this run's own copies of the named Packages it used: full path and elements (the namespace is never changed)
    pub named: Vec<(Vec<[u8; 4]>, Vec<El>)>,
    /// how many times this run has gone round its While loops, and how many loops the current method is inside
    pub loops: u32,
    pub whiles: u32,
    /// set by a Break or Continue until the While loop around it takes it
    pub flow: Flow,
}

/// Where a Package an Index points into lives: a Local, or this run's copy of a named Package (its place in `named`).
#[derive(Clone, Copy)]
pub enum Slot {
    Local(usize),
    Arg(usize),
    Named(usize),
}

/// The first byte of a NameString.
pub fn name_lead(b: u8) -> bool {
    matches!(b, b'\\' | b'^' | b'_' | b'A'..=b'Z' | 0x2E | 0x2F)
}

/// The embedded controller field `nm` refers to from inside `scope`: its region's index and the unit.
pub fn find_field(w: &Walk, scope: &[[u8; 4]], nm: &Name) -> Option<(usize, Unit)> {
    search(scope, nm, |p| {
        let (last, parent) = p.split_last()?;
        w.ec_fields.iter().find(|(_, u, s)| &u.name == last && s.as_slice() == parent).map(|(k, u, _)| (*k, *u))
    })
}

impl<'a> Eval<'a, '_> {
    /// This run's copy of the named Package `nm` refers to (found by the ACPI search rules), made from the tables'
    /// values the first time the run uses it. None when `nm` is not a named Package of integers.
    pub fn named_pkg(&mut self, nm: &Name) -> Option<usize> {
        let w = self.w;
        let path = search(self.scope, nm, |p| w.pkgs.iter().find(|(q, _)| q.as_slice() == p).map(|(q, _)| q))?;
        if let Some(i) = self.named.iter().position(|(q, _)| q == path) {
            return Some(i);
        }
        let v = w.pkgs.iter().find(|(q, _)| q == path)?.1.clone();
        self.named.push((path.clone(), v));
        Some(self.named.len() - 1)
    }

    /// The Package in slot `s`, if it holds one.
    pub fn slot(&mut self, s: Slot) -> Option<&mut Vec<El>> {
        match s {
            Slot::Local(n) => match &mut self.locals[n] {
                Some(Val::Pkg(p)) => Some(p),
                _ => None,
            },
            Slot::Arg(n) => match &mut self.args[n] {
                Some(Val::Pkg(p)) => Some(p),
                _ => None,
            },
            Slot::Named(n) => self.named.get_mut(n).map(|(_, p)| p),
        }
    }

    /// The method `nm` refers to from inside this run's scope: its path, argument count and body.
    pub fn callee(&self, nm: &Name) -> Option<&'a (Vec<[u8; 4]>, u8, Option<Vec<u8>>)> {
        let w: &'a Walk = self.w;
        search(self.scope, nm, |p| w.callees.iter().find(|(q, _, _)| q.as_slice() == p))
    }

    /// A call at `j` of method `m` (its name ends at `k`): its arguments are worked out here, then its body runs
    /// with its own Locals and Args, reading fields and sharing this run's named Package copies. What it returns
    /// (None when it ends without a Return) and where the call ends.
    pub fn call(&mut self, m: &'a (Vec<[u8; 4]>, u8, Option<Vec<u8>>), mut k: usize, e: usize, depth: usize) -> Result<(Option<Val>, usize), Why> {
        let last = m.0.last().copied().unwrap_or(*b"????");
        let Some(body) = m.2.as_deref() else { return Err(Why::Big(last)) };
        if self.calls >= EVAL_CALLS {
            return Err(Why::Calls);
        }
        let mut args: [Option<Val>; 7] = Default::default();
        for a in args.iter_mut().take(usize::from(m.1)) {
            let (v, n) = self.obj(k, e, depth + 1)?;
            *a = Some(v);
            k = n;
        }
        let saved = (self.t, self.scope, core::mem::take(&mut self.locals), core::mem::replace(&mut self.args, args));
        self.t = body;
        self.scope = &m.0;
        self.calls += 1;
        // a Break in the called method does not reach a loop in the caller
        let whiles = core::mem::replace(&mut self.whiles, 0);
        let r = self.list(0, body.len(), depth + 1);
        self.whiles = whiles;
        self.calls -= 1;
        (self.t, self.scope, self.locals, self.args) = saved;
        Ok((r?, k))
    }

    /// `Index (source, index, no target)` at `k`: where the Package lives, the element and where the form ends.
    /// The source must be a Local or an Arg holding a Package, or a named Package, and the index must be inside it.
    pub fn index(&mut self, k: usize, e: usize, depth: usize) -> Result<(Slot, usize, usize), Why> {
        let t = self.t;
        let (s, k) = match t.get(k + 1).filter(|_| k + 1 < e) {
            Some(&x @ 0x60..=0x67) => {
                let n = usize::from(x - 0x60);
                if self.locals[n].is_none() {
                    return Err(Why::Unset(n as u8));
                }
                (Slot::Local(n), k + 2)
            }
            // an Arg holding a Package (a copy of the caller's); an Arg holding anything else, or none, stops
            Some(&x @ 0x68..=0x6E) => (Slot::Arg(usize::from(x - 0x68)), k + 2),
            Some(b'\\' | b'^' | b'_' | b'A'..=b'Z' | 0x2E | 0x2F) => {
                let (nm, k) = name_string(t, k + 1, e).ok_or(Why::Bad)?;
                if nm.n == 0 {
                    return Err(Why::Bad);
                }
                (Slot::Named(self.named_pkg(&nm).ok_or(Why::Index)?), k)
            }
            Some(_) => return Err(Why::Index),
            None => return Err(Why::Bad),
        };
        let (i, k) = self.term(k, e, depth + 1)?;
        match t.get(k).filter(|_| k < e) {
            Some(0x00) => {}
            Some(&x) => return Err(Why::Write(x)),
            None => return Err(Why::Bad),
        }
        match self.slot(s) {
            Some(p) if i < p.len() as u64 => Ok((s, i as usize, k + 1)),
            _ => Err(Why::Index),
        }
    }

    /// The target at `k` of a form that gives `v`: none (a zero byte), a Local, the Debug object, or one element of
    /// a Package held in a Local. Anything else would be written outside the method, so it stops the evaluation.
    /// Returns where the target ends.
    pub fn target(&mut self, k: usize, e: usize, v: Val, depth: usize) -> Result<usize, Why> {
        let t = self.t;
        match t.get(k).filter(|_| k < e) {
            Some(0x00) => Ok(k + 1),
            Some(&x @ 0x60..=0x67) => {
                self.locals[usize::from(x - 0x60)] = Some(v);
                Ok(k + 1)
            }
            // an Arg holds a copy, so storing into it changes nothing outside the method
            Some(&x @ 0x68..=0x6E) => {
                self.args[usize::from(x - 0x68)] = Some(v);
                Ok(k + 1)
            }
            Some(0x5B) if t.get(k + 1) == Some(&0x31) && k + 1 < e => Ok(k + 2),
            Some(0x88) => {
                let (s, i, end) = self.index(k, e, depth)?;
                let x = match v {
                    Val::Int(x) => El::Int(x),
                    Val::Str(b) => El::Str(b),
                    Val::Pkg(_) => return Err(Why::Pkg),
                };
                if let Some(p) = self.slot(s) {
                    p[i] = x;
                }
                Ok(end)
            }
            Some(&x) => Err(Why::Write(x)),
            None => Err(Why::Bad),
        }
    }

    /// `Store (value, target)` at `j`: into a Local, Debug, or an element of a Package held in a Local only; a
    /// missing target is a broken form. Its value and where it ends.
    pub fn store(&mut self, j: usize, e: usize, depth: usize) -> Result<(Val, usize), Why> {
        let (v, k) = self.obj(j + 1, e, depth + 1)?;
        if self.t.get(k) == Some(&0x00) {
            return Err(Why::Bad);
        }
        let end = self.target(k, e, v.clone(), depth)?;
        Ok((v, end))
    }

    /// `Package (count) {elements}` at `j`, every element a string literal or an integer expression.
    pub fn package(&mut self, j: usize, e: usize, depth: usize) -> Result<(Val, usize), Why> {
        let (b, pe) = body_of(self.t, j + 1, e).ok_or(Why::Bad)?;
        let count = usize::from(*self.t.get(b).filter(|_| b < pe).ok_or(Why::Bad)?);
        let mut v = Vec::new();
        let mut k = b + 1;
        while k < pe {
            let (x, n) = match str_const(self.t, k, pe) {
                Some((s, n)) => (El::Str(s), n),
                None if self.t[k] == 0x0D => return Err(Why::Bad),
                None => self.term(k, pe, depth + 1).map(|(x, n)| (El::Int(x), n))?,
            };
            v.push(x);
            k = n;
        }
        match v.len().cmp(&count) {
            core::cmp::Ordering::Equal => Ok((Val::Pkg(v), pe)),
            core::cmp::Ordering::Less => Err(Why::Short),
            core::cmp::Ordering::Greater => Err(Why::Bad),
        }
    }

    /// One value at `j` where a Package is allowed (a Return, or the value a Store copies).
    pub fn obj(&mut self, j: usize, e: usize, depth: usize) -> Result<(Val, usize), Why> {
        if depth > EVAL_DEPTH {
            return Err(Why::Deep);
        }
        let t = self.t;
        match t.get(j).filter(|_| j < e) {
            Some(&x @ 0x60..=0x67) => match &self.locals[usize::from(x - 0x60)] {
                Some(v) => Ok((v.clone(), j + 1)),
                None => Err(Why::Unset(x - 0x60)),
            },
            Some(&x @ 0x68..=0x6E) => match &self.args[usize::from(x - 0x68)] {
                Some(v) => Ok((v.clone(), j + 1)),
                None => Err(Why::NoArg(x - 0x68)),
            },
            Some(0x12) => self.package(j, e, depth),
            Some(0x70) => self.store(j, e, depth),
            Some(0x0D) => str_const(t, j, e).map(|(s, k)| (Val::Str(s), k)).ok_or(Why::Bad),
            // a method call (whatever it returns), or a named Package: this run's copy of it
            Some(&x) if name_lead(x) => {
                let (nm, k) = name_string(t, j, e).ok_or(Why::Bad)?;
                match (nm.n, lookup(&self.w.ints, self.scope, &nm)) {
                    (0, _) => Err(Why::Bad),
                    (_, Some(_)) => self.term(j, e, depth).map(|(v, k)| (Val::Int(v), k)),
                    _ if self.callee(&nm).is_some() => {
                        let m = self.callee(&nm).ok_or(Why::Bad)?;
                        match self.call(m, k, e, depth)? {
                            (Some(v), k) => Ok((v, k)),
                            (None, _) => Err(Why::NoReturn),
                        }
                    }
                    _ => match self.named_pkg(&nm) {
                        Some(n) => Ok((Val::Pkg(self.named[n].1.clone()), k)),
                        None => self.term(j, e, depth).map(|(v, k)| (Val::Int(v), k)),
                    },
                }
            }
            _ => self.term(j, e, depth).map(|(v, k)| (Val::Int(v), k)),
        }
    }

    /// One integer expression at `j`, inside `t[..e]`: its value and where it ends.
    pub fn term(&mut self, j: usize, e: usize, depth: usize) -> Result<(u64, usize), Why> {
        if depth > EVAL_DEPTH {
            return Err(Why::Deep);
        }
        let t = self.t;
        if let Some(c) = int_const(t, j, e) {
            return Ok(c);
        }
        let op = *t.get(j).filter(|_| j < e).ok_or(Why::Bad)?;
        match op {
            // Add, Subtract, Multiply, ShiftLeft, ShiftRight, And, Or, XOr (a, b, target): the target may be none,
            // a Local, an Arg, Debug or a Package element. Integers wrap at 64 bits, and a shift by 64 or more gives
            // Zero, as ACPICA does for a table of revision 2 or later.
            0x72 | 0x74 | 0x77 | 0x79 | 0x7A | 0x7B | 0x7D | 0x7F => {
                let (a, k) = self.term(j + 1, e, depth + 1)?;
                let (b, k) = self.term(k, e, depth + 1)?;
                let v = match op {
                    0x72 => a.wrapping_add(b),
                    0x74 => a.wrapping_sub(b),
                    0x77 => a.wrapping_mul(b),
                    0x79 => if b < 64 { a << b } else { 0 },
                    0x7A => if b < 64 { a >> b } else { 0 },
                    0x7B => a & b,
                    0x7D => a | b,
                    _ => a ^ b,
                };
                Ok((v, self.target(k, e, Val::Int(v), depth)?))
            }
            // Divide (dividend, divisor, remainder target, quotient target) gives the quotient; Mod (a, b, target)
            // gives the remainder. A divisor of Zero stops the evaluation, as it does in ACPICA.
            0x78 | 0x85 => {
                let (a, k) = self.term(j + 1, e, depth + 1)?;
                let (b, k) = self.term(k, e, depth + 1)?;
                if b == 0 {
                    return Err(Why::Zero);
                }
                if op == 0x85 {
                    return Ok((a % b, self.target(k, e, Val::Int(a % b), depth)?));
                }
                let k = self.target(k, e, Val::Int(a % b), depth)?;
                Ok((a / b, self.target(k, e, Val::Int(a / b), depth)?))
            }
            // Increment, Decrement (a Local or an Arg): checked before anything is read, as anything else would be
            // written outside the method. Integers wrap at 64 bits.
            0x75 | 0x76 => {
                match t.get(j + 1).filter(|_| j + 1 < e) {
                    Some(0x60..=0x6E) => {}
                    Some(&x) => return Err(Why::Write(x)),
                    None => return Err(Why::Bad),
                }
                let (a, k) = self.term(j + 1, e, depth + 1)?;
                let v = if op == 0x75 { a.wrapping_add(1) } else { a.wrapping_sub(1) };
                self.target(j + 1, e, Val::Int(v), depth)?;
                Ok((v, k))
            }
            // Not (a, target)
            0x80 => {
                let (a, k) = self.term(j + 1, e, depth + 1)?;
                Ok((!a, self.target(k, e, Val::Int(!a), depth)?))
            }
            0x70 => match self.store(j, e, depth)? {
                (Val::Int(v), k) => Ok((v, k)),
                _ => Err(Why::Pkg),
            },
            // Local0 to Local7
            0x60..=0x67 => match &self.locals[usize::from(op - 0x60)] {
                Some(Val::Int(v)) => Ok((*v, j + 1)),
                Some(_) => Err(Why::Pkg),
                None => Err(Why::Unset(op - 0x60)),
            },
            // Arg0 to Arg6
            0x68..=0x6E => match &self.args[usize::from(op - 0x68)] {
                Some(Val::Int(v)) => Ok((*v, j + 1)),
                Some(_) => Err(Why::Pkg),
                None => Err(Why::NoArg(op - 0x68)),
            },
            // DerefOf (Index (source, index)): one element of a Package held in a Local or a named Package
            0x83 if t.get(j + 1) == Some(&0x88) => {
                let (s, i, k) = self.index(j + 1, e, depth)?;
                match self.slot(s).map(|p| if let El::Int(v) = p[i] { Ok(v) } else { Err(Why::Pkg) }) {
                    Some(Ok(v)) => Ok((v, k)),
                    Some(Err(y)) => Err(y),
                    None => Err(Why::Index),
                }
            }
            // LAnd, LOr, LEqual, LGreater, LLess: true is Ones, false is Zero
            0x90 | 0x91 | 0x93..=0x95 => {
                let (a, k) = self.term(j + 1, e, depth + 1)?;
                let (b, k) = self.term(k, e, depth + 1)?;
                let r = match op {
                    0x90 => a != 0 && b != 0,
                    0x91 => a != 0 || b != 0,
                    0x93 => a == b,
                    0x94 => a > b,
                    _ => a < b,
                };
                Ok((if r { u64::MAX } else { 0 }, k))
            }
            // LNot
            0x92 => {
                let (a, k) = self.term(j + 1, e, depth + 1)?;
                Ok((if a == 0 { u64::MAX } else { 0 }, k))
            }
            b'\\' | b'^' | b'_' | b'A'..=b'Z' | 0x2E | 0x2F => {
                let (nm, k) = name_string(t, j, e).ok_or(Why::Bad)?;
                if nm.n == 0 {
                    return Err(Why::Bad);
                }
                if let Some(v) = lookup(&self.w.ints, self.scope, &nm) {
                    return Ok((v, k));
                }
                if let Some(m) = self.callee(&nm) {
                    return match self.call(m, k, e, depth)? {
                        (Some(Val::Int(v)), k) => Ok((v, k)),
                        (Some(_), _) => Err(Why::Pkg),
                        (None, _) => Err(Why::NoReturn),
                    };
                }
                let Some((r, u)) = find_field(self.w, self.scope, &nm) else { return Err(Why::Name(nm.segs[nm.n - 1])) };
                self.reads += 1;
                if self.reads > EVAL_READS {
                    return Err(Why::Many);
                }
                Ok(((self.read)(r, &u)?, k))
            }
            _ => Err(Why::Op(op)),
        }
    }

    /// The statements in `t[j..e]`: Some(value) when one of them returns.
    pub fn list(&mut self, mut j: usize, e: usize, depth: usize) -> Result<Option<Val>, Why> {
        if depth > EVAL_DEPTH {
            return Err(Why::Deep);
        }
        let t = self.t;
        while j < e {
            match t[j] {
                0xA4 => return Ok(Some(self.obj(j + 1, e, depth + 1)?.0)),
                0xA3 => j += 1,
                // a Store, run for its target and the value dropped
                0x70 => j = self.store(j, e, depth + 1)?.1,
                // arithmetic and And, Or, XOr kept for their targets: run for those and the value dropped
                0x72 | 0x74..=0x7B | 0x7D | 0x7F | 0x80 | 0x85 => j = self.term(j, e, depth + 1)?.1,
                // Break and Continue: the While loop around them takes them
                0xA5 | 0x9F if self.whiles == 0 => return Err(Why::NoWhile),
                0xA5 | 0x9F => {
                    self.flow = if t[j] == 0xA5 { Flow::Break } else { Flow::Continue };
                    return Ok(None);
                }
                // While (predicate) {statements}: the predicate run again before every round
                0xA2 => {
                    let (b, we) = body_of(t, j + 1, e).ok_or(Why::Bad)?;
                    self.whiles += 1;
                    loop {
                        let (c, k) = self.term(b, we, depth + 1)?;
                        if c == 0 {
                            break;
                        }
                        self.loops += 1;
                        if self.loops > EVAL_LOOPS {
                            return Err(Why::Loops);
                        }
                        if let Some(v) = self.list(k, we, depth + 1)? {
                            return Ok(Some(v));
                        }
                        let f = core::mem::replace(&mut self.flow, Flow::Go);
                        if f == Flow::Break {
                            break;
                        }
                    }
                    self.whiles -= 1;
                    j = we;
                }
                0xA0 => {
                    let (b, ie) = body_of(t, j + 1, e).ok_or(Why::Bad)?;
                    let (c, k) = self.term(b, ie, depth + 1)?;
                    let (els, next) = match t.get(ie) {
                        Some(0xA1) if ie < e => {
                            let (eb, ee) = body_of(t, ie + 1, e).ok_or(Why::Bad)?;
                            (Some((eb, ee)), ee)
                        }
                        _ => (None, ie),
                    };
                    let r = match (c != 0, els) {
                        (true, _) => self.list(k, ie, depth + 1)?,
                        (false, Some((eb, ee))) => self.list(eb, ee, depth + 1)?,
                        (false, None) => None,
                    };
                    // a value returned, or a Break or Continue for the loop around this If
                    if r.is_some() || self.flow != Flow::Go {
                        return Ok(r);
                    }
                    j = next;
                }
                // a method call as a statement: run, and what it returns dropped
                x if name_lead(x) => {
                    let (nm, k) = name_string(t, j, e).ok_or(Why::Bad)?;
                    let m = self.callee(&nm).filter(|_| nm.n > 0).ok_or(Why::Op(x))?;
                    j = self.call(m, k, e, depth)?.1;
                }
                x => return Err(Why::Op(x)),
            }
        }
        Ok(None)
    }
}

/// Run a method body (after its flags byte) declared at `scope` (the method's own path), reading embedded
/// controller fields through `read`. Nothing is written anywhere.
pub fn eval_method(w: &Walk, scope: &[[u8; 4]], body: &[u8], read: &mut FieldRead<'_>) -> Result<Val, Why> {
    let mut ev = Eval { w, scope, t: body, read, reads: 0, locals: Default::default(), args: Default::default(), calls: 0, named: Vec::new(), loops: 0, whiles: 0, flow: Flow::Go };
    ev.list(0, body.len(), 0)?.ok_or(Why::NoReturn)
}

/// The same, for a method that must give a number (an adapter's _PSR).
pub fn eval_int(w: &Walk, scope: &[[u8; 4]], body: &[u8], read: &mut FieldRead<'_>) -> Result<u64, Why> {
    match eval_method(w, scope, body, read)? {
        Val::Int(v) => Ok(v),
        _ => Err(Why::Pkg),
    }
}

/// Read one field from the embedded controller that holds region `k`, through its ports from its _CRS.
pub fn ec_field(w: &Walk, k: usize, u: &Unit, io: &mut dyn EcIo) -> Result<u64, Why> {
    let dev = w.regions.get(k).and_then(|(rp, _)| w.devs.iter().find(|d| d.kind == Kind::Ec && rp.starts_with(&d.path)));
    let ports = dev.and_then(|d| match d.crs {
        Crs::Ports(p, 2) if p[0] != p[1] && !ec_port_refused(p[0]) && !ec_port_refused(p[1]) => Some((p, d.glk)),
        _ => None,
    });
    let Some((p, glk)) = ports else { return Err(Why::NoController(u.name)) };
    io.device(glk).map_err(|e| Why::Read(u.name, e))?;
    let (v, _, stop) = ec_read_units(io, p[0], p[1], core::slice::from_ref(u));
    match (v.first(), stop) {
        (Some(EcVal::Value(x)), _) => Ok(*x),
        (_, Some(e)) => Err(Why::Read(u.name, e)),
        _ => Err(Why::Wide(u.name)),
    }
}

/// A four-letter AML name as text.
pub fn seg_str(s: &[u8; 4]) -> &str {
    core::str::from_utf8(s).unwrap_or("????")
}

/// What a battery's _BIF or _BIX gives that `battery ec` shows.
#[derive(Debug, PartialEq, Eq)]
pub struct Info {
    /// the rate's and the capacity's units: ("mW", "mWh") or ("mA", "mAh")
    pub units: (&'static str, &'static str),
    pub design: u64,
    pub full: u64,
    pub volts: u64,
    /// _BIX only
    pub cycles: Option<u64>,
    /// model, serial, type
    pub text: [Vec<u8>; 3],
}

/// The fields of a _BIF Package (13 elements) or a _BIX Package (20 or more, a revision first), or None when the
/// Package is not the shape the ACPI spec gives (10.2.2.1 and 10.2.2.2).
pub fn info_from(bix: bool, p: &[El]) -> Option<Info> {
    let o = usize::from(bix);
    if p.len() < if bix { 20 } else { 13 } {
        return None;
    }
    let n = |i: usize| match p.get(i) {
        Some(El::Int(v)) => Some(*v),
        _ => None,
    };
    let t = |i: usize| match p.get(i) {
        Some(El::Str(s)) => Some(s.clone()),
        _ => None,
    };
    let units = match n(o)? {
        0 => ("mW", "mWh"),
        1 => ("mA", "mAh"),
        _ => return None,
    };
    let s0 = if bix { 16 } else { 9 };
    Some(Info {
        units,
        design: n(o + 1)?,
        full: n(o + 2)?,
        volts: n(o + 4)?,
        cycles: if bix { Some(n(8)?) } else { None },
        text: [t(s0)?, t(s0 + 1)?, t(s0 + 2)?],
    })
}

/// The charge as a percentage of the last full charge, when both are known (above 100 shows as 100).
pub fn charge_pct(remaining: u64, full: u64) -> Option<u64> {
    if remaining == 0xFFFF_FFFF || full == 0 || full == 0xFFFF_FFFF {
        return None;
    }
    Some((remaining.saturating_mul(100) / full).min(100))
}

/// Run the battery method `name` with body `body`, declared in device `d`.
pub fn run_bat(w: &Walk, d: &Dev, name: [u8; 4], body: &[u8], read: &mut FieldRead<'_>) -> Result<Val, Why> {
    let mut scope = d.path.clone();
    scope.push(name);
    eval_method(w, &scope, body, read)
}

/// A battery's _BIX or _BIF: its name and what it gives, run read only when it is a method. _BIX is used when the
/// battery has both. None when it has neither.
pub fn info_of(w: &Walk, d: &Dev, read: &mut FieldRead<'_>) -> Option<([u8; 4], Result<Val, Why>)> {
    if let Some((bix, body)) = &d.bif {
        let n = if *bix { *b"_BIX" } else { *b"_BIF" };
        return Some((n, run_bat(w, d, n, body, read)));
    }
    for n in [*b"_BIX", *b"_BIF"] {
        let mut p = d.path.clone();
        p.push(n);
        if let Some((_, v)) = w.pkgs.iter().find(|(q, _)| *q == p) {
            return Some((n, Ok(Val::Pkg(v.clone()))));
        }
    }
    None
}

/// A pretend controller for the selftest: 256 bytes of memory, the ACPI read handshake with a few polls of delay,
/// and the ways a real one can fail (nothing there, stuck busy, a byte left for another reader).
pub struct MockEc {
    pub ram: [u8; 256],
    pub dead: bool,
    pub stuck: bool,
    pub armed: bool,
    pub ans: Option<u8>,
    pub busy: u32,
    pub writes: u32,
    pub bad: bool,
    /// after this many answers taken, a stale byte appears in the output buffer (another reader's)
    pub stale_after: Option<u32>,
    pub answered: u32,
}

impl MockEc {
    pub fn new() -> Self {
        MockEc { ram: [0; 256], dead: false, stuck: false, armed: false, ans: None, busy: 0, writes: 0, bad: false, stale_after: None, answered: 0 }
    }
}

impl EcIo for MockEc {
    fn inb(&mut self, port: u16) -> u8 {
        match port {
            0x66 => {
                if self.dead {
                    return 0xFF;
                }
                let ibf = self.stuck || self.busy > 0;
                self.busy = self.busy.saturating_sub(1);
                (ibf as u8) << 1 | (self.ans.is_some() && !ibf) as u8
            }
            0x62 => match self.ans.take() {
                Some(x) => {
                    self.answered += 1;
                    if self.stale_after == Some(self.answered) {
                        self.ans = Some(0xEE); // a byte for another reader, left in the output buffer
                    }
                    x
                }
                None => {
                    self.bad = true;
                    0
                }
            },
            _ => {
                self.bad = true;
                0xFF
            }
        }
    }
    fn outb(&mut self, port: u16, v: u8) {
        self.writes += 1;
        match (port, self.armed) {
            (0x66, false) if v == EC_READ => {
                self.armed = true;
                self.busy = 3;
            }
            (0x62, true) => {
                self.armed = false;
                self.ans = Some(self.ram[v as usize]);
                self.busy = 2;
            }
            _ => self.bad = true,
        }
    }
}
pub const BAT: [u8; 4] = [0x41, 0xD0, 0x0C, 0x0A]; // PNP0C0A
pub const LID: [u8; 4] = [0x41, 0xD0, 0x0C, 0x0D]; // PNP0C0D
pub const EC: [u8; 4] = [0x41, 0xD0, 0x0C, 0x09]; // PNP0C09
