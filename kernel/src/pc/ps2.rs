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
//!
//! IRQ 12 (9 Nov 2026, HP ENVY field report F6: the touchpad was slow and unsteady): a real
//! i8042 holds ONE byte at a time and the pad waits while it is full, so reading only at
//! each timer tick (about 20 ms on a laptop) took about one byte per tick: 3-byte packets
//! at 100 a second fell behind and were lost. The aux port now interrupts (ISA IRQ 12 through
//! the IOAPIC) and each byte is taken as it lands; the timer tick still drains, as before.
//! The pad is also set as Linux sets it (100 samples a second, 8 counts per mm, twice its
//! power-on 4), and a part packet older than 40 ms (500 ms while polled only) is dropped, so
//! one lost byte cannot shift every packet after it.
//!
//! Acceleration (9 Nov 2026, F6 second part): each packet's speed is its counts over the time
//! since the packet before (at least the pad's own 10 ms, at most 100 ms), in mm/s at the set
//! resolution. Up to 100 mm/s the pointer moves 1:1 (fine work stays exact); above, the move
//! is multiplied, 2x at 250 mm/s and 3x from 400 mm/s, with the fractions carried over, so a
//! fast swipe crosses the screen. The console is drawn scaled onto the GOP (screen.rs), so a
//! count already covers more pixels on a bigger screen. `accel off` / `accel on` switch it.
//! Since 9 Nov 2026 the USB mouse's reports go through the same curve (`accelerate`).
//!
//! Wheel (9 Nov 2026, F7 last part: scroll the console with the touchpad or mouse): the
//! IntelliMouse knock at start-up (sample rates 200, 100, 80) turns a wheel mouse, and a
//! touchpad that offers scrolling in its PS/2 mode, into id 3 with 4-byte packets; the 4th
//! byte's notches scroll the console three lines each (conback.rs), not on the desktop.

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
static mut ALT: bool = false;
static mut SUPER: bool = false;
static mut SUPER_COMBO: bool = false;

/// A key that is a desktop shortcut while the desktop is shown (set-1 scancodes): Alt+Tab,
/// Alt+F4, Super+arrows, Esc.
unsafe fn desk_key(code: u8, ext: bool) -> Option<u32> {
    use crate::desk::*;
    Some(match (code, ext) {
        (0x0F, false) if ALT => KEY_ALT_TAB,
        (0x3E, false) if ALT => KEY_ALT_F4,
        (0x4B, true) if SUPER => KEY_SUPER_LEFT,
        (0x4D, true) if SUPER => KEY_SUPER_RIGHT,
        (0x48, true) if SUPER => KEY_SUPER_UP,
        (0x50, true) if SUPER => KEY_SUPER_DOWN,
        (0x01, false) if !ALT && !SUPER => KEY_ESC,
        // the keyboard walk (sovereign-ui.md 14): arrows and Enter while the Circle is open,
        // Super+1..4 for the Spine's apps
        (0x48, true) if !ALT && !SUPER && walking() => KEY_UP,
        (0x50, true) if !ALT && !SUPER && walking() => KEY_DOWN,
        (0x4B, true) if !ALT && !SUPER && walking() => KEY_LEFT,
        (0x4D, true) if !ALT && !SUPER && walking() => KEY_RIGHT,
        (0x1C, _) if !ALT && !SUPER && walking() => KEY_ENTER,
        (0x02..=0x05, false) if SUPER => KEY_SUPER_1 + (code - 0x02) as u32,
        _ => return None,
    })
}
static MOUSE: AtomicBool = AtomicBool::new(false);
static MOUSE_USED: AtomicBool = AtomicBool::new(false);
static mut MOUSE_ID: u8 = 0;
static mut PKT: [u8; 4] = [0; 4];
static mut PKT_N: usize = 0;
static mut PKT_LEN: usize = 3; // 4 once the IntelliMouse knock gives a wheel (id 3)
static mut WHEEL: u64 = 0; // wheel notches seen
static mut MBUT: u8 = 0;
static mut PACKETS: u64 = 0;
static mut MCLICKS: u64 = 0;
static mut RESYNCS: u64 = 0;
static mut OVERFLOWS: u64 = 0;
static mut STALE: u64 = 0;
static mut LAST_BYTE: u64 = 0; // timer::cycles() of the packet byte before
static mut RATE: u8 = 0; // samples a second set, 0 = left as the pad started
static mut RES: u8 = 0; // counts per mm set, 0 = left as the pad started
static AUX_IRQ: AtomicBool = AtomicBool::new(false);
static AUX_IRQS: AtomicU64 = AtomicU64::new(0);
static ACCEL: AtomicBool = AtomicBool::new(true);
static mut LAST_PKT: u64 = 0; // timer::cycles() when the packet before completed
static mut REM: (i32, i32) = (0, 0); // sixteenths of a count carried to the next packet
static mut SPED: u64 = 0; // packets moved more than 1:1

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
    // The IntelliMouse knock (rates 200, 100, 80, then ask the id again): a wheel mouse, or
    // a touchpad that offers scrolling in its PS/2 mode, answers id 3 and sends 4-byte
    // packets, the 4th byte the wheel. Anything else stays a plain 3-byte mouse.
    if MOUSE_ID == 0 && [200u8, 100, 80].iter().all(|&r| aux_cmd(0xF3) == Some(0xFA) && aux_cmd(r) == Some(0xFA)) {
        if aux_cmd(0xF2) == Some(0xFA) {
            MOUSE_ID = aux_read().unwrap_or(0);
            if MOUSE_ID == 3 {
                PKT_LEN = 4;
            }
        }
    }
    // Linux's psmouse settings: 100 samples a second, resolution 3 = 8 counts per mm
    if aux_cmd(0xF3) == Some(0xFA) && aux_cmd(100) == Some(0xFA) {
        RATE = 100;
    }
    if aux_cmd(0xE8) == Some(0xFA) && aux_cmd(3) == Some(0xFA) {
        RES = 8;
    }
    if aux_cmd(0xF4) != Some(0xFA) {
        crate::kprintln!("[ps2] the PS/2 mouse did not start streaming");
        return;
    }
    MOUSE.store(true, Ordering::Relaxed);
    crate::kprintln!(
        "[ps2] PS/2 mouse or touchpad (id {:#04x}) on the aux port, {}, polled",
        { MOUSE_ID },
        if PKT_LEN == 4 { "4-byte packets with a wheel" } else { "3-byte packets" }
    );
}

