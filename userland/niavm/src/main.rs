//! Nia mini-interpreter, compiled as an **EL0 userland app**.
//!
//! This is the real "interpreter in a userspace process": a small WASM
//! interpreter written in Rust, compiled for `aarch64-unknown-none`, run at
//! **EL0**. It executes an embedded WASM module; when the module calls its host
//! import (`print`), the interpreter reaches the OS through an **SVC syscall** —
//! the capability-checked boundary. The kernel-side Nia (nia.rs) runs the runtime
//! in the supervisor; this proves the runtime can run *as a sandboxed process*.
//!
//! Subset: sections (import/export/code) + i32.const/add/sub/mul/call/end — enough
//! for a straight-line module. Uses only the stack (no writable statics).
#![no_std]
#![no_main]

use core::arch::{asm, global_asm};
use core::panic::PanicInfo;

#[cfg(target_arch = "aarch64")] global_asm!(
    r#"
.section .text.entry
.global _start
_start:
    b   rust_main
"#
);

#[cfg(target_arch = "aarch64")] const SYS_DEBUG: u64 = 0;
#[cfg(target_arch = "aarch64")] const SYS_EXIT: u64 = 3;
#[cfg(target_arch = "aarch64")] const SYS_PUTC: u64 = 11;
#[cfg(target_arch = "aarch64")] const SYS_TIME: u64 = 12;
#[cfg(target_arch = "aarch64")] const SYS_AI: u64 = 13;
#[cfg(target_arch = "aarch64")] const SYS_SPEAK: u64 = 14;

#[cfg(target_arch = "aarch64")] fn sys_debug(v: u64) {
    // SAFETY: SYS_DEBUG prints x0; clobbers x0/x8.
    unsafe {
        asm!("svc #0", in("x8") SYS_DEBUG, in("x0") v, lateout("x0") _, options(nostack));
    }
}
#[cfg(target_arch = "aarch64")] fn sys_putc(v: u64) {
    unsafe {
        asm!("svc #0", in("x8") SYS_PUTC, in("x0") v, lateout("x0") _, options(nostack));
    }
}
#[cfg(target_arch = "aarch64")] fn sys_time() -> u64 {
    let r: u64;
    unsafe {
        asm!("svc #0", in("x8") SYS_TIME, out("x0") r, options(nostack));
    }
    r
}
#[cfg(target_arch = "aarch64")] fn sys_ai(prompt: u64, plen: u64, out: u64, cap: u64) -> u64 {
    let r: u64;
    unsafe {
        asm!(
            "svc #0",
            in("x8") SYS_AI, in("x0") prompt, in("x1") plen, in("x2") out, in("x3") cap,
            lateout("x0") r, options(nostack),
        );
    }
    r
}
#[cfg(target_arch = "aarch64")] fn sys_speak(text: u64, tlen: u64) {
    // SAFETY: SYS_SPEAK reads `tlen` bytes at `text` and speaks them; clobbers x0.
    unsafe {
        asm!(
            "svc #0",
            in("x8") SYS_SPEAK, in("x0") text, in("x1") tlen,
            lateout("x0") _, options(nostack),
        );
    }
}
#[cfg(target_arch = "aarch64")] fn sys_exit(code: u64) -> ! {
    unsafe {
        asm!("svc #0", in("x8") SYS_EXIT, in("x0") code, options(noreturn, nostack));
    }
}

