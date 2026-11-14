//! Userspace processes, capabilities, and IPC — the Phase 0 finale.
//!
//! Two **EL0** processes each run in their **own address space** (private L1/L2
//! page tables), can touch the kernel only through **SVC syscalls**, and may
//! send/receive on an IPC endpoint **only if they hold a capability granting
//! that right**. This is the capability-security spine the whole OS rests on:
//! no ambient authority — a process can do exactly what its capabilities allow,
//! and nothing more.
//!
//! Demo flow: P1 holds a SEND cap and P2 a RECV cap to the same endpoint. P1
//! sends `0x42`; P2 receives it; P2 then tries to SEND on its RECV-only cap and
//! is refused by the kernel. All I/O is via a debug syscall so the exchange is
//! visible on the console.
//!
//! Scheduling here is **cooperative** (driven by blocking IPC syscalls); EL0
//! runs with IRQs masked. Preemptive time-slicing of EL0 (the lower-EL IRQ
//! path) is a Phase 1 concern — Phase 0 only needs to prove the mechanism.

use crate::{cpu, frame, heap, paging};
use core::arch::{asm, global_asm};
use core::ptr::addr_of_mut;

// --- Layout of a user address space (each process, in its own TTBR0) ---------
const USER_ENTRY: u64 = 0x8000_0000; // VA of user code (L1[2], L2[0] → 2 MiB)
const USER_STACK_TOP: u64 = 0x8040_0000; // top of the 2 MiB stack region (L2[1])
const TWO_MIB: usize = 0x20_0000;

// --- Table sizes -------------------------------------------------------------
const N_USER: usize = 5; // 2 IPC demo + 2 preemption workers + reused for 3 GUI apps
const N_PROC: usize = N_USER + 1; // + idle (index 0)
const N_CAPS: usize = 4;
const N_ENDPOINTS: usize = 1;
/// One kernel stack per process — the same 16 KiB `sched.rs` takes per thread, and the
/// same constant, declared in the allocator that pays for it so `heap`/`ps` can state the
/// arena's room in processes rather than in bytes (2.735).
const KSTACK_SIZE: usize = heap::STACK_SIZE;
const FRAME_SLOTS: usize = 12; // callee-saved slots forged for cpu_switch

/// Why a spawn was refused. A heap refusal arrives already named by `heap::alloc_stack`;
/// this is the other ceiling `spawn` can hit — the frame allocator, out of physical
/// frames for the process's private L1/L2 tables. Both used to be panics.
const BOUND_FRAMES: &str = "no free frame for the process page tables (see `frame`)";

// --- Syscall numbers (passed in x8) ------------------------------------------
const SYS_DEBUG: u64 = 0;
const SYS_SEND: u64 = 1;
const SYS_RECV: u64 = 2;
const SYS_EXIT: u64 = 3;
const SYS_YIELD: u64 = 4;
const SYS_WIN_CREATE: u64 = 5; // (w, h) -> window id
const SYS_WIN_FILL: u64 = 6; // (wid, rx, ry, rw, rh, color)
const SYS_WIN_PRESENT: u64 = 7; // composite all app surfaces
const SYS_GETPID: u64 = 8; // -> current process id
const SYS_WIN_TEXT: u64 = 9; // (wid, x, y, char, color) -> draw a glyph
const SYS_WIN_GETKEY: u64 = 10; // block for the next input key -> key code
const SYS_PUTC: u64 = 11; // (char) -> print one character to the console
const SYS_TIME: u64 = 12; // -> seconds since epoch (PL031 RTC)
const SYS_AI: u64 = 13; // (prompt_ptr, prompt_len, out_ptr, out_cap) -> reply len
const SYS_SPEAK: u64 = 14; // (text_ptr, text_len) -> speak text (Asili Voice)
const SYS_SLEEP: u64 = 15; // (ms) -> block for ~ms, returns measured elapsed ms
// 2.991 probe-only syscalls (reached solely by the `sgi-el0` debug command).
const SYS_SGI_ARM: u64 = 16; // clear the probe counter and self-target a PROBE_SGI
const SYS_SGI_READ: u64 = 17; // -> deliveries seen; prints SGI-EL0-SEEN <n>

// --- Capability rights -------------------------------------------------------
const RIGHT_SEND: u8 = 1 << 0;
const RIGHT_RECV: u8 = 1 << 1;

/// Value returned to userspace when a syscall is refused.
const EPERM: u64 = u64::MAX;

/// Gap G2 (docs/security-model.md): may the kernel follow this user pointer for `len` bytes? Every process's
/// tables keep the kernel's DRAM mapped EL1 read/write, so an unchecked pointer would let EL0 code make the
/// kernel read or write kernel memory. Reads may come from the code or stack block (both mapped for every
/// process); writes only into the stack block (the code block is read-only at EL1 too, so a write there faults).
fn user_range(ptr: u64, len: u64, write: bool) -> bool {
    let lo = if write { USER_ENTRY + TWO_MIB as u64 } else { USER_ENTRY };
    match ptr.checked_add(len) {
        Some(end) => ptr >= lo && end <= USER_STACK_TOP,
        None => false,
    }
}

// --- User-block descriptor attributes (AArch64 stage-1) ----------------------
const AP_EL0_RW: u64 = 0b01 << 6; // EL0 + EL1 read/write
const AP_EL0_RO: u64 = 0b11 << 6; // EL0 + EL1 read-only
const PXN: u64 = 1 << 53; // privileged (EL1) execute-never
const UXN: u64 = 1 << 54; // unprivileged (EL0) execute-never

/// Executable, read-only user code block (EL0 may fetch+read, not write).
fn user_code_block(pa: u64) -> u64 {
    pa | paging::AF | paging::SH_INNER | paging::ATTR_NORMAL | AP_EL0_RO | PXN | paging::BLOCK_DESC
}
/// Writable, non-executable user data/stack block.
fn user_stack_block(pa: u64) -> u64 {
    pa | paging::AF
        | paging::SH_INNER
        | paging::ATTR_NORMAL
        | AP_EL0_RW
        | PXN
        | UXN
        | paging::BLOCK_DESC
}

/// EL0-accessible **Device** memory block (MAIR index 0), non-executable — the
/// hardware capability a userspace driver holds for its MMIO region. Granting
/// this maps exactly one device window into the driver's address space and
/// nothing else, so a driver can touch only its own hardware.
fn user_device_block(pa: u64) -> u64 {
    pa | paging::AF | AP_EL0_RW | PXN | UXN | paging::BLOCK_DESC
}

/// User VA where a driver's device-MMIO window is mapped (2 MiB, → PA 0x0900_0000,
/// the QEMU virt device region containing the PL031 RTC at PA 0x0901_0000).
/// The RTC driver reads the clock at VA 0x8061_0000 (this base + 0x10000).
#[allow(dead_code)] // documents the VA the driver asm hardcodes
const USER_MMIO_VA: u64 = 0x8060_0000;
const MMIO_PHYS_BASE: u64 = 0x0900_0000;

// --- Physical backing for user code/stack (2 MiB-aligned, in .bss) -----------
// One slot per process index (slot 0 = idle, unused). Indexing by process id
// keeps each process's memory distinct — sizing to N_PROC (not N_USER) is what
// makes that indexing in-bounds.
#[derive(Clone, Copy)]
#[repr(C, align(0x200000))]
struct Region2M([u8; TWO_MIB]);
const ZERO_REGION: Region2M = Region2M([0; TWO_MIB]);
static mut USER_CODE_MEM: [Region2M; N_PROC] = [ZERO_REGION; N_PROC];
static mut USER_STACK_MEM: [Region2M; N_PROC] = [ZERO_REGION; N_PROC];

// --- Editor persistence (2.86) ----------------------------------------------
// The `edit` EL0 app draws typed keys into its window; during an edit session
// the kernel also buffers those printable keys here (see SYS_WIN_GETKEY) so
// run_edit_app can persist the text to the VFS when the app exits. The asm app
// itself is unchanged — this is entirely kernel-side.
static mut EDIT_BUF: [u8; 256] = [0; 256];
static mut EDIT_LEN: usize = 0;
static mut EDITING: bool = false;

