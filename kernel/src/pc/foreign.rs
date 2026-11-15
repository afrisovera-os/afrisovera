//! Daraja on the PC: foreign processes (AFR-0001 7.7, COMPAT_STAGE0_PLAN.md). A process whose personality is not
//! NATIVE makes Linux (later Windows or macOS) system calls with the `syscall` instruction. The kernel never looks
//! them up in its own table: they go to the process's bridge, a sandboxed native program, which answers them with
//! AfriSovera's services. The kernel never reads a foreign program file either; the bridge loads it.
//!
//! Stage 0, step 1 (this file today): the hook only. There is no bridge yet, so a forwarded call is recorded and
//! named on the console, and the process ends. Step 2 adds the call channel and the fproc calls; step 3 the Linux
//! bridge (userland/daraja-linux).
//!
//! Written from: the AMD64 APM vol. 2 (SYSCALL, RFLAGS.TF, FS/GS bases), the Intel SDM vol. 3 (IST, single-step
//! traps), the System V AMD64 ABI and the Linux manual page syscall(2) (which registers carry a call: rax, then
//! rdi, rsi, rdx, r10, r8, r9). No Linux kernel source was read (the clean-room rule, ARCHITECT_REVIEWS 14 Nov).

use crate::cpu::TrapFrame;
use crate::proc;
use core::arch::global_asm;
use core::sync::atomic::{AtomicU64, Ordering};

/// Calls forwarded since boot.
pub static CALLS: AtomicU64 = AtomicU64::new(0);
/// `int 0x80` refused from foreign processes since boot.
pub static INT80_REFUSED: AtomicU64 = AtomicU64::new(0);

/// The last forwarded call: (slot, nr, the six arguments), for the selftest.
static mut LAST: (usize, u64, [u64; 6]) = (0, 0, [0; 6]);


// ------------------------------------------------------------------- `linux <name>` (test images, compat0) --
// Stage 0 step 3's end-to-end run (COMPAT_STAGE0_PLAN section 7): the Linux bridge program (userland/daraja-linux,
// built for x86_64-unknown-none and flattened) and one bundle per test program, all made by the cloud job and
// included at compile time. Only a build with the `compat0` feature has them.
#[cfg(feature = "compat0")]
static BRIDGE: &[u8] = include_bytes!(env!("DARAJA_BRIDGE_BIN"));
#[cfg(feature = "compat0")]
static BUNDLE_HELLO0: &[u8] = include_bytes!(concat!(env!("DARAJA_BUNDLE_DIR"), "/hello0.cpio"));
#[cfg(feature = "compat0")]
static BUNDLE_FAULT0: &[u8] = include_bytes!(concat!(env!("DARAJA_BUNDLE_DIR"), "/fault0.cpio"));
#[cfg(feature = "compat0")]
static BUNDLE_BENCH0: &[u8] = include_bytes!(concat!(env!("DARAJA_BUNDLE_DIR"), "/bench0.cpio"));

/// The shell's `linux <name>` (PC only, shell_pc.rs): run one of the test programs through the Linux bridge.
pub fn linux_command(name: &str) {
    #[cfg(feature = "compat0")]
    {
        let bundle = match name {
            "hello0" => BUNDLE_HELLO0,
            "fault0" => BUNDLE_FAULT0,
            "bench0" => BUNDLE_BENCH0,
            _ => {
                crate::kprintln!("linux: type linux hello0, linux fault0 or linux bench0 (the stage 0 test programs)");
                return;
            }
        };
        crate::kprintln!("[daraja] linux {}: begin (bridge {} bytes, bundle {} bytes)", name, BRIDGE.len(), bundle.len());
        if proc::spawn_pair(BRIDGE, proc::PERS_LINUX_X86_64, bundle).is_none() {
            crate::kprintln!("[daraja] linux {}: the pair was not spawned (reason above)", name);
            return;
        }
        proc::run_all();
        crate::kprintln!("[daraja] linux {}: end", name);
    }
    #[cfg(not(feature = "compat0"))]
    crate::kprintln!("linux {}: not in this build (test images with the compat0 feature only)", name);
}

/// `int 0x80` from a foreign process: refused, before any Mansa call can run (AFR-0001 7.7).
pub fn int80_refused(slot: usize, f: &TrapFrame) {
    INT80_REFUSED.fetch_add(1, Ordering::Relaxed);
    crate::kprintln!("[daraja] P{} int 0x80 refused (rax {:#x}): a foreign process never reaches the Mansa calls; process ended", slot, f.rax);
}

// ------------------------------------------------------------------------------------------- test programs --
// Each is loaded like any ring-3 program (pc/proc.rs layout: code at USER_BASE, a data page at DATA_VA, a stack).
global_asm!(
    r#"
// A Linux-shaped call with six known arguments. rcx is set to a decoy: the kernel must read r10 for the fourth.
.global daraja_prog_call
.global daraja_prog_call_end
daraja_prog_call:
    mov eax, 0x1234
    mov edi, 0x11
    mov esi, 0x22
    mov edx, 0x33
    mov r10d, 0x44
    mov r8d, 0x55
    mov r9d, 0x66
    mov ecx, 0xBAD
    syscall
    ud2
daraja_prog_call_end:

// A foreign program trying the native door: int 0x80 with SYS_PRINT (1). It must be refused, nothing printed.
.global daraja_prog_int80
.global daraja_prog_int80_end
daraja_prog_int80:
    mov eax, 1
    mov edi, 0x5A5A
    int 0x80
    ud2
daraja_prog_int80_end:

// Started with TF set (by its spawn), so its FIRST instruction must be `syscall` (TF traps after each instruction;
// any other first instruction would be stepped in ring 3). rax is 0 from the spawn. The kernel must not be stepped.
.global daraja_prog_tf
.global daraja_prog_tf_end
daraja_prog_tf:
    syscall
    ud2
daraja_prog_tf_end:

// FS and GS per process. Each writes two marks in its own data page, spins (the timer switches between the
// processes meanwhile), then reads fs:[0] and gs:[0] and makes a call with them in rdi and rsi.
.global daraja_prog_tls
.global daraja_prog_tls_end
daraja_prog_tls:
    movabs rcx, {data}
    mov qword ptr [rcx], 0xA1
    mov qword ptr [rcx + 0x800], 0xB2
    mov ecx, {spin}
2:
    pause
    dec ecx
    jnz 2b
    mov rdi, qword ptr fs:[0]
    mov rsi, qword ptr gs:[0]
    mov eax, 0x75
    syscall
    ud2
daraja_prog_tls_end:

// A NATIVE program, run beside the two above: FS must be 0 for it, so fs:[0] reads address 0, a kernel page,
// and faults. If a foreign base leaked into it, the read would land in its own data page and it would exit.
.global daraja_prog_fsleak
.global daraja_prog_fsleak_end
daraja_prog_fsleak:
    movabs rcx, {data}
    mov qword ptr [rcx], 0xA1
    mov ecx, {spin}
2:
    pause
    dec ecx
    jnz 2b
    mov rdi, qword ptr fs:[0]
    xor eax, eax
    int 0x80
daraja_prog_fsleak_end:

// A NATIVE program's `syscall`: still an invalid opcode at that instruction, as before EFER.SCE.
.global daraja_prog_native
.global daraja_prog_native_end
daraja_prog_native:
    syscall
    xor eax, eax
    xor edi, edi
    int 0x80
daraja_prog_native_end:

// A foreign program setting its own FS base: CR4.FSGSBASE is off, so wrfsbase is an invalid opcode.
.global daraja_prog_wrfs
.global daraja_prog_wrfs_end
daraja_prog_wrfs:
    xor eax, eax
    .byte 0xF3, 0x48, 0x0F, 0xAE, 0xD0   // wrfsbase rax (as bytes: no assembler feature needed)
    mov eax, 0x76
    syscall
    ud2
daraja_prog_wrfs_end:
"#,
    data = const proc::DATA_VA,
    spin = const 20_000_000u32,
);

extern "C" {
    static daraja_prog_call: u8;
    static daraja_prog_call_end: u8;
    static daraja_prog_int80: u8;
    static daraja_prog_int80_end: u8;
    static daraja_prog_tf: u8;
    static daraja_prog_tf_end: u8;
    static daraja_prog_tls: u8;
    static daraja_prog_tls_end: u8;
    static daraja_prog_fsleak: u8;
    static daraja_prog_fsleak_end: u8;
    static daraja_prog_native: u8;
    static daraja_prog_native_end: u8;
    static daraja_prog_wrfs: u8;
    static daraja_prog_wrfs_end: u8;
}

