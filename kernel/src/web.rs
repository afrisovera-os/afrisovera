//! Nia Web — a minimal **Web/PWA app runtime** scaffold.
//!
//! AfriSovera's cross-platform pitch is "write once, run everywhere": native Nia
//! (WASM) apps *and* web apps as first-class citizens. This module is the honest
//! v0 of the web side: it loads a `.webapp` package (a tiny **HTML subset**) from
//! the VFS, parses it into a small DOM, and renders it into a Tonga surface. A web
//! app is therefore already a real, capability-secured AfriSovera app you can ship
//! in the store and launch on screen.
//!
//! **Honest scope:** this is a *minimal DOM subset renderer*, not a browser engine.
//! Supported: `<title>`, `<h1>`, `<p>`, and `<box color=RRGGBB/>`, stacked
//! vertically. No CSS, no JavaScript, no layout engine, no network — those are the
//! larger build-out behind this same entry point. What exists here is real: a
//! working tag parser + DOM + on-screen render, verified by screenshot.

use crate::framebuffer::{self, HEIGHT, WIDTH};

const MAXNODES: usize = 32;

#[derive(Clone, Copy)]
enum Kind {
    Title,
    H1,
    P,
    Box,
}

#[derive(Clone, Copy)]
struct Node {
    kind: Kind,
    off: usize, // byte range into the source for text content
    len: usize,
    color: u32, // for <box>
}

/// Case-insensitive ASCII byte-slice equality.
fn eqi(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.to_ascii_lowercase() == y.to_ascii_lowercase())
}

/// Parse a hex nibble, or None.
fn nib(c: u8) -> Option<u32> {
    match c {
        b'0'..=b'9' => Some((c - b'0') as u32),
        b'a'..=b'f' => Some((c - b'a' + 10) as u32),
        b'A'..=b'F' => Some((c - b'A' + 10) as u32),
        _ => None,
    }
}

/// Find `color=RRGGBB` inside a tag's attribute bytes and return the RGB value.
fn find_color(attrs: &[u8]) -> u32 {
    let key = b"color=";
    let mut i = 0;
    while i + key.len() <= attrs.len() {
        if eqi(&attrs[i..i + key.len()], key) {
            let mut j = i + key.len();
            let mut v = 0u32;
            let mut got = 0;
            while j < attrs.len() && got < 6 {
                if let Some(d) = nib(attrs[j]) {
                    v = (v << 4) | d;
                    got += 1;
                    j += 1;
                } else {
                    break;
                }
            }
            if got == 6 {
                return v;
            }
        }
        i += 1;
    }
    0x8080_80
}

/// Parse the HTML-subset source into DOM nodes. Returns how many were produced.
fn parse(src: &[u8], nodes: &mut [Node]) -> usize {
    let mut i = 0usize;
    let mut n = 0usize;
    while i < src.len() && n < nodes.len() {
        if src[i] != b'<' {
            i += 1;
            continue;
        }
        i += 1; // past '<'
        if i < src.len() && src[i] == b'/' {
            // stray closing tag: skip to '>'
            while i < src.len() && src[i] != b'>' {
                i += 1;
            }
            continue;
        }
        let ts = i;
        while i < src.len() && src[i].is_ascii_alphanumeric() {
            i += 1;
        }
        let tag = &src[ts..i];
        let kind = if eqi(tag, b"title") {
            Kind::Title
        } else if eqi(tag, b"h1") {
            Kind::H1
        } else if eqi(tag, b"p") {
            Kind::P
        } else if eqi(tag, b"box") {
            Kind::Box
        } else {
            // unknown tag: skip past '>'
            while i < src.len() && src[i] != b'>' {
                i += 1;
            }
            continue;
        };

        if let Kind::Box = kind {
            let as_ = i;
            while i < src.len() && src[i] != b'>' {
                i += 1;
            }
            let color = find_color(&src[as_..i]);
            if i < src.len() {
                i += 1; // past '>'
            }
            nodes[n] = Node { kind, off: 0, len: 0, color };
            n += 1;
        } else {
            while i < src.len() && src[i] != b'>' {
                i += 1;
            }
            if i < src.len() {
                i += 1; // past '>'
            }
            let cs = i;
            while i < src.len() && src[i] != b'<' {
                i += 1;
            }
            let ce = i;
            // skip the closing tag
            while i < src.len() && src[i] != b'>' {
                i += 1;
            }
            if i < src.len() {
                i += 1;
            }
            nodes[n] = Node { kind, off: cs, len: ce - cs, color: 0 };
            n += 1;
        }
    }
    n
}

