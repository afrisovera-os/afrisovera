//! Dira's reader (BROWSER_PLAN.md section 4): a streaming HTML tokenizer that turns a page into readable text,
//! each letter with a colour role, plus the page's links. The raw HTML is never kept: bytes are read as they
//! arrive, in any split, and only the text is stored, so a page's size costs time, not memory.
//!
//! Pure code: no kernel calls, no `unsafe`, no allocation (host tests and fuzzing in tools/dira-host).
#![forbid(unsafe_code)]

use super::url::Buf;

/// Letters a page may hold; past that it ends with a line saying it was cut.
pub const LETTERS: usize = 48_000;
/// Places kept at the end of a page for Dira's own closing line.
pub const RESERVE: usize = 128;
pub const POOL: usize = 24 * 1024;
pub const NLINKS: usize = 256;
pub const LINK_MAX: usize = 512;
pub const TITLE_MAX: usize = 96;

/// Colour roles of the cell window (afriui.md section 24).
pub const R_TEXT: u8 = 0;
pub const R_DIM: u8 = 1;
pub const R_HEAD: u8 = 2;
pub const R_CODE: u8 = 5;
pub const R_ERROR: u8 = 13;
pub const R_LINK: u8 = 15;

/// A line ends where a letter is this byte.
pub const NL: u8 = b'\n';

/// The page as text: (letter, role) pairs, NL between lines; and its links.
pub struct Page {
    text: [u8; LETTERS * 2],
    pub n: usize,
    pool: [u8; POOL],
    pool_n: usize,
    links: [(u32, u16); NLINKS],
    pub nlinks: usize,
    pub title: Buf<TITLE_MAX>,
    pub cut: bool,
    /// A link was not numbered: the table (256) or the address pool (24 KiB) was full.
    pub links_full: bool,
}

impl Page {
    pub const fn new() -> Self {
        Page {
            text: [0; LETTERS * 2],
            n: 0,
            pool: [0; POOL],
            pool_n: 0,
            links: [(0, 0); NLINKS],
            nlinks: 0,
            title: Buf::new(),
            cut: false,
            links_full: false,
        }
    }

    pub fn reset(&mut self) {
        self.n = 0;
        self.pool_n = 0;
        self.nlinks = 0;
        self.title.clear();
        self.cut = false;
        self.links_full = false;
    }

    /// Letter i and its role.
    pub fn at(&self, i: usize) -> (u8, u8) {
        if i < self.n {
            (self.text[2 * i], self.text[2 * i + 1])
        } else {
            (NL, 0)
        }
    }

    /// Add one letter (a cell byte: 32 to 126, 0x80 for a letter the font lacks, or NL). The last RESERVE
    /// places are kept for `note`, so a full page can still say it was cut.
    pub fn put(&mut self, c: u8, role: u8) {
        if self.n >= LETTERS - RESERVE {
            self.cut = true;
            return;
        }
        self.text[2 * self.n] = c;
        self.text[2 * self.n + 1] = role;
        self.n += 1;
    }

    /// A line of Dira's own (an error, "the page was cut here"), on a line of its own, even on a full page.
    pub fn note(&mut self, s: &[u8], role: u8) {
        let nl_first = self.n > 0 && self.text[2 * (self.n - 1)] != NL;
        let push = |p: &mut Page, c: u8, r: u8| {
            if p.n < LETTERS {
                p.text[2 * p.n] = c;
                p.text[2 * p.n + 1] = r;
                p.n += 1;
            }
        };
        if nl_first {
            push(self, NL, 0);
        }
        for &c in s.iter().take(RESERVE - 2) {
            // a header value or an address may hold any byte: only letters the cells can draw are kept
            push(self, if (0x20..0x7f).contains(&c) { c } else { 0x80 }, role);
        }
        push(self, NL, 0);
    }

    /// Keep a link's address; its number (1-based), or None when the table or the pool is full.
    pub fn add_link(&mut self, href: &[u8]) -> Option<usize> {
        if self.nlinks == NLINKS || href.len() > LINK_MAX || href.len() > POOL - self.pool_n {
            self.links_full = true;
            return None;
        }
        self.pool[self.pool_n..self.pool_n + href.len()].copy_from_slice(href);
        self.links[self.nlinks] = (self.pool_n as u32, href.len() as u16);
        self.pool_n += href.len();
        self.nlinks += 1;
        Some(self.nlinks)
    }