// --- Kernel object model -----------------------------------------------------
#[derive(Clone, Copy)]
struct Cap {
    valid: bool,
    obj: usize, // endpoint index
    rights: u8,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Runnable,
    BlockedSend,
    BlockedRecv,
    Dead,
}

#[derive(Clone, Copy)]
struct Process {
    sp: u64,     // saved kernel stack pointer (for cpu_switch)
    ttbr0: u64,  // this process's address space
    is_user: bool,
    state: State,
    caps: [Cap; N_CAPS],
    syscall_ret: u64, // value handed back when a blocked syscall resumes
    preempt: bool,    // true → runs at EL0 with IRQs enabled (timer-preemptible)
}

#[derive(Clone, Copy)]
struct Endpoint {
    sender: Option<usize>,
    sender_msg: u64,
    receiver: Option<usize>,
}

const IDLE: Process = Process {
    sp: 0,
    ttbr0: 0,
    is_user: false,
    state: State::Runnable,
    caps: [Cap { valid: false, obj: 0, rights: 0 }; N_CAPS],
    syscall_ret: 0,
    preempt: false,
};

static mut PROCS: [Process; N_PROC] = [IDLE; N_PROC];
static mut ENDPOINTS: [Endpoint; N_ENDPOINTS] =
    [Endpoint { sender: None, sender_msg: 0, receiver: None }; N_ENDPOINTS];
static mut CURRENT: usize = 0;

extern "C" {
    // Defined in sched.rs (reused): swap kernel context between processes.
    fn cpu_switch(prev_sp: *mut u64, next_sp: u64);
    // First-run landing pad for a user process (asm below).
    fn user_entry_trampoline();
    // User program images (position-independent asm below).
    static user_a_start: u8;
    static user_a_end: u8;
    static user_b_start: u8;
    static user_b_end: u8;
    static user_worker_start: u8;
    static user_worker_end: u8;
    static user_sgi_start: u8;
    static user_sgi_end: u8;
    static user_sleep_start: u8;
    static user_sleep_end: u8;
    static user_fault_start: u8;
    static user_fault_end: u8;
    static user_ptr_start: u8;
    static user_ptr_end: u8;
    static user_gui_start: u8;
    static user_gui_end: u8;
    static user_edit_start: u8;
    static user_edit_end: u8;
    static user_rtc_start: u8;
    static user_rtc_end: u8;
    static user_rtc_client_start: u8;
    static user_rtc_client_end: u8;
}

// --- Raw accessors (single-core, IRQ-masked while at EL1 → no aliasing) -------
#[inline]
unsafe fn proc_ptr(i: usize) -> *mut Process {
    (addr_of_mut!(PROCS) as *mut Process).add(i)
}

fn state_name(s: State) -> &'static str {
    match s {
        State::Runnable => "runnable",
        State::BlockedSend => "blk-send",
        State::BlockedRecv => "blk-recv",
        State::Dead => "dead",
    }
}

/// Print a read-only snapshot of the kernel process table: one line per slot plus
/// a one-line summary. Slot 0 is the idle/boot (kernel) context that hosts the
/// shell; slots 1.. are EL0 user slots, which read `dead` once their demo/app has
/// exited (this is a cooperative single-shell system). Called by the shell `ps`.
pub fn dump_table() {
    // SAFETY: single-core, shell (idle) context; a read-only pass over PROCS.
    unsafe {
        crate::kprintln!("  SLOT  STATE     KIND    PREEMPT");
        let (mut runnable, mut blocked, mut dead) = (0u32, 0u32, 0u32);
        for i in 0..N_PROC {
            let p = proc_ptr(i);
            let st = (*p).state;
            match st {
                State::Runnable => runnable += 1,
                State::BlockedSend | State::BlockedRecv => blocked += 1,
                State::Dead => dead += 1,
            }
            let kind = if i == 0 {
                "idle"
            } else if (*p).is_user {
                "el0"
            } else {
                "kernel"
            };
            let pre = if (*p).preempt { "yes" } else { "no" };
            crate::kprintln!("  P{}    {:8}  {:6}  {}", i, state_name(st), kind, pre);
        }
        crate::kprintln!(
            "ps: {} slots, {} runnable, {} blocked, {} dead",
            N_PROC, runnable, blocked, dead
        );
        // Slots are not the only ceiling on `spawn`, and they are not the one that runs
        // out first: every process also costs a 16 KiB kernel stack from an arena that
        // never gives anything back (2.735). A free slot with no stack behind it is not
        // a process you can start, so `ps` states both.
        crate::kprintln!(
            "    stacks: {} served since boot, room for {} more at {} KiB (see `heap`)",
            crate::heap::stacks_served(),
            crate::heap::stacks_left(),
            crate::heap::STACK_SIZE / 1024
        );
    }
}
#[inline]
unsafe fn ep_ptr(i: usize) -> *mut Endpoint {
    (addr_of_mut!(ENDPOINTS) as *mut Endpoint).add(i)
}

/// Register the boot context as the idle process (index 0).
pub fn init() {
    // SAFETY: called once at boot, IRQs masked, single core.
    unsafe {
        let idle = proc_ptr(0);
        (*idle).ttbr0 = paging::kernel_ttbr0();
        (*idle).is_user = false;
        (*idle).state = State::Runnable;
        // Slots default to the IDLE template (Runnable); mark the unspawned user
        // slots Dead so the scheduler never jumps to an un-initialized process.
        for i in 1..N_PROC {
            (*proc_ptr(i)).state = State::Dead;
        }
        CURRENT = 0;
    }
}

/// Build the private page tables for user process `idx` and return its TTBR0, or `None`
/// if the frame allocator has nothing left to build them from.
/// `mmio` grants the device-MMIO window (a hardware capability for a driver).
///
/// 2.735: the two `frame::alloc().expect(...)` calls that used to be here were the same
/// mistake as the heap `assert!` below — a bound that takes the machine down instead of
/// failing the spawn. Both frames are taken up front so a half-built address space is
/// never left behind: if the second one is not there, the first is released.
unsafe fn build_address_space(idx: usize, mmio: bool) -> Option<u64> {
    let l1 = frame::alloc()? as *mut u64;
    let l2 = match frame::alloc() {
        Some(f) => f as *mut u64,
        None => {
            frame::free(l1 as usize);
            return None;
        }
    };
    // Zero both tables (frame allocator does not clear).
    for i in 0..512 {
        l1.add(i).write(0);
        l2.add(i).write(0);
    }

    let code_pa = addr_of_mut!(USER_CODE_MEM) as usize + idx * core::mem::size_of::<Region2M>();
    let stack_pa = addr_of_mut!(USER_STACK_MEM) as usize + idx * core::mem::size_of::<Region2M>();

    // L1: [0] devices (EL1), [1] DRAM (EL1) — so the kernel stays mapped; [2] → L2.
    l1.add(0).write(paging::device_block(0x0000_0000));
    // F7: the kernel's own L1[1] (its DRAM; a table since the stack guards split it), so the boot-stack
    // guards stay in force while this address space is live. EL1-only, like the block it replaced.
    l1.add(1).write(paging::kernel_l1_entry(1));
    l1.add(2).write(l2 as u64 | paging::TABLE_DESC);

    // L2 (covers VA 0x8000_0000..0xBFFF_FFFF): [0] code, [1] stack.
    l2.add(0).write(user_code_block(code_pa as u64)); // VA 0x8000_0000
    l2.add(1).write(user_stack_block(stack_pa as u64)); // VA 0x8020_0000

    // L2[3] (VA 0x8060_0000): optional device-MMIO window for a driver process.
    if mmio {
        l2.add(3).write(user_device_block(MMIO_PHYS_BASE)); // VA 0x8060_0000
    }

    Some(l1 as u64)
}

