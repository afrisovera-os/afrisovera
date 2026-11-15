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

// --- Process memory (AFR-0001 7.6, the Dira subset; Dira M3 S5a: ARM gets S1's calls) ---------------------------
const SYS_VMO_CREATE: u64 = 32; // (size) -> a VMO handle; reserves the size against the caps
const SYS_VMO_MAP: u64 = 33; // (h, 0, len, READ|WRITE) -> its user address; takes, zeroes and maps the frames
const SYS_VMO_UNMAP: u64 = 34; // (addr, len) -> 0; gives the frames back and closes the handle
/// v1's errors (AFR-0001 7.2), returned by the v0.5 calls only; the v0 calls keep EPERM.
const E_PERM: i64 = -1;
const E_BADHANDLE: i64 = -2;
const E_INVAL: i64 = -5;
const E_NOMEM: i64 = -6;
const E_FAULT: i64 = -4;
const E_WOULDBLOCK: i64 = -7;
const E_PEERCLOSED: i64 = -8;
// The cell window (AFR-0001 7.6; Dira M3 S5b: ARM gets S2's calls). On ARM every cell call is served by the idle
// context in mainline (IRQs on, its own large stack): the call blocks the process with the operation, idle runs the
// desk call and hands the result back through syscall_ret. Nothing draws in an exception (architect's S2 condition).
const SYS_CELL_OPEN: u64 = 64; // (h, min_cols, min_rows) -> 0
const SYS_CELL_PUT: u64 = 65; // (h, col, row, ptr, len) -> cells written
const SYS_CELL_CURSOR: u64 = 66; // (h, col, row, shape) -> 0
const SYS_CELL_SIZE: u64 = 67; // (h) -> cols << 14 | rows
const SYS_CELL_EVENT: u64 = 68; // (h, timeout_ms; negative = no limit) -> an event, or 0
fn err(e: i64) -> u64 {
    e as u64
}
/// The map area has its own L1 slot (1 GiB each: a 39-bit VA, T0SZ 25). It sits above the RAM any supported board
/// identity-maps (QEMU virt: 1 GiB from 0x4000_0000; the Pi 4: up to 8 GiB from 0), as the PC keeps its map area in
/// PML4 slot 2, so a process's objects can never alias the kernel's view of RAM (architect's S5 verdict, Q2).
/// `map_area_check` fails the boot loudly if a board's RAM ever reaches it.
const MAP_SLOT: usize = 256;
pub const MAP_LO: u64 = (MAP_SLOT as u64) << 30; // 0x40_0000_0000
pub const MAP_HI: u64 = MAP_LO + (64 << 20); // 64 MiB, as on the PC
/// The caps (7.6): 32 MiB a process, 128 MiB in all; pages exist only while mapped.
const VMO_PROC_PAGES: u32 = (32 << 20) / 4096;
const VMO_ALL_PAGES: usize = (128 << 20) / 4096;
static VMO_RESERVED: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
/// Frames the map area holds now (object pages and their L2/L3 tables), for the frames check: 0 when no process
/// has an object mapped.
static VMO_FRAMES: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
const PROT_READ: u64 = 1;
const PROT_WRITE: u64 = 2;
const PROT_EXECUTE: u64 = 4;
/// An L3 page descriptor (4 KiB) and a table entry's next-level address.
const PAGE_DESC: u64 = 0b11;
const DESC_ADDR: u64 = 0x0000_FFFF_FFFF_F000;

/// Gap G2 (docs/security-model.md): may the kernel follow this user pointer for `len` bytes? Every process's
/// tables keep the kernel's DRAM mapped EL1 read/write, so an unchecked pointer would let EL0 code make the
/// kernel read or write kernel memory. Reads may come from the code or stack block (both mapped for every
/// process); writes only into the stack block (the code block is read-only at EL1 too, so a write there faults).
fn user_range(ptr: u64, len: u64, write: bool) -> bool {
    let lo = if write { USER_ENTRY + TWO_MIB as u64 } else { USER_ENTRY };
    let Some(end) = ptr.checked_add(len) else { return false };
    if ptr >= lo && end <= USER_STACK_TOP {
        return true;
    }
    // the map area (S5a): every page of the range must be mapped in the caller's own tables (all are read-write)
    if ptr >= MAP_LO && end <= MAP_HI {
        // SAFETY: single core, inside the caller's system call; CURRENT is the caller.
        let pid = unsafe { CURRENT };
        let mut pg = ptr & !0xFFF;
        while pg < end.max(ptr + 1) {
            // SAFETY: as above; only reads the caller's tables.
            match unsafe { map_entry(pid, pg, false) } {
                Some(e) if unsafe { e.read_volatile() } & 1 != 0 => {} // uaccess: a page-table entry
                _ => return false,
            }
            pg += 4096;
        }
        return true;
    }
    false
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
    kind: CapKind,
}