fn prog(a: &'static u8, b: &'static u8) -> &'static [u8] {
    // SAFETY: each pair of labels brackets one program's machine code in .text.
    unsafe { core::slice::from_raw_parts(a as *const u8, b as *const u8 as usize - a as *const u8 as usize) } // uaccess: the kernel's own program code
}

/// Run what was spawned, with interrupts masked in the kernel as the other ring-3 commands do.
fn run() -> u64 {
    crate::cpu::cli();
    let n = proc::run_all();
    crate::cpu::sti();
    n
}

fn last() -> (usize, u64, [u64; 6]) {
    // SAFETY: single core; written only on the trap path, which has finished.
    unsafe { LAST }
}

/// The selftest's count: one PASS or FAIL line per check.
struct Tally {
    pass: u32,
    total: u32,
}
impl Tally {
    fn check(&mut self, ok: bool, what: core::fmt::Arguments) {
        crate::kprintln!("  {} {}", if ok { "PASS" } else { "FAIL" }, what);
        self.total += 1;
        if ok {
            self.pass += 1;
        }
    }
}

/// `daraja selftest`: stage 0 step 1's checks (pc-daraja grades them).
pub fn selftest() {
    use proc::PERS_LINUX_X86_64 as LINUX;
    crate::kprintln!("daraja selftest: the foreign-process hook (AFR-0001 7.7, stage 0 step 1; nothing is changed)");
    let frames0 = crate::frame::used_frames();
    let mut t = Tally { pass: 0, total: 0 };

    // 1. the CPU state the entry relies on
    let (sce, nofsgs, lstar, mask) = crate::cpu::syscall_state();
    t.check(sce && nofsgs && lstar && mask, format_args!(
        "entry: EFER.SCE {}, CR4.FSGSBASE off {}, LSTAR at syscall_entry {}, SFMASK clears IF TF DF AC {}",
        sce, nofsgs, lstar, mask
    ));
    let (tss, bsp, ap) = crate::cpu::ist_state();
    t.check(tss && bsp && ap, format_args!(
        "IST: stacks set for NMI, #DB, #MC {}, boot core's gates on IST 2, 3, 4 {}, core 1's gates on none {}",
        tss, bsp, ap
    ));

    // 2. a forwarded call carries rax and rdi rsi rdx r10 r8 r9 (not rcx), and never runs a Mansa call
    let c0 = CALLS.load(Ordering::Relaxed);
    // SAFETY: label addresses only.
    let code = prog(unsafe { &daraja_prog_call }, unsafe { &daraja_prog_call_end });
    let ok = match proc::spawn_as(code, &[], &[], false, LINUX, 0, 0, 0x2) {
        Some(_) => {
            run();
            let (_, nr, a) = last();
            CALLS.load(Ordering::Relaxed) == c0 + 1 && nr == 0x1234 && a == [0x11, 0x22, 0x33, 0x44, 0x55, 0x66]
        }
        None => false,
    };
    t.check(ok, format_args!("call: nr 0x1234 with arguments 0x11 to 0x66 from rdi rsi rdx r10 r8 r9 reached the hook"));

    // 3. int 0x80 from a foreign process: refused, the Mansa SYS_PRINT never ran
    let r0 = INT80_REFUSED.load(Ordering::Relaxed);
    // SAFETY: label addresses only.
    let code = prog(unsafe { &daraja_prog_int80 }, unsafe { &daraja_prog_int80_end });
    let ok = proc::spawn_as(code, &[], &[], false, LINUX, 0, 0, 0x2).is_some() && {
        run();
        INT80_REFUSED.load(Ordering::Relaxed) == r0 + 1
    };
    t.check(ok, format_args!("int 0x80 from a foreign process refused (no [P..] 0x5a5a line above)"));

    // 4. syscall with TF set: the kernel is not single-stepped, the call still arrives
    let (d0, c0) = (crate::cpu::DB_AT_ENTRY.load(Ordering::Relaxed), CALLS.load(Ordering::Relaxed));
    // SAFETY: label addresses only.
    let code = prog(unsafe { &daraja_prog_tf }, unsafe { &daraja_prog_tf_end });
    let ok = proc::spawn_as(code, &[], &[], false, LINUX, 0, 0, 0x102).is_some() && {
        run();
        CALLS.load(Ordering::Relaxed) == c0 + 1 && last().1 == 0
    };
    let db = crate::cpu::DB_AT_ENTRY.load(Ordering::Relaxed) - d0;
    t.check(ok, format_args!("syscall with TF set: call 0 arrived, the kernel was not stepped ({} trap(s) absorbed at the entry)", db));

    // 5. FS and GS per process, across timer switches; a native process beside them still has FS 0
    let f0 = proc::FAULTS.load(Ordering::Relaxed);
    let s0 = proc::TIMER_SWITCHES.load(Ordering::Relaxed);
    let (lo, hi) = (proc::DATA_VA, proc::DATA_VA + 0x800);
    // SAFETY: label addresses only.
    let tls = prog(unsafe { &daraja_prog_tls }, unsafe { &daraja_prog_tls_end });
    // SAFETY: label addresses only.
    let leak = prog(unsafe { &daraja_prog_fsleak }, unsafe { &daraja_prog_fsleak_end });
    let a = proc::spawn_as(tls, &[], &[], true, LINUX, lo, hi, 0x202);
    let n = proc::spawn(leak, &[], true);
    let b = proc::spawn_as(tls, &[], &[], true, LINUX, hi, lo, 0x202);
    let mut seen = [(0u64, 0u64); 2];
    let ok = match (a, n, b) {
        (Some(a), Some(_), Some(b)) => {
            // the two calls land in any order, so each is kept per slot while RECORD_ON is set
            crate::cpu::cli();
            RECORD_ON.store(1, Ordering::Relaxed);
            proc::run_all();
            RECORD_ON.store(0, Ordering::Relaxed);
            crate::cpu::sti();
            for (i, slot) in [a, b].iter().enumerate() {
                seen[i] = recorded(*slot);
            }
            seen[0] == (0xA1, 0xB2) && seen[1] == (0xB2, 0xA1) && proc::FAULTS.load(Ordering::Relaxed) == f0 + 1
        }
        _ => {
            run();
            false
        }
    };
    let switches = proc::TIMER_SWITCHES.load(Ordering::Relaxed) - s0;
    t.check(ok && switches >= 2, format_args!(
        "FS/GS per process: A read {:#x}/{:#x}, B read {:#x}/{:#x} (want 0xa1/0xb2 and 0xb2/0xa1), the native one faulted on fs:[0], {} timer switches",
        seen[0].0, seen[0].1, seen[1].0, seen[1].1, switches
    ));
    let (fs, gs) = proc::tls_now();
    t.check(fs == 0 && gs == 0, format_args!("back in the kernel: FS base {:#x}, GS base {:#x} (want 0, 0)", fs, gs));

    // 6. a native program's syscall: an invalid opcode at that instruction, as before
    let f0 = proc::FAULTS.load(Ordering::Relaxed);
    // SAFETY: label addresses only.
    let code = prog(unsafe { &daraja_prog_native }, unsafe { &daraja_prog_native_end });
    // (run_all counts every process that ended, by exit or by fault, so the fault count is what says how it ended)
    let ok = proc::spawn(code, &[], false).is_some() && {
        run();
        proc::FAULTS.load(Ordering::Relaxed) == f0 + 1
    };
    t.check(ok, format_args!("a native program's syscall ended it as an invalid opcode (fault line above: vector 6 at USER_BASE)"));

    // 7. ring 3 cannot set its own FS base
    let (f0, c0) = (proc::FAULTS.load(Ordering::Relaxed), CALLS.load(Ordering::Relaxed));
    // SAFETY: label addresses only.
    let code = prog(unsafe { &daraja_prog_wrfs }, unsafe { &daraja_prog_wrfs_end });
    let ok = proc::spawn_as(code, &[], &[], false, LINUX, 0, 0, 0x2).is_some() && {
        run();
        proc::FAULTS.load(Ordering::Relaxed) == f0 + 1 && CALLS.load(Ordering::Relaxed) == c0
    };
    t.check(ok, format_args!("wrfsbase in a foreign process: an invalid opcode (CR4.FSGSBASE off)"));

    // 8. spawn refusals: a foreign process with a handle, and the reserved personalities
    // SAFETY: label addresses only.
    let code = prog(unsafe { &daraja_prog_call }, unsafe { &daraja_prog_call_end });
    let with_cap = proc::spawn_as(code, &[(0, proc::RIGHT_SEND)], &[], false, LINUX, 0, 0, 0x2).is_none();
    let cell = [proc::Grant::Cell { title: b"daraja (test)" }];
    let with_grant = proc::spawn_as(code, &[], &cell, false, LINUX, 0, 0, 0x2).is_none();
    let windows = proc::spawn_as(code, &[], &[], false, 2, 0, 0, 0x2).is_none();
    let darwin = proc::spawn_as(code, &[], &[], false, 3, 0, 0, 0x2).is_none();
    run(); // nothing should be there to run
    t.check(with_cap && with_grant && windows && darwin, format_args!(
        "refused: a foreign process with a handle {}, with a grant {}, personality 2 {}, personality 3 {}",
        with_cap, with_grant, windows, darwin
    ));

    // 9. step 2: the pair, the memory vectors, the call channel, the ends
    step2(&mut t);

    // 10. nothing left behind
    let frames1 = crate::frame::used_frames();
    t.check(frames1 == frames0, format_args!("frames in use before {} and after {}", frames0, frames1));

    crate::kprintln!("daraja selftest: {} of {} PASS", t.pass, t.total);
}