// Embedded WASM module chaining POINTER-ARG host calls at EL0:
//   run() { let n = ai(&"WHO ARE YOU",11,&mem[64],192); speak(&mem[64],n); print(n); }
// The EL0 app asks the on-device AI and SPEAKS the answer — all via syscalls.
#[cfg(not(feature = "sandbox-test"))]
#[rustfmt::skip]
static WASM: &[u8] = &[
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x15, 0x04, 0x60,
    0x04, 0x7F, 0x7F, 0x7F, 0x7F, 0x01, 0x7F, 0x60, 0x02, 0x7F, 0x7F, 0x00,
    0x60, 0x01, 0x7F, 0x00, 0x60, 0x00, 0x00, 0x02, 0x22, 0x03, 0x03, 0x65,
    0x6E, 0x76, 0x02, 0x61, 0x69, 0x00, 0x00, 0x03, 0x65, 0x6E, 0x76, 0x05,
    0x73, 0x70, 0x65, 0x61, 0x6B, 0x00, 0x01, 0x03, 0x65, 0x6E, 0x76, 0x05,
    0x70, 0x72, 0x69, 0x6E, 0x74, 0x00, 0x02, 0x03, 0x02, 0x01, 0x03, 0x05,
    0x03, 0x01, 0x00, 0x01, 0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00,
    0x03, 0x0A, 0x1F, 0x01, 0x1D, 0x01, 0x01, 0x7F, 0x41, 0x00, 0x41, 0x0B,
    0x41, 0xC0, 0x00, 0x41, 0xC0, 0x01, 0x10, 0x00, 0x21, 0x00, 0x41, 0xC0,
    0x00, 0x20, 0x00, 0x10, 0x01, 0x20, 0x00, 0x10, 0x02, 0x0B, 0x0B, 0x11,
    0x01, 0x00, 0x41, 0x00, 0x0B, 0x0B, 0x57, 0x48, 0x4F, 0x20, 0x41, 0x52,
    0x45, 0x20, 0x59, 0x4F, 0x55,
];

// The test build (feature `sandbox-test`, built into the test images only; release-guard refuses it) runs these two
// modules, one after the other: the two sandbox findings of OS_REQUESTS item 9 (scripts/niavm-sandbox-test.sh).
#[cfg(feature = "sandbox-test")]
#[rustfmt::skip]
static TEST_UNKNOWN: &[u8] = &[
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x0B, 0x03, 0x60,
    0x00, 0x00, 0x60, 0x01, 0x7F, 0x00, 0x60, 0x00, 0x00, 0x02, 0x18, 0x02,
    0x03, 0x65, 0x6E, 0x76, 0x04, 0x6E, 0x6F, 0x70, 0x65, 0x00, 0x00, 0x03,
    0x65, 0x6E, 0x76, 0x05, 0x70, 0x72, 0x69, 0x6E, 0x74, 0x00, 0x01, 0x03,
    0x02, 0x01, 0x02, 0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x02,
    0x0A, 0x0A, 0x01, 0x08, 0x00, 0x10, 0x00, 0x41, 0x01, 0x10, 0x01, 0x0B,
];
#[cfg(feature = "sandbox-test")]
#[rustfmt::skip]
static TEST_OOB: &[u8] = &[
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x0D, 0x03, 0x60,
    0x02, 0x7F, 0x7F, 0x00, 0x60, 0x01, 0x7F, 0x00, 0x60, 0x00, 0x00, 0x02,
    0x19, 0x02, 0x03, 0x65, 0x6E, 0x76, 0x05, 0x73, 0x70, 0x65, 0x61, 0x6B,
    0x00, 0x00, 0x03, 0x65, 0x6E, 0x76, 0x05, 0x70, 0x72, 0x69, 0x6E, 0x74,
    0x00, 0x01, 0x03, 0x02, 0x01, 0x02, 0x07, 0x07, 0x01, 0x03, 0x72, 0x75,
    0x6E, 0x00, 0x02, 0x0A, 0x10, 0x01, 0x0E, 0x00, 0x41, 0xE8, 0x07, 0x41,
    0xE4, 0x00, 0x10, 0x00, 0x41, 0x02, 0x10, 0x01, 0x0B,
];

/// Text to the console through putc (the kernel prints it as the process's output).
fn say(t: &[u8]) {
    for &b in t {
        sys_putc(b as u64);
    }
}

/// How a run ended: 0 the module returned; anything else stopped it (the line was printed first).
const STOPPED: u64 = 1;

