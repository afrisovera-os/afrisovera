//! Nia's load-time validator (Nia speed step 2a, NIA_STEP2_PLAN.md; architect's OK 14 Nov 2026).
//!
//! It checks a whole module once, before any of it runs, the way the WebAssembly 1.0 specification's validation
//! algorithm does: an operand stack of value types and a control stack of frames, with the "unreachable"
//! (polymorphic) stack after `br`, `br_table`, `return` and `unreachable`. It checks types, operand stack heights,
//! block nesting and block types, branch depths, local, global, function and type indexes, memory alignment
//! immediates, LEB128 encodings, section order and sizes, and that every body ends exactly at its `end`.
//!
//! Every refusal is one of two kinds:
//! - **invalid:** the module breaks the WebAssembly 1.0 rules (any validator, wasmparser included, refuses it);
//! - **limit:** the module is valid WebAssembly but needs something Nia does not have: more than Nia's limits
//!   (types, functions, imports, globals, locals, operand height, nesting), an instruction Nia does not run
//!   (`call_indirect` among them), an import that is not a function, or a start function.
//!   The differential test against wasmparser skips limit refusals.
//!
//! Step 2a order: this file first reports only (parse prints a line and refuses nothing). It refuses once it has
//! been fuzzed against wasmparser, together with nia.py's check in nia-apps.
//!
//! Pure: it reads only the bytes it is given and Nia's limits (`super::`). No `crate::` paths, no allocation, no
//! `unsafe`, no indexing that can panic on any input (every array access is checked first), so fuzz-host compiles
//! it unchanged.

use super::{CTRL_CAP, LOCALS_CAP, MAX_FUNCS, MAX_GLOBALS, MAX_IMPORTS, MAX_TYPES, STACK_CAP};

pub const I32: u8 = 0x7F;
pub const I64: u8 = 0x7E;
pub const F32: u8 = 0x7D;
pub const F64: u8 = 0x7C;
/// A type the polymorphic stack makes up after an unconditional branch; it matches any type.
const ANY: u8 = 0;
/// "No value" for a block or function result.
const NONE: u8 = 0;

/// Exports a module may have (a Nia limit, so the duplicate-name check stays small).
pub const MAX_EXPORTS: usize = 64;
/// Parameters an imported (host) function may take: Nia's host calls read at most 8.
pub const MAX_HOST_PARAMS: usize = 8;

/// Why a module was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Why {
    pub what: &'static str,
    /// true: valid WebAssembly that needs something Nia does not have. false: not valid WebAssembly 1.0.
    pub limit: bool,
}

/// A refusal: where and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fail {
    /// The function (in the function index space, imports first), when the refusal is inside a body.
    pub func: Option<u32>,
    /// The byte offset in the module where it was found.
    pub offset: usize,
    pub why: Why,
}

impl core::fmt::Display for Fail {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let kind = if self.why.limit { "Nia limit" } else { "invalid" };
        match self.func {
            Some(i) => write!(f, "function {}, byte {}: {} ({})", i, self.offset, self.why.what, kind),
            None => write!(f, "byte {}: {} ({})", self.offset, self.why.what, kind),
        }
    }
}

/// What a module that passes looks like, for the step 2a sizing (side table, shared stack).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Defined functions (imports not counted).
    pub funcs: usize,
    /// Side-table entries the module needs: one per `if`, `else`, `br` and `br_if`, and one per `br_table` label
    /// (default included). The same count as the sizing script in NIA_STEP2_PLAN.md.
    pub side_entries: usize,
    /// The most side-table entries in one function.
    pub side_entries_max_fn: usize,
    /// The largest operand stack height in any function.
    pub max_height: usize,
    /// The most locals (parameters included) in any function.
    pub max_locals: usize,
    /// The largest frame (locals plus largest height) in any function.
    pub max_frame: usize,
}

type Res<T> = Result<T, Why>;

fn bad<T>(what: &'static str) -> Res<T> {
    Err(Why { what, limit: false })
}
fn lim<T>(what: &'static str) -> Res<T> {
    Err(Why { what, limit: true })
}