// The FS/GS test needs every call of one run, not only the last: while RECORD_ON is set, call() also keeps them
// per slot here.
static RECORD_ON: AtomicU64 = AtomicU64::new(0);
static mut RECORDED: [(u64, u64); 8] = [(0, 0); 8];

fn recorded(slot: usize) -> (u64, u64) {
    // SAFETY: single core; the run has finished.
    unsafe { RECORDED.get(slot).copied().unwrap_or((0, 0)) }
}

/// Called by `call` with each forwarded call while RECORD_ON is set.
fn record(slot: usize, a0: u64, a1: u64) {
    if RECORD_ON.load(Ordering::Relaxed) != 0 {
        // SAFETY: single core, trap handler.
        unsafe {
            if let Some(r) = (*core::ptr::addr_of_mut!(RECORDED)).get_mut(slot) {
                *r = (a0, a1);
            }
        }
    }
}

// ------------------------------------------------------------------- the user-memory door (step 2) --
// Every byte Daraja's calls move between a bridge's own memory and the kernel goes through `to_user` or
// `from_user`, nowhere else (the architect's rule, 15 Nov 2026), so the security engineer's PC user-memory module
// (G2 part 3: one uaccess module, SMAP where the CPU has it) replaces these two functions and nothing else. Today
// each checks the whole range with proc::user_buffer_ok (user_range plus the page walk, in the caller's own loaded
// space) before the first byte moves. The foreign process's memory is never reached through a user pointer: its
// frames are read and written through the kernel's 1:1 map (`copy` below).

/// `pid`'s own address space is the one loaded: the door checks and copies only there (the architect, 15 Nov). A
/// call with another space loaded would check one process's tables and copy into another's, so it is refused: a
/// debug assertion stops the suites on it, and a release build answers E_FAULT.
fn own_space(pid: usize) -> bool {
    let ok = crate::mmu::cr3() & !0xFFF == proc::root_of(pid) & !0xFFF;
    debug_assert!(ok, "Daraja user-memory door used for P{} with another space loaded", pid);
    ok
}

/// The whole range [ptr, ptr+len) is the bridge's own memory (and writable for ring 3 when `write`, because the kernel
/// will write it): the door's check alone, for a call that must refuse before its first byte moves (fproc_read and
/// fproc_write span several pages). G2 part 3: the walk is pc/uaccess.rs's.
fn user_ok(pid: usize, ptr: u64, len: u64, write: bool) -> bool {
    own_space(pid) && proc::user_buffer_ok(pid, ptr, len) && crate::uaccess::user_pages_ok(ptr, len, write)
}

/// Copy `src` to the bridge's own memory at `ptr`.
fn to_user(pid: usize, ptr: u64, src: &[u8]) -> Result<(), i64> {
    if !own_space(pid) || !proc::user_buffer_ok(pid, ptr, src.len() as u64) {
        return Err(proc::E_FAULT);
    }
    // G2 part 3: every byte through the one door (pc/uaccess.rs: its own table walk, SMAP's stac..clac)
    crate::uaccess::to_user(ptr, src)
}

/// Copy from the bridge's own memory at `ptr` into `dst`.
fn from_user(pid: usize, ptr: u64, dst: &mut [u8]) -> Result<(), i64> {
    if !own_space(pid) || !proc::user_buffer_ok(pid, ptr, dst.len() as u64) {
        return Err(proc::E_FAULT);
    }
    crate::uaccess::from_user(ptr, dst)
}

// ---------------------------------------------------------------- foreign memory (AFR-0001 7.7, step 2) --
// A foreign process's memory is never a VMO (7.7 as amended): `fproc_map` takes fresh zeroed frames into the foreign
// space only (R or RW, never executable), the bridge fills them with `fproc_write`, and `fproc_protect` drops WRITE
// one way (to R, or to RX before `fproc_start`). Each mapping is one range in this table; every call works on whole
// ranges, as 7.7 says. The frames are counted on the foreign process's own vmo_pages (proc::charge_foreign), so its
// end gives frames and reservation back together.

/// The one bridge and foreign process pair of stage 0 (bridge slot, foreign slot), set when the kernel spawns them.
static mut PAIR: Option<(usize, usize)> = None;

/// The foreign slot whose bridge is `pid`, if `pid` is a bridge.
pub fn partner(pid: usize) -> Option<usize> {
    // SAFETY: single core; written only by the spawn of the pair and by run_all's clean-up.
    match unsafe { *core::ptr::addr_of!(PAIR) } {
        Some((b, f)) if b == pid => Some(f),
        _ => None,
    }
}

/// Mappings per foreign process (a static program has three or four segments, a stack, a heap).
const RANGES_PER: usize = 16;
const P_R: u8 = 1;
const P_W: u8 = 2;
const P_X: u8 = 4;

#[derive(Clone, Copy)]
struct Range {
    va: u64,
    pages: u32,
    prot: u8,
}

static mut RANGES: [[Option<Range>; RANGES_PER]; proc::SLOTS] = [[None; RANGES_PER]; proc::SLOTS];
/// fproc_start has run for this slot: from then on no range may gain EXECUTE (7.7 rule 2).
static mut STARTED: [bool; proc::SLOTS] = [false; proc::SLOTS];

/// Where foreign pages may lie: PML4 slot 2, minus the native map area (proc::MAP_LO..MAP_HI), so a foreign mapping
/// can never alias one the native pointer check accepts (the browser engineer's note, COMPAT_STAGE0_PLAN 13).
const AREA_LO: u64 = crate::mmu::USER_BASE;
const AREA_HI: u64 = crate::mmu::USER_BASE + (1 << 40);
/// At most this many bytes move in one fproc_read or fproc_write (bounds the time the kernel spends in one call).
const COPY_MAX: u64 = 1 << 20;

fn ranges(slot: usize) -> &'static mut [Option<Range>; RANGES_PER] {
    // SAFETY: single core; touched only from the trap handler of the slot's bridge, or from run_all's clean-up.
    unsafe { &mut (*core::ptr::addr_of_mut!(RANGES))[slot] }
}
fn started(slot: usize) -> bool {
    // SAFETY: as ranges.
    unsafe { (*core::ptr::addr_of!(STARTED))[slot] }
}

/// The range holding `va` in `slot`, if any.
fn range_at(slot: usize, va: u64) -> Option<usize> {
    ranges(slot).iter().position(|r| r.is_some_and(|r| va >= r.va && va < r.va + r.pages as u64 * 4096))
}

/// The range that is exactly [addr, addr+len), as 7.7's "a whole mapping".
fn whole(slot: usize, addr: u64, len: u64) -> Option<usize> {
    ranges(slot).iter().position(|r| r.is_some_and(|r| r.va == addr && r.pages as u64 * 4096 == len))
}

/// The foreign process's ranges are forgotten when its memory is given back early (proc::reap); the pair stays, so
/// its bridge still reads the last message and then PEER_CLOSED.
pub fn forget_memory(slot: usize) {
    *ranges(slot) = [None; RANGES_PER];
}

