//! Ring 3 on the PC: running a program at the CPU's lowest privilege, the services it may
//! ask for (system calls through `int 0x80`), and what happens when it misbehaves.
//!
//! A program is a few position-independent instructions copied into a fresh frame mapped
//! for ring 3 at `mmu::USER_BASE`, with one stack page above it. `run` enters it with
//! `iretq` (CS/SS = the ring-3 selectors) and returns when it ends: by the exit call, or by
//! a fault, which the trap handler turns into a stop of THAT program — the kernel goes on.
//! Kernel memory carries no user bit in the kernel's tables, so any touch of it from ring 3
//! is a page fault; privileged instructions (cli, hlt, mov cr3) are a general-protection
//! fault. This is the x86-64 twin of the ARM kernel's EL0 tasks, in its first, single-task
//! form: no scheduler yet, the kernel waits for each program to end.

use crate::cpu::{TrapFrame, USER_CS, USER_DS};
use crate::{frame, mmu};
use core::arch::global_asm;
use core::sync::atomic::{AtomicU64, Ordering};

/// System-call numbers (rax), argument in rdi, result in rax.
pub const SYS_EXIT: u64 = 0;
pub const SYS_PRINT: u64 = 1;
pub const SYS_WHOAMI: u64 = 2; // the privilege level the caller ran at, as the CPU saw it

/// `run`'s result for a program stopped by a fault: this bit plus the vector.
pub const KILLED: u64 = 1 << 63;

pub static SYSCALLS: AtomicU64 = AtomicU64::new(0);
pub static LAST_FAULT_ERROR: AtomicU64 = AtomicU64::new(0);
pub static LAST_FAULT_ADDR: AtomicU64 = AtomicU64::new(0);

global_asm!(
    r#"
// user_enter(rip = rdi, rsp = rsi, arg = rdx) -> rax: the program's exit code.
// Saves the kernel's callee-saved registers and stack, then iretq to ring 3.
.global user_enter
user_enter:
    push rbx
    push rbp
    push r12
    push r13
    push r14
    push r15
    mov [rip + user_ksave], rsp
    push {uds}          // SS
    push rsi            // RSP
    push 0x2            // RFLAGS: interrupts off (no scheduler yet)
    push {ucs}          // CS
    push rdi            // RIP
    mov rdi, rdx        // the program's one argument
    xor eax, eax        // nothing of the kernel's leaks into ring 3
    xor ebx, ebx
    xor ecx, ecx
    xor edx, edx
    xor esi, esi
    xor ebp, ebp
    xor r8d, r8d
    xor r9d, r9d
    xor r10d, r10d
    xor r11d, r11d
    xor r12d, r12d
    xor r13d, r13d
    xor r14d, r14d
    xor r15d, r15d
    iretq

// user_enter_frame(frame = rdi) -> rax: like user_enter, but every register, the flags
// and the stack come from a saved TrapFrame (a process's first or resumed state): point
// rsp at the frame and take the common trap exit (pop the registers, iretq).
.global user_enter_frame
user_enter_frame:
    push rbx
    push rbp
    push r12
    push r13
    push r14
    push r15
    mov [rip + user_ksave], rsp
    mov rsp, rdi
    jmp isr_restore

// user_return(code = rdi) -> !: called from a trap handler; drops the trap frame and
// returns from user_enter with `code`.
.global user_return
user_return:
    mov rsp, [rip + user_ksave]
    mov rax, rdi
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbp
    pop rbx
    ret

// The programs. Position-independent, copied into a ring-3 page before each run.
.global user_prog_hello
.global user_prog_hello_end
user_prog_hello:
    mov eax, 1          // print(42)
    mov edi, 42
    int 0x80
    mov eax, 2          // whoami() -> the privilege level the CPU saw
    int 0x80
    mov rdi, rax
    mov eax, 1          // print(it)
    int 0x80
    mov eax, 0          // exit(7)
    mov edi, 7
    int 0x80
    ud2                 // never reached: exit does not return
user_prog_hello_end:

.global user_prog_snoop
.global user_prog_snoop_end
user_prog_snoop:        // rdi = an address inside the kernel
    mov rax, [rdi]      // must fault: kernel pages have no user bit
    mov rdi, rax
    mov eax, 1          // would leak it -- never reached
    int 0x80
    mov eax, 0
    int 0x80
user_prog_snoop_end:

.global user_prog_cli
.global user_prog_cli_end
user_prog_cli:
    cli                 // privileged: must fault at ring 3
    mov eax, 0
    xor edi, edi
    int 0x80
user_prog_cli_end:

.data
.balign 8
user_ksave:
    .quad 0
.text
"#,
    uds = const USER_DS,
    ucs = const USER_CS,
);