/// Spawn user process `idx` running the image `[src_start, src_end)`, or refuse **by
/// name** without starting it.
/// `preemptible` = timer-preemptible EL0; `mmio` = grant the device-MMIO window
/// (a hardware capability), turning this process into a userspace driver.
///
/// 2.735: the kernel stack is taken FIRST, before the program is copied and before any
/// frame is spent on page tables, so a refusal costs nothing and leaves nothing behind.
/// It used to be taken last and answered with `assert!(!kstack.is_null(), "no heap for
/// kernel stack")` — a panic, after two frames had already been consumed.
unsafe fn spawn(
    idx: usize,
    src_start: *const u8,
    src_end: *const u8,
    preemptible: bool,
    mmio: bool,
) -> Result<(), &'static str> {
    // Kernel stack + forged initial frame so the first cpu_switch lands in
    // user_entry_trampoline with x19 = idx (see sched.rs for the slot layout).
    let kstack = heap::alloc_stack()?;

    // Copy the (position-independent) program into its physical code region.
    let len = src_end as usize - src_start as usize;
    let code_dst = addr_of_mut!(USER_CODE_MEM) as *mut u8;
    let dst = code_dst.add(idx * core::mem::size_of::<Region2M>());
    core::ptr::copy_nonoverlapping(src_start, dst, len);
    cpu::sync_icache_range(dst as usize, len);

    let ttbr0 = match build_address_space(idx, mmio) {
        Some(t) => t,
        None => return Err(BOUND_FRAMES),
    };

    let frame_base = ((kstack as usize) + KSTACK_SIZE - FRAME_SLOTS * 8) as *mut u64;
    for i in 0..FRAME_SLOTS {
        frame_base.add(i).write(0);
    }
    frame_base.add(1).write(user_entry_trampoline as *const () as u64); // x30
    frame_base.add(10).write(idx as u64); // x19 = process index

    let p = proc_ptr(idx);
    (*p).sp = frame_base as u64;
    (*p).ttbr0 = ttbr0;
    (*p).is_user = true;
    (*p).state = State::Runnable;
    (*p).syscall_ret = 0;
    (*p).preempt = preemptible;
    Ok(())
}

/// [`spawn`], with the refusal reported on the console and folded into a bool the demo
/// call sites can bail on.
///
/// The point of 2.735 is that the bound reaches the *caller*: the allocator knows the
/// arena filled up but not whether the machine can carry on without this process, and
/// the caller knows the second but never used to be told the first. Every `spawn` site
/// below goes through here, so a refusal ends one demo instead of the kernel.
#[must_use = "a refused spawn produced no process — do not switch to the slot"]
unsafe fn spawn_or_refuse(
    idx: usize,
    src_start: *const u8,
    src_end: *const u8,
    preemptible: bool,
    mmio: bool,
) -> bool {
    match spawn(idx, src_start, src_end, preemptible, mmio) {
        Ok(()) => true,
        Err(why) => {
            crate::kprintln!("[proc] P{} NOT spawned — {}", idx, why);
            crate::kprintln!(
                "[proc] the machine is still up; {} kernel stack(s) served, room for {} more",
                heap::stacks_served(),
                heap::stacks_left()
            );
            false
        }
    }
}

/// Grant process `idx` a capability at `handle`: `rights` over endpoint `obj`.
unsafe fn grant(idx: usize, handle: usize, obj: usize, rights: u8) {
    (*proc_ptr(idx)).caps[handle] = Cap { valid: true, obj, rights };
}

/// Set up and launch the two-process capability-secured IPC demo. Returns once
/// both user processes have exited (control back on the idle context).
pub fn run_demo() {
    // SAFETY: single-core, IRQs masked; this is the sole driver of the process
    // subsystem until the demo completes.
    unsafe {
        // --- Cooperative capability-secured IPC demo (deterministic) ---
        // A refusal ends the demo, not the kernel (2.735). The pair is all-or-nothing:
        // a sender with no receiver would block forever on an endpoint nobody reads.
        if !spawn_or_refuse(1, &user_a_start as *const u8, &user_a_end as *const u8, false, false) {
            return;
        }
        grant(1, 0, 0, RIGHT_SEND); // P1: SEND cap to endpoint 0

        if !spawn_or_refuse(2, &user_b_start as *const u8, &user_b_end as *const u8, false, false) {
            crate::kprintln!("[proc] IPC demo abandoned — P1 spawned but has no peer to send to");
            (*proc_ptr(1)).state = State::Dead; // never scheduled; do not leave it Runnable
            return;
        }
        grant(2, 0, 0, RIGHT_RECV); // P2: RECV cap to endpoint 0

        crate::kprintln!("[mansa] P1 = sender (SEND cap), P2 = receiver (RECV cap)");
        crate::kprintln!("[mansa] handing the CPU to userspace (EL0)...");
        crate::kprintln!();
        schedule_switch(); // runs P1/P2 to completion (both exit)

        // --- Preemptive multitasking demo (timer time-slices two workers) ---
        crate::kprintln!();
        crate::kprintln!("[mansa] preemption demo: 2 CPU-bound EL0 workers, timer time-slices them");
        // One worker still proves preemption (it time-slices against idle), so a refusal
        // of the second is survivable; a refusal of the first leaves nothing to run.
        if !spawn_or_refuse(3, &user_worker_start as *const u8, &user_worker_end as *const u8, true, false) {
            return;
        }
        let _ = spawn_or_refuse(4, &user_worker_start as *const u8, &user_worker_end as *const u8, true, false);
        schedule_switch(); // interleaves P3/P4 via timer preemption until both exit
        crate::kprintln!("[mansa] workers exited — preemptive EL0 multitasking proven");
    }
}

/// Spawn one EL0 app that blocks via the `SYS_SLEEP` syscall, proving the kernel
/// can suspend a userspace process for a bounded, measured duration. Runs to exit
/// and returns to the shell. Called from the shell `syssleep` command.
pub fn run_sleep_demo() {
    // SAFETY: single-core, shell (idle) context; the sole driver here.
    unsafe {
        if spawn_or_refuse(3, &user_sleep_start as *const u8, &user_sleep_end as *const u8, false, false) {
            schedule_switch();
        }
    }
}

/// 2.991 falsify-first probe for the missing SGI arm in `rust_irq_from_el0`.
/// Both arms run on ONE boot, same SGI id, same self-target, so the only
/// difference between them is the exception entry point:
///   * control (EL1): the shell self-SGIs here and prints `SGI-EL1-SEEN <n>`;
///   * subject (EL0): a preemptible EL0 process arms the same SGI via a syscall
///     and prints `SGI-EL0-SEEN <n>` after the eret has let it be taken.
/// Without the control, `SGI-EL0-SEEN 0` would be equally explained by the SGI
/// never being delivered at all (2.766/2.765 both shipped that mistake).
pub fn run_sgi_el0_probe() {
    // SAFETY: single-core, shell (idle) context; the sole driver here.
    unsafe {
        use core::sync::atomic::Ordering;
        crate::SGI_PROBE_SEEN.store(0, Ordering::Release);
        crate::gic::send_sgi(cpu::core_id() as u32, crate::PROBE_SGI);
        for _ in 0..2_000_000u32 {
            core::hint::spin_loop();
        }
        crate::kprintln!("SGI-EL1-SEEN {}", crate::SGI_PROBE_SEEN.load(Ordering::Acquire));
        if spawn_or_refuse(3, &user_sgi_start as *const u8, &user_sgi_end as *const u8, true, false) {
            schedule_switch();
        } else {
            crate::kprintln!("SGI-EL0-SEEN spawn-refused");
        }
    }
}