/// A reader over `b[..end]`. `p` is where the next byte is; after a refusal it is where the refusal was found.
struct R<'a> {
    b: &'a [u8],
    p: usize,
    end: usize,
}

impl<'a> R<'a> {
    fn byte(&mut self) -> Res<u8> {
        if self.p >= self.end {
            return bad("unexpected end");
        }
        match self.b.get(self.p) {
            Some(&v) => {
                self.p += 1;
                Ok(v)
            }
            None => bad("unexpected end"),
        }
    }

    /// An unsigned LEB128 u32 as the specification has it: at most 5 bytes, the unused bits of the last byte zero.
    fn u32(&mut self) -> Res<u32> {
        let mut r: u32 = 0;
        for i in 0..5u32 {
            let b = self.byte()?;
            if i == 4 && b & 0xF0 != 0 {
                return if b & 0x80 != 0 { bad("integer representation too long") } else { bad("integer too large") };
            }
            r |= u32::from(b & 0x7F) << (7 * i);
            if b & 0x80 == 0 {
                return Ok(r);
            }
        }
        bad("integer representation too long")
    }

    fn count(&mut self) -> Res<usize> {
        self.u32().map(|v| v as usize)
    }

    /// A signed LEB128 of `bits` (32 or 64): at most ceil(bits / 7) bytes, the unused bits of the last byte a copy
    /// of the sign.
    fn sleb(&mut self, bits: u32) -> Res<i64> {
        let max = (bits + 6) / 7;
        let mut r: i64 = 0;
        let mut shift: u32 = 0;
        for i in 0..max {
            let b = self.byte()?;
            if i == max - 1 {
                if b & 0x80 != 0 {
                    return bad("integer representation too long");
                }
                // The last byte's 7 bits, sign-extended, must fit in the bits that are left.
                let used = bits - shift;
                let sv = i64::from(((b << 1) as i8) >> 1);
                let lo = -(1i64 << (used - 1));
                let hi = 1i64 << (used - 1);
                if sv < lo || sv >= hi {
                    return bad("integer too large");
                }
                return Ok(r | (sv << shift));
            }
            r |= i64::from(b & 0x7F) << shift;
            shift += 7;
            if b & 0x80 == 0 {
                if b & 0x40 != 0 {
                    r |= -1i64 << shift;
                }
                return Ok(r);
            }
        }
        bad("integer representation too long")
    }

    fn skip(&mut self, n: usize) -> Res<()> {
        match self.p.checked_add(n) {
            Some(e) if e <= self.end => {
                self.p = e;
                Ok(())
            }
            _ => bad("unexpected end"),
        }
    }

    /// A name: a byte count, then that many bytes of UTF-8. Returns where the bytes are.
    fn name(&mut self) -> Res<(usize, usize)> {
        let n = self.count()?;
        let at = self.p;
        self.skip(n)?;
        match self.b.get(at..at + n) {
            Some(s) if core::str::from_utf8(s).is_ok() => Ok((at, n)),
            Some(_) => bad("malformed UTF-8 encoding"),
            None => bad("unexpected end"),
        }
    }

    fn valtype(&mut self) -> Res<u8> {
        match self.byte()? {
            t @ (I32 | I64 | F32 | F64) => Ok(t),
            _ => bad("invalid value type"),
        }
    }
}

#[derive(Clone, Copy)]
struct FnType {
    np: u8,
    params: [u8; LOCALS_CAP],
    result: u8, // NONE or a value type
}
const FNTYPE_EMPTY: FnType = FnType { np: 0, params: [NONE; LOCALS_CAP], result: NONE };

/// What the module-level sections declared, for checking the bodies.
struct Decl {
    types: [FnType; MAX_TYPES],
    ntypes: usize,
    // The function index space: imported functions first, then defined ones. Each holds its type index.
    func_types: [u8; MAX_IMPORTS + MAX_FUNCS],
    nimports: usize,
    nfuncs: usize, // defined functions, from the function section
    globals: [(u8, bool); MAX_GLOBALS], // (type, mutable)
    nglobals: usize,
    has_table: bool,
    has_mem: bool,
}