// The assembly above takes its arguments the System V way (rdi, rsi, rdx); this target's
// default "C" convention is Microsoft x64 (rcx, rdx, r8), so the ABI is spelled out.
extern "sysv64" {
    fn user_enter(rip: u64, rsp: u64, arg: u64) -> u64;
    fn user_return(code: u64) -> !;
    fn user_enter_frame(frame: *const TrapFrame) -> u64;
}

/// Start ring 3 from a complete saved frame; returns when `back_to_kernel` is called.
pub fn enter_frame(f: &TrapFrame) -> u64 {
    // SAFETY: the frame describes a ring-3 context (user CS/SS, mapped RIP/RSP) in the
    // current address space; the kernel's context is saved for back_to_kernel.
    unsafe { user_enter_frame(f) }
}

/// From a ring-3 trap path: abandon ring 3 and return `code` from `run`/`enter_frame`.
pub fn back_to_kernel(code: u64) -> ! {
    // SAFETY: only called on the trap path of ring 3 started by run/enter_frame.
    unsafe { user_return(code) }
}
extern "C" {
    static user_prog_hello: u8;
    static user_prog_hello_end: u8;
    static user_prog_snoop: u8;
    static user_prog_snoop_end: u8;
    static user_prog_cli: u8;
    static user_prog_cli_end: u8;
}

pub enum Prog {
    Hello,
    Snoop,
    Cli,
}

fn bytes(p: &Prog) -> &'static [u8] {
    // SAFETY: each pair of labels brackets one program's machine code in .text.
    unsafe {
        let (a, b) = match p {
            Prog::Hello => (&user_prog_hello as *const u8, &user_prog_hello_end as *const u8),
            Prog::Snoop => (&user_prog_snoop as *const u8, &user_prog_snoop_end as *const u8),
            Prog::Cli => (&user_prog_cli as *const u8, &user_prog_cli_end as *const u8),
        };
        core::slice::from_raw_parts(a, b as usize - a as usize)
    }
}

const CODE_VA: u64 = mmu::USER_BASE;
const STACK_VA: u64 = mmu::USER_BASE + 0x10_000;

static CODE_FRAME: AtomicU64 = AtomicU64::new(0);

/// Map the code page (read-only to ring 3) and the stack page once. False if out of frames.
pub fn init() -> bool {
    let (Some(code), Some(stack)) = (frame::alloc(), frame::alloc()) else { return false };
    CODE_FRAME.store(code as u64, Ordering::Relaxed);
    mmu::map_user(CODE_VA, code as u64, false) && mmu::map_user(STACK_VA, stack as u64, true)
}

/// Run one program in ring 3 until it exits (its code) or faults (`KILLED | vector`).
pub fn run(p: Prog, arg: u64) -> u64 {
    let code = bytes(&p);
    let dst = CODE_FRAME.load(Ordering::Relaxed);
    // SAFETY: the code frame is the kernel's (1:1 mapped) and ring 3 is not running.
    unsafe {
        core::ptr::write_bytes(dst as *mut u8, 0xCC, 4096); // int3 filler: a stray jump traps
        core::ptr::copy_nonoverlapping(code.as_ptr(), dst as *mut u8, code.len());
        user_enter(CODE_VA, STACK_VA + 4096, arg)
    }
}

/// `int 0x80` from ring 3.
pub fn syscall(f: &mut TrapFrame) {
    SYSCALLS.fetch_add(1, Ordering::Relaxed);
    match f.rax {
        // SAFETY: called on the trap path of a ring-3 program started by `run`.
        SYS_EXIT if f.cs & 3 == 3 => unsafe { user_return(f.rdi) },
        SYS_PRINT => {
            crate::kprintln!("[user] print {}", f.rdi);
            f.rax = 0;
        }
        SYS_WHOAMI => f.rax = f.cs & 3,
        _ => f.rax = u64::MAX,
    }
}

/// A fault raised in ring 3: stop the program, keep the kernel.
pub fn fault(vector: u64, error: u64, rip: u64, addr: u64) -> ! {
    LAST_FAULT_ERROR.store(error, Ordering::Relaxed);
    LAST_FAULT_ADDR.store(if vector == 14 { addr } else { rip }, Ordering::Relaxed);
    // SAFETY: only reached from the trap path of a ring-3 program started by `run`.
    unsafe { user_return(KILLED | vector) }
}