/// The DENIED line nia.rs prints for a host call the app may not make, then the TRAP line.
fn denied(name: &[u8]) -> u64 {
    say(b"    [nia] DENIED: app called host '");
    say(name);
    say(b"' without the capability\n    [nia] TRAP: capability denied\n");
    STOPPED
}

/// The bytes [off, off + len) of the module's memory, as WebAssembly says: both unsigned, no wrap, inside `mem`.
/// None is a TRAP (nia.rs's mem_range).
fn mem_range(mem: &[u8], off: i32, len: i32) -> Option<(u64, u64)> {
    let (o, l) = (off as u32 as u64, len as u32 as u64);
    if o + l > mem.len() as u64 {
        return None;
    }
    Some((mem.as_ptr() as u64 + o, l))
}

fn oob() -> u64 {
    say(b"    [nia] TRAP: out of bounds memory access\n");
    STOPPED
}

fn uleb(b: &[u8], start: usize) -> (usize, usize) {
    let mut c = start;
    let mut r = 0usize;
    let mut s = 0;
    loop {
        let byte = b[c];
        c += 1;
        r |= ((byte & 0x7f) as usize) << s;
        if byte & 0x80 == 0 {
            break;
        }
        s += 7;
    }
    (r, c - start)
}
fn sleb(b: &[u8], start: usize) -> (i32, usize) {
    let mut c = start;
    let mut r = 0i64;
    let mut s = 0u32;
    let mut byte;
    loop {
        byte = b[c];
        c += 1;
        r |= ((byte & 0x7f) as i64) << s;
        s += 7;
        if byte & 0x80 == 0 {
            break;
        }
    }
    if s < 64 && (byte & 0x40) != 0 {
        r |= -1i64 << s;
    }
    (r as i32, c - start)
}

fn run_module(m: &[u8], mem: &mut [u8]) -> u64 {
    if m.len() < 8 || &m[0..4] != b"\0asm" {
        sys_debug(0xBAD0);
        return STOPPED;
    }
    let mut c = 8;
    let mut imports = 0usize;
    let mut import_host = [255u8; 8]; // host id per import (0=print,1=putc,2=time,3=ai,4=speak)
    let mut import_name: [&[u8]; 8] = [b""; 8]; // for the DENIED line
    let mut run_idx = usize::MAX;
    let mut code_off = [0usize; 8];
    let mut code_len = [0usize; 8];
    let mut ncode = 0usize;
    while c < m.len() {
        let id = m[c];
        c += 1;
        let (sz, a) = uleb(m, c);
        c += a;
        let end = c + sz;
        if id == 2 {
            let (cnt, a) = uleb(m, c);
            c += a;
            for _ in 0..cnt {
                let (ml, a) = uleb(m, c);
                c += a + ml;
                let (nl, a) = uleb(m, c);
                c += a;
                let name = &m[c..c + nl];
                c += nl;
                let kind = m[c];
                c += 1;
                let (_d, a) = uleb(m, c);
                c += a;
                if kind == 0 {
                    let hid: u8 = if name == b"print" {
                        0
                    } else if name == b"putc" {
                        1
                    } else if name == b"time" {
                        2
                    } else if name == b"ai" {
                        3
                    } else if name == b"speak" {
                        4
                    } else {
                        255
                    };
                    if imports >= 8 {
                        // more imports than this interpreter keeps: refused, never a call into the wrong one
                        say(b"    [nia] parse error: more than 8 imports\n");
                        return STOPPED;
                    }
                    import_host[imports] = hid;
                    import_name[imports] = name;
                    imports += 1;
                }
            }
        } else if id == 7 {
            let (cnt, a) = uleb(m, c);
            c += a;
            for _ in 0..cnt {
                let (nl, a) = uleb(m, c);
                c += a;
                let name = &m[c..c + nl];
                c += nl;
                let kind = m[c];
                c += 1;
                let (idx, a) = uleb(m, c);
                c += a;
                if kind == 0 && name == b"run" {
                    run_idx = idx;
                }
            }
        } else if id == 10 {
            let (cnt, a) = uleb(m, c);
            c += a;
            for i in 0..cnt {
                let (bsz, a) = uleb(m, c);
                c += a;
                if i < 8 {
                    code_off[i] = c;
                    code_len[i] = bsz;
                    ncode += 1;
                }
                c += bsz;
            }
        } else if id == 11 {
            // data segments -> copy into linear memory
            let (cnt, a) = uleb(m, c);
            c += a;
            for _ in 0..cnt {
                let (flags, a) = uleb(m, c);
                c += a;
                let mut off = 0usize;
                if flags == 0 {
                    if m[c] == 0x41 {
                        let (v, a) = sleb(m, c + 1);
                        off = v as usize;
                        c += 1 + a;
                    }
                    c += 1; // 0x0B end of offset expr
                }
                let (len, a) = uleb(m, c);
                c += a;
                for i in 0..len {
                    if off + i < mem.len() {
                        mem[off + i] = m[c + i];
                    }
                }
                c += len;
            }
        }
        c = end;
    }
    if run_idx == usize::MAX || run_idx < imports {
        sys_debug(0xBAD1);
        return STOPPED;
    }
    let li = run_idx - imports;
    if li >= ncode {
        sys_debug(0xBAD2);
        return STOPPED;
    }
    exec(&m[code_off[li]..code_off[li] + code_len[li]], imports, &import_host, &import_name, mem)
}

