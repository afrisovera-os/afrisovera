//! Raspberry Pi (BCM2837) framebuffer via the **VideoCore mailbox** property
//! interface. Real hardware (and QEMU `-M raspi3b`) has no `ramfb`; the GPU owns
//! the display and the ARM core asks for a framebuffer through a mailbox message.
//! This is the Pi equivalent of `framebuffer.rs` (which is QEMU-virt/ramfb only).
//! Board-gated: only compiled for `--features board-raspi`.

use core::ptr::{addr_of_mut, read_volatile, write_volatile};

// Sourced from the board map (`board::MAILBOX_BASE`, BCM2837 = 0x3F00_B880) rather
// than repeated here, so a board with a different mailbox base never drifts out of
// sync with this driver by editing only one of the two copies.
const MBOX_BASE: usize = crate::board::MAILBOX_BASE;
const MBOX_READ: usize = MBOX_BASE + 0x00;
const MBOX_STATUS: usize = MBOX_BASE + 0x18;
const MBOX_WRITE: usize = MBOX_BASE + 0x20;
const MBOX_FULL: u32 = 0x8000_0000;
const MBOX_EMPTY: u32 = 0x4000_0000;
const CHANNEL_PROP: u32 = 8; // ARM -> VideoCore property channel

pub const WIDTH: usize = 640;
pub const HEIGHT: usize = 480;

// The property message must be 16-byte aligned in RAM (low 4 bits carry the channel).
#[repr(C, align(16))]
struct Mbox {
    m: [u32; 36],
}
static mut MBOX: Mbox = Mbox { m: [0; 36] };

static mut FB_BASE: usize = 0;
static mut FB_PITCH: usize = 0;

/// Ask the GPU to allocate a 640x480x32 framebuffer. Returns true on success.
pub fn init() -> bool {
    // SAFETY: single-threaded bring-up; MMIO + a RAM message buffer.
    unsafe {
        let m = addr_of_mut!(MBOX.m) as *mut u32;
        #[rustfmt::skip]
        let msg: [u32; 35] = [
            35 * 4, 0,                                   // total size, request
            0x0004_8003, 8, 8, WIDTH as u32, HEIGHT as u32, // set physical w/h
            0x0004_8004, 8, 8, WIDTH as u32, HEIGHT as u32, // set virtual w/h
            0x0004_8009, 8, 8, 0, 0,                     // set virtual offset
            0x0004_8005, 4, 4, 32,                       // set depth (bpp)
            0x0004_8006, 4, 4, 1,                        // set pixel order (RGB)
            0x0004_0001, 8, 8, 4096, 0,                  // allocate buffer -> base, size
            0x0004_0008, 4, 4, 0,                        // get pitch -> pitch
            0,                                           // end tag
        ];
        for (i, &v) in msg.iter().enumerate() {
            write_volatile(m.add(i), v);
        }
        core::arch::asm!("dsb sy");

        let addr = addr_of_mut!(MBOX) as u32;
        let val = (addr & !0xF) | CHANNEL_PROP;
        while read_volatile(MBOX_STATUS as *const u32) & MBOX_FULL != 0 {}
        write_volatile(MBOX_WRITE as *mut u32, val);
        loop {
            while read_volatile(MBOX_STATUS as *const u32) & MBOX_EMPTY != 0 {}
            if read_volatile(MBOX_READ as *const u32) == val {
                break;
            }
        }
        core::arch::asm!("dsb sy");

        if read_volatile(m.add(1)) != 0x8000_0000 {
            crate::kprintln!("[pi-fb] mailbox request failed (resp={:#x})", read_volatile(m.add(1)));
            return false;
        }
        // The GPU returns a bus address; mask to the ARM physical address.
        let base = (read_volatile(m.add(28)) & 0x3FFF_FFFF) as usize;
        let pitch = read_volatile(m.add(33)) as usize;
        if base == 0 || pitch == 0 {
            crate::kprintln!("[pi-fb] no framebuffer (base={:#x} pitch={})", base, pitch);
            return false;
        }
        FB_BASE = base;
        FB_PITCH = pitch;
        crate::kprintln!("[pi-fb] framebuffer @ {:#010x}, pitch {}, {}x{}x32", base, pitch, WIDTH, HEIGHT);
        true
    }
}

fn px(x: usize, y: usize, rgb: u32) {
    // SAFETY: bounds-checked write into the GPU framebuffer.
    unsafe {
        if FB_BASE == 0 || x >= WIDTH || y >= HEIGHT {
            return;
        }
        // QEMU's Pi framebuffer is BGR — swap R and B so 0xRRGGBB shows correctly.
        let (r, g, b) = ((rgb >> 16) & 0xff, (rgb >> 8) & 0xff, rgb & 0xff);
        let bgr = (b << 16) | (g << 8) | r;
        let fb = FB_BASE as *mut u32;
        write_volatile(fb.add(y * (FB_PITCH / 4) + x), bgr);
    }
}

/// Draw the AfriSovera splash (pan-African bands + gold border).
pub fn splash() {
    // SAFETY: FB_BASE guarded in px().
    unsafe {
        if FB_BASE == 0 {
            return;
        }
    }
    const GREEN: u32 = 0x2a_8f3f;
    const GOLD: u32 = 0xf2_c14e;
    const RED: u32 = 0xc8_1e2a;
    const DARK: u32 = 0x0d_0d14;
    for y in 0..HEIGHT {
        // three horizontal bands with dark margins
        let band = if y < 20 || y >= HEIGHT - 20 {
            DARK
        } else if y < HEIGHT / 3 {
            GREEN
        } else if y < 2 * HEIGHT / 3 {
            GOLD
        } else {
            RED
        };
        for x in 0..WIDTH {
            // gold border
            let c = if x < 8 || x >= WIDTH - 8 || y < 8 || y >= HEIGHT - 8 { GOLD } else { band };
            px(x, y, c);
        }
    }
}