impl Decl {
    fn total_funcs(&self) -> usize {
        self.nimports + self.nfuncs
    }
    fn func_type(&self, idx: usize) -> Res<FnType> {
        if idx >= self.total_funcs() {
            return bad("unknown function");
        }
        let ti = match self.func_types.get(idx) {
            Some(&t) => usize::from(t),
            None => return bad("unknown function"),
        };
        match self.types.get(ti) {
            Some(&t) if ti < self.ntypes => Ok(t),
            _ => bad("unknown type"),
        }
    }
}

/// Validates the whole module. Ok: the module may run on Nia, with what it needs. Err: why not, and where.
pub fn validate(m: &[u8]) -> Result<Report, Fail> {
    let mut r = R { b: m, p: 0, end: m.len() };
    let mut func: Option<u32> = None;
    let mut rep = Report::default();
    match module(&mut r, &mut func, &mut rep) {
        Ok(()) => Ok(rep),
        Err(why) => Err(Fail { func, offset: r.p, why }),
    }
}

fn module(r: &mut R, func: &mut Option<u32>, rep: &mut Report) -> Res<()> {
    let m = r.b;
    if m.len() < 4 || m.get(0..4) != Some(b"\0asm".as_slice()) {
        return bad("magic header not detected");
    }
    if m.len() < 8 || m.get(4..8) != Some([1u8, 0, 0, 0].as_slice()) {
        r.p = 4;
        return bad("unknown binary version");
    }
    r.p = 8;
    let mut d = Decl {
        types: [FNTYPE_EMPTY; MAX_TYPES],
        ntypes: 0,
        func_types: [0; MAX_IMPORTS + MAX_FUNCS],
        nimports: 0,
        nfuncs: 0,
        globals: [(I32, false); MAX_GLOBALS],
        nglobals: 0,
        has_table: false,
        has_mem: false,
    };
    let mut last_id = 0u8;
    let mut ncode: Option<usize> = None;
    while r.p < m.len() {
        let id = r.byte()?;
        let size = r.count()?;
        let start = r.p;
        let end = match start.checked_add(size) {
            Some(e) if e <= m.len() => e,
            _ => return bad("section size mismatch: past the end of the module"),
        };
        if id != 0 {
            if id > 11 {
                return if id == 12 { bad("data count section (not WebAssembly 1.0)") } else { bad("malformed section id") };
            }
            if id <= last_id {
                return bad("section out of order or repeated");
            }
            last_id = id;
        }
        r.end = end;
        match id {
            0 => {
                r.name()?;
                r.p = end; // a custom section's contents are its own business (parse reads afri.permissions)
            }
            1 => type_section(r, &mut d)?,
            2 => import_section(r, &mut d)?,
            3 => {
                let n = r.count()?;
                if n > MAX_FUNCS {
                    return lim("more than 128 functions");
                }
                for i in 0..n {
                    let ti = r.count()?;
                    if ti >= d.ntypes {
                        return bad("unknown type");
                    }
                    // ti < MAX_TYPES (32) fits a u8; the slot exists (imports <= 16, i < 128).
                    match d.func_types.get_mut(d.nimports + i) {
                        Some(s) => *s = ti as u8,
                        None => return lim("more than 128 functions"),
                    }
                }
                d.nfuncs = n;
            }
            4 => {
                let n = r.count()?;
                if n > 1 {
                    return bad("multiple tables");
                }
                for _ in 0..n {
                    if r.byte()? != 0x70 {
                        return bad("invalid table element type");
                    }
                    limits(r, u32::MAX as u64)?;
                    d.has_table = true;
                }
            }
            5 => {
                let n = r.count()?;
                if n > 1 {
                    return bad("multiple memories");
                }
                for _ in 0..n {
                    limits(r, 65536)?;
                    d.has_mem = true;
                }
            }
            6 => {
                let n = r.count()?;
                if n > MAX_GLOBALS {
                    return lim("more than 16 globals");
                }
                for i in 0..n {
                    let t = r.valtype()?;
                    let mutable = match r.byte()? {
                        0 => false,
                        1 => true,
                        _ => return bad("malformed mutability"),
                    };
                    const_expr(r, &d, t)?;
                    if let Some(g) = d.globals.get_mut(i) {
                        *g = (t, mutable);
                    }
                    d.nglobals = i + 1;
                }
            }
            7 => export_section(r, &d)?,
            8 => {
                let f = r.count()?;
                let t = d.func_type(f)?;
                if t.np != 0 || t.result != NONE {
                    return bad("start function must take and return nothing");
                }
                return lim("a start function (Nia does not run one)");
            }
            9 => {
                let n = r.count()?;
                for _ in 0..n {
                    if r.count()? != 0 || !d.has_table {
                        return bad("unknown table");
                    }
                    const_expr(r, &d, I32)?;
                    let k = r.count()?;
                    for _ in 0..k {
                        let f = r.count()?;
                        if f >= d.total_funcs() {
                            return bad("unknown function");
                        }
                    }
                }
            }
            10 => {
                let n = r.count()?;
                if n != d.nfuncs {
                    return bad("function and code section have inconsistent lengths");
                }
                for i in 0..n {
                    let size = r.count()?;
                    let bstart = r.p;
                    let bend = match bstart.checked_add(size) {
                        Some(e) if e <= end => e,
                        _ => return bad("function body past the end of the code section"),
                    };
                    let fidx = d.nimports + i;
                    *func = Some(fidx as u32);
                    r.end = bend;
                    body(r, &d, fidx, rep)?;
                    if r.p != bend {
                        return bad("operators remaining after the function's end");
                    }
                    r.end = end;
                    *func = None;
                }
                rep.funcs = n;
                ncode = Some(n);
            }
            11 => {
                let n = r.count()?;
                for _ in 0..n {
                    if r.count()? != 0 || !d.has_mem {
                        return bad("unknown memory");
                    }
                    const_expr(r, &d, I32)?;
                    let len = r.count()?;
                    r.skip(len)?;
                }
            }
            _ => return bad("malformed section id"),
        }
        if r.p != end {
            return bad("section size mismatch");
        }
        r.end = m.len();
    }
    if ncode.unwrap_or(0) != d.nfuncs {
        return bad("function and code section have inconsistent lengths");
    }
    Ok(())
}