/// What a handle names (AFR-0001 7.6).
#[derive(Clone, Copy, PartialEq, Eq)]
enum CapKind {
    Endpoint,
    /// a memory object: its size in pages, and where it is mapped (0 = not mapped)
    Vmo { pages: u32, va: u64 },
    /// the desk's cell window, with the title the kernel gave it at spawn (never the process's own words)
    Cell { title: &'static [u8] },
}

/// A cell call waiting for idle to serve it (S5b).
#[derive(Clone, Copy)]
struct CellOp {
    call: u64, // SYS_CELL_*; 0 = none
    a: i32,
    b: i32,
    c: i32,
    len: usize,
    buf: [u8; 240],
    deadline: u64, // cell_event: counter value when it returns 0 (0 = no limit)
}
const NO_OP: CellOp = CellOp { call: 0, a: 0, b: 0, c: 0, len: 0, buf: [0; 240], deadline: 0 };

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Runnable,
    BlockedSend,
    BlockedRecv,
    /// waiting for idle to serve its cell call (S5b)
    BlockedCell,
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
    map_l2: u64,      // the L2 table of this process's map area (0: none yet)
    vmo_pages: u32,   // pages reserved by this process's memory objects (the 32 MiB cap)
    cell_op: CellOp,  // the cell call idle serves while the process is BlockedCell (S5b)
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
    caps: [Cap { valid: false, obj: 0, rights: 0, kind: CapKind::Endpoint }; N_CAPS],
    map_l2: 0,
    vmo_pages: 0,
    cell_op: NO_OP,
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
    static user_vmo_start: u8;
    static user_vmo_end: u8;
    static user_vmox_start: u8;
    static user_vmox_end: u8;
    static user_cell_start: u8;
    static user_cell_end: u8;
    static user_nocell_start: u8;
    static user_nocell_end: u8;
    static user_busy_start: u8;
    static user_busy_end: u8;
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
        State::BlockedCell => "cell",
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
                State::BlockedSend | State::BlockedRecv | State::BlockedCell => blocked += 1,
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
    map_area_check(); // S5a: the map area must lie above this board's RAM
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
        l1.add(i).write(0); // uaccess: a page-table entry
        l2.add(i).write(0); // uaccess: a page-table entry
    }

    let code_pa = addr_of_mut!(USER_CODE_MEM) as usize + idx * core::mem::size_of::<Region2M>();
    let stack_pa = addr_of_mut!(USER_STACK_MEM) as usize + idx * core::mem::size_of::<Region2M>();

    // L1: [0] devices (EL1), [1] DRAM (EL1) — so the kernel stays mapped; [2] → L2.
    l1.add(0).write(paging::device_block(0x0000_0000)); // uaccess: a page-table entry
    // F7: the kernel's own L1[1] (its DRAM; a table since the stack guards split it), so the boot-stack
    // guards stay in force while this address space is live. EL1-only, like the block it replaced.
    l1.add(1).write(paging::kernel_l1_entry(1)); // uaccess: a page-table entry
    l1.add(2).write(l2 as u64 | paging::TABLE_DESC); // uaccess: a page-table entry

    // L2 (covers VA 0x8000_0000..0xBFFF_FFFF): [0] code, [1] stack.
    l2.add(0).write(user_code_block(code_pa as u64)); // uaccess: a page-table entry; VA 0x8000_0000
    l2.add(1).write(user_stack_block(stack_pa as u64)); // uaccess: a page-table entry; VA 0x8020_0000

    // L2[3] (VA 0x8060_0000): optional device-MMIO window for a driver process.
    if mmio {
        l2.add(3).write(user_device_block(MMIO_PHYS_BASE)); // uaccess: a page-table entry; VA 0x8060_0000
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
    core::ptr::copy_nonoverlapping(src_start, dst, len); // uaccess: the program image into its code region by physical address, before it runs
    cpu::sync_icache_range(dst as usize, len);

    let ttbr0 = match build_address_space(idx, mmio) {
        Some(t) => t,
        None => return Err(BOUND_FRAMES),
    };

    let frame_base = ((kstack as usize) + KSTACK_SIZE - FRAME_SLOTS * 8) as *mut u64;
    for i in 0..FRAME_SLOTS {
        frame_base.add(i).write(0); // uaccess: the new process's kernel-stack frame
    }
    frame_base.add(1).write(user_entry_trampoline as *const () as u64); // uaccess: the new process's kernel-stack frame; x30
    frame_base.add(10).write(idx as u64); // uaccess: the new process's kernel-stack frame; x19 = process index

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
    (*proc_ptr(idx)).caps[handle] = Cap { valid: true, obj, rights, kind: CapKind::Endpoint };
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
            // G2 part 2, condition 1: LDTR/STTR protect only pages that deny EL0, so no kernel page may be reachable
            // from EL0 at all. Walk this process's own tables before it runs.
            let (leaves, outside) = el0_reach_audit((*proc_ptr(3)).ttbr0, false);
            crate::kprintln!(
                "[g2] P3 tables: {} EL0-reachable leaf entr{}, {} outside its user blocks {:#x}..{:#x}{}",
                leaves,
                if leaves == 1 { "y" } else { "ies" },
                outside,
                USER_ENTRY,
                USER_STACK_TOP,
                if outside == 0 { " (ok)" } else { " (WRONG)" }
            );
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

/// G2 part 2, condition 1: walk the stage-1 tables at `ttbr0` (39-bit VA, 4 KiB granule, the walk starts at level 1)
/// and count the leaf entries (blocks and pages) EL0 may access (AP[1], bit 6), and how many of them lie outside the
/// process's user blocks [USER_ENTRY, USER_STACK_TOP), its map area [MAP_LO, MAP_HI) (S5a) and, for a driver granted
/// one, the device window.
unsafe fn el0_reach_audit(ttbr0: u64, mmio: bool) -> (usize, usize) {
    const ADDR: u64 = 0x0000_FFFF_FFFF_F000;
    unsafe fn walk(table: u64, level: u32, va_base: u64, mmio: bool, leaves: &mut usize, outside: &mut usize) {
        let shift = 39 - 9 * level; // level 1: 30, level 2: 21, level 3: 12
        for i in 0..512u64 {
            // SAFETY: page tables are 1:1 mapped frames the kernel built (or its own static tables).
            let e = unsafe { (table as *const u64).add(i as usize).read_volatile() }; // uaccess: a page-table entry
            if e & 1 == 0 {
                continue;
            }
            let va = va_base + (i << shift);
            if level < 3 && e & 0b10 != 0 {
                unsafe { walk(e & ADDR, level + 1, va, mmio, leaves, outside) };
                continue;
            }
            if e & (1 << 6) == 0 {
                continue; // EL1 only
            }
            *leaves += 1;
            let end = va + (1u64 << shift);
            let in_user = va >= USER_ENTRY && end <= USER_STACK_TOP;
            let in_mmio = mmio && va >= USER_MMIO_VA && end <= USER_MMIO_VA + TWO_MIB as u64;
            // the map area (S5a, L1 slot 256): the process's own memory objects
            let in_map = va >= MAP_LO && end <= MAP_HI;
            if !in_user && !in_mmio && !in_map {
                *outside += 1;
                crate::kprintln!("[g2] EL0-reachable leaf OUTSIDE the user blocks: VA {:#x}, level {}, entry {:#x}", va, level, e);
            }
        }
    }
    let (mut leaves, mut outside) = (0usize, 0usize);
    // SAFETY: as above.
    unsafe { walk(ttbr0 & ADDR, 1, 0, mmio, &mut leaves, &mut outside) };
    (leaves, outside)
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
            let buf = core::slice::from_raw_parts(addr_of_mut!(EDIT_BUF) as *const u8, n); // uaccess: the kernel's own edit buffer
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
    // while a cell window is owned, its desk pass is due every 10 ms whichever process the tick interrupted (a busy
    // second process must not freeze the window: architect's S5b point 1). It runs in idle's mainline, never here
    // (S5b); the tick only hands the CPU over, and its masked stretch is measured.
    let t0 = crate::timer::cycles();
    // due after half a tick: a pass just after one tick is due again at the next (a full 10 ms test would skip a tick
    // and leave a key waiting about 20 ms plus the busy slice: the 57 ms gap of the first S5b run)
    if cell_pid().is_some() && t0.wrapping_sub(CELL_PASS_AT) >= crate::timer::hz() / 200 {
        let next = 0;
        let us = crate::timer::cycles().wrapping_sub(t0) * 1_000_000 / crate::timer::hz().max(1);
        if us > TICK_MASKED_MAX_US {
            TICK_MASKED_MAX_US = us;
        }
        CURRENT = next;
        cpu::set_ttbr0(paging::kernel_ttbr0());
        let prev_sp = addr_of_mut!((*proc_ptr(cur)).sp);
        cpu_switch(prev_sp, (*proc_ptr(next)).sp);
        return;
    }
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
            vmo_release(pid); // its objects' pages, the map area's tables and the reservation (S5a)
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
            SYS_VMO_CREATE => vmo_create(pid, a0),
            SYS_VMO_MAP => vmo_map(pid, a0, a1, a2, a3),
            SYS_VMO_UNMAP => vmo_unmap(pid, a0, a1),
            SYS_CELL_OPEN | SYS_CELL_PUT | SYS_CELL_CURSOR | SYS_CELL_SIZE | SYS_CELL_EVENT => {
                cell_call(pid, num, a0, a1, a2, a3, a4)
            }
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
                                addr_of_mut!(EDIT_BUF).cast::<u8>().add(n).write(c); // uaccess: the kernel's own edit buffer
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
                core::ptr::read_volatile(0x0901_0000 as *const u32) as u64 // uaccess: the PL031 RTC register
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
                // G2 part 2: the bytes move only through the door (kernel/src/uaccess.rs: LDTRB/STTRB, checked with
                // EL0's permissions); a fault part way refuses the whole call and the buffer is discarded
                if crate::uaccess::from_user(pptr as u64, &mut pb[..pn]).is_err() {
                    crate::kprintln!("    [P{} @EL0] AI refused: a buffer is outside the process's memory (the door faulted on the prompt)", pid);
                    *frame.add(0) = EPERM;
                    return;
                }
                let mut ob = [0u8; 256];
                let on = crate::nia::respond(&pb[..pn], &mut ob);
                let n = on.min(ocap);
                if crate::uaccess::to_user(optr as u64, &ob[..n]).is_err() {
                    crate::kprintln!("    [P{} @EL0] AI refused: a buffer is outside the process's memory (the door faulted on the reply)", pid);
                    *frame.add(0) = EPERM;
                    return;
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
                if crate::uaccess::from_user(tptr as u64, &mut tb[..tn]).is_err() {
                    crate::kprintln!("    [P{} @EL0] SPEAK refused: the text is outside the process's memory (the door faulted)", pid);
                    *frame.add(0) = EPERM;
                    return;
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
                vmo_release(pid);
                schedule_switch();
                return; // never resumes
            }
            _ => EPERM,
        };
        *frame.add(0) = ret; // syscall return value → x0
    }
}


