//! AfriStore v0 — a sovereign, no-gatekeeper app catalog.
//!
//! Enumerates the `.wasm` apps installed in the filesystem and presents them —
//! as a text listing and a graphical catalog composited onto the framebuffer.
//! In the full design ([`docs/08`], [`docs/12`]) AfriStore is *a* store, not
//! *the* store: federated, sideloading first-class, apps signature-verified (not
//! permission-gated), monetization via AfriPay with a tiny protocol fee. This is
//! the catalog surface over the local app files, with a first pass at a premium
//! look (gradients, cards, chips) over the bring-up bitmap UI.

use crate::framebuffer as fb;
use crate::vfs;

const GOLD: u32 = 0xF2_B7_05;
const GRAY: u32 = 0x8B_9B_B4;
const WHITE: u32 = 0xF0_F3_F8;
const GREEN: u32 = 0x2E_CC_71;
const NAV: u32 = 0x0E_13_20;

fn is_wasm(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() > 5 && &b[b.len() - 5..] == b".wasm"
}

fn lerp(a: u32, b: u32, t: usize, d: usize) -> u32 {
    let ch = |s: u32| -> u32 {
        let av = ((a >> s) & 0xff) as i32;
        let bv = ((b >> s) & 0xff) as i32;
        (av + (bv - av) * (t as i32) / (d as i32)) as u32
    };
    (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

fn fmt_u32(mut v: u32, buf: &mut [u8; 12]) -> &str {
    if v == 0 {
        buf[0] = b'0';
        return core::str::from_utf8(&buf[..1]).unwrap_or("0");
    }
    let mut tmp = [0u8; 12];
    let mut i = 0;
    while v > 0 {
        tmp[i] = b'0' + (v % 10) as u8;
        v /= 10;
        i += 1;
    }
    for j in 0..i {
        buf[j] = tmp[i - 1 - j];
    }
    core::str::from_utf8(&buf[..i]).unwrap_or("?")
}

fn getkey() -> u8 {
    loop {
        if let Some(c) = crate::uart::getc() {
            return c;
        }
        core::hint::spin_loop();
    }
}

/// Open the store: an interactive graphical catalog. Press an app's number to
/// launch it, any key to come back, 'q' to exit to the console.
pub fn open() {
    crate::kprintln!("AfriStore -- installed WASM apps (sideload freely, no gatekeeper):");
    let mut names: [&str; 12] = [""; 12];
    let mut n = 0;
    for i in 0..vfs::MAX_FILES {
        if vfs::is_used(i) && is_wasm(vfs::name_of(i)) && n < 12 {
            crate::kprintln!("  [{}] {:<16} {:>5} B", n + 1, vfs::name_of(i), vfs::size_of(i));
            names[n] = vfs::name_of(i);
            n += 1;
        }
    }
    crate::kprintln!("  ({} apps. Press a number to launch, 'q' to exit.)", n);

    fb::set_console(false);
    loop {
        draw_catalog(n);
        let k = getkey();
        if k == b'q' || k == b'Q' || k == 0x1B {
            break;
        }
        if (b'1'..=b'9').contains(&k) {
            let idx = (k - b'1') as usize;
            if idx < n {
                crate::nia::run_file(names[idx]);
                crate::kprintln!("[store] press any key to return to AfriStore...");
                let _ = getkey();
            }
        }
    }
    crate::framebuffer::console_reset();
}

fn draw_catalog(count: usize) {
    let w = fb::WIDTH;
    let h = fb::HEIGHT;

    // Vertical gradient background.
    for y in 0..h {
        fb::fill_rect(0, y, w, 1, lerp(0x0C_10_1D, 0x05_06_0C, y, h));
    }

    // Top navigation bar.
    fb::fill_rect(0, 0, w, 44, NAV);
    fb::fill_rect(0, 44, w, 2, GOLD);
    fb::fill_rect(16, 14, 12, 16, GOLD); // diamond-ish mark
    fb::draw_string(36, 12, "AFRISTORE", GOLD, 2);
    // search pill
    fb::fill_rect(220, 12, 240, 20, 0x1B_22_33);
    fb::fill_rect(220, 12, 240, 1, 0x2C_36_4E);
    fb::draw_string(230, 18, "SEARCH APPS...", 0x6B_7A_92, 1);
    // app count (right)
    let mut cb = [0u8; 12];
    fb::draw_string(w - 96, 18, fmt_u32(count as u32, &mut cb), WHITE, 1);
    fb::draw_string(w - 72, 18, "APPS", GRAY, 1);

    // Category chips.
    let chips = ["ALL", "DEV", "TOOLS", "GAMES", "AI"];
    let mut cx = 16;
    for (i, c) in chips.iter().enumerate() {
        let active = i == 0;
        let cw = c.len() * 8 + 20;
        fb::fill_rect(cx, 56, cw, 22, if active { GOLD } else { 0x18_1F_2E });
        fb::draw_string(cx + 10, 61, c, if active { 0x10_10_10 } else { GRAY }, 1);
        cx += cw + 8;
    }

    // App cards.
    let accents = [0xE0_3A_3A, 0xF2_B7_05, 0x2E_CC_71, 0x2A_6B_E0, 0x9B_59_F6, 0x1F_B6_C9];
    let mut idx = 0;
    for i in 0..vfs::MAX_FILES {
        if vfs::is_used(i) && is_wasm(vfs::name_of(i)) {
            let col = idx % 3;
            let row = idx / 3;
            let cardx = 16 + col * 204;
            let cardy = 92 + row * 116;
            draw_card(cardx, cardy, vfs::name_of(i), vfs::size_of(i) as u32, accents[idx % 6], idx + 1);
            idx += 1;
        }
    }

    // Footer bar.
    fb::fill_rect(0, h - 30, w, 30, NAV);
    fb::fill_rect(0, h - 30, w, 1, 0x2C_36_4E);
    fb::draw_string(16, h - 21, "SIGNED=ED25519 PUBLISHER   VERIFIED=SHA256 INTEGRITY", GREEN, 1);
    fb::draw_string(w - 300, h - 21, "NUMBER TO LAUNCH    Q TO EXIT", GRAY, 1);
}

fn draw_card(x: usize, y: usize, name: &str, size: u32, accent: u32, num: usize) {
    let cw = 188;
    let cardh = 100;
    // shadow + card body with a subtle top-to-bottom sheen
    fb::fill_rect(x + 3, y + 4, cw, cardh, 0x02_03_07);
    for j in 0..cardh {
        fb::fill_rect(x, y + j, cw, 1, lerp(0x1A_22_33, 0x12_18_26, j, cardh));
    }
    // top accent stripe
    fb::fill_rect(x, y, cw, 3, accent);

    // icon tile with the app initial
    fb::fill_rect(x + 14, y + 16, 54, 54, accent);
    fb::fill_rect(x + 14, y + 16, 54, 2, 0xFF_FF_FF); // top highlight
    if let Some(&c0) = name.as_bytes().first() {
        fb::draw_char(x + 33, y + 34, c0, 0x10_10_10, 2);
    }

    // name + meta
    fb::draw_string(x + 78, y + 18, name, WHITE, 1);
    let mut b = [0u8; 12];
    fb::draw_string(x + 78, y + 34, fmt_u32(size, &mut b), GRAY, 1);
    fb::draw_string(x + 78 + 26, y + 34, "B  WASM", GRAY, 1);

    // number badge (top-right) — press this key to launch
    let badge = [b'0' + (num % 10) as u8];
    fb::fill_rect(x + cw - 24, y + 8, 16, 16, accent);
    fb::draw_string(x + cw - 19, y + 10, core::str::from_utf8(&badge).unwrap_or("?"), 0x10_10_10, 1);

    // "GET" pill
    fb::fill_rect(x + 78, y + 60, 46, 20, GREEN);
    fb::draw_string(x + 90, y + 66, "GET", 0x08_1A_10, 1);
    // rating dots
    for k in 0..5 {
        fb::fill_rect(x + 136 + k * 9, y + 66, 6, 6, if k < 4 { GOLD } else { 0x33_3B_4C });
    }

    // Trust badge (crypto verdict) at the card's bottom-left, under the icon.
    let data = vfs::read_raw(name).unwrap_or(&[]);
    let (label, bg, fg) = match crate::integrity::trust(name, data) {
        crate::integrity::Trust::Signed => ("SIGNED", GREEN, 0x08_1A_10),
        crate::integrity::Trust::Verified => ("VERIFIED", 0x2E_6A_C8, WHITE),
        crate::integrity::Trust::Unverified => ("UNVERIFIED", 0x39_20_20, 0xC8_7A_7A),
    };
    let bw = label.len() * 6 + 10;
    fb::fill_rect(x + 14, y + 78, bw, 15, bg);
    fb::draw_string(x + 19, y + 82, label, fg, 1);
}