fn exec(code: &[u8], imports: usize, import_host: &[u8; 8], import_name: &[&[u8]; 8], mem: &mut [u8]) -> u64 {
    let mut c = 0;
    let (ngroups, a) = uleb(code, c);
    c += a;
    for _ in 0..ngroups {
        let (_n, a) = uleb(code, c);
        c += a + 1;
    }
    let mut st = [0i32; 64];
    let mut sp = 0usize;
    let mut locals = [0i32; 16]; // WASM locals (zero-initialized)
    while c < code.len() {
        let op = code[c];
        c += 1;
        match op {
            0x41 => {
                let (v, a) = sleb(code, c);
                c += a;
                st[sp] = v;
                sp += 1;
            }
            0x20 => {
                // local.get
                let (i, a) = uleb(code, c);
                c += a;
                st[sp] = locals[i & 15];
                sp += 1;
            }
            0x21 => {
                // local.set
                let (i, a) = uleb(code, c);
                c += a;
                sp -= 1;
                locals[i & 15] = st[sp];
            }
            0x6A | 0x6B | 0x6C => {
                sp -= 1;
                let b = st[sp];
                let a = st[sp - 1];
                st[sp - 1] = match op {
                    0x6A => a.wrapping_add(b),
                    0x6B => a.wrapping_sub(b),
                    _ => a.wrapping_mul(b),
                };
            }
            0x10 => {
                let (f, a) = uleb(code, c);
                c += a;
                if f >= imports {
                    // a call to one of the module's own functions: this interpreter cannot make it, so it stops
                    // rather than skip it
                    say(b"    [nia] TRAP: calls between functions are not supported here\n");
                    return STOPPED;
                }
                {
                    // dispatch the host import to the matching OS syscall
                    match import_host[f] {
                        0 => {
                            sp -= 1;
                            sys_debug(st[sp] as u32 as u64);
                        } // print
                        1 => {
                            sp -= 1;
                            sys_putc(st[sp] as u32 as u64);
                        } // putc
                        2 => {
                            st[sp] = sys_time() as i32;
                            sp += 1;
                        } // time (result)
                        3 => {
                            // ai(prompt_off, len, out_off, cap) -> reply_len.
                            // Translate WASM offsets to real addresses in this
                            // app's memory and hand them to the kernel syscall.
                            let (cap, out_off, len, prompt_off) = (st[sp - 1], st[sp - 2], st[sp - 3], st[sp - 4]);
                            sp -= 4;
                            let (Some((prompt, len)), Some((out, cap))) =
                                (mem_range(mem, prompt_off, len), mem_range(mem, out_off, cap))
                            else {
                                return oob();
                            };
                            let r = sys_ai(prompt, len, out, cap);
                            st[sp] = r as i32;
                            sp += 1;
                        }
                        4 => {
                            // speak(text_off, len) -> () : Asili Voice at EL0.
                            let (len, text_off) = (st[sp - 1], st[sp - 2]);
                            sp -= 2;
                            let Some((text, len)) = mem_range(mem, text_off, len) else {
                                return oob();
                            };
                            sys_speak(text, len);
                        }
                        _ => return denied(import_name[f]),
                    }
                }
            }
            0x0B => return 0,
            _ => {
                sys_debug(0xBADC0DE);
                return STOPPED;
            }
        }
    }
    0
}