// ------------------------------------------------------------------ the cell window (AFR-0001 7.6, Dira M3 S5b) --

/// The process holding the cell window (one owner at a time, the desk's rule), so its end closes it.
static mut CELL_PID: Option<usize> = None;

/// The window's owner, read by value (no reference to the mutable static).
fn cell_pid() -> Option<usize> {
    // SAFETY: a plain copy of a static that only idle's mainline and the calls it serves write, single core.
    unsafe { core::ptr::read_volatile(core::ptr::addr_of!(CELL_PID)) } // uaccess: the kernel's own CELL_PID
}
/// When idle last ran the desk for the owner (counter), for the 10 ms pace.
static mut CELL_PASS_AT: u64 = 0;
/// For arm-cell3 (architect's S5b points 3 and the grades): the longest masked stretch in the tick's hand-over, the
/// longest gap between two of idle's passes while a window is owned (a key waits at most that long), and idle's
/// share of the CPU while the run went on.
static mut TICK_MASKED_MAX_US: u64 = 0;
static mut PASS_GAP_MAX_US: u64 = 0;
/// The longest of idle's own visits (the desk pass and the calls it served), reported beside the gap.
static mut PASS_LEN_MAX_US: u64 = 0;
static mut IDLE_CYCLES: u64 = 0;

/// A desk owner-API result as a v1 result (AFR-0001 7.6's mapping, as on the PC).
fn desk_result(r: i32) -> u64 {
    use crate::desk::cell_abi as abi;
    match r {
        r if r >= 0 => r as u64,
        abi::ERR_ALREADY_OPEN => err(E_WOULDBLOCK),
        abi::ERR_NO_DESK | abi::ERR_NO_WINDOW | abi::ERR_BAD_WINDOW => err(E_PEERCLOSED),
        _ => err(E_INVAL),
    }
}