fn lerp(a: u32, b: u32, t: usize, d: usize) -> u32 {
    let mut out = 0u32;
    let mut sh = 0;
    while sh < 24 {
        let ca = (a >> sh) & 0xff;
        let cb = (b >> sh) & 0xff;
        let cc = ca + (cb.wrapping_sub(ca)) * (t as u32) / (d as u32);
        out |= (cc & 0xff) << sh;
        sh += 8;
    }
    out
}

/// Trim ASCII whitespace at both ends of a byte slice.
fn trim(s: &[u8]) -> &[u8] {
    let mut a = 0;
    let mut b = s.len();
    while a < b && s[a].is_ascii_whitespace() {
        a += 1;
    }
    while b > a && s[b - 1].is_ascii_whitespace() {
        b -= 1;
    }
    &s[a..b]
}

/// Parse `src` and render the page to the framebuffer (graphics mode).
pub fn render(src: &[u8]) {
    let mut nodes = [Node { kind: Kind::P, off: 0, len: 0, color: 0 }; MAXNODES];
    let n = parse(src, &mut nodes);

    framebuffer::set_console(false);
    // page background gradient
    for y in 0..HEIGHT {
        framebuffer::fill_rect(0, y, WIDTH, 1, lerp(0x0a0a_14, 0x12_1e33, y, HEIGHT));
    }
    // browser-ish chrome / title bar
    framebuffer::fill_rect(0, 0, WIDTH, 40, 0x1a2a_44);
    framebuffer::fill_rect(0, 40, WIDTH, 2, 0xffcc_44);

    // title (from <title>), else a default
    let mut title = "Web App";
    for node in nodes.iter().take(n) {
        if let Kind::Title = node.kind {
            title = core::str::from_utf8(trim(&src[node.off..node.off + node.len])).unwrap_or("Web App");
        }
    }
    framebuffer::draw_string(16, 12, "Nia Web", 0xffcc_44, 2);
    framebuffer::draw_string(140, 12, title, 0xffff_ff, 2);

    let mut y = 60usize;
    for node in nodes.iter().take(n) {
        if y > HEIGHT - 30 {
            break;
        }
        match node.kind {
            Kind::Title => {}
            Kind::H1 => {
                let t = core::str::from_utf8(trim(&src[node.off..node.off + node.len])).unwrap_or("");
                framebuffer::draw_string(24, y, t, 0xffcc_44, 3);
                y += 44;
            }
            Kind::P => {
                let t = core::str::from_utf8(trim(&src[node.off..node.off + node.len])).unwrap_or("");
                framebuffer::draw_string(24, y, t, 0xe6ec_f2, 2);
                y += 26;
            }
            Kind::Box => {
                framebuffer::fill_rect(24, y, WIDTH - 48, 36, node.color);
                y += 48;
            }
        }
    }

    // footer
    framebuffer::fill_rect(0, HEIGHT - 26, WIDTH, 26, 0x14_1e33);
    framebuffer::draw_string(16, HEIGHT - 20, "AfriSovera Web Runtime (v0)  -  capability-secured, no gatekeeper", 0x88_94a4, 1);
}

/// A sample `.webapp` (HTML subset) seeded into the VFS at boot.
pub fn sample() -> &'static [u8] {
    b"<title>Sovereign Notes</title>\
<h1>Habari, AfriSovera</h1>\
<p>This is a web app running as a first-class citizen.</p>\
<box color=2a8f3f/>\
<p>Same capability model as native apps: no ambient authority.</p>\
<box color=c81e2a/>\
<p>Write once - Nia WASM and web, side by side.</p>"
}
