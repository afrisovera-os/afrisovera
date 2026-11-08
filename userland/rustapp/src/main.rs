//! A real **compiled-Rust** EL0 userland app for AfriSovera.
//!
//! Unlike the hand-written asm user stubs in the kernel, this is ordinary Rust,
//! compiled for `aarch64-unknown-none`, linked at the user code VA (0x8000_0000),
//! and run at **EL0** by the kernel. It reaches the OS only through **SVC
//! syscalls** — the same capability-checked boundary as everything else. This is
//! the foundation for hosting larger userland programs (e.g. the Nia interpreter)
//! as EL0 processes.
#![no_std]
#![no_main]

use core::arch::{asm, global_asm};
use core::panic::PanicInfo;

// _start must be the very first byte (0x8000_0000). Keep it in .text.entry and
// jump to the Rust entry.
#[cfg(target_arch = "aarch64")]
global_asm!(
    r#"
.section .text.entry
.global _start
_start:
    b   rust_main
"#
);

#[cfg(target_arch = "aarch64")]
const SYS_DEBUG: u64 = 0;
#[cfg(target_arch = "aarch64")]
const SYS_EXIT: u64 = 3;

#[cfg(target_arch = "aarch64")]
#[inline(always)]
fn sys_debug(v: u64) {
    // SAFETY: SYS_DEBUG prints x0; clobbers x0/x8 only.
    unsafe {
        asm!("svc #0", in("x8") SYS_DEBUG, in("x0") v, lateout("x0") _, options(nostack));
    }
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
fn sys_exit() -> ! {
    // SAFETY: SYS_EXIT never returns to the app.
    unsafe {
        asm!("svc #0", in("x8") SYS_EXIT, options(noreturn, nostack));
    }
}

// The PC (x86-64, ring 3, 5 Nov 2026): the same program through the PC kernel's
// `int 0x80` ABI (rax = call, rdi = argument; SYS_PRINT 1, SYS_EXIT 0 -- kernel/src/pc/proc.rs),
// linked at the PC's user code VA by linker-x86_64.ld. Built by scripts/build-userapp.sh
// into kernel/src/pc/rustapp-x86_64.bin.
#[cfg(target_arch = "x86_64")]
global_asm!(
    r#"
.section .text.entry
.global _start
_start:
    jmp rust_main
"#
);

#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn sys_debug(v: u64) {
    // SAFETY: SYS_PRINT prints rdi and returns 0 in rax.
    unsafe {
        asm!("int 0x80", inlateout("rax") 1u64 => _, in("rdi") v, options(nostack));
    }
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn sys_exit() -> ! {
    // SAFETY: SYS_EXIT never returns to the app.
    unsafe {
        asm!("int 0x80", in("rax") 0u64, in("rdi") 0u64, options(noreturn, nostack));
    }
}

#[no_mangle]
pub extern "C" fn rust_main() -> ! {
    // Real Rust logic running at EL0: a loop and some arithmetic on the stack.
    let mut acc: u64 = 0;
    for i in 1..=10u64 {
        acc = acc.wrapping_add(i);
    }
    sys_debug(acc); // prints 55 (0x37)
    sys_debug(0xC0DE); // marker: compiled Rust executed at EL0
    sys_exit();
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    sys_exit();
}