/// Spawn an EL0 process that deliberately violates memory safety (writes to an
/// unmapped address). The hardware raises a translation fault, the kernel's
/// exception path kills the offending process, and control returns here -- the
/// OS survives. Demonstrates hardware-enforced fault isolation (the microkernel
/// guarantee: a userspace crash cannot crash the kernel). Called by `faultdemo`.
pub fn run_fault_demo() {
    // SAFETY: single-core, shell (idle) context; the sole driver here.
    unsafe {
        if spawn_or_refuse(3, &user_fault_start as *const u8, &user_fault_end as *const u8, false, false) {
            schedule_switch();
        }
    }
}

/// Gap G2 probe (`el0-ptr`): an EL0 program hands SYS_AI a KERNEL address as its reply buffer and SYS_SPEAK a
/// kernel address as its text. `G2_CANARY` is that kernel memory. A kernel that follows user pointers unchecked
/// overwrites the canary and speaks its bytes; a fixed one refuses both calls and the canary stays as it was.
/// The same program then calls WIN_CREATE, WIN_FILL and WIN_TEXT with u64::MAX coordinates and sizes: they take no
/// pointer, but must clip rather than overflow (a panic stops the debug kernel), then prints 0x5afe.
#[no_mangle]
static mut G2_CANARY: [u8; 16] = *b"G2-CANARY-INTACT";

pub fn run_el0_ptr_probe() {
    // SAFETY: single-core, shell (idle) context; the sole driver here.
    unsafe {
        *addr_of_mut!(G2_CANARY) = *b"G2-CANARY-INTACT";
        crate::tonga::wm_reset(); // a free window for the u64::MAX WIN_FILL / WIN_TEXT case
        if spawn_or_refuse(3, &user_ptr_start as *const u8, &user_ptr_end as *const u8, false, false) {
            schedule_switch();
        }
        let c = *addr_of_mut!(G2_CANARY);
        if &c == b"G2-CANARY-INTACT" {
            crate::kprintln!("G2-CANARY intact");
        } else {
            crate::kprintln!("G2-CANARY OVERWRITTEN by an EL0 pointer: \"{}\"", core::str::from_utf8(&c).unwrap_or("?"));
        }
    }
}

/// Spawn three EL0 "GUI apps"; each creates a window via the compositor and
/// draws into it. Runs them to completion (surfaces persist for display).
/// Called from the shell `wm` command (console already suspended).
pub fn run_gui_demo() {
    // SAFETY: single-core, shell (idle) context; the sole driver here.
    unsafe {
        crate::tonga::wm_reset();
        // Independent windows: however many the arena affords is a working demo, so each
        // refusal costs one window rather than the compositor (2.735).
        let mut windows = 0;
        for idx in 3..=5 {
            if spawn_or_refuse(idx, &user_gui_start as *const u8, &user_gui_end as *const u8, false, false) {
                windows += 1;
            }
        }
        if windows > 0 {
            schedule_switch(); // runs each app: create window, draw, present, exit
        }
    }
}

/// A real **compiled-Rust** EL0 app (built from `userland/rustapp`, objcopy'd to
/// a raw binary linked at 0x8000_0000). Proves that ordinary compiled Rust — not
/// just hand-written asm — runs at EL0 through the syscall boundary. This is the
/// foundation for hosting larger userland programs (the Nia interpreter) as EL0
/// processes.
static RUSTAPP: &[u8] = include_bytes!("rustapp.bin");

/// Spawn and run the compiled-Rust EL0 app.
pub fn run_rustapp() {
    // SAFETY: single-core, shell (idle) context.
    unsafe {
        let s = RUSTAPP.as_ptr();
        let e = s.add(RUSTAPP.len());
        if spawn_or_refuse(3, s, e, false, false) {
            schedule_switch();
        }
    }
}

/// The Nia mini WASM interpreter, compiled as an EL0 app (`userland/niavm`). It
/// runs a WASM module **at EL0**; the module's host `print` becomes a syscall.
static NIAVM: &[u8] = include_bytes!("niavm.bin");

/// Spawn and run the Nia mini-interpreter as an EL0 process.
pub fn run_niavm() {
    // SAFETY: single-core, shell (idle) context.
    unsafe {
        let s = NIAVM.as_ptr();
        let e = s.add(NIAVM.len());
        if spawn_or_refuse(3, s, e, false, false) {
            schedule_switch();
        }
    }
}

/// Spawn one interactive "text editor" EL0 app. It creates a window, then loops
/// blocking on key input (routed from the kernel) and drawing each key into its
/// surface — until the user presses '~'. Returns to the shell when it exits.
pub fn run_edit_app() {
    // SAFETY: single-core, shell (idle) context.
    unsafe {
        crate::tonga::wm_reset();
        // Arm an edit session: SYS_WIN_GETKEY buffers the typed keys into EDIT_BUF.
        EDIT_LEN = 0;
        EDITING = true;
        // A refused spawn must disarm the edit session it armed, or the next key the
        // shell reads would still be buffered into a document nobody is editing (2.735).
        if !spawn_or_refuse(3, &user_edit_start as *const u8, &user_edit_end as *const u8, false, false) {
            EDITING = false;
            return;
        }
        schedule_switch();
        // The app exited (user pressed '~'). Persist what was typed to the VFS.
        EDITING = false;
        let n = EDIT_LEN;
        if n > 0 {
            let buf = core::slice::from_raw_parts(addr_of_mut!(EDIT_BUF) as *const u8, n);
            if crate::vfs::write("edited.txt", buf) {
                crate::kprintln!("[edit] saved {} bytes to edited.txt", n);
            } else {
                crate::kprintln!("[edit] save FAILED (filesystem full?)");
            }
        } else {
            crate::kprintln!("[edit] nothing typed; edited.txt unchanged");
        }
    }
}

/// Userspace driver demo: spawn an RTC **driver** process (granted the device-MMIO
/// capability + a SEND cap) and a **client** (RECV cap). The driver reads the
/// PL031 real-time clock directly from EL0 through its MMIO window and serves the
/// time to the client over capability-secured IPC — a driver as a sandboxed
/// userspace process, the true microkernel structure.
pub fn run_rtc_demo() {
    // SAFETY: single-core, shell (idle) context.
    unsafe {
        // reset endpoint 0
        (*ep_ptr(0)).sender = None;
        (*ep_ptr(0)).receiver = None;
        // Driver and client are a pair, like the IPC demo: one without the other blocks
        // forever on an endpoint with no counterpart, so a refusal ends the demo (2.735).
        if !spawn_or_refuse(3, &user_rtc_start as *const u8, &user_rtc_end as *const u8, false, true) {
            return;
        }
        grant(3, 0, 0, RIGHT_SEND); // driver: SEND the time
        if !spawn_or_refuse(4, &user_rtc_client_start as *const u8, &user_rtc_client_end as *const u8, false, false) {
            crate::kprintln!("[proc] RTC demo abandoned — the driver has no client to serve");
            (*proc_ptr(3)).state = State::Dead; // never scheduled; do not leave it Runnable
            return;
        }
        grant(4, 0, 0, RIGHT_RECV); // client: RECV the time
        schedule_switch();
    }
}

/// Pick the next runnable process, preferring user processes over idle.
unsafe fn pick_next(cur: usize) -> usize {
    for k in 1..N_PROC {
        let i = (cur + k) % N_PROC;
        if i != 0 && (*proc_ptr(i)).state == State::Runnable {
            return i;
        }
    }
    0 // fall back to idle
}

/// Switch from the current process to the next runnable one (or idle).
unsafe fn schedule_switch() {
    let cur = CURRENT;
    let next = pick_next(cur);
    if next == cur {
        return;
    }
    CURRENT = next;
    let np = proc_ptr(next);
    cpu::set_ttbr0(if (*np).is_user { (*np).ttbr0 } else { paging::kernel_ttbr0() });
    let prev_sp = addr_of_mut!((*proc_ptr(cur)).sp);
    let next_sp = (*np).sp;
    cpu_switch(prev_sp, next_sp);
    // Control returns here only when `cur` is scheduled again.
}