fn limits(r: &mut R, max_allowed: u64) -> Res<()> {
    let flag = r.byte()?;
    let min = u64::from(r.u32()?);
    if min > max_allowed {
        return bad("size minimum too large");
    }
    match flag {
        0 => Ok(()),
        1 => {
            let max = u64::from(r.u32()?);
            if max > max_allowed {
                return bad("size maximum too large");
            }
            if min > max {
                return bad("size minimum must not be greater than maximum");
            }
            Ok(())
        }
        _ => bad("invalid limits flags"),
    }
}

/// A constant expression of type `t`: one `t.const`, then `end`. (WebAssembly 1.0 also allows `global.get` of an
/// imported global; Nia imports only functions, so there is none to refer to.)
fn const_expr(r: &mut R, d: &Decl, t: u8) -> Res<()> {
    let op = r.byte()?;
    let got = match op {
        0x41 => {
            r.sleb(32)?;
            I32
        }
        0x42 => {
            r.sleb(64)?;
            I64
        }
        0x43 => {
            r.skip(4)?;
            F32
        }
        0x44 => {
            r.skip(8)?;
            F64
        }
        0x23 => {
            let g = r.count()?;
            return if g < d.nglobals { bad("constant expression required") } else { bad("unknown global") };
        }
        _ => return bad("constant expression required"),
    };
    if got != t {
        return bad("type mismatch in constant expression");
    }
    if r.byte()? != 0x0B {
        return bad("constant expression required");
    }
    Ok(())
}

