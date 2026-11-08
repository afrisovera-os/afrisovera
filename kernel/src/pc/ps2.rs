//! PS/2 (i8042) keyboard on the PC (5 Nov 2026), for PC step 4: most laptops' BUILT-IN
//! keyboard still reaches the CPU through an i8042-compatible controller, and so does QEMU
//! q35's default keyboard. Scancode set 1 (the controller's translation, which UEFI leaves
//! on), polled from the local-APIC timer interrupt like the USB keyboard (xhci.rs); every
//! key becomes a character on the console input ring COM1 fills (uart::inject).
//!
//! PS/2 mouse (7 Nov 2026), PC step 3: a laptop's built-in touchpad (in its PS/2-compatible
//! mode, which every Synaptics/Elan/ALPS pad starts in) and a PS/2 mouse reach the CPU
//! through the i8042's second (aux) port. Set to defaults (0xF6) and streaming (0xF4), its
//! standard 3-byte packets move the same pointer the USB mouse moves (xhci::pointer_input),
//! so window dragging and dock clicks work from a touchpad too. The pointer is shown from
//! its first packet on (a laptop's pad, not on every PC that only has the port).

use core::arch::asm;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const DATA: u16 = 0x60;
const STATUS: u16 = 0x64; // read: status; write: command

static ON: AtomicBool = AtomicBool::new(false);
static KEYS: AtomicU64 = AtomicU64::new(0);
static SCANCODES: AtomicU64 = AtomicU64::new(0);
static mut SHIFT: bool = false;
static mut CAPS: bool = false;
static mut E0: bool = false;
static MOUSE: AtomicBool = AtomicBool::new(false);
static MOUSE_USED: AtomicBool = AtomicBool::new(false);
static mut MOUSE_ID: u8 = 0;
static mut PKT: [u8; 3] = [0; 3];
static mut PKT_N: usize = 0;
static mut MBUT: u8 = 0;
static mut PACKETS: u64 = 0;
static mut MCLICKS: u64 = 0;
static mut RESYNCS: u64 = 0;
static mut OVERFLOWS: u64 = 0;

unsafe fn inb(port: u16) -> u8 {
    let v: u8;
    asm!("in al, dx", out("al") v, in("dx") port, options(nomem, nostack, preserves_flags));
    v
}
unsafe fn outb(port: u16, v: u8) {
    asm!("out dx, al", in("dx") port, in("al") v, options(nomem, nostack, preserves_flags));
}

fn wait_write() -> bool {
    // SAFETY: reads the i8042 status port.
    (0..200_000).any(|_| unsafe { inb(STATUS) } & 2 == 0)
}
fn wait_read() -> bool {
    // SAFETY: as above.
    (0..200_000).any(|_| unsafe { inb(STATUS) } & 1 != 0)
}

/// Find the controller and turn its keyboard port on, interrupts off (we poll).
pub fn init() {
    // SAFETY: i8042 port I/O at boot, single core, before the timer polls it.
    unsafe {
        if inb(STATUS) == 0xFF {
            return; // no controller: the bus floats high
        }
        for _ in 0..64 {
            if inb(STATUS) & 1 == 0 {
                break;
            }
            inb(DATA); // drop whatever the firmware left
        }
        if !wait_write() {
            return;
        }
        outb(STATUS, 0x20); // read the configuration byte
        if !wait_read() {
            crate::kprintln!("[ps2] i8042 did not answer -- no PS/2 keyboard");
            return;
        }
        let cfg = inb(DATA);
        let new = (cfg & !0b11) | 1 << 6; // no IRQ1/IRQ12 (polled), keep translation to set 1
        if !wait_write() {
            return;
        }
        outb(STATUS, 0x60);
        if !wait_write() {
            return;
        }
        outb(DATA, new);
        if wait_write() {
            outb(STATUS, 0xAE); // enable the keyboard port
        }
        ON.store(true, Ordering::Relaxed);
        crate::kprintln!("[ps2] i8042 keyboard controller (config {:#04x} -> {:#04x}), scancode set 1, polled", cfg, new);
        mouse_init();
    }
}