/// A slot's memory is forgotten when the slot is cleared (proc::run_all's end frees its frames and its count).
pub fn forget(slot: usize) {
    *ranges(slot) = [None; RANGES_PER];
    // SAFETY: as partner.
    unsafe {
        if (*core::ptr::addr_of!(PAIR)).is_some_and(|(b, f)| b == slot || f == slot) {
            *core::ptr::addr_of_mut!(PAIR) = None;
        }
    }
    // SAFETY: as ranges.
    unsafe { (*core::ptr::addr_of_mut!(STARTED))[slot] = false };
}

fn page_range_ok(addr: u64, len: u64) -> Result<(u64, u32), i64> {
    if addr % 4096 != 0 || len == 0 || len % 4096 != 0 {
        return Err(proc::E_INVAL);
    }
    let end = addr.checked_add(len).ok_or(proc::E_INVAL)?;
    if addr < AREA_LO || end > AREA_HI || (addr < proc::MAP_HI && end > proc::MAP_LO) {
        return Err(proc::E_INVAL);
    }
    let pages = u32::try_from(len / 4096).map_err(|_| proc::E_INVAL)?;
    Ok((end, pages))
}

/// 49 `fproc_map(fproc h, addr, len, prot)`: fresh zeroed frames, R or RW, never executable.
pub fn fproc_map(pid: usize, h: u64, addr: u64, len: u64, prot: u64) -> Result<u64, i64> {
    let fs = proc::fproc_of(pid, h, proc::R_MAP)?;
    if !crate::mmu::nx() {
        return Err(proc::E_PERM); // W^X and the seal cannot be promised without NX (7.7, as for vmo_map)
    }
    let p = prot as u8;
    if prot > 7 || p & P_X != 0 || (p != P_R && p != P_R | P_W) {
        return Err(proc::E_INVAL);
    }
    let (end, pages) = page_range_ok(addr, len)?;
    // never over an existing page: that is how nothing returns to WRITE (7.7 rule 1)
    if ranges(fs).iter().flatten().any(|r| addr < r.va + r.pages as u64 * 4096 && r.va < end) {
        return Err(proc::E_INVAL);
    }
    let Some(free) = ranges(fs).iter().position(|r| r.is_none()) else { return Err(proc::E_NOMEM) };
    proc::charge_foreign(fs, pid, pages)?;
    let root = proc::root_of(fs);
    for i in 0..pages as u64 {
        let ok = match crate::frame::alloc() {
            Some(fr) => {
                // SAFETY: a frame the pool just gave us, 1:1 mapped, not yet in any ring-3 space.
                unsafe { core::ptr::write_bytes(fr as *mut u8, 0, 4096) }; // uaccess: a fresh frame by its 1:1 address
                crate::mmu::map_user_prot_in(root, addr + i * 4096, fr as u64, p & P_W != 0, false) || {
                    crate::frame::free(fr);
                    false
                }
            }
            None => false,
        };
        if !ok {
            for j in 0..i {
                if let Some(fr) = crate::mmu::unmap_user_in(root, addr + j * 4096) {
                    crate::frame::free(fr as usize);
                }
            }
            proc::uncharge(fs, pages);
            return Err(proc::E_NOMEM); // nothing half made
        }
    }
    ranges(fs)[free] = Some(Range { va: addr, pages, prot: p });
    Ok(0)
}

/// 50 `fproc_unmap(fproc h, addr, len)`: a whole mapping; its frames go back to the pool.
pub fn fproc_unmap(pid: usize, h: u64, addr: u64, len: u64) -> Result<u64, i64> {
    let fs = proc::fproc_of(pid, h, proc::R_MAP)?;
    let i = whole(fs, addr, len).ok_or(proc::E_INVAL)?;
    let Some(r) = ranges(fs)[i] else { return Err(proc::E_INVAL) };
    let root = proc::root_of(fs);
    for k in 0..r.pages as u64 {
        if let Some(fr) = crate::mmu::unmap_user_in(root, r.va + k * 4096) {
            crate::frame::free(fr as usize);
        }
    }
    ranges(fs)[i] = None;
    proc::uncharge(fs, r.pages);
    Ok(0)
}

/// 56 `fproc_protect(fproc h, addr, len, prot)`: RW to R, or RW to RX before the start. One way: nothing gains WRITE,
/// and EXECUTE comes only straight from RW (7.7 rules 1 and 2).
pub fn fproc_protect(pid: usize, h: u64, addr: u64, len: u64, prot: u64) -> Result<u64, i64> {
    let p = prot as u8;
    if prot > 7 || p & P_W != 0 || (p != P_R && p != P_R | P_X) {
        return Err(proc::E_INVAL);
    }
    let need = if p & P_X != 0 { proc::R_MAP | proc::R_EXECUTE } else { proc::R_MAP };
    let fs = proc::fproc_of(pid, h, need)?;
    if p & P_X != 0 && started(fs) {
        return Err(proc::E_PERM);
    }
    let i = whole(fs, addr, len).ok_or(proc::E_INVAL)?;
    let Some(mut r) = ranges(fs)[i] else { return Err(proc::E_INVAL) };
    if r.prot != P_R | P_W {
        return Err(proc::E_INVAL); // only a writable range is sealed; R and RX never change again
    }
    let root = proc::root_of(fs);
    for k in 0..r.pages as u64 {
        let va = r.va + k * 4096;
        let Some(fr) = crate::mmu::user_frame_in(root, va) else { return Err(proc::E_INVAL) };
        crate::mmu::map_user_prot_in(root, va, fr, false, p & P_X != 0);
    }
    r.prot = p;
    ranges(fs)[i] = Some(r);
    Ok(0)
}

/// Every page of [faddr, faddr+len) in `fs` lies in a range with all of `need`; and the bridge's own buffer is its own.
/// `bridge_write`: the kernel writes the bridge's buffer (fproc_read), so its pages must be writable for ring 3.
fn copy_checks(pid: usize, fs: usize, faddr: u64, ptr: u64, len: u64, need: u8, bridge_write: bool) -> Result<(), i64> {
    if len == 0 || len > COPY_MAX {
        return Err(proc::E_INVAL);
    }
    let fend = faddr.checked_add(len).ok_or(proc::E_FAULT)?;
    if !user_ok(pid, ptr, len, bridge_write) {
        return Err(proc::E_FAULT);
    }
    let mut pg = faddr & !0xFFF;
    while pg < fend {
        let Some(i) = range_at(fs, pg) else { return Err(proc::E_FAULT) };
        let Some(r) = ranges(fs)[i] else { return Err(proc::E_FAULT) };
        if r.prot & need != need {
            return Err(proc::E_FAULT);
        }
        pg += 4096;
    }
    Ok(())
}

/// Copy between the foreign space (through the kernel's 1:1 map of its frames) and the bridge's own buffer, which
/// is in the loaded space. Every page was checked first, so no copy starts that cannot finish.
fn copy(pid: usize, fs: usize, faddr: u64, ptr: u64, len: u64, to_foreign: bool) -> Result<u64, i64> {
    let root = proc::root_of(fs);
    let mut done = 0u64;
    while done < len {
        let va = faddr + done;
        let off = va & 0xFFF;
        let n = (4096 - off).min(len - done);
        let fr = crate::mmu::user_frame_in(root, va & !0xFFF).ok_or(proc::E_FAULT)?;
        // SAFETY: `fr + off .. + n` lies inside one frame of the foreign process (checked by copy_checks and found just
        // above), reached through the kernel's 1:1 map; nothing else holds a reference to it during this call.
        let frame = unsafe { core::slice::from_raw_parts_mut((fr + off) as *mut u8, n as usize) }; // uaccess: a foreign frame by its 1:1 address
        if to_foreign {
            from_user(pid, ptr + done, frame)?;
        } else {
            to_user(pid, ptr + done, frame)?;
        }
        done += n;
    }
    Ok(done)
}

/// 51 `fproc_read(fproc h, faddr, my ptr, len)`: bytes copied out of the foreign process.
pub fn fproc_read(pid: usize, h: u64, faddr: u64, ptr: u64, len: u64) -> Result<u64, i64> {
    let fs = proc::fproc_of(pid, h, proc::R_READ)?;
    copy_checks(pid, fs, faddr, ptr, len, P_R, true)?;
    copy(pid, fs, faddr, ptr, len, false)
}