fn type_section(r: &mut R, d: &mut Decl) -> Res<()> {
    let n = r.count()?;
    if n > MAX_TYPES {
        return lim("more than 32 types");
    }
    for i in 0..n {
        if r.byte()? != 0x60 {
            return bad("malformed function type");
        }
        let np = r.count()?;
        if np > LOCALS_CAP {
            return lim("a function type with more than 32 parameters");
        }
        let mut t = FNTYPE_EMPTY;
        for k in 0..np {
            let v = r.valtype()?;
            if let Some(s) = t.params.get_mut(k) {
                *s = v;
            }
        }
        t.np = np as u8; // np <= 32
        let nr = r.count()?;
        if nr > 1 {
            return bad("multiple results (multi-value is not WebAssembly 1.0)");
        }
        if nr == 1 {
            t.result = r.valtype()?;
        }
        if let Some(s) = d.types.get_mut(i) {
            *s = t;
        }
    }
    d.ntypes = n;
    Ok(())
}

fn import_section(r: &mut R, d: &mut Decl) -> Res<()> {
    let n = r.count()?;
    for _ in 0..n {
        let (ma, ml) = r.name()?;
        r.name()?;
        let module = r.b.get(ma..ma + ml).unwrap_or(&[]);
        let kind = r.byte()?;
        match kind {
            0 => {
                let ti = r.count()?;
                if ti >= d.ntypes {
                    return bad("unknown type");
                }
                if module != b"afri" && module != b"env" {
                    return lim("an import from a module Nia does not offer");
                }
                if d.nimports >= MAX_IMPORTS {
                    return lim("more than 16 imported functions");
                }
                let np = d.types.get(ti).map_or(0, |t| usize::from(t.np));
                if np > MAX_HOST_PARAMS {
                    return lim("an imported function with more than 8 parameters");
                }
                if let Some(s) = d.func_types.get_mut(d.nimports) {
                    *s = ti as u8; // ti < 32
                }
                d.nimports += 1;
            }
            1..=3 => return lim("an import that is not a function (Nia imports only functions)"),
            _ => return bad("malformed import kind"),
        }
    }
    Ok(())
}

fn export_section(r: &mut R, d: &Decl) -> Res<()> {
    let n = r.count()?;
    if n > MAX_EXPORTS {
        return lim("more than 64 exports");
    }
    let mut seen = [(0usize, 0usize); MAX_EXPORTS];
    for i in 0..n {
        let (at, len) = r.name()?;
        let name = r.b.get(at..at + len).unwrap_or(&[]);
        for &(pa, pl) in seen.iter().take(i) {
            if r.b.get(pa..pa + pl) == Some(name) {
                return bad("duplicate export name");
            }
        }
        if let Some(s) = seen.get_mut(i) {
            *s = (at, len);
        }
        let kind = r.byte()?;
        let idx = r.count()?;
        let ok = match kind {
            0 => idx < d.total_funcs(),
            1 => idx == 0 && d.has_table,
            2 => idx == 0 && d.has_mem,
            3 => idx < d.nglobals,
            _ => return bad("malformed export kind"),
        };
        if !ok {
            return bad("unknown export index");
        }
    }
    Ok(())
}

// ---- function bodies ----

const K_FUNC: u8 = 0;
const K_BLOCK: u8 = 1;
const K_LOOP: u8 = 2;
const K_IF: u8 = 3;
const K_ELSE: u8 = 4;

#[derive(Clone, Copy)]
struct Frame {
    kind: u8,
    result: u8, // NONE or a value type
    height: usize,
    unreachable: bool,
}
const FRAME_EMPTY: Frame = Frame { kind: K_FUNC, result: NONE, height: 0, unreachable: false };

/// The control frames: the function's own, then up to CTRL_CAP nested blocks (Nia's own nesting limit).
const FRAMES: usize = CTRL_CAP + 1;

struct V {
    ops: [u8; STACK_CAP],
    sp: usize,
    max_sp: usize,
    ctrl: [Frame; FRAMES],
    cp: usize, // frames in use (>= 1 while the body runs)
}

impl V {
    fn top(&self) -> Res<Frame> {
        match self.cp.checked_sub(1).and_then(|i| self.ctrl.get(i)) {
            Some(&f) => Ok(f),
            None => bad("operators remaining after the function's end"),
        }
    }

    fn push(&mut self, t: u8) -> Res<()> {
        match self.ops.get_mut(self.sp) {
            Some(s) => *s = t,
            None => return lim("operands deeper than 256 values"),
        }
        self.sp += 1;
        if self.sp > self.max_sp {
            self.max_sp = self.sp;
        }
        Ok(())
    }