    /// Link k's address as the page wrote it (1-based).
    pub fn link(&self, k: usize) -> Option<&[u8]> {
        if k == 0 || k > self.nlinks {
            return None;
        }
        let (a, l) = self.links[k - 1];
        self.pool.get(a as usize..a as usize + l as usize)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum St {
    Text,
    Tag,
    Comment,
    Bang, // <!DOCTYPE ...>, <?xml ...?>, <![CDATA[...]]> (dropped)
    Raw,  // inside script, style, textarea: wait for the closing tag
}

const TAG_MAX: usize = 1024;

/// The tokenizer and the text builder.
pub struct Html {
    st: St,
    plain: bool,
    pub latin1: bool,
    tag: [u8; TAG_MAX],
    tag_n: usize,
    tag_over: bool,
    quote: u8,
    prev: u8,
    dashes: u8,
    raw: [u8; 10],
    raw_n: usize,
    raw_match: usize,
    ent: [u8; 12],
    ent_n: usize,
    in_ent: bool,
    utf: u32,
    utf_need: u8,
    in_title: bool,
    skip: u16,
    pre: u16,
    head: u16,
    code: u16,
    link: bool,
    lists: [u16; 8], // 0 for a bulleted list, else the next number
    nlists: usize,
    brk: u8,
    space: bool,
    col: usize,
    last_blank: bool,
}

fn lower(c: u8) -> u8 {
    c.to_ascii_lowercase()
}

fn eq_ci(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| lower(*x) == lower(*y))
}

/// Latin-1 letters 0xC0 to 0xFF read without their accents (the cell font has ASCII only; M4 shows them).
const FOLD: &[u8; 64] = b"AAAAAAACEEEEIIIIDNOOOOOxOUUUUYTsaaaaaaaceeeeiiiidnooooo/ouuuuyty";

/// Windows-1252's 0x80 to 0x9F, as Unicode (0 where it has none).
const CP1252: [u16; 32] = [
    0x20AC, 0, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0, 0x017D, 0,
    0, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0, 0x017E, 0x0178,
];

/// How a code point is shown in cells: ASCII as itself, a few common signs as ASCII, the rest as the box.
fn cells_for(cp: u32, out: &mut [u8; 4]) -> usize {
    let s: &[u8] = match cp {
        0x20..=0x7E => {
            out[0] = cp as u8;
            return 1;
        }
        0xC0..=0xFF => {
            out[0] = FOLD[(cp - 0xC0) as usize];
            return 1;
        }
        0xA0 => b" ",
        0xA9 => b"(c)",
        0xAE => b"(R)",
        0xAB => b"<<",
        0xBB => b">>",
        0xB7 | 0x2022 | 0x25CF => b"*",
        0x2010..=0x2015 | 0x2212 => b"-",
        0x2018 | 0x2019 | 0x201A | 0x2032 => b"'",
        0x201C | 0x201D | 0x201E | 0x2033 => b"\"",
        0x2026 => b"...",
        0x2122 => b"TM",
        0x20AC => b"EUR",
        0x2039 => b"<",
        0x203A => b">",
        _ => b"\x80",
    };
    out[..s.len()].copy_from_slice(s);
    s.len()
}

/// The named entities pages use most; the others stay as written.
fn named(name: &[u8]) -> Option<u32> {
    const T: &[(&[u8], u32)] = &[
        (b"amp", 38), (b"lt", 60), (b"gt", 62), (b"quot", 34), (b"apos", 39), (b"nbsp", 0xA0), (b"copy", 0xA9),
        (b"reg", 0xAE), (b"trade", 0x2122), (b"mdash", 0x2014), (b"ndash", 0x2013), (b"hellip", 0x2026),
        (b"lsquo", 0x2018), (b"rsquo", 0x2019), (b"ldquo", 0x201C), (b"rdquo", 0x201D), (b"bull", 0x2022),
        (b"middot", 0xB7), (b"laquo", 0xAB), (b"raquo", 0xBB), (b"times", 0xD7), (b"divide", 0xF7),
        (b"euro", 0x20AC), (b"eacute", 0xE9), (b"egrave", 0xE8), (b"agrave", 0xE0), (b"ccedil", 0xE7),
        (b"ocirc", 0xF4), (b"uuml", 0xFC), (b"ouml", 0xF6), (b"auml", 0xE4), (b"szlig", 0xDF), (b"minus", 0x2212),
    ];
    T.iter().find(|(n, _)| *n == name).map(|(_, v)| *v)
}

/// `&...;` content (without & and ;) as a code point.
fn entity(e: &[u8]) -> Option<u32> {
    if e.first() == Some(&b'#') {
        let (digits, radix) = match e.get(1) {
            Some(b'x') | Some(b'X') => (&e[2..], 16),
            _ => (&e[1..], 10),
        };
        if digits.is_empty() || digits.len() > 7 {
            return None;
        }
        let mut v = 0u32;
        for &c in digits {
            v = v * radix + (c as char).to_digit(radix)?;
        }
        // condition 2: 0, the C1 range, surrogates and past U+10FFFF all become U+FFFD (the box)
        let bad = v == 0 || (0x80..0xA0).contains(&v) || (0xD800..0xE000).contains(&v) || v > 0x10FFFF;
        return Some(if bad { 0xFFFD } else { v });
    }
    named(e)
}

/// An attribute's value inside a tag's bytes (`a href="x"` gives `x` for `href`).
fn attr<'t>(tag: &'t [u8], key: &[u8]) -> Option<&'t [u8]> {
    let mut i = tag.iter().position(|c| c.is_ascii_whitespace() || *c == b'/')?;
    loop {
        while i < tag.len() && (tag[i].is_ascii_whitespace() || tag[i] == b'/') {
            i += 1;
        }
        if i >= tag.len() {
            return None;
        }
        let ns = i;
        while i < tag.len() && !tag[i].is_ascii_whitespace() && tag[i] != b'=' && tag[i] != b'/' {
            i += 1;
        }
        let name = &tag[ns..i];
        while i < tag.len() && tag[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut val: &[u8] = b"";
        if i < tag.len() && tag[i] == b'=' {
            i += 1;
            while i < tag.len() && tag[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < tag.len() && (tag[i] == b'"' || tag[i] == b'\'') {
                let q = tag[i];
                let vs = i + 1;
                let ve = tag[vs..].iter().position(|&c| c == q).map(|p| vs + p).unwrap_or(tag.len());
                val = &tag[vs..ve];
                i = ve + 1;
            } else {
                let vs = i;
                while i < tag.len() && !tag[i].is_ascii_whitespace() {
                    i += 1;
                }
                val = &tag[vs..i];
            }
        }
        if eq_ci(name, key) {
            return Some(val);
        }
    }
}

/// Decode the entities of an attribute value (as UTF-8) into `out`; false when it does not fit.
fn decode_attr(v: &[u8], out: &mut Buf<LINK_MAX>) -> bool {
    let mut i = 0;
    while i < v.len() {
        if v[i] == b'&' {
            let end = v[i + 1..].iter().take(11).position(|&c| c == b';').map(|p| i + 1 + p);
            if let Some(e) = end {
                if let Some(cp) = entity(&v[i + 1..e]) {
                    let mut tmp = [0u8; 4];
                    let s = char::from_u32(cp).unwrap_or('\u{FFFD}').encode_utf8(&mut tmp);
                    if !out.push(s.as_bytes()) {
                        return false;
                    }
                    i = e + 1;
                    continue;
                }
            }
        }
        if !out.push_byte(v[i]) {
            return false;
        }
        i += 1;
    }
    true
}

/// Tags that start a new line, and those that also leave a blank line.
fn block_kind(name: &[u8]) -> u8 {
    const PARA: &[&[u8]] = &[b"p", b"pre", b"table", b"blockquote", b"form", b"figure", b"section", b"article", b"header", b"footer", b"nav", b"main", b"aside", b"details", b"fieldset", b"address"];
    const LINE: &[&[u8]] = &[b"div", b"tr", b"dt", b"dd", b"dl", b"figcaption", b"summary", b"center", b"caption", b"legend", b"tbody", b"thead", b"tfoot", b"option", b"body", b"html"];
    if PARA.contains(&name) {
        2
    } else if LINE.contains(&name) {
        1
    } else {
        0
    }
}

/// Elements whose whole content is dropped (pictures, embedded frames, templates).
fn skipped(name: &[u8]) -> bool {
    const S: &[&[u8]] = &[b"svg", b"template", b"iframe", b"object", b"canvas", b"video", b"audio", b"select", b"map"];
    S.contains(&name)
}

impl Html {
    pub const fn new() -> Self {
        Html {
            st: St::Text,
            plain: false,
            latin1: false,
            tag: [0; TAG_MAX],
            tag_n: 0,
            tag_over: false,
            quote: 0,
            prev: 0,
            dashes: 0,
            raw: [0; 10],
            raw_n: 0,
            raw_match: 0,
            ent: [0; 12],
            ent_n: 0,
            in_ent: false,
            utf: 0,
            utf_need: 0,
            in_title: false,
            skip: 0,
            pre: 0,
            head: 0,
            code: 0,
            link: false,
            lists: [0; 8],
            nlists: 0,
            brk: 0,
            space: false,
            col: 0,
            last_blank: false,
        }
    }

    /// Start a new page: `plain` for text/plain (shown as written), `latin1` for one-byte pages.
    pub fn start(&mut self, plain: bool, latin1: bool) {
        *self = Html::new();
        self.plain = plain;
        self.latin1 = latin1;
    }

    fn role(&self) -> u8 {
        if self.link {
            R_LINK
        } else if self.head > 0 {
            R_HEAD
        } else if self.pre > 0 || self.code > 0 || self.plain {
            R_CODE
        } else {
            R_TEXT
        }
    }

    fn nl(&mut self, p: &mut Page) {
        p.put(NL, 0);
        self.last_blank = self.col == 0;
        self.col = 0;
    }

    fn block(&mut self, n: u8) {
        self.brk = self.brk.max(n);
        self.space = false;
    }

    /// Before any letter: the line breaks asked for, then one space if the words had one between them.
    fn flush(&mut self, p: &mut Page) {
        if self.brk > 0 {
            if self.col > 0 {
                self.nl(p);
            }
            if self.brk == 2 && !self.last_blank && p.n > 0 {
                self.nl(p);
            }
            self.brk = 0;
            self.space = false;
        }
        if self.space {
            if self.col > 0 {
                p.put(b' ', R_TEXT);
                self.col += 1;
            }
            self.space = false;
        }
    }

    fn word(&mut self, p: &mut Page, s: &[u8], role: u8) {
        self.flush(p);
        for &c in s {
            p.put(c, role);
            self.col += 1;
        }
    }

    /// One code point of page text.
    fn cp(&mut self, p: &mut Page, cp: u32) {
        if self.in_title {
            if p.title.len() < TITLE_MAX {
                let mut o = [0u8; 4];
                let n = cells_for(if cp < 0x20 { 0x20 } else { cp }, &mut o);
                let t = &o[..n];
                if !(t == b" " && (p.title.len() == 0 || p.title.get().ends_with(b" "))) {
                    p.title.push(&t[..t.len().min(TITLE_MAX - p.title.len())]);
                }
            }
            return;
        }
        if self.skip > 0 {
            return;
        }
        let pre = self.pre > 0 || self.plain;
        match cp {
            0x0A if pre && self.brk > 0 => self.flush(p), // the newline just after <pre> starts no line
            0x0A if pre => {
                self.brk = 0;
                self.space = false;
                self.nl(p);
            }
            0x0D if pre => {}
            0x09 if pre => {
                let n = 4 - self.col % 4;
                for _ in 0..n {
                    self.word(p, b" ", R_CODE);
                }
            }
            0x20 if pre => self.word(p, b" ", R_CODE),
            0x09 | 0x0A | 0x0C | 0x0D | 0x20 => self.space = true,
            0..=0x1F | 0x7F..=0x9F => {}
            _ => {
                let mut o = [0u8; 4];
                let n = cells_for(cp, &mut o);
                let role = self.role();
                self.word(p, &o[..n], role);
            }
        }
    }

    /// A byte of text (not markup): UTF-8 or one-byte, then entities.
    fn text_byte(&mut self, p: &mut Page, c: u8) {
        if self.latin1 {
            let cp = if (0x80..0xA0).contains(&c) { CP1252[(c - 0x80) as usize] as u32 } else { c as u32 };
            return self.ent_cp(p, cp);
        }
        if self.utf_need > 0 {
            if c & 0xC0 == 0x80 {
                self.utf = (self.utf << 6) | (c & 0x3F) as u32;
                self.utf_need -= 1;
                if self.utf_need == 0 {
                    let cp = self.utf;
                    self.ent_cp(p, cp);
                }
                return;
            }
            self.utf_need = 0;
            self.ent_cp(p, 0xFFFD); // a broken sequence; the byte is read afresh below
        }
        match c {
            0..=0x7F => self.ent_cp(p, c as u32),
            0xC2..=0xDF => {
                self.utf = (c & 0x1F) as u32;
                self.utf_need = 1;
            }
            0xE0..=0xEF => {
                self.utf = (c & 0x0F) as u32;
                self.utf_need = 2;
            }
            0xF0..=0xF4 => {
                self.utf = (c & 0x07) as u32;
                self.utf_need = 3;
            }
            _ => self.ent_cp(p, 0xFFFD),
        }
    }

    /// Text after UTF-8 decoding: `&name;` and `&#N;` are read here (plain text has none).
    fn ent_cp(&mut self, p: &mut Page, cp: u32) {
        if self.plain {
            return self.cp(p, cp);
        }
        if self.in_ent {
            let c = cp as u8;
            if cp == b';' as u32 {
                self.in_ent = false;
                let e = self.ent;
                match entity(&e[..self.ent_n]) {
                    Some(v) => self.cp(p, v),
                    None => self.ent_literal(p, true),
                }
                return;
            }
            if cp < 0x80 && (c.is_ascii_alphanumeric() || c == b'#') && self.ent_n < self.ent.len() {
                self.ent[self.ent_n] = c;
                self.ent_n += 1;
                return;
            }
            self.in_ent = false;
            self.ent_literal(p, false);
        }
        if cp == b'&' as u32 {
            self.in_ent = true;
            self.ent_n = 0;
            return;
        }
        self.cp(p, cp);
    }

    /// An `&...` that was not an entity, shown as written.
    fn ent_literal(&mut self, p: &mut Page, semi: bool) {
        self.cp(p, b'&' as u32);
        let e = self.ent;
        for &c in &e[..self.ent_n] {
            self.cp(p, c as u32);
        }
        if semi {
            self.cp(p, b';' as u32);
        }
    }

    /// Read more of the page.
    pub fn feed(&mut self, p: &mut Page, input: &[u8]) {
        let mut i = 0;
        while i < input.len() {
            let c = input[i];
            match self.st {
                St::Text => {
                    if c == b'<' && !self.plain {
                        if self.in_ent {
                            self.in_ent = false;
                            self.ent_literal(p, false);
                        }
                        self.st = St::Tag;
                        self.tag_n = 0;
                        self.tag_over = false;
                        self.quote = 0;
                        self.prev = 0;
                    } else {
                        self.text_byte(p, c);
                    }
                }
                St::Tag => {
                    if self.tag_n == 0 && !(c.is_ascii_alphabetic() || c == b'/' || c == b'!' || c == b'?') {
                        // a '<' that starts no tag ("a < b") is a letter
                        self.st = St::Text;
                        self.text_byte(p, b'<');
                        continue; // read c again as text
                    }
                    if self.quote != 0 {
                        if c == self.quote {
                            self.quote = 0;
                        }
                    } else if (c == b'"' || c == b'\'') && self.prev == b'=' {
                        self.quote = c;
                    } else if c == b'>' {
                        self.st = St::Text;
                        self.tag_done(p);
                        i += 1;
                        continue;
                    }
                    if !c.is_ascii_whitespace() {
                        self.prev = c;
                    }
                    if self.tag_n < TAG_MAX {
                        self.tag[self.tag_n] = c;
                        self.tag_n += 1;
                    } else {
                        self.tag_over = true;
                    }
                    if self.tag_n == 3 && &self.tag[..3] == b"!--" {
                        self.st = St::Comment;
                        self.dashes = 0;
                    } else if self.tag_n == 1 && (c == b'!' || c == b'?') {
                        self.st = St::Bang;
                        self.quote = 0;
                    }
                }
                St::Bang => {
                    // still watching for "!--" after the '!'
                    if self.tag_n < 3 {
                        self.tag[self.tag_n] = c;
                        self.tag_n += 1;
                        if self.tag_n == 3 && &self.tag[..3] == b"!--" {
                            self.st = St::Comment;
                            self.dashes = 0;
                            i += 1;
                            continue;
                        }
                    }
                    if c == b'>' {
                        self.st = St::Text;
                    }
                }
                St::Comment => {
                    if c == b'>' && self.dashes >= 2 {
                        self.st = St::Text;
                    }
                    self.dashes = if c == b'-' { self.dashes.saturating_add(1) } else { 0 };
                }
                St::Raw => {
                    // "</" then the element's name, in any case
                    let want = if self.raw_match < 2 { b"</"[self.raw_match] } else { self.raw[self.raw_match - 2] };
                    if lower(c) == want {
                        self.raw_match += 1;
                        if self.raw_match == 2 + self.raw_n {
                            self.st = St::Tag;
                            self.tag[0] = b'/';
                            self.tag[1..1 + self.raw_n].copy_from_slice(&self.raw[..self.raw_n]);
                            self.tag_n = 1 + self.raw_n;
                            self.tag_over = false;
                            self.quote = 0;
                            self.prev = 0;
                        }
                    } else {
                        self.raw_match = if c == b'<' { 1 } else { 0 };
                    }
                }
            }
            i += 1;
        }
    }

    /// The page has ended: anything half read is let go, the last line closed.
    pub fn finish(&mut self, p: &mut Page) {
        if self.in_ent {
            self.in_ent = false;
            self.ent_literal(p, false);
        }
        if self.col > 0 {
            self.nl(p);
        }
    }

    fn tag_done(&mut self, p: &mut Page) {
        let all = self.tag;
        let t = &all[..self.tag_n];
        let close = t.first() == Some(&b'/');
        let body = if close { &t[1..] } else { t };
        let nend = body.iter().position(|c| !c.is_ascii_alphanumeric()).unwrap_or(body.len());
        if nend == 0 || nend > 10 {
            return;
        }
        let mut nb = [0u8; 10];
        for (k, &c) in body[..nend].iter().enumerate() {
            nb[k] = lower(c);
        }
        let name = &nb[..nend];
        let selfclose = t.last() == Some(&b'/');
        if !close && !selfclose && (name == b"script" || name == b"style" || name == b"textarea") {
            self.st = St::Raw;
            self.raw[..nend].copy_from_slice(name);
            self.raw_n = nend;
            self.raw_match = 0;
            return;
        }
        if name == b"title" {
            self.in_title = !close && !selfclose;
            return;
        }
        if skipped(name) {
            if close {
                self.skip = self.skip.saturating_sub(1);
            } else if !selfclose {
                self.skip = self.skip.saturating_add(1);
            }
            return;
        }
        if self.skip > 0 {
            return;
        }
        let bk = block_kind(name);
        if bk > 0 {
            self.block(bk);
        }
        match name {
            b"br" => {
                self.brk = 0;
                self.space = false;
                self.nl(p);
            }
            b"hr" => {
                self.block(1);
                self.word(p, b"--------------------", R_DIM);
                self.block(1);
            }
            b"h1" | b"h2" | b"h3" | b"h4" | b"h5" | b"h6" => {
                self.block(2);
                self.head = if close { self.head.saturating_sub(1) } else { self.head.saturating_add(1) };
            }
            b"pre" | b"listing" | b"xmp" => {
                self.pre = if close { self.pre.saturating_sub(1) } else { self.pre.saturating_add(1) };
            }
            b"code" | b"kbd" | b"samp" | b"tt" => {
                self.code = if close { self.code.saturating_sub(1) } else { self.code.saturating_add(1) };
            }
            b"ul" | b"ol" | b"menu" => {
                self.block(if self.nlists == 0 { 2 } else { 1 });
                if close {
                    self.nlists = self.nlists.saturating_sub(1);
                } else {
                    if self.nlists < self.lists.len() {
                        self.lists[self.nlists] = if name == b"ol" { 1 } else { 0 };
                    }
                    self.nlists += 1;
                }
            }
            b"li" if !close => {
                self.block(1);
                self.flush(p);
                let depth = self.nlists.max(1).min(self.lists.len());
                for _ in 1..depth {
                    self.word(p, b"  ", R_TEXT);
                }
                let k = depth - 1;
                if self.nlists > 0 && self.lists[k] > 0 {
                    let mut d = [0u8; 6];
                    let mut m = 0;
                    let mut v = self.lists[k];
                    while v > 0 && m < 5 {
                        d[4 - m] = b'0' + (v % 10) as u8;
                        v /= 10;
                        m += 1;
                    }
                    d[5] = b'.';
                    self.word(p, &d[5 - m..], R_TEXT);
                    self.word(p, b" ", R_TEXT);
                    self.lists[k] = self.lists[k].saturating_add(1);
                } else {
                    self.word(p, b"* ", R_TEXT);
                }
                self.space = false;
            }
            b"dd" if !close => {
                self.flush(p);
                self.word(p, b"    ", R_TEXT);
            }
            b"td" | b"th" if !close => {
                if self.col > 0 {
                    self.flush(p);
                    self.word(p, b"  ", R_TEXT);
                }
                self.space = false;
            }
            b"a" => {
                self.link = false;
                if close {
                    return;
                }
                let Some(href) = attr(t, b"href") else { return };
                let mut h: Buf<LINK_MAX> = Buf::new();
                if self.tag_over || !decode_attr(href, &mut h) {
                    p.links_full = true; // an address longer than Dira keeps: not numbered, and the page says so
                    return;
                }
                let hv = h.get();
                let hv = &hv[hv.iter().position(|c| !c.is_ascii_whitespace()).unwrap_or(hv.len())..];
                if hv.is_empty() || hv[0] == b'#' || (hv.len() >= 11 && eq_ci(&hv[..11], b"javascript:")) {
                    return;
                }
                if let Some(k) = p.add_link(hv) {
                    let mut d = [0u8; 6];
                    let mut m = 0;
                    let mut v = k;
                    while v > 0 && m < 4 {
                        d[4 - m] = b'0' + (v % 10) as u8;
                        v /= 10;
                        m += 1;
                    }
                    d[5] = b']';
                    d[4 - m] = b'[';
                    self.word(p, &d[4 - m..], R_LINK);
                    self.link = true;
                }
            }
            b"img" if !close => {
                if let Some(alt) = attr(t, b"alt") {
                    let mut h: Buf<LINK_MAX> = Buf::new();
                    decode_attr(alt, &mut h);
                    let a = h.get();
                    if a.iter().any(|c| !c.is_ascii_whitespace()) {
                        self.word(p, b"[", R_DIM);
                        self.dim_text(p, a);
                        self.word(p, b"]", R_DIM);
                    }
                }
            }
            b"meta" if !close => {
                if let Some(cs) = attr(t, b"charset") {
                    self.latin1 = eq_ci(cs, b"iso-8859-1") || eq_ci(cs, b"windows-1252") || eq_ci(cs, b"latin1");
                }
            }
            _ => {}
        }
    }

    /// An image's alt text (UTF-8, entities already decoded) in the dim role, spaces folded.
    fn dim_text(&mut self, p: &mut Page, a: &[u8]) {
        let s = match core::str::from_utf8(a) {
            Ok(s) => s,
            Err(e) => core::str::from_utf8(&a[..e.valid_up_to()]).unwrap_or(""),
        };
        let mut gap = false;
        for ch in s.chars() {
            if ch.is_whitespace() {
                gap = true;
                continue;
            }
            if gap && self.col > 0 {
                self.word(p, b" ", R_DIM);
            }
            gap = false;
            let mut o = [0u8; 4];
            let n = cells_for(ch as u32, &mut o);
            self.word(p, &o[..n], R_DIM);
        }
    }
}

/// Wrap the page to `width` letters a row: `starts` gets each row's first letter. Returns the number of rows
/// (at most `starts.len()`).
pub fn wrap(p: &Page, width: usize, starts: &mut [u32]) -> usize {
    let width = width.max(8);
    let mut rows = 0;
    let mut a = 0;
    while a < p.n && rows < starts.len() {
        let mut b = a;
        while b < p.n && p.text[2 * b] != NL {
            b += 1;
        }
        // the line [a, b)
        let mut pos = a;
        loop {
            if rows == starts.len() {
                break;
            }
            starts[rows] = pos as u32;
            rows += 1;
            if b - pos <= width {
                break;
            }
            // the last space that keeps the row within width
            let mut cut = pos + width;
            let mut k = pos + width;
            while k > pos {
                if p.text[2 * k] == b' ' {
                    break;
                }
                k -= 1;
            }
            if k > pos {
                cut = k + 1;
            }
            pos = cut;
        }
        a = b + 1;
    }
    rows
}

/// Row r's letters: [start, end), without the line end or trailing spaces.
pub fn row(p: &Page, starts: &[u32], rows: usize, r: usize, width: usize) -> (usize, usize) {
    if r >= rows {
        return (0, 0);
    }
    let s = starts[r] as usize;
    let lim = if r + 1 < rows { starts[r + 1] as usize } else { p.n };
    let mut e = s;
    while e < lim && e - s < width.max(8) && p.text[2 * e] != NL {
        e += 1;
    }
    while e > s && p.text[2 * (e - 1)] == b' ' {
        e -= 1;
    }
    (s, e)
}
