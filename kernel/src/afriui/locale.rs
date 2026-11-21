//! Each language's ways with numbers, counts and the clock (LANGUAGES_PLAN.md stage 5; the architect's OK of
//! 14 Nov), from Unicode CLDR. The data is a generated table (locale_data.rs, made in the cloud by
//! tools/lang/cldr_src.py from one pinned CLDR release); nothing is parsed at run time.
//!
//! - `plural(l, n)`: which plural form a whole number takes in a language (one, few, many...), by CLDR's cardinal
//!   rules compiled to integer tests. The desk only counts whole things, so CLDR's fraction operands are 0.
//! - `group(l, n, put)`: a number with the language's grouping (65,536 / 65 536 / 65.536).
//! - `memory(l, mib, put)`: a memory size, MiB below 1 GiB, GiB with one decimal from there (bytes / 2^30).
//! - `am(l)`, `pm(l)`: the clock's day halves. `h24(l)`: whether the language's likely region uses a 24-hour clock.

use super::lang::Lang;

/// A plural form (CLDR's categories). `Other` is every language's last resort.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Form {
    Zero,
    One,
    Two,
    Few,
    Many,
    Other,
}

impl Form {
    pub fn name(self) -> &'static str {
        match self {
            Form::Zero => "zero",
            Form::One => "one",
            Form::Two => "two",
            Form::Few => "few",
            Form::Many => "many",
            Form::Other => "other",
        }
    }
    pub fn from_name(s: &[u8]) -> Option<Form> {
        [Form::Zero, Form::One, Form::Two, Form::Few, Form::Many, Form::Other]
            .iter()
            .copied()
            .find(|f| f.name().as_bytes() == s)
    }
}

/// One relation: `n % m` (or `n` when m is 0) is in one of the ranges, or with `ne`, in none of them.
pub struct Cond {
    pub m: u32,
    pub ne: bool,
    pub r: &'static [(u32, u32)],
}

/// A form holds when any of its groups holds, and a group holds when all its relations do.
pub struct Rule {
    pub form: Form,
    pub any: &'static [&'static [Cond]],
}

/// One language's data.
pub struct Loc {
    pub code: &'static str,
    pub group: &'static str,
    pub decimal: &'static str,
    /// Group only numbers of at least 3 + min_group digits (Spanish: 2, so 1000 stays whole, 10.000 is grouped).
    pub min_group: u8,
    pub h24: bool,
    pub am: &'static str,
    pub pm: &'static str,
    pub plural: &'static [Rule],
}

include!("locale_data.rs");

/// A language's data: LOCS follows the order of lang::LANGS, checked by `probe`.
pub fn loc(l: Lang) -> &'static Loc {
    if l.is_test() {
        return &LOCS[0]; // the test language counts and writes numbers like English
    }
    &LOCS[(l as usize).min(LOCS.len() - 1)]
}

fn holds(c: &Cond, n: u64) -> bool {
    let v = if c.m == 0 { n } else { n % c.m as u64 };
    c.r.iter().any(|&(lo, hi)| v >= lo as u64 && v <= hi as u64) != c.ne
}

/// The plural form of the whole number `n` in language `l`.
pub fn plural(l: Lang, n: u64) -> Form {
    for rule in loc(l).plural.iter() {
        if rule.any.iter().any(|g| g.iter().all(|c| holds(c, n))) {
            return rule.form;
        }
    }
    Form::Other
}

/// Does language `l` ever use form `f` for a whole number? (`desk lang check`: a message with forms must have each.)
pub fn uses(l: Lang, f: Form) -> bool {
    f == Form::Other || loc(l).plural.iter().any(|r| r.form == f)
}

/// Write `n` with the language's grouping separator every three digits (when it has enough digits).
pub fn group(l: Lang, n: u64, put: &mut dyn FnMut(&[u8])) {
    let lc = loc(l);
    let mut d = [0u8; 20];
    let mut k = d.len();
    let mut v = n;
    loop {
        k -= 1;
        d[k] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    let digits = &d[k..];
    let len = digits.len();
    if len < 3 + lc.min_group as usize {
        put(digits);
        return;
    }
    let first = match len % 3 {
        0 => 3,
        r => r,
    };
    put(&digits[..first]);
    let mut i = first;
    while i < len {
        put(lc.group.as_bytes());
        put(&digits[i..i + 3]);
        i += 3;
    }
}

/// A memory size given in MiB: "512 MiB" below 1 GiB, else GiB with one decimal ("3,8 GiB" in French), the decimal
/// left out when it is 0 ("16 GiB"). GiB is 2^30 bytes, the same unit everywhere on the desk.
pub fn memory(l: Lang, mib: u64, put: &mut dyn FnMut(&[u8])) {
    if mib < 1024 {
        group(l, mib, put);
        put(b" MiB");
        return;
    }
    let tenths = mib.saturating_mul(10).saturating_add(512) / 1024;
    group(l, tenths / 10, put);
    if tenths % 10 != 0 {
        put(loc(l).decimal.as_bytes());
        put(&[b'0' + (tenths % 10) as u8]);
    }
    put(b" GiB");
}

pub fn am(l: Lang) -> &'static [u8] {
    loc(l).am.as_bytes()
}

pub fn pm(l: Lang) -> &'static [u8] {
    loc(l).pm.as_bytes()
}

/// The language's usual clock: 24-hour, or 12-hour with am and pm (the default of the Settings switch until the
/// person chooses, stage 5b).
pub fn h24(l: Lang) -> bool {
    loc(l).h24
}

/// Check the evaluator against every CLDR sample in the table: (held, total), and the first that failed.
pub fn check_samples() -> (usize, usize, Option<(Lang, u32, Form, Form)>) {
    let langs = super::lang::LANGS;
    let mut ok = 0;
    let mut first = None;
    for &(li, n, want) in SAMPLES.iter() {
        let l = langs[(li as usize).min(langs.len() - 1)];
        let got = plural(l, n as u64);
        if got == want {
            ok += 1;
        } else if first.is_none() {
            first = Some((l, n, want, got));
        }
    }
    (ok, SAMPLES.len(), first)
}

/// Is the table in the order of lang::LANGS?
pub fn order_ok() -> bool {
    LOCS.len() == super::lang::LANGS.len() && super::lang::LANGS.iter().zip(LOCS.iter()).all(|(l, d)| l.code() == d.code)
}