/// 52 `fproc_write(fproc h, faddr, my ptr, len)`: bytes copied into the foreign process; only into WRITE pages, so
/// never into an executable or sealed one.
pub fn fproc_write(pid: usize, h: u64, faddr: u64, ptr: u64, len: u64) -> Result<u64, i64> {
    let fs = proc::fproc_of(pid, h, proc::R_WRITE)?;
    copy_checks(pid, fs, faddr, ptr, len, P_W, false)?;
    copy(pid, fs, faddr, ptr, len, true)
}

// ------------------------------------------------------------------ the call channel (AFR-0001 7.7, step 2) --
// The kernel is one end of each foreign process's call channel, its bridge the other. A CALL message goes out when
// the foreign process makes a call (it then waits, InCall), FAULT when it faults, EXITED when the kernel ends it.
// The bridge reads them (channel_read, v1 call 18), waits for them (wait_one, 24) and answers a CALL with a 24-byte
// reply (channel_write, 17). One CALL in flight; a reply never queues.

pub const MSG_LEN: usize = 96;
const REPLY_LEN: u64 = 24;
const KIND_CALL: u16 = 1;
const KIND_FAULT: u16 = 2;
const KIND_EXITED: u16 = 3;
const REPLY_RESULT: u16 = 1;
const REPLY_EXIT: u16 = 2;
/// wait_one's signals (AFR-0001 7.7): bit 0 READABLE, bit 2 PEER_CLOSED.
pub const SIG_READABLE: u64 = 1;
pub const SIG_PEER_CLOSED: u64 = 4;
/// Messages waiting for the bridge: at most one CALL plus the last FAULT or EXITED, so four is room to spare.
const QUEUE: usize = 4;

struct Chan {
    q: [[u8; MSG_LEN]; QUEUE],
    head: usize,
    len: usize,
    seq: u64,
    /// the seq of the CALL whose reply is awaited
    waiting: Option<u64>,
    /// the foreign process has ended (its last message may still be queued)
    gone: bool,
}
static mut CHAN: Chan = Chan { q: [[0; MSG_LEN]; QUEUE], head: 0, len: 0, seq: 0, waiting: None, gone: false };

fn chan() -> &'static mut Chan {
    // SAFETY: single core; touched only from trap handlers and the pair's spawn and clean-up.
    unsafe { &mut *core::ptr::addr_of_mut!(CHAN) }
}

/// The pair's slots (bridge, foreign), if there is one.
fn pair() -> Option<(usize, usize)> {
    // SAFETY: as partner.
    unsafe { *core::ptr::addr_of!(PAIR) }
}

/// Set up the pair at spawn (step 2's spawn, proc::spawn_pair).
pub fn pair_begin(bridge: usize, foreign: usize) {
    // SAFETY: as partner; no process runs yet.
    unsafe { *core::ptr::addr_of_mut!(PAIR) = Some((bridge, foreign)) };
    *chan() = Chan { q: [[0; MSG_LEN]; QUEUE], head: 0, len: 0, seq: 0, waiting: None, gone: false };
}

fn push(kind: u16, personality: u8, nr: u64, args: [u64; 6], rip: u64, rsp: u64) -> u64 {
    let c = chan();
    c.seq += 1;
    let mut m = [0u8; MSG_LEN];
    m[0..2].copy_from_slice(&1u16.to_le_bytes());
    m[2..4].copy_from_slice(&kind.to_le_bytes());
    m[4..6].copy_from_slice(&(personality as u16).to_le_bytes());
    // 6..8: the thread, 0 in v0.5
    m[8..16].copy_from_slice(&c.seq.to_le_bytes());
    m[16..24].copy_from_slice(&nr.to_le_bytes());
    for (i, a) in args.iter().enumerate() {
        m[24 + i * 8..32 + i * 8].copy_from_slice(&a.to_le_bytes());
    }
    m[72..80].copy_from_slice(&rip.to_le_bytes());
    m[80..88].copy_from_slice(&rsp.to_le_bytes());
    if c.len < QUEUE {
        let at = (c.head + c.len) % QUEUE;
        c.q[at] = m;
        c.len += 1;
    }
    c.seq
}

/// What proc::syscall_insn does with a foreign call.
pub enum Outcome {
    /// no bridge (the step 1 selftest's programs): named on the console, the process ends
    End,
    /// a CALL went to the bridge in this slot: the caller waits (InCall), the bridge is woken
    Wait(usize),
}

/// A foreign process's `syscall` (proc::syscall_insn).
pub fn call(slot: usize, personality: u8, f: &mut TrapFrame) -> Outcome {
    CALLS.fetch_add(1, Ordering::Relaxed);
    // the Linux x86-64 order: rdi rsi rdx r10 r8 r9 (rcx holds the return address after `syscall`)
    let args = [f.rdi, f.rsi, f.rdx, f.r10, f.r8, f.r9];
    // SAFETY: single core, trap handler.
    unsafe { LAST = (slot, f.rax, args) };
    record(slot, args[0], args[1]);
    match pair() {
        Some((b, fs)) if fs == slot => {
            if !QUIET.load(Ordering::Relaxed) {
                crate::kprintln!("[daraja] P{} call {:#x} ({:#x}) to its bridge P{}", slot, f.rax, args[0], b);
            }
            let seq = push(KIND_CALL, personality, f.rax, args, f.rip, f.rsp);
            chan().waiting = Some(seq);
            Outcome::Wait(b)
        }
        _ => {
            crate::kprintln!(
                "[daraja] P{} personality {} call {:#x} ({:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x}); no bridge yet (stage 0 step 1), process ended",
                slot, personality, f.rax, args[0], args[1], args[2], args[3], args[4], args[5]
            );
            Outcome::End
        }
    }
}

/// A foreign process faulted, or tried `int 0x80` (vector 0x80): its last message, FAULT, for its bridge. Returns
/// the bridge to wake, if there is one.
pub fn ended_by_fault(slot: usize, personality: u8, vector: u64, error: u64, addr: u64, rip: u64, rsp: u64) -> Option<usize> {
    let (b, fs) = pair()?;
    if fs != slot || chan().gone {
        return None;
    }
    push(KIND_FAULT, personality, vector, [error, addr, 0, 0, 0, 0], rip, rsp);
    let c = chan();
    c.waiting = None;
    c.gone = true;
    Some(b)
}

/// The kernel ended the foreign process (fproc_kill, or an EXIT reply): its last message, EXITED.
fn ended_by_kernel(slot: usize, code: u64) {
    if chan().gone {
        return;
    }
    push(KIND_EXITED, proc::PERS_LINUX_X86_64, code, [0; 6], 0, 0);
    let c = chan();
    c.waiting = None;
    c.gone = true;
    crate::kprintln!("[daraja] P{} ended by its bridge, code {}", slot, code);
}

/// The bridge ended (exit or fault): its foreign process ends with it, at once (7.7: no orphans).
pub fn bridge_ended(pid: usize) -> Option<usize> {
    let (b, fs) = pair()?;
    if b != pid {
        return None;
    }
    let c = chan();
    c.gone = true;
    c.waiting = None;
    crate::kprintln!("[daraja] P{} ended: its bridge P{} is gone", fs, b);
    Some(fs)
}

fn signals_now() -> u64 {
    let c = chan();
    if c.len > 0 {
        SIG_READABLE
    } else if c.gone {
        SIG_PEER_CLOSED
    } else {
        0
    }
}

/// The bridge to wake after the foreign side changed, and what wait_one returns to it.
pub fn bridge_signals() -> u64 {
    signals_now()
}

/// 18 `channel_read(ch h, ptr, cap, 0, 0)`: the next message, 96 bytes.
pub fn channel_read(pid: usize, h: u64, ptr: u64, cap: u64, hptr: u64, hcap: u64) -> Result<u64, i64> {
    let slot = proc::chan_of(pid, h, proc::R_READ)?;
    if pair() != Some((pid, slot)) || hptr != 0 || hcap != 0 {
        return Err(proc::E_INVAL);
    }
    let c = chan();
    if c.len == 0 {
        return Err(if c.gone { proc::E_PEERCLOSED } else { proc::E_WOULDBLOCK });
    }
    if cap < MSG_LEN as u64 {
        // 7.5: the sizes in the first 8 bytes, the message stays queued
        if cap >= 8 {
            let mut s = [0u8; 8];
            s[0..4].copy_from_slice(&(MSG_LEN as u32).to_le_bytes());
            let _ = to_user(pid, ptr, &s); // a bad pointer leaves only E_TOOBIG, as 7.5 allows
        }
        return Err(proc::E_TOOBIG);
    }
    let m = c.q[c.head];
    to_user(pid, ptr, &m)?; // nothing is taken from the queue unless the copy succeeded
    c.head = (c.head + 1) % QUEUE;
    c.len -= 1;
    Ok(0)
}