#[no_mangle]
pub extern "C" fn rust_main() -> ! {
    sys_debug(0xE10); // marker: Nia mini-interpreter starting at EL0
    let mut mem = [0u8; 1024]; // the module's linear memory (on the EL0 stack)
    #[cfg(not(feature = "sandbox-test"))]
    let code = run_module(WASM, &mut mem);
    #[cfg(feature = "sandbox-test")]
    let code = {
        // each module on fresh memory; the exit code has bit 0 for the first stopped, bit 1 for the second
        let a = run_module(TEST_UNKNOWN, &mut mem);
        say(if a == 0 { b"[niavm] test 1 (unknown import): ran to the end\n" } else { b"[niavm] test 1 (unknown import): stopped\n" });
        mem = [0u8; 1024];
        let b = run_module(TEST_OOB, &mut mem);
        say(if b == 0 { b"[niavm] test 2 (offset past memory): ran to the end\n" } else { b"[niavm] test 2 (offset past memory): stopped\n" });
        (a & 1) | ((b & 1) << 1)
    };
    sys_exit(code);
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    sys_exit(4);
}

// The PC (x86-64 ring 3, 5 Nov 2026): the same interpreter through the PC kernel's int 0x80
// ABI -- rax = call (PRINT 1, EXIT 0; PUTC/TIME/AI/SPEAK keep the ARM numbers 11-14),
// arguments in rdi, rsi, rdx, rcx (kernel/src/pc/proc.rs). Linked at the PC's USER_BASE by
// linker-x86_64.ld; built by scripts/build-userapp.sh into kernel/src/pc/niavm-x86_64.bin.
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
fn sys(n: u64, a: u64, b: u64, c: u64, d: u64) -> u64 {
    let r: u64;
    // SAFETY: one system call; the kernel preserves every register but rax.
    unsafe {
        asm!("int 0x80", inlateout("rax") n => r, in("rdi") a, in("rsi") b, in("rdx") c, in("rcx") d, options(nostack));
    }
    r
}
#[cfg(target_arch = "x86_64")]
fn sys_debug(v: u64) {
    sys(1, v, 0, 0, 0);
}
#[cfg(target_arch = "x86_64")]
fn sys_putc(v: u64) {
    sys(11, v, 0, 0, 0);
}
#[cfg(target_arch = "x86_64")]
fn sys_time() -> u64 {
    sys(12, 0, 0, 0, 0)
}
#[cfg(target_arch = "x86_64")]
fn sys_ai(prompt: u64, plen: u64, out: u64, cap: u64) -> u64 {
    sys(13, prompt, plen, out, cap)
}
#[cfg(target_arch = "x86_64")]
fn sys_speak(text: u64, tlen: u64) {
    sys(14, text, tlen, 0, 0);
}
#[cfg(target_arch = "x86_64")]
fn sys_exit(code: u64) -> ! {
    // SAFETY: SYS_EXIT never returns to the program.
    unsafe {
        asm!("int 0x80", in("rax") 0u64, in("rdi") code, options(noreturn, nostack));
    }
}