/// One byte for the aux (mouse) port, and its answer: Some(0xFA) is the mouse's ACK.
/// Keyboard bytes that arrive meanwhile are dropped (nothing is typed while the PC boots).
unsafe fn aux_cmd(cmd: u8) -> Option<u8> {
    if !wait_write() {
        return None;
    }
    outb(STATUS, 0xD4); // the next data byte goes to port 2
    if !wait_write() {
        return None;
    }
    outb(DATA, cmd);
    aux_read()
}

/// The next byte from the aux port, if one comes in time.
unsafe fn aux_read() -> Option<u8> {
    for _ in 0..8 {
        if !wait_read() {
            return None;
        }
        let st = inb(STATUS);
        let b = inb(DATA);
        if st & 0x20 != 0 {
            return Some(b);
        }
    }
    None
}

/// Turn the aux port on and the mouse or touchpad behind it to streaming, polled like the
/// keyboard (IRQ12 stays off). No ACK = no PS/2 mouse; the keyboard is unaffected.
unsafe fn mouse_init() {
    if !wait_write() {
        return;
    }
    outb(STATUS, 0xA8); // enable the aux port
    if !wait_write() {
        return;
    }
    outb(STATUS, 0x20);
    if !wait_read() {
        return;
    }
    let cfg = inb(DATA);
    let new = cfg & !(1 << 5 | 0b11); // aux clock on, both interrupts off
    if !wait_write() {
        return;
    }
    outb(STATUS, 0x60);
    if !wait_write() {
        return;
    }
    outb(DATA, new);
    if aux_cmd(0xF6) != Some(0xFA) {
        crate::kprintln!("[ps2] no PS/2 mouse or touchpad on the aux port");
        return;
    }
    if aux_cmd(0xF2) == Some(0xFA) {
        MOUSE_ID = aux_read().unwrap_or(0);
    }
    if aux_cmd(0xF4) != Some(0xFA) {
        crate::kprintln!("[ps2] the PS/2 mouse did not start streaming");
        return;
    }
    MOUSE.store(true, Ordering::Relaxed);
    crate::kprintln!("[ps2] PS/2 mouse or touchpad (id {:#04x}) on the aux port, 3-byte packets, polled", { MOUSE_ID });
}

/// One mouse byte; every third (from a byte with bit 3 set) completes a packet.
unsafe fn mouse_byte(b: u8) {
    if PKT_N == 0 && b & 0x08 == 0 {
        RESYNCS += 1; // not a packet's first byte: wait for one
        return;
    }
    PKT[PKT_N] = b;
    PKT_N += 1;
    if PKT_N < 3 {
        return;
    }
    PKT_N = 0;
    let [h, x, y] = PKT;
    PACKETS += 1;
    MOUSE_USED.store(true, Ordering::Relaxed);
    let buttons = h & 7;
    MCLICKS += (buttons & !MBUT).count_ones() as u64;
    MBUT = buttons;
    let (mut dx, mut dy) = (0i32, 0i32);
    if h & 0xC0 != 0 {
        OVERFLOWS += 1; // the counts are not to be trusted; keep the buttons only
    } else {
        dx = x as i32 - ((h as i32 & 0x10) << 4);
        dy = -(y as i32 - ((h as i32 & 0x20) << 3)); // PS/2 counts up; the screen down
    }
    crate::xhci::pointer_input(buttons, dx, dy);
}