/// A cell call from pid: checked here (the handle, the lengths, the pointer, copied into the kernel), then served by
/// idle. The process blocks until idle has run the desk call.
unsafe fn cell_call(pid: usize, n: u64, h: u64, a1: u64, a2: u64, a3: u64, a4: u64) -> u64 {
    let p = proc_ptr(pid);
    let Some(c) = (*p).caps.get(h as usize).copied().filter(|c| c.valid) else { return err(E_BADHANDLE) };
    let CapKind::Cell { .. } = c.kind else { return err(E_PERM) };
    if n != SYS_CELL_OPEN && cell_pid() != Some(pid) {
        return err(E_PEERCLOSED);
    }
    let mut op = NO_OP;
    op.call = n;
    match n {
        SYS_CELL_OPEN => {
            if (a1 as i64) < 0 || (a2 as i64) < 0 {
                return err(E_INVAL);
            }
            op.a = a1 as i32;
            op.b = a2 as i32;
        }
        SYS_CELL_PUT => {
            if a4 % 2 != 0 || a4 > 240 || (a1 as i64) < 0 || (a2 as i64) < 0 {
                return err(E_INVAL);
            }
            if !user_range(a3, a4, false) {
                return err(E_FAULT);
            }
            // G2 part 2: the text comes through the one door (kernel/src/uaccess.rs: LDTRB, EL0's permissions)
            if crate::uaccess::from_user(a3, &mut op.buf[..a4 as usize]).is_err() {
                return err(E_FAULT);
            }
            op.a = a1 as i32;
            op.b = a2 as i32;
            op.len = a4 as usize;
        }
        SYS_CELL_CURSOR => {
            op.a = a1 as i32;
            op.b = a2 as i32;
            op.c = a3 as i32;
        }
        SYS_CELL_EVENT => {
            let t = a1 as i64;
            op.deadline = if t < 0 {
                0
            } else {
                crate::timer::cycles().wrapping_add((t.min(3_600_000) as u64 * crate::timer::hz() / 1000).max(1))
            };
        }
        _ => {}
    }
    (*p).cell_op = op;
    (*p).state = State::BlockedCell;
    schedule_switch();
    (*p).syscall_ret
}

/// Serve the cell call of a BlockedCell process (idle, mainline): true when it was answered.
unsafe fn cell_serve(pid: usize) -> bool {
    use crate::desk::cell_abi as abi;
    let p = proc_ptr(pid);
    let op = (*p).cell_op;
    let ret = match op.call {
        SYS_CELL_OPEN => {
            let CapKind::Cell { title } = (*p).caps.iter().find(|c| c.valid && matches!(c.kind, CapKind::Cell { .. })).map_or(CapKind::Endpoint, |c| c.kind) else {
                return false;
            };
            let r = crate::desk::app_cell_open(title, true, op.a, op.b);
            if r >= 0 {
                CELL_PID = Some(pid);
                CELL_PASS_AT = crate::timer::cycles();
                0
            } else {
                desk_result(r)
            }
        }
        SYS_CELL_PUT => desk_result(crate::desk::app_cell_put(abi::WIN, op.a, op.b, &op.buf[..op.len])),
        SYS_CELL_CURSOR => desk_result(crate::desk::app_cell_cursor(abi::WIN, op.a, op.b, op.c)),
        SYS_CELL_SIZE => desk_result(crate::desk::app_cell_size(abi::WIN)),
        SYS_CELL_EVENT => {
            // one desk pass (input, redraws), then the next event if there is one
            let ev = crate::desk::app_cell_event(0);
            CELL_PASS_AT = crate::timer::cycles();
            if ev != 0 {
                desk_result(ev)
            } else if op.deadline != 0 && crate::timer::cycles() >= op.deadline {
                0
            } else {
                return false; // nothing yet: the process stays blocked
            }
        }
        _ => err(E_INVAL),
    };
    (*p).syscall_ret = ret;
    (*p).cell_op = NO_OP;
    (*p).state = State::Runnable;
    true
}

/// pid ended (exit, fault, Close anyway, the end of the run): if it had the window, it closes (idle, mainline).
unsafe fn cell_ended(pid: usize) {
    if cell_pid() == Some(pid) {
        CELL_PID = None;
        crate::desk::cell_app_ended();
    }
}

/// Idle's work between process slices (S5b): the serial line to the window, the owner's desk pass, the cell calls
/// that wait, Close anyway, and the window of an owner that ended. Mainline, interrupts on.
unsafe fn cell_pump() {
    if let Some(owner) = cell_pid() {
        if (*proc_ptr(owner)).state == State::Dead {
            cell_ended(owner);
        }
    }
    if let Some(owner) = cell_pid() {
        // keys and Ctrl+\ (the close request, then "Close anyway"), even while the owner computes without a call;
        // after Close anyway an owner waiting in cell_event gets kind 4 (EV_CLOSED) from it at once (cell_serve),
        // and one that computes ends as a fault would (below)
        while let Some(b) = crate::uart::getc() {
            crate::desk::cell_serial_byte(b);
        }
        if (*proc_ptr(owner)).state != State::BlockedCell {
            crate::desk::app_tick();
            CELL_PASS_AT = crate::timer::cycles();
        }
        let st = (*proc_ptr(owner)).state;
        if crate::desk::cell_forced() && st != State::Dead && st != State::BlockedCell {
            crate::kprintln!("[P{}] stopped: Close anyway (the window's owner did not answer its close request)", owner);
            (*proc_ptr(owner)).state = State::Dead;
            vmo_release(owner);
            cell_ended(owner);
        }
    }
    for i in 1..N_PROC {
        if (*proc_ptr(i)).state == State::BlockedCell {
            cell_serve(i);
        }
    }
}

/// Run the spawned processes with idle serving the cell window between their slices, until none can run.
/// The one runner after schedule_switch (architect's S5b point 2): celltest3 and the ARM `dira` both use it.
unsafe fn run_with_cell() {
    let start = crate::timer::cycles();
    let mut last_pass = 0u64;
    IDLE_CYCLES = 0;
    PASS_GAP_MAX_US = 0;
    PASS_LEN_MAX_US = 0;
    TICK_MASKED_MAX_US = 0;
    loop {
        schedule_switch();
        let t_idle = crate::timer::cycles();
        // the gap a key can wait: from the END of idle's last visit (a key that came during it was read there) to the
        // start of this one; the visit's own length is reported apart (the desk pass, slow in a debug build)
        if cell_pid().is_some() && last_pass != 0 {
            let gap = t_idle.wrapping_sub(last_pass) * 1_000_000 / crate::timer::hz().max(1);
            if gap > PASS_GAP_MAX_US {
                PASS_GAP_MAX_US = gap;
            }
        }
        let mut live = false;
        let mut runnable = false;
        for i in 1..N_PROC {
            match (*proc_ptr(i)).state {
                State::Runnable => {
                    live = true;
                    runnable = true;
                }
                State::BlockedCell => live = true,
                _ => {}
            }
        }
        cell_pump();
        if cell_pid().is_some() {
            let len = crate::timer::cycles().wrapping_sub(t_idle) * 1_000_000 / crate::timer::hz().max(1);
            if len > PASS_LEN_MAX_US {
                PASS_LEN_MAX_US = len;
            }
            last_pass = crate::timer::cycles();
        } else {
            last_pass = 0;
        }
        if !live {
            break;
        }
        if !runnable && !(1..N_PROC).any(|i| (*proc_ptr(i)).state == State::Runnable) {
            cpu::wfi(); // nothing due: a tick or a key wakes it (point 3)
        }
        IDLE_CYCLES = IDLE_CYCLES.wrapping_add(crate::timer::cycles().wrapping_sub(t_idle));
    }
    let total = crate::timer::cycles().wrapping_sub(start).max(1);
    // copies first: the format macro would take references to the mutable statics
    let (idle, gap, len, masked) = (IDLE_CYCLES, PASS_GAP_MAX_US, PASS_LEN_MAX_US, TICK_MASKED_MAX_US);
    crate::kprintln!(
        "[cell] idle took {}% of the CPU; the longest gap between desk passes {} ms; the longest pass {} ms; the tick's longest masked hand-over {} us",
        idle * 100 / total,
        gap / 1000,
        len / 1000,
        masked
    );
    if let Some(owner) = cell_pid() {
        cell_ended(owner);
    }
}