/// 24 `wait_one(ch h, signals, timeout_ns)`: the signals now, or wait for one. A wait with no limit (negative) or a
/// positive limit waits until a message or the end (v0.5 has no timed wait; the foreign side always ends one way or
/// the other, and the bridge's own end ends it). 0 only looks.
pub enum Wait {
    Now(u64),
    Block,
}
pub fn wait_one(pid: usize, h: u64, signals: u64, timeout_ns: i64) -> Result<Wait, i64> {
    let slot = proc::chan_of(pid, h, proc::R_WAIT)?;
    if pair() != Some((pid, slot)) || signals & !(SIG_READABLE | SIG_PEER_CLOSED) != 0 || signals == 0 {
        return Err(proc::E_INVAL);
    }
    let now = signals_now();
    if now & signals != 0 || now == SIG_PEER_CLOSED || timeout_ns == 0 {
        return Ok(Wait::Now(now));
    }
    Ok(Wait::Block)
}

/// What a valid reply asks the kernel to do with the waiting foreign process.
pub enum Reply {
    Result(usize, u64),
    Exit(usize, u64),
}

/// 17 `channel_write(ch h, ptr, len, hptr, hn)`: the 24-byte reply to the CALL waiting. Anything else is E_INVAL and
/// changes nothing.
pub fn channel_write(pid: usize, h: u64, ptr: u64, len: u64, hptr: u64, hn: u64) -> Result<Reply, i64> {
    let slot = proc::chan_of(pid, h, proc::R_WRITE)?;
    if pair() != Some((pid, slot)) || hptr != 0 || hn != 0 || len != REPLY_LEN {
        return Err(proc::E_INVAL);
    }
    let mut r = [0u8; 24];
    from_user(pid, ptr, &mut r)?;
    let version = u16::from_le_bytes([r[0], r[1]]);
    let kind = u16::from_le_bytes([r[2], r[3]]);
    let reserved = u32::from_le_bytes([r[4], r[5], r[6], r[7]]);
    let seq = u64::from_le_bytes(r[8..16].try_into().unwrap_or([0; 8]));
    let value = u64::from_le_bytes(r[16..24].try_into().unwrap_or([0; 8]));
    let c = chan();
    if c.gone {
        return Err(proc::E_PEERCLOSED);
    }
    if version != 1 || reserved != 0 || c.waiting != Some(seq) || !proc::in_call(slot) {
        return Err(proc::E_INVAL);
    }
    match kind {
        REPLY_RESULT => {
            c.waiting = None;
            Ok(Reply::Result(slot, value))
        }
        REPLY_EXIT => {
            ended_by_kernel(slot, value);
            Ok(Reply::Exit(slot, value))
        }
        _ => Err(proc::E_INVAL),
    }
}

/// 53 `fproc_start(fproc h, rip, rsp)`: once; rip in an executable range, rsp just above a writable one.
pub fn fproc_start(pid: usize, h: u64, rip: u64, rsp: u64) -> Result<(usize, u64, u64), i64> {
    let fs = proc::fproc_of(pid, h, proc::R_SIGNAL)?;
    if started(fs) || !proc::unstarted(fs) {
        return Err(proc::E_INVAL);
    }
    let in_prot = |va: u64, need: u8| {
        range_at(fs, va).and_then(|i| ranges(fs)[i]).is_some_and(|r| r.prot & need == need)
    };
    if !in_prot(rip, P_X) || rsp < 8 || rsp % 16 != 0 || !in_prot(rsp - 8, P_W) {
        return Err(proc::E_INVAL);
    }
    // SAFETY: as ranges.
    unsafe { (*core::ptr::addr_of_mut!(STARTED))[fs] = true };
    Ok((fs, rip, rsp))
}

/// 54 `fproc_set_tls(fproc h, which, value)`: canonical and inside the foreign area, so the switch's wrmsr can never
/// fault (the browser engineer's note).
pub fn fproc_set_tls(pid: usize, h: u64, which: u64, value: u64) -> Result<(usize, u64, u64), i64> {
    let fs = proc::fproc_of(pid, h, proc::R_WRITE)?;
    if which > 1 || value < AREA_LO || value >= AREA_HI {
        return Err(proc::E_INVAL);
    }
    Ok((fs, which, value))
}

/// 55 `fproc_kill(fproc h, code)`: the foreign process ends; the bridge gets EXITED, then PEER_CLOSED.
pub fn fproc_kill(pid: usize, h: u64, code: u64) -> Result<usize, i64> {
    let fs = proc::fproc_of(pid, h, proc::R_SIGNAL)?;
    ended_by_kernel(fs, code);
    Ok(fs)
}