    fn pop(&mut self) -> Res<u8> {
        let f = self.top()?;
        if self.sp == f.height {
            return if f.unreachable { Ok(ANY) } else { bad("type mismatch: a value is missing") };
        }
        self.sp -= 1;
        Ok(self.ops.get(self.sp).copied().unwrap_or(ANY))
    }

    fn pop_t(&mut self, want: u8) -> Res<()> {
        let got = self.pop()?;
        if got != want && got != ANY && want != ANY {
            return bad("type mismatch");
        }
        Ok(())
    }

    fn push_frame(&mut self, kind: u8, result: u8) -> Res<()> {
        let height = self.sp;
        match self.ctrl.get_mut(self.cp) {
            Some(s) => *s = Frame { kind, result, height, unreachable: false },
            None => return lim("blocks nested deeper than 32"),
        }
        self.cp += 1;
        Ok(())
    }

    /// The frame's results are on the stack and nothing else above its height.
    fn check_end(&mut self, f: Frame) -> Res<()> {
        if f.result != NONE {
            self.pop_t(f.result)?;
        }
        if self.sp != f.height {
            return bad("type mismatch: values left on the stack at the end of a block");
        }
        Ok(())
    }

    fn set_unreachable(&mut self) -> Res<()> {
        let i = self.cp.checked_sub(1).ok_or(Why { what: "no open block", limit: false })?;
        match self.ctrl.get_mut(i) {
            Some(f) => {
                self.sp = f.height;
                f.unreachable = true;
                Ok(())
            }
            None => bad("no open block"),
        }
    }

    /// The value a branch to label `l` carries (a loop's label carries none in WebAssembly 1.0).
    fn label_type(&self, l: usize) -> Res<u8> {
        if l >= self.cp {
            return bad("unknown label: branch depth too large");
        }
        match self.ctrl.get(self.cp - 1 - l) {
            Some(f) if f.kind == K_LOOP => Ok(NONE),
            Some(f) => Ok(f.result),
            None => bad("unknown label: branch depth too large"),
        }
    }
}

fn blocktype(r: &mut R) -> Res<u8> {
    match r.byte()? {
        0x40 => Ok(NONE),
        t @ (I32 | I64 | F32 | F64) => Ok(t),
        _ => bad("invalid block type (a type index is multi-value, not WebAssembly 1.0)"),
    }
}

fn memarg(r: &mut R, d: &Decl, natural: u32) -> Res<()> {
    let align = r.u32()?;
    r.u32()?; // offset
    if !d.has_mem {
        return bad("unknown memory 0");
    }
    if align > natural {
        return bad("alignment must not be larger than natural");
    }
    Ok(())
}