/// `celltest3` (suite arm-cell3, the test rig only): the EL0 `cell` program with a CELL handle titled
/// "celltest3 (test)", preemptible; then a program with only an endpoint that calls cell_open (E_PERM).
pub fn run_cell_test() {
    if crate::fwcfg::find_file("opt/afrisovera/test-grants").is_none() {
        crate::kprintln!("[cell3] refused: only the test rig (fw_cfg opt/afrisovera/test-grants) may grant a cell window");
        return;
    }
    if !crate::desk::shown() {
        crate::kprintln!("[cell3] the desk is not showing (open it with: desk)");
        return;
    }
    let f0 = frame::used_frames();
    // SAFETY: single-core, shell (idle) context; the sole driver here.
    unsafe {
        let a = spawn_or_refuse(3, &user_cell_start as *const u8, &user_cell_end as *const u8, true, false);
        if a {
            if let Some(h) = (*proc_ptr(3)).caps.iter().position(|c| !c.valid) {
                (*proc_ptr(3)).caps[h] = Cap { valid: true, obj: 0, rights: 0, kind: CapKind::Cell { title: b"celltest3 (test)" } };
            }
        }
        let b = spawn_or_refuse(4, &user_nocell_start as *const u8, &user_nocell_end as *const u8, false, false);
        if b {
            (*proc_ptr(4)).caps[0] = Cap { valid: true, obj: 0, rights: RIGHT_SEND, kind: CapKind::Endpoint };
        }
        // a CPU-bound second process while the owner waits: the window must still answer keys (point 1)
        spawn_or_refuse(5, &user_busy_start as *const u8, &user_busy_end as *const u8, true, false);
        run_with_cell();
    }
    let f1 = frame::used_frames();
    crate::kprintln!(
        "[cell3] ended; frames in use {} before and {} after (ARM's spawn keeps two table frames a process; map area {})",
        f0,
        f1,
        VMO_FRAMES.load(core::sync::atomic::Ordering::Relaxed)
    );
}

// ------------------------------------------------------------- process memory (AFR-0001 7.6, Dira M3 S5a) --

/// When set (`vmotest` only), a successful vmo_map prints the G2 walk of the caller's tables.
static VMO_WALK: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// The L3 entry for `va` in process `pid`'s map area; `make` builds the L2 and L3 tables it needs (zeroed frames,
/// counted in VMO_FRAMES). `None`: no table there (or no frame to make one).
unsafe fn map_entry(pid: usize, va: u64, make: bool) -> Option<*mut u64> {
    let p = proc_ptr(pid);
    if (*p).ttbr0 == 0 || !(MAP_LO..MAP_HI).contains(&va) {
        return None;
    }
    let l1 = (*p).ttbr0 as *mut u64;
    if (*p).map_l2 == 0 {
        if !make {
            return None;
        }
        let t = frame::alloc()?;
        core::ptr::write_bytes(t as *mut u8, 0, 4096); // uaccess: a fresh table frame by its 1:1 address
        VMO_FRAMES.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        (*p).map_l2 = t as u64;
        l1.add(MAP_SLOT).write_volatile(t as u64 | paging::TABLE_DESC); // uaccess: a page-table entry
    }
    let l2 = (*p).map_l2 as *mut u64;
    let i2 = ((va - MAP_LO) >> 21) as usize;
    let e2 = l2.add(i2).read_volatile(); // uaccess: a page-table entry
    let l3 = if e2 & 1 == 0 {
        if !make {
            return None;
        }
        let t = frame::alloc()?;
        core::ptr::write_bytes(t as *mut u8, 0, 4096); // uaccess: a fresh table frame by its 1:1 address
        VMO_FRAMES.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        l2.add(i2).write_volatile(t as u64 | paging::TABLE_DESC); // uaccess: a page-table entry
        t as u64
    } else {
        e2 & DESC_ADDR
    };
    Some((l3 as *mut u64).add(((va >> 12) & 511) as usize))
}

/// A writable, never-executable EL0 page (UXN and PXN always set: an object's pages are data).
fn vmo_page_desc(pa: u64) -> u64 {
    pa | PAGE_DESC | paging::AF | paging::SH_INNER | paging::ATTR_NORMAL | AP_EL0_RW | PXN | UXN
}

/// Drop the TLB's copy of one page (no ASIDs here: every address space), then wait for it everywhere.
fn tlb_drop(va: u64) {
    // SAFETY: TLB maintenance only; the operand is the page number.
    unsafe { core::arch::asm!("dsb ishst", "tlbi vaae1is, {p}", "dsb ish", "isb", p = in(reg) va >> 12, options(nostack)) };
}

