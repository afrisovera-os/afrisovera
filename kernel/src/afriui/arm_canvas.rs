//! The desktop's canvas on the phone (ARM), step P2 of docs/design/afriui-phone-plan.md
//! (architect's OK, 11 Nov 2026). Compiled only by the ARM kernel, as `canvas`; the PC has its
//! own (kernel/src/pc/canvas.rs). The Sovereign desktop (kernel/src/afriui/) calls only these.
//!
//! The canvas IS the ramfb framebuffer (framebuffer.rs, 640x480): the desktop draws straight
//! into the buffer QEMU scans out, at UI scale 1, with no second buffer (decision 2; a back
//! buffer and a page flip come with P3, a real phone panel). So there is nothing to copy:
//! `mark` and `flush` do nothing and `copy_perf` reports no copies.

/// The tallest canvas: verify's two row tables are sized from it (decision 3: 2 x 480 x 8 bytes
/// = 7.5 KB on the phone).
pub const MAX_H: usize = crate::framebuffer::HEIGHT;

pub fn width() -> usize {
    crate::framebuffer::WIDTH
}
pub fn height() -> usize {
    crate::framebuffer::HEIGHT
}
pub fn scale() -> usize {
    1
}

/// The canvas pixels, `width()` to a row: the framebuffer itself.
pub fn pixels() -> *const u32 {
    crate::framebuffer::pixels()
}

/// Fill a rectangle, clipped to the screen, straight into the framebuffer.
pub fn fill_rect(x0: usize, y0: usize, w: usize, h: usize, rgb: u32) {
    crate::framebuffer::fill_rect(x0, y0, w, h, rgb);
}

/// Text coverage mixed into the framebuffer (afriui.md section 26).
pub fn blend(x0: usize, y0: usize, w: usize, h: usize, a: &[u8], astride: usize, rgb: u32, gamma: &[u8; 256]) {
    crate::framebuffer::blend(x0, y0, w, h, a, astride, rgb, gamma);
}

/// Black at alphas over the framebuffer (soft shadows, afriui.md section 30).
pub fn shade(x0: usize, y0: usize, w: usize, h: usize, a: &[u8], astride: usize) {
    crate::framebuffer::shade(x0, y0, w, h, a, astride);
}

/// Black at one alpha over a block of the framebuffer.
pub fn shade_const(x0: usize, y0: usize, w: usize, h: usize, al: u32) {
    crate::framebuffer::shade_const(x0, y0, w, h, al);
}

/// One edge pixel mixed into the framebuffer.
pub fn mix(x: usize, y: usize, rgb: u32, al: u32) {
    crate::framebuffer::mix(x, y, rgb, al);
}

/// Something was drawn: ramfb shows it as it is (nothing to copy).
pub fn mark() {}

/// Nothing to copy to the screen on the phone.
pub fn flush() {}

/// (last, worst) copy time, copies and rows: always zero on the phone (no copy).
pub fn copy_perf() -> (u64, u64, u64, u64) {
    (0, 0, 0, 0)
}