fn body(r: &mut R, d: &Decl, fidx: usize, rep: &mut Report) -> Res<()> {
    let ft = d.func_type(fidx)?;
    let mut locals = [NONE; LOCALS_CAP];
    let mut nlocals = usize::from(ft.np);
    for k in 0..nlocals {
        if let (Some(s), Some(&p)) = (locals.get_mut(k), ft.params.get(k)) {
            *s = p;
        }
    }
    let groups = r.count()?;
    for _ in 0..groups {
        let n = r.count()?;
        let t = r.valtype()?;
        if n > LOCALS_CAP - nlocals {
            return lim("more than 32 locals (parameters included)");
        }
        for k in nlocals..nlocals + n {
            if let Some(s) = locals.get_mut(k) {
                *s = t;
            }
        }
        nlocals += n;
    }

    let mut v = V { ops: [ANY; STACK_CAP], sp: 0, max_sp: 0, ctrl: [FRAME_EMPTY; FRAMES], cp: 0 };
    v.push_frame(K_FUNC, ft.result)?;
    let mut entries = 0usize;

    while v.cp > 0 {
        let op = r.byte()?;
        match op {
            0x00 => v.set_unreachable()?,
            0x01 => {}
            0x02 | 0x03 => {
                let bt = blocktype(r)?;
                v.push_frame(if op == 0x02 { K_BLOCK } else { K_LOOP }, bt)?;
            }
            0x04 => {
                let bt = blocktype(r)?;
                v.pop_t(I32)?;
                v.push_frame(K_IF, bt)?;
                entries += 1;
            }
            0x05 => {
                let f = v.top()?;
                if f.kind != K_IF {
                    return bad("else without a matching if");
                }
                v.check_end(f)?;
                if let Some(t) = v.ctrl.get_mut(v.cp - 1) {
                    t.kind = K_ELSE;
                    t.unreachable = false;
                }
                v.sp = f.height;
                entries += 1;
            }
            0x0B => {
                let f = v.top()?;
                if f.kind == K_IF && f.result != NONE {
                    return bad("type mismatch: an if with a result needs an else");
                }
                v.check_end(f)?;
                v.cp -= 1;
                if f.kind != K_FUNC && f.result != NONE {
                    v.push(f.result)?;
                }
            }
            0x0C => {
                let l = r.count()?;
                let t = v.label_type(l)?;
                if t != NONE {
                    v.pop_t(t)?;
                }
                v.set_unreachable()?;
                entries += 1;
            }
            0x0D => {
                let l = r.count()?;
                let t = v.label_type(l)?;
                v.pop_t(I32)?;
                if t != NONE {
                    v.pop_t(t)?;
                    v.push(t)?;
                }
                entries += 1;
            }
            0x0E => {
                let n = r.count()?;
                // Each label is at least one byte: a count the body cannot hold is refused before the loop.
                if n > r.end.saturating_sub(r.p) {
                    return bad("unexpected end");
                }
                let mut first: Option<u8> = None;
                for _ in 0..=n {
                    let l = r.count()?;
                    let t = v.label_type(l)?;
                    match first {
                        None => first = Some(t),
                        Some(f) if f != t => return bad("type mismatch: br_table labels of different types"),
                        _ => {}
                    }
                }
                v.pop_t(I32)?;
                if let Some(t) = first {
                    if t != NONE {
                        v.pop_t(t)?;
                    }
                }
                v.set_unreachable()?;
                entries += n + 1;
            }
            0x0F => {
                if ft.result != NONE {
                    v.pop_t(ft.result)?;
                }
                v.set_unreachable()?;
            }
            0x10 => {
                let f = r.count()?;
                let t = d.func_type(f)?;
                for k in (0..usize::from(t.np)).rev() {
                    v.pop_t(t.params.get(k).copied().unwrap_or(ANY))?;
                }
                if t.result != NONE {
                    v.push(t.result)?;
                }
            }
            0x11 => {
                r.count()?;
                r.byte()?;
                return lim("call_indirect (Nia does not run it)");
            }
            0x1A => {
                v.pop()?;
            }
            0x1B => {
                v.pop_t(I32)?;
                let b = v.pop()?;
                let a = v.pop()?;
                if a != b && a != ANY && b != ANY {
                    return bad("type mismatch: select operands differ");
                }
                v.push(if a == ANY { b } else { a })?;
            }
            0x20..=0x22 => {
                let i = r.count()?;
                if i >= nlocals {
                    return bad("unknown local");
                }
                let t = locals.get(i).copied().unwrap_or(ANY);
                match op {
                    0x20 => v.push(t)?,
                    0x21 => v.pop_t(t)?,
                    _ => {
                        v.pop_t(t)?;
                        v.push(t)?;
                    }
                }
            }
            0x23 | 0x24 => {
                let i = r.count()?;
                let (t, mutable) = match d.globals.get(i) {
                    Some(&g) if i < d.nglobals => g,
                    _ => return bad("unknown global"),
                };
                if op == 0x23 {
                    v.push(t)?;
                } else {
                    if !mutable {
                        return bad("global is immutable");
                    }
                    v.pop_t(t)?;
                }
            }
            // loads Nia runs: (natural alignment, result type)
            0x28 | 0x29 | 0x2C..=0x35 => {
                let (natural, t) = match op {
                    0x28 => (2, I32),
                    0x29 => (3, I64),
                    0x2C | 0x2D => (0, I32),
                    0x2E | 0x2F => (1, I32),
                    0x30 | 0x31 => (0, I64),
                    0x32 | 0x33 => (1, I64),
                    _ => (2, I64), // 0x34, 0x35
                };
                memarg(r, d, natural)?;
                v.pop_t(I32)?;
                v.push(t)?;
            }
            // stores Nia runs: (natural alignment, value type)
            0x36 | 0x37 | 0x3A..=0x3E => {
                let (natural, t) = match op {
                    0x36 => (2, I32),
                    0x37 => (3, I64),
                    0x3A => (0, I32),
                    0x3B => (1, I32),
                    0x3C => (0, I64),
                    0x3D => (1, I64),
                    _ => (2, I64), // 0x3E
                };
                memarg(r, d, natural)?;
                v.pop_t(t)?;
                v.pop_t(I32)?;
            }
            0x2A | 0x2B | 0x38 | 0x39 => {
                // f32.load, f64.load, f32.store, f64.store: valid, but Nia does not run them
                let natural = if op == 0x2A || op == 0x38 { 2 } else { 3 };
                memarg(r, d, natural)?;
                return lim("a float load or store (Nia does not run it)");
            }
            0x3F | 0x40 => {
                if r.byte()? != 0 {
                    return bad("zero byte expected");
                }
                if !d.has_mem {
                    return bad("unknown memory 0");
                }
                if op == 0x40 {
                    v.pop_t(I32)?;
                }
                v.push(I32)?;
            }
            0x41 => {
                r.sleb(32)?;
                v.push(I32)?;
            }
            0x42 => {
                r.sleb(64)?;
                v.push(I64)?;
            }
            0x43 => {
                r.skip(4)?;
                v.push(F32)?;
            }
            0x44 => {
                r.skip(8)?;
                v.push(F64)?;
            }
            0x45 => un(&mut v, I32, I32)?,
            0x46..=0x4F => bin(&mut v, I32, I32)?,
            0x50 => un(&mut v, I64, I32)?,
            0x51..=0x5A => bin(&mut v, I64, I32)?,
            0x5B | 0x5D | 0x5E => bin(&mut v, F32, I32)?,
            0x61 | 0x63 | 0x64 => bin(&mut v, F64, I32)?,
            0x6A..=0x78 => bin(&mut v, I32, I32)?,
            0x7C..=0x8A => bin(&mut v, I64, I64)?,
            0x92..=0x95 => bin(&mut v, F32, F32)?,
            0xA0..=0xA3 => bin(&mut v, F64, F64)?,
            0xA7 => un(&mut v, I64, I32)?,
            0xA8 => un(&mut v, F32, I32)?,
            0xAA => un(&mut v, F64, I32)?,
            0xAC | 0xAD => un(&mut v, I32, I64)?,
            0xB2 => un(&mut v, I32, F32)?,
            0xB7 => un(&mut v, I32, F64)?,
            // The rest of WebAssembly 1.0's numeric instructions: valid, but Nia does not run them.
            0x5C | 0x5F | 0x60 | 0x62 | 0x65 | 0x66..=0x69 | 0x79..=0x7B | 0x8B..=0x91 | 0x96..=0x9F | 0xA4..=0xA6
            | 0xA9 | 0xAB | 0xAE..=0xB1 | 0xB3..=0xB6 | 0xB8..=0xBF => {
                return lim("an instruction Nia does not run");
            }
            _ => return bad("illegal opcode"),
        }
    }

    let height = v.max_sp;
    rep.side_entries += entries;
    rep.side_entries_max_fn = rep.side_entries_max_fn.max(entries);
    rep.max_height = rep.max_height.max(height);
    rep.max_locals = rep.max_locals.max(nlocals);
    rep.max_frame = rep.max_frame.max(nlocals + height);
    Ok(())
}

fn un(v: &mut V, a: u8, out: u8) -> Res<()> {
    v.pop_t(a)?;
    v.push(out)
}

fn bin(v: &mut V, a: u8, out: u8) -> Res<()> {
    v.pop_t(a)?;
    v.pop_t(a)?;
    v.push(out)
}