/// `vmo_create(size)`: a memory object of `size` bytes (a whole number of pages). Only the reservation is made here
/// (the process's and the total cap); the frames come at vmo_map.
fn vmo_create(pid: usize, size: u64) -> u64 {
    use core::sync::atomic::Ordering;
    if size == 0 || size % 4096 != 0 || size > (MAP_HI - MAP_LO) {
        return err(E_INVAL);
    }
    let pages = (size / 4096) as u32;
    // SAFETY: single core, in pid's own system call.
    let p = unsafe { proc_ptr(pid) };
    // SAFETY: as above.
    let (held, slot) = unsafe { ((*p).vmo_pages, (*p).caps.iter().position(|c| !c.valid)) };
    if held + pages > VMO_PROC_PAGES {
        return err(E_NOMEM);
    }
    let Some(h) = slot else { return err(E_NOMEM) };
    if VMO_RESERVED
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |r| (r + pages as usize <= VMO_ALL_PAGES).then_some(r + pages as usize))
        .is_err()
    {
        return err(E_NOMEM);
    }
    // SAFETY: as above.
    unsafe {
        (*p).vmo_pages += pages;
        (*p).caps[h] = Cap { valid: true, obj: 0, rights: 0, kind: CapKind::Vmo { pages, va: 0 } };
    }
    h as u64
}

/// The first free place in the map area for `pages`, with an unmapped guard page after every object.
unsafe fn map_area_fit(pid: usize, pages: u32) -> Option<u64> {
    let p = proc_ptr(pid);
    let len = pages as u64 * 4096 + 4096;
    let mut at = MAP_LO;
    'next: while at + len <= MAP_HI {
        for c in (*p).caps.iter() {
            if let CapKind::Vmo { pages: n, va } = c.kind {
                if c.valid && va != 0 && at < va + n as u64 * 4096 + 4096 && va < at + len {
                    at = va + n as u64 * 4096 + 4096;
                    continue 'next;
                }
            }
        }
        return Some(at);
    }
    None
}

/// `vmo_map(h, 0, len, prot)`: map the whole object, zeroed, read-write and never executable.
fn vmo_map(pid: usize, h: u64, offset: u64, len: u64, prot: u64) -> u64 {
    // SAFETY: single core, in pid's own system call; the tables are this process's own.
    unsafe {
        let p = proc_ptr(pid);
        let Some(c) = (*p).caps.get(h as usize).copied().filter(|c| c.valid) else { return err(E_BADHANDLE) };
        let CapKind::Vmo { pages, va } = c.kind else { return err(E_BADHANDLE) };
        if prot & PROT_EXECUTE != 0 {
            return err(E_PERM); // an object's pages are never executable (7.6)
        }
        if va != 0 || offset != 0 || len != pages as u64 * 4096 || prot != PROT_READ | PROT_WRITE {
            return err(E_INVAL);
        }
        let Some(at) = map_area_fit(pid, pages) else { return err(E_NOMEM) };
        for i in 0..pages as u64 {
            let ok = match (frame::alloc(), map_entry(pid, at + i * 4096, true)) {
                (Some(fr), Some(e)) => {
                    core::ptr::write_bytes(fr as *mut u8, 0, 4096); // uaccess: a fresh frame by its 1:1 address
                    VMO_FRAMES.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                    e.write_volatile(vmo_page_desc(fr as u64)); // uaccess: a page-table entry
                    true
                }
                (Some(fr), None) => {
                    frame::free(fr);
                    false
                }
                _ => false,
            };
            if !ok {
                // out of frames part-way: give back what this map took, as if it never happened
                unmap_pages(pid, at, i);
                return err(E_NOMEM);
            }
        }
        core::arch::asm!("dsb ishst", "isb", options(nostack)); // the new entries before EL0 uses them
        (*p).caps[h as usize].kind = CapKind::Vmo { pages, va: at };
        if VMO_WALK.load(core::sync::atomic::Ordering::Relaxed) {
            el0_walk(pid);
        }
        at
    }
}

/// Clear `n` pages from `at` in pid's map area: each entry, then its TLB copy, then the frame back to the pool.
unsafe fn unmap_pages(pid: usize, at: u64, n: u64) {
    for i in 0..n {
        let va = at + i * 4096;
        if let Some(e) = map_entry(pid, va, false) {
            let d = e.read_volatile(); // uaccess: a page-table entry
            if d & 1 != 0 {
                e.write_volatile(0); // uaccess: a page-table entry
                tlb_drop(va);
                frame::free((d & DESC_ADDR) as usize);
                VMO_FRAMES.fetch_sub(1, core::sync::atomic::Ordering::Relaxed);
            }
        }
    }
}

/// `vmo_unmap(addr, len)`: unmap a whole mapped object; its pages go back and its handle closes.
fn vmo_unmap(pid: usize, addr: u64, len: u64) -> u64 {
    // SAFETY: single core, in pid's own system call.
    unsafe {
        let p = proc_ptr(pid);
        let found = (*p).caps.iter().position(
            |c| matches!(c.kind, CapKind::Vmo { pages, va } if c.valid && va != 0 && va == addr && pages as u64 * 4096 == len),
        );
        let Some(h) = found else { return err(E_INVAL) };
        let CapKind::Vmo { pages, .. } = (*p).caps[h].kind else { return err(E_INVAL) };
        unmap_pages(pid, addr, pages as u64);
        (*p).caps[h] = Cap { valid: false, obj: 0, rights: 0, kind: CapKind::Endpoint };
        (*p).vmo_pages -= pages;
        VMO_RESERVED.fetch_sub(pages as usize, core::sync::atomic::Ordering::Relaxed);
        0
    }
}

/// Process `pid` ended (exit or fault): every object's pages, then the map area's L3 and L2 tables, then the
/// reservation. The L1 slot is cleared and the TLB dropped before a table frame goes back.
fn vmo_release(pid: usize) {
    // SAFETY: single core; pid is dead and no longer runs.
    unsafe {
        let p = proc_ptr(pid);
        for h in 0..N_CAPS {
            if let CapKind::Vmo { pages, va } = (*p).caps[h].kind {
                if (*p).caps[h].valid && va != 0 {
                    unmap_pages(pid, va, pages as u64);
                }
                (*p).caps[h] = Cap { valid: false, obj: 0, rights: 0, kind: CapKind::Endpoint };
            }
        }
        if (*p).map_l2 != 0 {
            let l2 = (*p).map_l2 as *mut u64;
            ((*p).ttbr0 as *mut u64).add(MAP_SLOT).write_volatile(0); // uaccess: a page-table entry
            core::arch::asm!("dsb ishst", "tlbi vmalle1is", "dsb ish", "isb", options(nostack));
            for i in 0..512 {
                let e = l2.add(i).read_volatile(); // uaccess: a page-table entry
                if e & 1 != 0 {
                    frame::free((e & DESC_ADDR) as usize);
                    VMO_FRAMES.fetch_sub(1, core::sync::atomic::Ordering::Relaxed);
                }
            }
            frame::free(l2 as usize);
            VMO_FRAMES.fetch_sub(1, core::sync::atomic::Ordering::Relaxed);
            (*p).map_l2 = 0;
        }
        VMO_RESERVED.fetch_sub((*p).vmo_pages as usize, core::sync::atomic::Ordering::Relaxed);
        (*p).vmo_pages = 0;
    }
}