// ------------------------------------------------------------- step 2's test pair (pc-daraja, AFR-0001 7.7) --
// bridge0 is a native program spawned by spawn_pair with FPROC (handle 0), CHAN (1) and a bundle (2). It runs 7.7's
// memory vectors on its foreign process, loads the four-call program fp0 into it, starts it, answers its calls (one
// wrong-seq reply first), and checks the ends: wait_one after the foreign process has gone, and a v0 window call.
// Every result is printed with SYS_PRINT, in this order (pc-daraja grades the list):
//    1 map A RW                     0           11 read across the end of S  E_FAULT
//    2 map A again (over it)        E_INVAL     12 FS base = S               0
//    3 map B RWX                    E_INVAL     13 start at A, stack S+4096  0
//    4 write fp0 into A             its length  14 protect S RX after start  E_PERM
//    5 protect A RX (the seal)      0           then each CALL's nr (0x10, 0x20, 0x30), the wrong-seq reply's
//    6 write into sealed A          E_FAULT     E_INVAL after 0x10, the EXITED kind (3) after the EXIT reply,
//    7 protect A back to RW         E_INVAL     wait_one after the end (PEER_CLOSED, 4) and call 20 (E_NOSYS).
//    8 map A R over the sealed page E_INVAL
//    9 map S RW (the stack)         0
//   10 write the mark into S        8
// fp0 (in the foreign process) reads fs:[0] (the mark 0x5a5a, so the FS base was set) and makes three calls, each
// with the previous answer: 0x10 (0x5a5a), 0x20 (0x11), 0x30 (0x21); the bridge answers nr + 1, and EXIT 7 to 0x30.
global_asm!(
    r#"
.global daraja_bridge0
.global daraja_bridge0_end
daraja_bridge0:
    movabs r12, {data}
    mov qword ptr [r12], 0x5A5A
    xor r13d, r13d

    mov eax, 49                 // 1 fproc_map(0, A, 4096, RW)
    xor edi, edi
    movabs rsi, {fa}
    mov edx, 4096
    mov ecx, 3
    int 0x80
    call 90f
    mov eax, 49                 // 2 again, over it
    xor edi, edi
    movabs rsi, {fa}
    mov edx, 4096
    mov ecx, 3
    int 0x80
    call 90f
    mov eax, 49                 // 3 fproc_map(0, B, 4096, RWX)
    xor edi, edi
    movabs rsi, {fb}
    mov edx, 4096
    mov ecx, 7
    int 0x80
    call 90f
    lea rcx, [rip + daraja_fp0_end] // 4 fproc_write(0, A, fp0, its length): the length worked out at run time
    lea rdx, [rip + daraja_fp0]
    sub rcx, rdx
    mov eax, 52
    xor edi, edi
    movabs rsi, {fa}
    int 0x80
    call 90f
    mov eax, 56                 // 5 fproc_protect(0, A, 4096, RX)
    xor edi, edi
    movabs rsi, {fa}
    mov edx, 4096
    mov ecx, 5
    int 0x80
    call 90f
    mov eax, 52                 // 6 fproc_write into the sealed page
    xor edi, edi
    movabs rsi, {fa}
    lea rdx, [rip + daraja_fp0]
    mov ecx, 8
    int 0x80
    call 90f
    mov eax, 56                 // 7 fproc_protect(0, A, 4096, RW)
    xor edi, edi
    movabs rsi, {fa}
    mov edx, 4096
    mov ecx, 3
    int 0x80
    call 90f
    mov eax, 49                 // 8 fproc_map(0, A, 4096, R) over the sealed page
    xor edi, edi
    movabs rsi, {fa}
    mov edx, 4096
    mov ecx, 1
    int 0x80
    call 90f
    mov eax, 49                 // 9 fproc_map(0, S, 4096, RW)
    xor edi, edi
    movabs rsi, {fsa}
    mov edx, 4096
    mov ecx, 3
    int 0x80
    call 90f
    mov eax, 52                 // 10 fproc_write(0, S, the mark, 8)
    xor edi, edi
    movabs rsi, {fsa}
    mov rdx, r12
    mov ecx, 8
    int 0x80
    call 90f
    mov eax, 51                 // 11 fproc_read(0, S + 4088, data + 256, 16): across the end
    xor edi, edi
    movabs rsi, {fsa}
    add rsi, 4088
    lea rdx, [r12 + 256]
    mov ecx, 16
    int 0x80
    call 90f
    mov eax, 54                 // 12 fproc_set_tls(0, FS, S)
    xor edi, edi
    xor esi, esi
    movabs rdx, {fsa}
    int 0x80
    call 90f
    mov eax, 53                 // 13 fproc_start(0, A, S + 4096)
    xor edi, edi
    movabs rsi, {fa}
    movabs rdx, {fsa}
    add rdx, 4096
    int 0x80
    call 90f
    mov eax, 56                 // 14 fproc_protect(0, S, 4096, RX) after the start
    xor edi, edi
    movabs rsi, {fsa}
    mov edx, 4096
    mov ecx, 5
    int 0x80
    call 90f

2:                              // the call loop
    mov eax, 24                 // wait_one(1, READABLE | PEER_CLOSED, no limit)
    mov edi, 1
    mov esi, 5
    mov rdx, -1
    int 0x80
    test eax, 1
    jz 4f
    mov eax, 18                 // channel_read(1, data + 64, 96)
    mov edi, 1
    lea rsi, [r12 + 64]
    mov edx, 96
    xor ecx, ecx
    xor r8d, r8d
    int 0x80
    movzx eax, word ptr [r12 + 66]
    cmp eax, 1
    je 3f
    call 90f                    // a FAULT or EXITED message: print its kind
    jmp 2b
3:
    mov rax, qword ptr [r12 + 80]
    call 90f                    // the CALL's nr
    mov word ptr [r12 + 192], 1
    mov word ptr [r12 + 194], 1 // RESULT
    mov dword ptr [r12 + 196], 0
    mov rax, qword ptr [r12 + 72]
    mov qword ptr [r12 + 200], rax
    mov rax, qword ptr [r12 + 80]
    lea rcx, [rax + 1]
    mov qword ptr [r12 + 208], rcx
    cmp rax, 0x30
    jne 5f
    mov word ptr [r12 + 194], 2 // EXIT, code 7
    mov qword ptr [r12 + 208], 7
5:
    test r13d, r13d
    jnz 6f
    mov r13d, 1
    inc qword ptr [r12 + 200]   // first, a reply with the wrong seq
    mov eax, 17
    mov edi, 1
    lea rsi, [r12 + 192]
    mov edx, 24
    xor ecx, ecx
    xor r8d, r8d
    int 0x80
    call 90f
    dec qword ptr [r12 + 200]
6:
    mov eax, 17                 // channel_write(1, the reply, 24)
    mov edi, 1
    lea rsi, [r12 + 192]
    mov edx, 24
    xor ecx, ecx
    xor r8d, r8d
    int 0x80
    jmp 2b

4:
    mov eax, 24                 // wait_one after the end: PEER_CLOSED
    mov edi, 1
    mov esi, 5
    mov rdx, -1
    int 0x80
    call 90f
    mov eax, 20                 // a v0 window call from a bridge: E_NOSYS
    mov edi, 10
    mov esi, 10
    int 0x80
    call 90f
    xor eax, eax                // exit(0)
    xor edi, edi
    int 0x80

90:                             // print rax (SYS_PRINT)
    mov rdi, rax
    mov eax, 1
    int 0x80
    ret

// fp0 lies INSIDE bridge0's range: only [daraja_bridge0, daraja_bridge0_end) is copied into the bridge process, so
// its fproc_write source must be there (first the cloud runner run, 15 Nov: placed before the range, it was E_FAULT, correctly)
.global daraja_fp0
.global daraja_fp0_end
daraja_fp0:
    mov rdi, qword ptr fs:[0]
    mov eax, 0x10
    syscall
    mov rdi, rax
    mov eax, 0x20
    syscall
    mov rdi, rax
    mov eax, 0x30
    syscall
    ud2
daraja_fp0_end:
daraja_bridge0_end:
"#,
    data = const proc::DATA_VA,
    fa = const crate::mmu::USER_BASE + 0x40_0000,
    fb = const crate::mmu::USER_BASE + 0x60_0000,
    fsa = const crate::mmu::USER_BASE + 0x50_0000,
);

extern "C" {
    static daraja_bridge0: u8;
    static daraja_bridge0_end: u8;
}

/// What bridge0 prints, in order (None: any value, the length of fp0).
pub const BRIDGE0_EXPECT: [Option<u64>; 21] = [
    Some(0), Some(E_INVAL_U), Some(E_INVAL_U), None, Some(0), Some(E_FAULT_U), Some(E_INVAL_U), Some(E_INVAL_U),
    Some(0), Some(8), Some(E_FAULT_U), Some(0), Some(0), Some(E_PERM_U),
    Some(0x10), Some(E_INVAL_U), Some(0x20), Some(0x30), Some(3), Some(4), Some(E_NOSYS_U),
];
const E_INVAL_U: u64 = -5i64 as u64;
const E_FAULT_U: u64 = -4i64 as u64;
const E_PERM_U: u64 = -1i64 as u64;
const E_NOSYS_U: u64 = -3i64 as u64;

/// Step 2's checks in `daraja selftest`: the pair, its run, and what came back.
fn step2(t: &mut Tally) {
    let frames0 = crate::frame::used_frames();
    let (res0, _, _) = proc::vmo_stats();
    let c0 = CALLS.load(Ordering::Relaxed);
    // SAFETY: label addresses only.
    let code = prog(unsafe { &daraja_bridge0 }, unsafe { &daraja_bridge0_end });
    let pair = proc::spawn_pair(code, proc::PERS_LINUX_X86_64, b"bundle0: not read in step 2");
    t.check(pair.is_some(), format_args!("step 2: the pair spawned (a foreign process and its bridge with FPROC, CHAN, the bundle)"));
    if pair.is_none() {
        return;
    }
    proc::run_all();
    let calls = CALLS.load(Ordering::Relaxed) - c0;
    let (_, nr, a) = last();
    t.check(calls == 3 && nr == 0x30 && a[0] == 0x21, format_args!(
        "step 2: fp0's three calls went through its bridge ({} calls, the last {:#x} with {:#x}; want 3, 0x30, 0x21)",
        calls, nr, a[0]
    ));
    let frames1 = crate::frame::used_frames();
    let (res1, _, _) = proc::vmo_stats();
    t.check(frames1 == frames0 && res1 == res0, format_args!(
        "step 2: frames in use {} before and {} after, reserved pages {} before and {} after",
        frames0, frames1, res0, res1
    ));
}

// ---------------------------------------------------- `daraja bench` (COMPAT_STAGE0_PLAN 17, the perf method) --
// The cost of one forwarded call, beside two native floors, in one boot (the performance engineer's method):
//   (a) native-null: a native `int 0x80` with an unknown number, refused at once;
//   (b) native-ipc: a native round trip, SEND to a server process and RECV of its answer (the fair comparison: the
//       bridge is a server);
//   (c) forwarded: a foreign `syscall` through a minimal bridge (daraja_b_bridge: read the CALL, answer RESULT
//       -EBADF), with no per-call console lines (QUIET).
// Each is timed over N calls and over 0 calls in the same boot, so start-up and teardown cancel: the line gives the
// time of the N calls alone, in microseconds of the kernel's clock (under the perf rig's -icount, 1 instruction is
// 1 ns). (b) and (c) both take two process switches per call (to the server or bridge and back), so (c) minus (b)
// is the forwarding protocol's own per-call work: the CALL and reply messages, wait_one and the channel copies.
// Every build has it (no cloud-built bridge needed), so pc-perf and pc-perf-release can run it.
//   [daraja] bench: 10000 calls: native-null N us, native-ipc N us, forwarded N us