/// Preemptive round-robin among runnable **user** processes (never idle).
/// Called from the EL0 timer-IRQ handler; if no other user process is runnable,
/// the current one keeps the CPU (no pointless switch to idle mid-task).
unsafe fn preempt() {
    let cur = CURRENT;
    let mut next = cur;
    for k in 1..N_PROC {
        let i = (cur + k) % N_PROC;
        if i != 0 && i != cur && (*proc_ptr(i)).state == State::Runnable {
            next = i;
            break;
        }
    }
    if next == cur {
        return;
    }
    CURRENT = next;
    let np = proc_ptr(next);
    cpu::set_ttbr0(if (*np).is_user { (*np).ttbr0 } else { paging::kernel_ttbr0() });
    let prev_sp = addr_of_mut!((*proc_ptr(cur)).sp);
    cpu_switch(prev_sp, (*np).sp);
}

/// Timer IRQ taken while a (preemptible) process runs at EL0. Acknowledge, rearm
/// the tick, EOI, then time-slice to the next runnable user process.
#[no_mangle]
extern "C" fn rust_irq_from_el0() {
    // SAFETY: at EL1 with IRQs masked (exception entry); single accessor.
    unsafe {
        let iar = crate::gic::acknowledge();
        let intid = iar & 0x3FF;
        if intid == crate::timer::TIMER_INTID {
            crate::timer::rearm();
            crate::gic::end_of_interrupt(iar);
            preempt();
        } else if intid == crate::uart::UART_RX_INTID {
            // Console input arriving while an EL0 process runs. The PL011 RX
            // interrupt is LEVEL-sensitive: EOI alone does not deassert it, so
            // without draining the FIFO here the same interrupt re-fires the
            // instant we `eret`, and the EL0 process never advances again.
            crate::uart::rx_isr();
            crate::gic::end_of_interrupt(iar);
        } else if intid < 16 {
            // Software-generated interrupt taken while this core was at EL0.
            // Mirrors exceptions.rs:55-59: without this arm the SGI is
            // acknowledged, EOI'd by the fallthrough and DROPPED, and because
            // SGIs are edge-type the loss is silent (finding-2.990). Whether
            // `c1_on_sgi` should itself be core-gated is a separate defect
            // present on BOTH entry points (design-2.991 §4) - not folded in
            // here, so the probe grades exactly one change.
            crate::c1_on_sgi(intid);
            crate::gic::end_of_interrupt(iar); // EOI the raw IAR (keeps SGI source bits)
        } else if intid < 1020 {
            crate::gic::end_of_interrupt(iar);
        }
    }
}

// --- Syscalls ----------------------------------------------------------------
unsafe fn cap_lookup(pid: usize, handle: usize, need: u8) -> Option<usize> {
    if handle >= N_CAPS {
        return None;
    }
    let c = (*proc_ptr(pid)).caps[handle];
    if c.valid && (c.rights & need) == need {
        Some(c.obj)
    } else {
        None
    }
}

unsafe fn sys_send(pid: usize, handle: usize, msg: u64) -> u64 {
    let ep = match cap_lookup(pid, handle, RIGHT_SEND) {
        Some(e) => e,
        None => return EPERM,
    };
    let e = ep_ptr(ep);
    if let Some(rpid) = (*e).receiver.take() {
        // A receiver is already waiting: hand it the message and wake it.
        (*proc_ptr(rpid)).syscall_ret = msg;
        (*proc_ptr(rpid)).state = State::Runnable;
        0
    } else {
        // No receiver yet: park as sender until one arrives.
        (*e).sender = Some(pid);
        (*e).sender_msg = msg;
        (*proc_ptr(pid)).state = State::BlockedSend;
        schedule_switch();
        0 // resumed: send completed
    }
}

unsafe fn sys_recv(pid: usize, handle: usize) -> u64 {
    let ep = match cap_lookup(pid, handle, RIGHT_RECV) {
        Some(e) => e,
        None => return EPERM,
    };
    let e = ep_ptr(ep);
    if let Some(spid) = (*e).sender.take() {
        // A sender is waiting: take its message and wake it.
        let msg = (*e).sender_msg;
        (*proc_ptr(spid)).syscall_ret = 0;
        (*proc_ptr(spid)).state = State::Runnable;
        msg
    } else {
        // No sender yet: park as receiver until a message arrives.
        (*e).receiver = Some(pid);
        (*proc_ptr(pid)).state = State::BlockedRecv;
        schedule_switch();
        (*proc_ptr(pid)).syscall_ret // resumed: delivered message
    }
}