/// One mouse byte; every third (fourth with a wheel), from a byte with bit 3 set, completes a packet.
unsafe fn mouse_byte(b: u8) {
    let now = crate::timer::cycles();
    let ms = if AUX_IRQ.load(Ordering::Relaxed) { 40 } else { 500 };
    if PKT_N > 0 && now.wrapping_sub(LAST_BYTE) > crate::timer::hz() / 1000 * ms {
        STALE += 1; // the rest of that packet never came: start again from this byte
        PKT_N = 0;
    }
    LAST_BYTE = now;
    if PKT_N == 0 && b & 0x08 == 0 {
        RESYNCS += 1; // not a packet's first byte: wait for one
        return;
    }
    PKT[PKT_N] = b;
    PKT_N += 1;
    if PKT_N < PKT_LEN {
        return;
    }
    PKT_N = 0;
    let [h, x, y, z] = PKT;
    if PKT_LEN == 4 && z != 0 {
        // the wheel (signed; negative = rolled away from the user): scroll the console back
        WHEEL += (z as i8).unsigned_abs() as u64;
        if !crate::desk::shown() {
            crate::uart::console_wheel(z as i8 as i32);
        }
    }
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
    let ms = (now.wrapping_sub(LAST_PKT) / (crate::timer::hz() / 1000).max(1)).clamp(1000 / RATE.max(100) as u64, 100);
    LAST_PKT = now;
    if ms == 100 {
        REM = (0, 0); // after a pause, start fresh
    }
    let (ax, ay, sped) = accelerate(dx, dy, ms as u32, if RES == 0 { 4 } else { RES as u32 }, &mut *core::ptr::addr_of_mut!(REM));
    SPED += sped as u64;
    crate::xhci::pointer_input(buttons, ax, ay);
}

/// One move through the acceleration curve (the touchpad's packets and the USB mouse's
/// reports alike): `ms` since the move before, `per_mm` counts per millimetre, `rem` the
/// sixteenths carried from the move before. Returns the move to make and whether it was
/// sped up; 1:1 when acceleration is off or the move is slow.
pub(crate) fn accelerate(dx: i32, dy: i32, ms: u32, per_mm: u32, rem: &mut (i32, i32)) -> (i32, i32, bool) {
    if !ACCEL.load(Ordering::Relaxed) || (dx == 0 && dy == 0) {
        return (dx, dy, false);
    }
    let (ax, ay) = (dx.unsigned_abs(), dy.unsigned_abs());
    let counts = ax.max(ay) + ax.min(ay) / 2; // about the length of the move
    let g = gain16(counts * 1000 / ms.max(1) / per_mm.max(1)) as i32;
    let (fx, fy) = (dx * g + rem.0, dy * g + rem.1);
    let (mx, my) = (fx / 16, fy / 16);
    *rem = (fx - mx * 16, fy - my * 16);
    (mx, my, g > 16)
}

/// The acceleration at a speed in mm/s, in sixteenths: 1x up to 100 mm/s, rising in a
/// straight line to 3x at 400 mm/s and staying there.
fn gain16(mm_s: u32) -> u32 {
    if mm_s <= 100 {
        16
    } else if mm_s >= 400 {
        48
    } else {
        16 + (mm_s - 100) * 32 / 300
    }
}