/// While set, a pair's calls print no per-call line (the bench; the strace comparison needs them otherwise).
static QUIET: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
const BENCH_N: u32 = 10_000;

global_asm!(
    r#"
// (a) N refused calls: rax = 999 is no call of the Mansa table; the count comes in the data page
.global daraja_b_null
.global daraja_b_null_end
daraja_b_null:
    movabs rcx, {data}
    mov r12d, dword ptr [rcx]
2:
    test r12d, r12d
    jz 3f
    mov eax, 999
    int 0x80
    dec r12d
    jmp 2b
3:
    xor eax, eax
    xor edi, edi
    int 0x80
daraja_b_null_end:

// (b) the client: N round trips, SEND(0, i) then RECV(1)
.global daraja_b_client
.global daraja_b_client_end
daraja_b_client:
    movabs rcx, {data}
    mov r12d, dword ptr [rcx]
2:
    test r12d, r12d
    jz 3f
    mov eax, 3
    xor edi, edi
    mov esi, r12d
    int 0x80
    mov eax, 4
    mov edi, 1
    int 0x80
    dec r12d
    jmp 2b
3:
    xor eax, eax
    xor edi, edi
    int 0x80
daraja_b_client_end:

// (b) the server: N times RECV(0) then SEND(1, the value back)
.global daraja_b_server
.global daraja_b_server_end
daraja_b_server:
    movabs rcx, {data}
    mov r12d, dword ptr [rcx]
2:
    test r12d, r12d
    jz 3f
    mov eax, 4
    xor edi, edi
    int 0x80
    mov rsi, rax
    mov eax, 3
    mov edi, 1
    int 0x80
    dec r12d
    jmp 2b
3:
    xor eax, eax
    xor edi, edi
    int 0x80
daraja_b_server_end:

// (c) the minimal bridge: wait, read, answer every CALL with RESULT -9 (EBADF); exit at PEER_CLOSED. Its foreign
// program, bench_fp, lies inside this range (fproc_write's source must be the bridge's own memory). The count of
// calls comes in the data page; the bridge writes it into the foreign program's data before starting it.
.global daraja_b_bridge
.global daraja_b_bridge_end
daraja_b_bridge:
    movabs r12, {data}
    mov eax, 49                 // fproc_map(0, A, 4096, RW)
    xor edi, edi
    movabs rsi, {fa}
    mov edx, 4096
    mov ecx, 3
    int 0x80
    lea rcx, [rip + daraja_b_fp_end]
    lea rdx, [rip + daraja_b_fp]
    sub rcx, rdx
    mov eax, 52                 // fproc_write(0, A, fp, its length)
    xor edi, edi
    movabs rsi, {fa}
    int 0x80
    mov eax, 56                 // fproc_protect(0, A, 4096, RX)
    xor edi, edi
    movabs rsi, {fa}
    mov edx, 4096
    mov ecx, 5
    int 0x80
    mov eax, 49                 // fproc_map(0, S, 4096, RW): its stack and its count
    xor edi, edi
    movabs rsi, {fsa}
    mov edx, 4096
    mov ecx, 3
    int 0x80
    mov eax, 52                 // fproc_write(0, S, the count, 4)
    xor edi, edi
    movabs rsi, {fsa}
    mov rdx, r12
    mov ecx, 4
    int 0x80
    mov eax, 53                 // fproc_start(0, A, S + 4096)
    xor edi, edi
    movabs rsi, {fa}
    movabs rdx, {fsa}
    add rdx, 4096
    int 0x80
2:
    mov eax, 24                 // wait_one(1, READABLE | PEER_CLOSED, no limit)
    mov edi, 1
    mov esi, 5
    mov rdx, -1
    int 0x80
    test eax, 1
    jz 4f
    mov eax, 18                 // channel_read(1, data + 64, 96)
    mov edi, 1
    lea rsi, [r12 + 64]
    mov edx, 96
    xor ecx, ecx
    xor r8d, r8d
    int 0x80
    cmp word ptr [r12 + 66], 1
    jne 2b                      // FAULT or EXITED: wait for the end
    mov word ptr [r12 + 192], 1
    mov word ptr [r12 + 194], 1
    mov dword ptr [r12 + 196], 0
    mov rax, qword ptr [r12 + 72]
    mov qword ptr [r12 + 200], rax
    mov qword ptr [r12 + 208], -9
    mov eax, 17                 // channel_write(1, the reply, 24)
    mov edi, 1
    lea rsi, [r12 + 192]
    mov edx, 24
    xor ecx, ecx
    xor r8d, r8d
    int 0x80
    jmp 2b
4:
    xor eax, eax
    xor edi, edi
    int 0x80

// the foreign program: its count is at S (written by the bridge), N Linux write(-1, 0, 0) calls, then ud2 (a FAULT
// ends it; the bridge then sees PEER_CLOSED)
daraja_b_fp:
    movabs rcx, {fsa}
    mov r12d, dword ptr [rcx]
2:
    test r12d, r12d
    jz 3f
    mov eax, 1
    mov rdi, -1
    xor esi, esi
    xor edx, edx
    syscall
    dec r12d
    jmp 2b
3:
    ud2
daraja_b_fp_end:
daraja_b_bridge_end:
"#,
    data = const proc::DATA_VA,
    fa = const crate::mmu::USER_BASE + 0x40_0000,
    fsa = const crate::mmu::USER_BASE + 0x50_0000,
);

extern "C" {
    static daraja_b_null: u8;
    static daraja_b_null_end: u8;
    static daraja_b_client: u8;
    static daraja_b_client_end: u8;
    static daraja_b_server: u8;
    static daraja_b_server_end: u8;
    static daraja_b_bridge: u8;
    static daraja_b_bridge_end: u8;
}

/// Spawn `progs` with the count `n` in each one's data page (written before it runs), run them, and give the time
/// run_all took in microseconds; None if a spawn failed.
fn timed(n: u32, spawn: &dyn Fn() -> Option<[Option<usize>; 2]>) -> Option<u64> {
    let slots = spawn()?;
    for s in slots.iter().flatten() {
        proc::poke_data_u32(*s, n);
    }
    let t0 = crate::timer::cycles();
    proc::run_all();
    let us = crate::timer::cycles().saturating_sub(t0) * 1_000_000 / crate::timer::hz().max(1);
    Some(us)
}

/// The N calls alone: the run with N minus the run with 0.
fn net(spawn: &dyn Fn() -> Option<[Option<usize>; 2]>) -> Option<u64> {
    let full = timed(BENCH_N, spawn)?;
    let empty = timed(0, spawn)?;
    Some(full.saturating_sub(empty))
}

/// `daraja bench`: the three numbers and the one line.
pub fn bench() {
    // SAFETY: label addresses only, for every program below.
    let null = prog(unsafe { &daraja_b_null }, unsafe { &daraja_b_null_end });
    let client = prog(unsafe { &daraja_b_client }, unsafe { &daraja_b_client_end });
    let server = prog(unsafe { &daraja_b_server }, unsafe { &daraja_b_server_end });
    let bridge = prog(unsafe { &daraja_b_bridge }, unsafe { &daraja_b_bridge_end });
    let a = net(&|| Some([Some(proc::spawn(null, &[], false)?), None]));
    let b = net(&|| {
        let s = proc::spawn(server, &[(0, proc::RIGHT_RECV), (1, proc::RIGHT_SEND)], false)?;
        let c = proc::spawn(client, &[(0, proc::RIGHT_SEND), (1, proc::RIGHT_RECV)], false)?;
        Some([Some(s), Some(c)])
    });
    QUIET.store(true, Ordering::Relaxed);
    let c = net(&|| {
        let (br, _) = proc::spawn_pair(bridge, proc::PERS_LINUX_X86_64, b"bench: no bundle is read")?;
        Some([Some(br), None])
    });
    QUIET.store(false, Ordering::Relaxed);
    match (a, b, c) {
        (Some(a), Some(b), Some(c)) => crate::kprintln!(
            "[daraja] bench: {} calls: native-null {} us, native-ipc {} us, forwarded {} us",
            BENCH_N, a, b, c
        ),
        _ => crate::kprintln!("[daraja] bench: a program could not be spawned (null {:?}, ipc {:?}, forwarded {:?})", a, b, c),
    }
}