/// SVC dispatcher, called from `svc_entry` with a pointer to the saved register
/// frame (`frame[N]` == xN; frame[32] == SP_EL0; frame[33] == SPSR_EL1).
#[no_mangle]
extern "C" fn rust_svc_dispatch(frame: *mut u64) {
    // SAFETY: at EL1 with IRQs masked; single accessor of process state.
    unsafe {
        let esr: u64;
        asm!("mrs {}, esr_el1", out(reg) esr, options(nomem, nostack));
        let ec = esr >> 26;
        let pid = CURRENT;

        if ec != 0x15 {
            // Not an SVC — a genuine EL0 fault. Kill the offender, keep the OS up.
            crate::kprintln!("[mansa] EL0 fault in P{}: ESR={:#018x} - killing", pid, esr);
            (*proc_ptr(pid)).state = State::Dead;
            schedule_switch();
            return;
        }

        let num = *frame.add(8); // x8 = syscall number
        let a0 = *frame.add(0); // x0
        let a1 = *frame.add(1); // x1
        let a2 = *frame.add(2);
        let a3 = *frame.add(3);
        let a4 = *frame.add(4);
        let a5 = *frame.add(5);

        let ret = match num {
            SYS_DEBUG => {
                crate::kprintln!("    [P{} @EL0] {:#x}", pid, a0);
                0
            }
            SYS_SEND => {
                let r = sys_send(pid, a0 as usize, a1);
                if r == EPERM {
                    crate::kprintln!("    [P{} @EL0] SEND refused (no capability)", pid);
                } else {
                    crate::kprintln!("    [P{} @EL0] SEND ok ({:#x})", pid, a1);
                }
                r
            }
            SYS_RECV => {
                let r = sys_recv(pid, a0 as usize);
                if r == EPERM {
                    crate::kprintln!("    [P{} @EL0] RECV refused (no capability)", pid);
                } else {
                    crate::kprintln!("    [P{} @EL0] RECV got {:#x}", pid, r);
                }
                r
            }
            SYS_YIELD => {
                schedule_switch();
                0
            }
            SYS_WIN_CREATE => crate::tonga::win_create(pid, a0 as usize, a1 as usize) as u64,
            SYS_WIN_FILL => {
                crate::tonga::win_fill(a0 as usize, a1 as usize, a2 as usize, a3 as usize, a4 as usize, a5 as u32);
                0
            }
            SYS_WIN_PRESENT => {
                crate::tonga::win_present();
                0
            }
            SYS_GETPID => pid as u64,
            SYS_WIN_TEXT => {
                crate::tonga::win_text(a0 as usize, a1 as usize, a2 as usize, a3 as u8, a4 as u32);
                0
            }
            SYS_WIN_GETKEY => {
                // Block until a key arrives on the console UART, routed to the
                // (only, focused) app. Tight poll — we are the sole runnable task.
                loop {
                    if let Some(c) = crate::uart::getc() {
                        // In an edit session, buffer printable keys so the editor
                        // can persist the text on exit. Exclude '~' (0x7e), the
                        // quit sentinel, and cap at the buffer size.
                        if EDITING && c >= 0x20 && c < 0x7e {
                            let n = EDIT_LEN;
                            if n < 256 {
                                addr_of_mut!(EDIT_BUF).cast::<u8>().add(n).write(c);
                                EDIT_LEN = n + 1;
                            }
                        }
                        break c as u64;
                    }
                    core::hint::spin_loop();
                }
            }
            SYS_PUTC => {
                let c = a0 as u8;
                if c == b'\n' {
                    crate::kprintln!();
                } else {
                    crate::kprint!("{}", c as char);
                }
                0
            }
            SYS_TIME => {
                // PL031 RTC seconds since epoch (already in an unsafe context).
                core::ptr::read_volatile(0x0901_0000 as *const u32) as u64
            }
            SYS_AI => {
                // Pointer-arg host call: read the prompt from the CALLER's memory
                // (a0..a0+a1, an EL0 VA reachable at EL1 under the app's TTBR0),
                // run the on-device responder, write the reply back into a2.
                let (pptr, plen, optr, ocap) = (a0 as usize, a1 as usize, a2 as usize, a3 as usize);
                let mut pb = [0u8; 128];
                let pn = plen.min(128);
                // G2: both buffers must be the caller's own memory (the reply at most 256 bytes is written).
                if !user_range(pptr as u64, pn as u64, false) || !user_range(optr as u64, ocap.min(256) as u64, true) {
                    crate::kprintln!("    [P{} @EL0] AI refused: a buffer is outside the process's memory", pid);
                    *frame.add(0) = EPERM;
                    return;
                }
                for i in 0..pn {
                    pb[i] = core::ptr::read_volatile((pptr + i) as *const u8);
                }
                let mut ob = [0u8; 256];
                let on = crate::nia::respond(&pb[..pn], &mut ob);
                let n = on.min(ocap);
                for i in 0..n {
                    core::ptr::write_volatile((optr + i) as *mut u8, ob[i]);
                }
                crate::kprintln!("    [asili @EL0] \"{}\"", core::str::from_utf8(&ob[..n]).unwrap_or("?"));
                n as u64
            }
            SYS_SPEAK => {
                // Asili Voice (pointer-arg): read text from the caller's memory
                // and speak it. v0 emits text (real audio = driver + Piper, docs/14).
                let (tptr, tlen) = (a0 as usize, a1 as usize);
                let mut tb = [0u8; 160];
                let tn = tlen.min(160);
                if !user_range(tptr as u64, tn as u64, false) {
                    crate::kprintln!("    [P{} @EL0] SPEAK refused: the text is outside the process's memory", pid);
                    *frame.add(0) = EPERM;
                    return;
                }
                for i in 0..tn {
                    tb[i] = core::ptr::read_volatile((tptr + i) as *const u8);
                }
                crate::kprintln!(
                    "    [asili voice @EL0] speaking: \"{}\"",
                    core::str::from_utf8(&tb[..tn]).unwrap_or("?")
                );
                // Synthesize the text to real audio on-device (Asili Voice v0)
                // and play it through the virtio-sound device (captured by QEMU's
                // wav audiodev). Robotic formant synthesis today; neural word-level
                // TTS (Piper, docs/14) upgrades the same path later.
                if crate::virtio::sound_present() {
                    crate::voice::say(&tb[..tn]);
                }
                0
            }
            SYS_SLEEP => {
                // Block the calling EL0 process for a bounded duration. We are at
                // EL1 with IRQs masked, so we wait on the always-running physical
                // counter (not the tick-driven sleep, which needs the timer IRQ).
                // Clamp so a userspace process can't wedge the CPU for too long.
                let ms = a0.min(5000);
                let got = crate::timer::busywait_ms(ms);
                crate::kprintln!("    [P{} @EL0] slept {}ms (asked {}ms)", pid, got, ms);
                got
            }
            SYS_SGI_ARM => {
                // Still at EL1 with IRQs masked (exception entry), so the SGI is
                // only made PENDING here; it is taken on the eret back to EL0.
                crate::SGI_PROBE_SEEN.store(0, core::sync::atomic::Ordering::Release);
                crate::gic::send_sgi(cpu::core_id() as u32, crate::PROBE_SGI);
                0
            }
            SYS_SGI_READ => {
                let n = crate::SGI_PROBE_SEEN.load(core::sync::atomic::Ordering::Acquire);
                crate::kprintln!("SGI-EL0-SEEN {}", n);
                n as u64
            }
            SYS_EXIT => {
                crate::kprintln!("    [P{} @EL0] exit", pid);
                (*proc_ptr(pid)).state = State::Dead;
                schedule_switch();
                return; // never resumes
            }
            _ => EPERM,
        };
        *frame.add(0) = ret; // syscall return value → x0
    }
}

/// First-run landing for a user process: install its address space and drop to
/// EL0. Reached via `user_entry_trampoline` after the very first `cpu_switch`.
#[no_mangle]
extern "C" fn user_bootstrap(idx: usize) -> ! {
    // SAFETY: idx is a valid, freshly-spawned user process; registers below are
    // set immediately before the eret that begins EL0 execution.
    unsafe {
        let ttbr0 = (*proc_ptr(idx)).ttbr0;
        cpu::set_ttbr0(ttbr0);
        // SPSR EL0t: preemptible → IRQ unmasked (0x340 masks D/A/F, enables I);
        // cooperative → all DAIF masked (0x3C0).
        let spsr: u64 = if (*proc_ptr(idx)).preempt { 0x340 } else { 0x3C0 };
        asm!(
            "msr sp_el0, {sp}",
            "msr elr_el1, {entry}",
            "msr spsr_el1, {spsr}",
            "eret",
            sp = in(reg) USER_STACK_TOP,
            entry = in(reg) USER_ENTRY,
            spsr = in(reg) spsr,
            options(noreturn),
        );
    }
}