/// Set 1 make code -> (plain, shifted) ASCII.
fn ascii(sc: u8) -> Option<(u8, u8)> {
    const ROW1: &[u8; 13] = b"1234567890-=\x7f";
    const ROW1S: &[u8; 13] = b"!@#$%^&*()_+\x7f";
    const ROW2: &[u8; 12] = b"qwertyuiop[]";
    const ROW2S: &[u8; 12] = b"QWERTYUIOP{}";
    const ROW3: &[u8; 12] = b"asdfghjkl;'`";
    const ROW3S: &[u8; 12] = b"ASDFGHJKL:\"~";
    const ROW4: &[u8; 11] = b"\\zxcvbnm,./";
    const ROW4S: &[u8; 11] = b"|ZXCVBNM<>?";
    Some(match sc {
        0x01 => (0x1B, 0x1B),
        0x02..=0x0E => (ROW1[(sc - 0x02) as usize], ROW1S[(sc - 0x02) as usize]),
        0x0F => (b'\t', b'\t'),
        0x10..=0x1B => (ROW2[(sc - 0x10) as usize], ROW2S[(sc - 0x10) as usize]),
        0x1C => (b'\r', b'\r'),
        0x1E..=0x29 => (ROW3[(sc - 0x1E) as usize], ROW3S[(sc - 0x1E) as usize]),
        0x2B..=0x35 => (ROW4[(sc - 0x2B) as usize], ROW4S[(sc - 0x2B) as usize]),
        0x39 => (b' ', b' '),
        _ => return None,
    })
}

/// Called from the local-APIC timer interrupt: every key pressed since the last tick
/// becomes console input.
pub fn poll() {
    if !ON.load(Ordering::Relaxed) {
        return;
    }
    // SAFETY: i8042 port reads; runs inside the timer interrupt, so never re-entered.
    unsafe {
        for _ in 0..32 {
            let st = inb(STATUS);
            if st & 1 == 0 {
                break;
            }
            let sc = inb(DATA);
            if st & 0x20 != 0 {
                if MOUSE.load(Ordering::Relaxed) {
                    mouse_byte(sc);
                }
                continue; // a mouse byte (port 2), not a key
            }
            SCANCODES.fetch_add(1, Ordering::Relaxed);
            if sc == 0xE0 {
                E0 = true;
                continue;
            }
            let ext = E0;
            E0 = false;
            let release = sc & 0x80 != 0;
            let code = sc & 0x7F;
            match code {
                0x2A | 0x36 => SHIFT = !release,
                0x3A if !release => CAPS = !CAPS,
                _ if release || ext && code != 0x1C => {} // releases; arrows and the like
                _ => {
                    if let Some((plain, shifted)) = ascii(code) {
                        let letter = plain.is_ascii_lowercase();
                        let up = SHIFT ^ (CAPS && letter);
                        crate::uart::inject(if up { shifted } else { plain });
                        KEYS.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }
    }
}

/// `lsps2`: the controller and its counters.
pub fn list() {
    if !ON.load(Ordering::Relaxed) {
        crate::kprintln!("ps2: no i8042 keyboard controller");
        return;
    }
    crate::kprintln!(
        "ps2: i8042 keyboard, polled; {} scancodes read, {} keys typed",
        SCANCODES.load(Ordering::Relaxed),
        KEYS.load(Ordering::Relaxed)
    );
    // SAFETY: scalar reads of the mouse counters.
    unsafe {
        if !MOUSE.load(Ordering::Relaxed) {
            crate::kprintln!("ps2: no PS/2 mouse or touchpad");
            return;
        }
        crate::kprintln!(
            "ps2: mouse/touchpad id {:#04x}, polled; {} packets, buttons {:03b}, {} clicks",
            { MOUSE_ID }, { PACKETS }, { MBUT }, { MCLICKS }
        );
        match crate::xhci::pointer() {
            Some((x, y)) if MOUSE_USED.load(Ordering::Relaxed) => crate::kprintln!("ps2: pointer at {},{}", x, y),
            _ => crate::kprintln!("ps2: not moved yet (the pointer appears at the first touch)"),
        }
        if RESYNCS + OVERFLOWS > 0 {
            crate::kprintln!("ps2: {} stray byte(s) skipped, {} overflowing packet(s)", { RESYNCS }, { OVERFLOWS });
        }
    }
}

/// True when the i8042 keyboard is on.
pub fn present() -> bool {
    ON.load(Ordering::Relaxed)
}

/// True when a PS/2 mouse or touchpad answered and is streaming.
pub fn mouse_present() -> bool {
    MOUSE.load(Ordering::Relaxed)
}

/// True once the PS/2 mouse or touchpad has sent a packet (the pointer is shown from then on).
pub fn mouse_used() -> bool {
    MOUSE_USED.load(Ordering::Relaxed)
}