/// G2 part 2's condition 1 (architect's S5 verdict): every page EL0 can reach in pid's tables lies inside its code,
/// its stack or its map area (or the device window a driver was granted). Walks L1, L2 and L3 and prints one line.
unsafe fn el0_walk(pid: usize) {
    let l1 = (*proc_ptr(pid)).ttbr0 as *const u64;
    let (mut n, mut bad) = (0u32, 0u32);
    let inside = |va: u64, size: u64| -> bool {
        let end = va + size;
        (va >= USER_ENTRY && end <= USER_STACK_TOP)
            || (va >= MAP_LO && end <= MAP_HI)
            || (va >= USER_MMIO_VA && end <= USER_MMIO_VA + TWO_MIB as u64)
    };
    let el0 = |d: u64| d & (1 << 6) != 0; // AP[1]: EL0 may access
    for i1 in 0..512u64 {
        let d1 = l1.add(i1 as usize).read_volatile(); // uaccess: a page-table entry
        if d1 & 1 == 0 {
            continue;
        }
        let va1 = i1 << 30;
        if d1 & 2 == 0 {
            if el0(d1) {
                n += 1;
                bad += (!inside(va1, 1 << 30)) as u32;
            }
            continue;
        }
        let l2 = (d1 & DESC_ADDR) as *const u64;
        for i2 in 0..512u64 {
            let d2 = l2.add(i2 as usize).read_volatile(); // uaccess: a page-table entry
            if d2 & 1 == 0 {
                continue;
            }
            let va2 = va1 + (i2 << 21);
            if d2 & 2 == 0 {
                if el0(d2) {
                    n += 1;
                    bad += (!inside(va2, 1 << 21)) as u32;
                }
                continue;
            }
            let l3 = (d2 & DESC_ADDR) as *const u64;
            for i3 in 0..512u64 {
                let d3 = l3.add(i3 as usize).read_volatile(); // uaccess: a page-table entry
                if d3 & 3 == 3 && el0(d3) {
                    n += 1;
                    bad += (!inside(va2 + (i3 << 12), 4096)) as u32;
                }
            }
        }
    }
    crate::kprintln!(
        "[vmo] walk P{}: {} EL0 entries, {} outside its code, stack and map area ({})",
        pid,
        n,
        bad,
        if bad == 0 { "ok" } else { "WRONG" }
    );
}

/// The boot check for the map area (architect's S5 verdict): a board whose RAM reaches MAP_LO would put objects in
/// the kernel's view of RAM, so the boot stops here, loudly, rather than run that way.
pub fn map_area_check() {
    let end = crate::board::resolved().ram_end();
    if end > MAP_LO {
        panic!("map area: this board's RAM ends at {:#x}, at or past the map area at {:#x} (process.rs MAP_SLOT)", end, MAP_LO);
    }
}