/// `accel` / `accel on` / `accel off`: the pointer acceleration of the touchpad and the USB
/// mouse (one switch, one curve).
pub fn accel_cmd(arg: &str) {
    match arg.trim() {
        "on" => ACCEL.store(true, Ordering::Relaxed),
        "off" => ACCEL.store(false, Ordering::Relaxed),
        "" => {}
        _ => {
            crate::kprintln!("accel: say on or off (accel alone shows it)");
            return;
        }
    }
    accel_line();
    crate::xhci::accel_line();
}

/// Whether pointer acceleration is on (`accel on`, the default).
pub(crate) fn accel_on() -> bool {
    ACCEL.load(Ordering::Relaxed)
}

fn accel_line() {
    if !ACCEL.load(Ordering::Relaxed) {
        crate::kprintln!("ps2: acceleration off (1:1); `accel on` switches it on");
        return;
    }
    let x = |v: u32| {
        let g = gain16(v);
        alloc::format!("x{}.{:02}", g / 16, g % 16 * 100 / 16)
    };
    // SAFETY: scalar read of the counter.
    let sped = unsafe { SPED };
    crate::kprintln!(
        "ps2: acceleration on: {} at 50 mm/s, {} at 100, {} at 175, {} at 250, {} from 400; {} packet(s) sped up",
        x(50), x(100), x(175), x(250), x(400), sped
    );
}

/// Route IRQ 12 to the aux port and switch its interrupt on in the i8042 (after the IOAPIC
/// and the local APIC are up). Keys stay polled; nothing changes when there is no mouse.
pub fn enable_aux_irq() {
    if !MOUSE.load(Ordering::Relaxed) {
        return;
    }
    crate::cpu::cli();
    // SAFETY: i8042 port I/O with interrupts off, so the timer's poll cannot interleave.
    unsafe {
        if wait_write() {
            outb(STATUS, 0x20);
            if wait_read() {
                let cfg = inb(DATA);
                if wait_write() {
                    outb(STATUS, 0x60);
                    if wait_write() {
                        crate::cpu::ioapic_route(12, crate::cpu::PS2_VECTOR);
                        outb(DATA, cfg | 0b10); // aux interrupt on; the keyboard's stays off
                        AUX_IRQ.store(true, Ordering::Relaxed);
                    }
                }
            }
        }
    }
    poll(); // a byte already waiting raised no edge: take it now
    crate::cpu::sti();
}

/// IRQ 12: the aux port has a byte (and the keyboard may too): drain as the tick does.
pub fn aux_irq() {
    AUX_IRQS.fetch_add(1, Ordering::Relaxed);
    poll();
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

/// Called from the local-APIC timer interrupt and from IRQ 12 (both interrupt gates on the
/// boot core, so never nested): every key pressed since the last tick becomes console input.
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
            if code == 0x38 {
                ALT = !release; // left Alt, or right Alt (E0 38)
                continue;
            }
            if ext && (code == 0x5B || code == 0x5C) {
                // a Windows key: alone, it opens the desktop's launcher when released
                if release {
                    if crate::desk::shown() && !SUPER_COMBO {
                        crate::desk::post_key(crate::desk::KEY_SUPER);
                    }
                    SUPER_COMBO = false;
                }
                SUPER = !release;
                continue;
            }
            if !release && crate::desk::shown() {
                if SUPER {
                    SUPER_COMBO = true;
                }
                if let Some(k) = desk_key(code, ext) {
                    crate::desk::post_key(k);
                    continue;
                }
            }
            match code {
                0x2A | 0x36 => SHIFT = !release,
                0x3A if !release => CAPS = !CAPS,
                0x48 | 0x50 if ext && !release => crate::uart::history_key(code == 0x48), // Up/Down: shell history
                0x49 | 0x51 if ext && !release && SHIFT => crate::uart::console_page(code == 0x49), // Shift+PgUp/PgDn
                _ if release || ext && code != 0x1C => {} // releases; Left/Right and the like
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
        let set = |v: u8, unit: &'static str| if v == 0 { alloc::format!("as found") } else { alloc::format!("{} {}", v, unit) };
        crate::kprintln!(
            "ps2: rate {}, resolution {}; {}",
            set(RATE, "samples/s"),
            set(RES, "counts/mm"),
            if AUX_IRQ.load(Ordering::Relaxed) {
                alloc::format!("IRQ 12, {} interrupts", AUX_IRQS.load(Ordering::Relaxed))
            } else {
                alloc::format!("polled only (no IRQ 12)")
            }
        );
        accel_line();
        if PKT_LEN == 4 {
            crate::kprintln!("ps2: wheel on (4-byte packets); {} notch(es), 3 console lines each", { WHEEL });
        } else {
            crate::kprintln!("ps2: no wheel (3-byte packets)");
        }
        if RESYNCS + OVERFLOWS + STALE > 0 {
            crate::kprintln!(
                "ps2: {} stray byte(s) skipped, {} overflowing packet(s), {} part packet(s) dropped",
                { RESYNCS }, { OVERFLOWS }, { STALE }
            );
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