// ---------------------------------------------------------------------------
// Assembly: the SVC entry (save EL0 frame → dispatch → restore → eret), the
// new-process trampoline, and the two position-independent user programs.
// ---------------------------------------------------------------------------
global_asm!(
    r#"
.section .text

// SVC / synchronous-from-EL0 entry. Saves x0–x30, ELR, SPSR, SP_EL0 (272 B),
// dispatches, then restores and returns to EL0. A context switch inside the
// dispatcher parks this process here on its own kernel stack until resumed.
.global svc_entry
svc_entry:
    sub sp, sp, #272
    stp x0,  x1,  [sp, #16*0]
    stp x2,  x3,  [sp, #16*1]
    stp x4,  x5,  [sp, #16*2]
    stp x6,  x7,  [sp, #16*3]
    stp x8,  x9,  [sp, #16*4]
    stp x10, x11, [sp, #16*5]
    stp x12, x13, [sp, #16*6]
    stp x14, x15, [sp, #16*7]
    stp x16, x17, [sp, #16*8]
    stp x18, x19, [sp, #16*9]
    stp x20, x21, [sp, #16*10]
    stp x22, x23, [sp, #16*11]
    stp x24, x25, [sp, #16*12]
    stp x26, x27, [sp, #16*13]
    stp x28, x29, [sp, #16*14]
    mrs x9, elr_el1
    stp x30, x9,  [sp, #16*15]   // x30 (frame[30]) + ELR_EL1 (frame[31])
    mrs x9, sp_el0
    mrs x10, spsr_el1
    str x9,  [sp, #256]          // SP_EL0  (frame[32])
    str x10, [sp, #264]          // SPSR_EL1(frame[33])

    mov x0, sp                   // &frame
    bl  rust_svc_dispatch

    ldr x9,  [sp, #256]
    msr sp_el0, x9
    ldr x10, [sp, #264]
    msr spsr_el1, x10
    ldp x30, x9, [sp, #16*15]
    msr elr_el1, x9
    ldp x28, x29, [sp, #16*14]
    ldp x26, x27, [sp, #16*13]
    ldp x24, x25, [sp, #16*12]
    ldp x22, x23, [sp, #16*11]
    ldp x20, x21, [sp, #16*10]
    ldp x18, x19, [sp, #16*9]
    ldp x16, x17, [sp, #16*8]
    ldp x14, x15, [sp, #16*7]
    ldp x12, x13, [sp, #16*6]
    ldp x10, x11, [sp, #16*5]
    ldp x8,  x9,  [sp, #16*4]
    ldp x6,  x7,  [sp, #16*3]
    ldp x4,  x5,  [sp, #16*2]
    ldp x2,  x3,  [sp, #16*1]
    ldp x0,  x1,  [sp, #16*0]
    add sp, sp, #272
    eret

// New user process: cpu_switch restored x19 = idx; hand off to user_bootstrap.
.global user_entry_trampoline
user_entry_trampoline:
    mov x0, x19
    b   user_bootstrap

// ---- User program A: send 0x42, then exit. Position-independent. ----
.global user_a_start
.global user_a_end
user_a_start:
    mov  x8, #0            // SYS_DEBUG
    movz x0, #0xA1         // marker: "A started"
    svc  #0
    mov  x8, #1            // SYS_SEND
    mov  x0, #0            // cap handle 0 (SEND)
    movz x1, #0x42         // message
    svc  #0
    mov  x8, #0            // SYS_DEBUG: show send result (x0)
    svc  #0
    mov  x8, #3            // SYS_EXIT
    svc  #0
0:  b 0b
user_a_end:

// ---- User program B: receive, print it, then try an illegal send. ----
.global user_b_start
.global user_b_end
user_b_start:
    mov  x8, #0            // SYS_DEBUG
    movz x0, #0xB1         // marker: "B started"
    svc  #0
    mov  x8, #2            // SYS_RECV
    mov  x0, #0            // cap handle 0 (RECV)
    svc  #0               // x0 = received message (expect 0x42)
    mov  x8, #0            // SYS_DEBUG: show received value
    svc  #0
    mov  x8, #1            // SYS_SEND on a RECV-only cap → must be refused
    mov  x0, #0
    movz x1, #0x99
    svc  #0               // x0 = EPERM
    mov  x8, #0            // SYS_DEBUG: show the refusal code
    svc  #0
    mov  x8, #3            // SYS_EXIT
    svc  #0
0:  b 0b
user_b_end:

// IRQ taken from EL0 (lower EL, AArch64). Same frame as svc_entry; dispatches to
// the preemptive scheduler. A context switch here parks the interrupted process
// on its own kernel stack until it is scheduled again.
.global irq_lower_entry
irq_lower_entry:
    sub sp, sp, #272
    stp x0,  x1,  [sp, #16*0]
    stp x2,  x3,  [sp, #16*1]
    stp x4,  x5,  [sp, #16*2]
    stp x6,  x7,  [sp, #16*3]
    stp x8,  x9,  [sp, #16*4]
    stp x10, x11, [sp, #16*5]
    stp x12, x13, [sp, #16*6]
    stp x14, x15, [sp, #16*7]
    stp x16, x17, [sp, #16*8]
    stp x18, x19, [sp, #16*9]
    stp x20, x21, [sp, #16*10]
    stp x22, x23, [sp, #16*11]
    stp x24, x25, [sp, #16*12]
    stp x26, x27, [sp, #16*13]
    stp x28, x29, [sp, #16*14]
    mrs x9, elr_el1
    stp x30, x9,  [sp, #16*15]
    mrs x9, sp_el0
    mrs x10, spsr_el1
    str x9,  [sp, #256]
    str x10, [sp, #264]

    bl  rust_irq_from_el0

    ldr x9,  [sp, #256]
    msr sp_el0, x9
    ldr x10, [sp, #264]
    msr spsr_el1, x10
    ldp x30, x9, [sp, #16*15]
    msr elr_el1, x9
    ldp x28, x29, [sp, #16*14]
    ldp x26, x27, [sp, #16*13]
    ldp x24, x25, [sp, #16*12]
    ldp x22, x23, [sp, #16*11]
    ldp x20, x21, [sp, #16*10]
    ldp x18, x19, [sp, #16*9]
    ldp x16, x17, [sp, #16*8]
    ldp x14, x15, [sp, #16*7]
    ldp x12, x13, [sp, #16*6]
    ldp x10, x11, [sp, #16*5]
    ldp x8,  x9,  [sp, #16*4]
    ldp x6,  x7,  [sp, #16*3]
    ldp x4,  x5,  [sp, #16*2]
    ldp x2,  x3,  [sp, #16*1]
    ldp x0,  x1,  [sp, #16*0]
    add sp, sp, #272
    eret

// ---- CPU-bound worker: loop with a delay, print the iteration, exit after 8.
// Never blocks, so it only yields via timer preemption — interleaved P3/P4
// output proves the timer is time-slicing EL0. Position-independent.
.global user_worker_start
.global user_worker_end
user_worker_start:
    mov  x19, #0              // iteration counter (callee-saved; survives IRQ)
1:  add  x19, x19, #1
    movz x20, #0x0080, lsl #16   // ~8.4M-iteration busy delay
2:  subs x20, x20, #1
    bne  2b
    mov  x8, #0              // SYS_DEBUG — prints "[P<pid> @EL0] <count>"
    mov  x0, x19
    svc  #0
    cmp  x19, #8
    blt  1b
    mov  x8, #3             // SYS_EXIT
    svc  #0
3:  b 3b
user_worker_end:

// ---- SGI probe app (2.991): ask the kernel to self-target an SGI while we are
// at EL1 inside the syscall, eret back to EL0 (which unmasks I), spin so the
// pending SGI is taken HERE, then read the counter back. Position-independent.
.global user_sgi_start
.global user_sgi_end
user_sgi_start:
    mov  x8, #16           // SYS_SGI_ARM: clear counter + self-SGI (taken on the eret)
    svc  #0
    movz x20, #0x0020, lsl #16
1:  subs x20, x20, #1
    bne  1b
    mov  x8, #17           // SYS_SGI_READ: prints "SGI-EL0-SEEN <n>"
    svc  #0
    mov  x8, #3            // SYS_EXIT
    svc  #0
2:  b 2b
user_sgi_end:

// ---- Sleep app: block ~300 ms via the SYS_SLEEP syscall, then report the
// kernel-measured elapsed ms and exit. Proves an EL0 process can ask the kernel
// to block it for a bounded, real duration. Position-independent.
.global user_sleep_start
.global user_sleep_end
user_sleep_start:
    mov  x8, #0            // SYS_DEBUG
    movz x0, #0x51         // marker: "sleep app started"
    svc  #0
    mov  x8, #15           // SYS_SLEEP
    movz x0, #300          // ask for 300 ms
    svc  #0               // x0 = kernel-measured elapsed ms
    mov  x8, #0            // SYS_DEBUG: echo the measured elapsed ms
    svc  #0
    mov  x8, #3            // SYS_EXIT
    svc  #0
0:  b 0b
user_sleep_end:

// ---- Fault app: deliberately violate memory safety by writing to the unmapped
// VA 0. The store raises an EL0 translation fault (data abort); the kernel's
// synchronous-exception path kills this process and the OS keeps running. Proves
// hardware-enforced fault isolation. Position-independent.
.global user_fault_start
.global user_fault_end
user_fault_start:
    mov  x0, #0            // an address that is unmapped for EL0
    str  x0, [x0]          // store to it -> translation fault -> kernel kills us
0:  b 0b                   // (never reached)
user_fault_end:

// ---- G2 probe (`el0-ptr`): pass KERNEL addresses to SYS_AI (reply buffer) and SYS_SPEAK (text). The literal
// pool holds G2_CANARY's absolute address (the kernel is identity mapped), so it is copied with the program.
.global user_ptr_start
.global user_ptr_end
user_ptr_start:
    sub  sp, sp, #16
    movz w9, #0x6968       // "hi", the prompt, on our own stack
    strh w9, [sp]
    mov  x0, sp            // prompt: ours
    mov  x1, #2
    ldr  x2, =G2_CANARY    // reply buffer: the KERNEL's
    mov  x3, #16
    mov  x8, #13           // SYS_AI
    svc  #0
    mov  x8, #0            // SYS_DEBUG: the result (EPERM once G2 is fixed)
    svc  #0
    ldr  x0, =G2_CANARY    // text: the KERNEL's
    mov  x1, #16
    mov  x8, #14           // SYS_SPEAK
    svc  #0
    mov  x8, #0            // SYS_DEBUG
    svc  #0
    // Full 64-bit window arguments (WIN_FILL and WIN_TEXT take no pointer, but x + w and x + col must not wrap):
    // a window of u64::MAX by u64::MAX, then fills and glyphs at u64::MAX. The kernel must clip, not panic.
    mvn  x0, xzr           // w = u64::MAX
    mvn  x1, xzr           // h = u64::MAX
    mov  x8, #5            // SYS_WIN_CREATE
    svc  #0
    mov  x19, x0           // wid
    cmn  x0, #1            // -1: no free window, nothing below would run
    b.eq 7f
    mov  x0, x19           // FILL(wid, x = MAX, y = 0, w = 1, h = 1)
    mvn  x1, xzr
    mov  x2, #0
    mov  x3, #1
    mov  x4, #1
    mov  x5, #0xff
    mov  x8, #6
    svc  #0
    mov  x0, x19           // FILL(wid, 1, 1, w = MAX, h = MAX)
    mov  x1, #1
    mov  x2, #1
    mvn  x3, xzr
    mvn  x4, xzr
    mov  x5, #0xff
    mov  x8, #6
    svc  #0
    mov  x0, x19           // FILL(wid, MAX, MAX, MAX, MAX)
    mvn  x1, xzr
    mvn  x2, xzr
    mvn  x3, xzr
    mvn  x4, xzr
    mov  x5, #0xff
    mov  x8, #6
    svc  #0
    mov  x0, x19           // TEXT(wid, x = MAX, y = MAX, 'A')
    mvn  x1, xzr
    mvn  x2, xzr
    mov  x3, #0x41
    mov  x4, #0xff
    mov  x8, #9
    svc  #0
    mov  x0, x19           // TEXT(wid, x = MAX, y = 0, 'A')
    mvn  x1, xzr
    mov  x2, #0
    mov  x3, #0x41
    mov  x4, #0xff
    mov  x8, #9
    svc  #0
    mov  x0, x19           // TEXT(wid, x = 0, y = MAX, 'A')
    mov  x1, #0
    mvn  x2, xzr
    mov  x3, #0x41
    mov  x4, #0xff
    mov  x8, #9
    svc  #0
    movz x0, #0x5afe       // SYS_DEBUG 0x5afe: every max-argument call returned
    mov  x8, #0
    svc  #0
    b    8f
7:  movz x0, #0xbad1       // SYS_DEBUG 0xbad1: no window, the case did not run
    mov  x8, #0
    svc  #0
8:  mov  x8, #3            // SYS_EXIT
    svc  #0
0:  b 0b
.ltorg
user_ptr_end:

// ---- GUI app: get pid, create a window, draw a little UI, present, exit.
// One program, spawned 3x; each colors its surface by its pid so the windows
// differ. Uses only the window-server syscalls. Position-independent.
.global user_gui_start
.global user_gui_end
user_gui_start:
    mov  x8, #8              // SYS_GETPID
    svc  #0
    mov  x21, x0            // pid
    mov  x8, #5             // SYS_WIN_CREATE(w=200, h=100)
    movz x0, #200
    movz x1, #100
    svc  #0
    mov  x19, x0           // wid

    // background colour = 0x203040 + pid*0x081018 (varies per app)
    movz x5, #0x3040
    movk x5, #0x0020, lsl #16
    mov  x6, x21
    movz x7, #0x1018
    movk x7, #0x0008, lsl #16
    mul  x6, x6, x7
    add  x5, x5, x6
    mov  x8, #6             // FILL whole window with bg
    mov  x0, x19
    mov  x1, #0
    mov  x2, #0
    movz x3, #200
    movz x4, #100
    svc  #0

    movz x5, #0xA000        // gold header bar (0x00E0A000)
    movk x5, #0x00E0, lsl #16
    mov  x8, #6
    mov  x0, x19
    mov  x1, #0
    mov  x2, #0
    movz x3, #200
    movz x4, #18
    svc  #0

    movz x5, #0xC93F        // green block (0x0027C93F)
    movk x5, #0x0027, lsl #16
    mov  x8, #6
    mov  x0, x19
    movz x1, #14
    movz x2, #30
    movz x3, #78
    movz x4, #52
    svc  #0

    movz x5, #0x5F56        // red block (0x00FF5F56)
    movk x5, #0x00FF, lsl #16
    mov  x8, #6
    mov  x0, x19
    movz x1, #104
    movz x2, #30
    movz x3, #82
    movz x4, #52
    svc  #0

    mov  x8, #7             // SYS_WIN_PRESENT (composite)
    svc  #0
    mov  x8, #3             // SYS_EXIT
    svc  #0
4:  b 4b
user_gui_end:

// ---- Interactive text editor: create a window, then loop reading keys and
// drawing them into the surface until '~' (0x7E). Proof of input routing to a
// focused EL0 app. Position-independent.
.global user_edit_start
.global user_edit_end
user_edit_start:
    mov  x8, #5             // WIN_CREATE(w=210, h=110)
    movz x0, #210
    movz x1, #110
    svc  #0
    mov  x19, x0           // wid
    // dark body fill
    movz x5, #0x1A28
    movk x5, #0x0014, lsl #16
    mov  x8, #6
    mov  x0, x19
    mov  x1, #0
    mov  x2, #0
    movz x3, #210
    movz x4, #110
    svc  #0
    // header bar
    movz x5, #0x3A6B
    movk x5, #0x001B, lsl #16
    mov  x8, #6
    mov  x0, x19
    mov  x1, #0
    mov  x2, #0
    movz x3, #210
    movz x4, #18
    svc  #0
    mov  x8, #7             // present the empty editor
    svc  #0
    // cursor
    movz x22, #8           // cursor x
    movz x23, #28          // cursor y
5:  mov  x8, #10           // WIN_GETKEY (blocking)
    svc  #0
    mov  x24, x0           // key
    cmp  x24, #0x7E        // '~' -> quit
    beq  6f
    mov  x8, #9            // WIN_TEXT(wid, x, y, char, white)
    mov  x0, x19
    mov  x1, x22
    mov  x2, x23
    mov  x3, x24
    movz x4, #0xE6F0
    movk x4, #0x00E0, lsl #16
    svc  #0
    add  x22, x22, #8      // advance cursor
    cmp  x22, #200
    blt  7f
    movz x22, #8           // wrap to next line
    add  x23, x23, #10
7:  mov  x8, #7            // present
    svc  #0
    b    5b
6:  mov  x8, #7            // final present
    svc  #0
    mov  x8, #3            // EXIT
    svc  #0
8:  b 8b
user_edit_end:

// ---- Userspace RTC driver: read the PL031 clock through the device-MMIO window
// mapped at VA 0x8061_0000 (PA 0x0901_0000), then SEND the time to a client over
// IPC and exit. Holds a hardware capability (the MMIO map) + a SEND cap only.
.global user_rtc_start
.global user_rtc_end
user_rtc_start:
    movz x9, #0x0000            // build MMIO VA 0x8061_0000 (RTC data register)
    movk x9, #0x8061, lsl #16
    ldr  w10, [x9]             // read seconds-since-epoch from hardware
    mov  x8, #0               // DEBUG: show the raw value we read
    mov  x0, x10
    svc  #0
    mov  x8, #1               // SEND time to the client on cap 0
    mov  x0, #0
    mov  x1, x10
    svc  #0
    mov  x8, #3               // EXIT
    svc  #0
9:  b 9b
user_rtc_end:

// ---- RTC client: RECV the time from the driver and print it. Has a RECV cap
// only; no hardware access whatsoever.
.global user_rtc_client_start
.global user_rtc_client_end
user_rtc_client_start:
    mov  x8, #2               // RECV on cap 0
    mov  x0, #0
    svc  #0                   // x0 = time delivered by the driver
    mov  x8, #0               // DEBUG: print the received time
    svc  #0
    mov  x8, #3               // EXIT
    svc  #0
10: b 10b
user_rtc_client_end:
"#
);