/// `vmotest` (suite arm-vmo): the EL0 `vmo` program, then the one that jumps into its object; then the frames
/// check (every frame the map area held is back) and the reservation.
pub fn run_vmo_test() {
    use core::sync::atomic::Ordering;
    let (f0, r0) = (frame::used_frames(), VMO_RESERVED.load(Ordering::Relaxed));
    VMO_WALK.store(true, Ordering::Relaxed);
    // SAFETY: single-core, shell (idle) context; the sole driver here.
    unsafe {
        if spawn_or_refuse(3, &user_vmo_start as *const u8, &user_vmo_end as *const u8, false, false) {
            schedule_switch();
        }
        if spawn_or_refuse(4, &user_vmox_start as *const u8, &user_vmox_end as *const u8, false, false) {
            schedule_switch();
        }
    }
    VMO_WALK.store(false, Ordering::Relaxed);
    let (f1, held, r1) = (frame::used_frames(), VMO_FRAMES.load(Ordering::Relaxed), VMO_RESERVED.load(Ordering::Relaxed));
    crate::kprintln!(
        "[vmo] test: map area frames held {}, reserved pages {} before and {} after ({}); frames in use {} before, {} after",
        held,
        r0,
        r1,
        if held == 0 && r0 == r1 { "ok" } else { "WRONG" },
        f0,
        f1
    );
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


// ---- the cell window (AFR-0001 7.6, Dira M3 S5b; `celltest3`, suite arm-cell3), handle 0 a CELL. Prints, in
// order: cell_open (0); a second open (E_WOULDBLOCK); cell_size; a put of 4 cells from its stack (4); a put from
// 0x1000 (E_FAULT); 241 bytes (E_INVAL); a put on row 100 (0, clipped); cell_cursor (0). Then every event: a close
// request (kind 3) ends it with 7; the key 's' makes it spin with no calls (for Close anyway); kind 4 or an error, 9.
.global user_cell_start
.global user_cell_end
user_cell_start:
    sub  sp, sp, #32
    mov  x0, #0
    mov  x1, #64
    mov  x2, #24
    mov  x8, #64           // cell_open
    svc  #0
    mov  x8, #0
    svc  #0
    mov  x0, #0
    mov  x1, #64
    mov  x2, #24
    mov  x8, #64           // again: the window is held
    svc  #0
    mov  x8, #0
    svc  #0
    mov  x0, #0
    mov  x8, #67           // cell_size
    svc  #0
    mov  x8, #0
    svc  #0
    movz x9, #0x0f41
    movk x9, #0x0f72, lsl #16
    movk x9, #0x0f69, lsl #32
    movk x9, #0x0f44, lsl #48 // "AriD", attribute 0x0f
    str  x9, [sp]
    mov  x0, #0
    mov  x1, #2
    mov  x2, #1
    mov  x3, sp
    mov  x4, #8
    mov  x8, #65           // cell_put, 4 cells
    svc  #0
    mov  x8, #0
    svc  #0
    mov  x0, #0
    mov  x1, #0
    mov  x2, #0
    mov  x3, #0x1000       // not ours
    mov  x4, #8
    mov  x8, #65
    svc  #0
    mov  x8, #0
    svc  #0
    mov  x0, #0
    mov  x1, #0
    mov  x2, #0
    mov  x3, sp
    mov  x4, #241          // odd and too long
    mov  x8, #65
    svc  #0
    mov  x8, #0
    svc  #0
    mov  x0, #0
    mov  x1, #0
    mov  x2, #100          // past the grid: clipped
    mov  x3, sp
    mov  x4, #8
    mov  x8, #65
    svc  #0
    mov  x8, #0
    svc  #0
    mov  x0, #0
    mov  x1, #6
    mov  x2, #1
    mov  x3, #1
    mov  x8, #66           // cell_cursor
    svc  #0
    mov  x8, #0
    svc  #0
    mov  x0, #0
    mov  x1, #300          // the deadline: 300 ms with nothing typed returns 0
    mov  x8, #68
    svc  #0
    mov  x8, #0
    svc  #0
    mov  x20, #0           // 1 after the key `w`: close requests are ignored (Close anyway then gives kind 4)
1:
    mov  x0, #0
    mvn  x1, xzr           // no limit
    mov  x8, #68           // cell_event
    svc  #0
    mov  x19, x0
    mov  x8, #0
    svc  #0
    tbnz x19, #63, 3f      // an error: 9
    lsr  x9, x19, #28
    cmp  x9, #3
    b.eq 2f
    cmp  x9, #4
    b.eq 3f
    movz x10, #0x0073
    movk x10, #0x1000, lsl #16 // a key 's'
    cmp  x19, x10
    b.eq 4f
    movz x10, #0x0077
    movk x10, #0x1000, lsl #16 // a key 'w'
    cmp  x19, x10
    b.ne 1b
    mov  x20, #1
    b    1b
2:
    cbnz x20, 1b           // ignoring close requests
    mov  x0, #7
    mov  x8, #3            // SYS_EXIT (the close request answered)
    svc  #0
3:
    mov  x0, #9
    mov  x8, #3
    svc  #0
4:
    b    4b                // spin with no calls: only Close anyway ends it
user_cell_end:

// ---- no CELL handle (handle 0 is an endpoint): cell_open is refused (E_PERM); it prints that and exits.
.global user_nocell_start
.global user_nocell_end
user_nocell_start:
    mov  x0, #0
    mov  x1, #10
    mov  x2, #5
    mov  x8, #64
    svc  #0
    mov  x8, #0
    svc  #0
    mov  x8, #3
    svc  #0
user_nocell_end:

// ---- CPU-bound and preemptible, no calls until it exits (`celltest3`): the window must keep answering meanwhile.
.global user_busy_start
.global user_busy_end
user_busy_start:
    movz x9, #0x2000, lsl #16 // about 5e8 turns
5:
    subs x9, x9, #1
    b.ne 5b
    mov  x8, #3
    svc  #0
user_busy_end:

// ---- process memory (AFR-0001 7.6, Dira M3 S5a; `vmotest`, suite arm-vmo). Prints, in order: vmo_create(1 MiB)
// (a handle); vmo_map (the address, in the map area); the words read back from its first and last 8 bytes after
// writing them; a map with EXECUTE (E_PERM); vmo_create(33 MiB) (E_NOMEM, the 32 MiB cap); a 4 KiB object mapped
// (its address) and unmapped (0); then it touches the unmapped page, which must fault (the kernel ends it).
.global user_vmo_start
.global user_vmo_end
user_vmo_start:
    mov  x0, #0x100000
    mov  x8, #32           // vmo_create(1 MiB)
    svc  #0
    mov  x19, x0
    mov  x8, #0            // SYS_DEBUG
    svc  #0
    mov  x0, x19
    mov  x1, #0
    mov  x2, #0x100000
    mov  x3, #3            // READ | WRITE
    mov  x8, #33           // vmo_map
    svc  #0
    mov  x20, x0
    mov  x8, #0
    svc  #0
    movz x9, #0xbeef
    str  x9, [x20]
    mov  x10, #0xfff8
    movk x10, #0xf, lsl #16 // 0xffff8: the last 8 bytes
    movz x11, #0xcafe
    str  x11, [x20, x10]
    ldr  x0, [x20]
    mov  x8, #0
    svc  #0
    ldr  x0, [x20, x10]
    mov  x8, #0
    svc  #0
    mov  x0, #4096
    mov  x8, #32
    svc  #0
    mov  x21, x0           // a 4 KiB object
    mov  x0, x21
    mov  x1, #0
    mov  x2, #4096
    mov  x3, #7            // READ | WRITE | EXECUTE: refused
    mov  x8, #33
    svc  #0
    mov  x8, #0
    svc  #0
    mov  x0, #0x2100000    // 33 MiB: past the cap
    mov  x8, #32
    svc  #0
    mov  x8, #0
    svc  #0
    mov  x0, x21
    mov  x1, #0
    mov  x2, #4096
    mov  x3, #3
    mov  x8, #33
    svc  #0
    mov  x22, x0
    mov  x8, #0
    svc  #0
    mov  x0, x22
    mov  x1, #4096
    mov  x8, #34           // vmo_unmap
    svc  #0
    mov  x8, #0
    svc  #0
    ldr  x0, [x22]         // the unmapped page: must fault
    mov  x8, #0
    svc  #0                // never reached
    mov  x8, #3            // SYS_EXIT
    svc  #0
user_vmo_end:

// ---- the jump into an object (`vmotest`): map 4 KiB, write a `ret` there, print its address, and branch to it.
// The page is UXN, so the fetch must fault (the kernel ends the process).
.global user_vmox_start
.global user_vmox_end
user_vmox_start:
    mov  x0, #4096
    mov  x8, #32
    svc  #0
    mov  x1, #0
    mov  x2, #4096
    mov  x3, #3
    mov  x8, #33
    svc  #0
    mov  x20, x0
    mov  x8, #0
    svc  #0
    movz w9, #0x03c0
    movk w9, #0xd65f, lsl #16 // `ret`
    str  w9, [x20]
    blr  x20               // must fault: the object is never executable
    mov  x8, #3
    svc  #0
user_vmox_end:

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
