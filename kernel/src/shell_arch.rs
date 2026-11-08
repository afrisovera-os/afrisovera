// The architecture seam of `shell.rs` (PC track step 2, docs/05): textually included at
// the top of `shell.rs` with `include!`, so `shell.rs` keeps every line number it had.
//
// THE RULE: on aarch64 every macro here expands to EXACTLY the tokens that stood at the
// call site before, so the ARM kernel compiles to the same bytes (checked by md5 after each
// change). On x86-64 (the PC kernel, `mansa-pc`) they route to `crate::shell_pc`.

// The ARM system-register reads (`core::arch::asm!("mrs ...")`) are NOT wrapped in a
// macro: measured 4 Nov 2026, routing them through one moved their debug-line records into
// this file and changed the ARM binary's md5 (the code bytes stayed equal). Instead each
// site carries `#[cfg(target_arch = "aarch64")]` at the END of the line before it (so the
// asm keeps its line AND column), plus, after it on the same line, a
// `#[cfg(not(target_arch = "aarch64"))]` assignment from `crate::shell_pc::sysreg`.

/// The first statement of `exec`: on the PC, commands whose ARM form touches ARM-only
/// hardware (MMIO bases that a PC does not have, ARM system registers) are answered by
/// `shell_pc::intercept` first. On aarch64 it expands to nothing.
#[cfg(target_arch = "aarch64")]
macro_rules! pc_intercept {
    ($line:ident) => {};
}
#[cfg(not(target_arch = "aarch64"))]
macro_rules! pc_intercept {
    ($line:ident) => {
        if crate::shell_pc::intercept($line) {
            return;
        }
    };
}
