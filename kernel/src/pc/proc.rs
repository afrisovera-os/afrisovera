//! Processes on the PC: several ring-3 programs, each in its OWN address space, that can
//! reach each other only through an IPC endpoint they hold a capability for, and that take
//! turns on the CPU, by blocking in IPC, or by the local-APIC timer slicing them.
//!
//! The x86-64 twin of the ARM kernel's process.rs (Phase 0's finale), built the simplest
//! way the PC allows: switching process is swapping the saved trap frame (and CR3) before
//! the trap returns. Each slot owns a 16 KiB kernel stack from the shared heap
//! (`heap::alloc_stack`, the ARM kernel's one door for kernel stacks), and the TSS points
//! at the running process's stack, so its traps land on memory that is its own.
//!
//! Capabilities: a process names an endpoint by a small handle; the kernel looks the handle
//! up in THAT process's table and checks the right (SEND or RECV). No handle, no right, no
//! access: there is no ambient authority. An endpoint holds one message.

use crate::cpu::{TrapFrame, USER_CS, USER_DS};
use crate::{frame, mmu, user};
use core::arch::global_asm;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub const SYS_EXIT: u64 = 0;
pub const SYS_PRINT: u64 = 1;
pub const SYS_SEND: u64 = 3; // (handle, value) -> 0, or EPERM
pub const SYS_RECV: u64 = 4; // (handle) -> the value, or EPERM
pub const SYS_PREEMPTED: u64 = 5; // -> how many times the timer has sliced the caller
pub const SYS_GETPID: u64 = 8; // -> the caller's slot (the ARM number)
pub const SYS_PUTC: u64 = 11; // (char) -> 0: one character on the console (the ARM number)
pub const SYS_TIME: u64 = 12; // -> seconds since 1970 (CMOS clock)
pub const SYS_AI: u64 = 13; // (prompt, len, out, cap) -> reply length (the on-device responder)
pub const SYS_SPEAK: u64 = 14; // (text, len) -> 0 (Asili Voice, text form as on ARM)
pub const SYS_SLEEP: u64 = 15; // (ms) -> block ~ms on the TSC, returns the ms slept (the ARM number)
// The window calls (the shared tonga window manager, as the ARM kernel's EL0 apps use it);
// numbered 20.. because this PC ABI already gives 5 to SYS_PREEMPTED.
pub const SYS_WIN_CREATE: u64 = 20; // (w, h) -> window id
pub const SYS_WIN_FILL: u64 = 21; // (wid, x, y, w, h, colour)
pub const SYS_WIN_PRESENT: u64 = 22;
pub const SYS_WIN_TEXT: u64 = 23; // (wid, x, y, char, colour)
pub const SYS_WIN_GETKEY: u64 = 24; // -> the next key typed on the console (blocks)
// Process memory, AFR-0001 7.6 "v0.5: the Dira subset" (S1 of Dira M3): v1's numbers and v1's errors.
pub const SYS_VMO_CREATE: u64 = 32; // (size) -> a VMO handle; reserves the size against the caps
pub const SYS_VMO_MAP: u64 = 33; // (h, 0, len, READ|WRITE) -> its user address; takes, zeroes and maps the frames
pub const SYS_CHANNEL_WRITE: u64 = 17; // Daraja (AFR-0001 7.7): a bridge's call channel
pub const SYS_CHANNEL_READ: u64 = 18;
/// 24 is v1's wait_one, and the PC's v0 SYS_WIN_GETKEY: for a process spawned as a bridge (the flag set at spawn) it
/// is wait_one, for every other process WIN_GETKEY as before. A bridge has no v0 window calls (20 to 23 are E_NOSYS).
pub const SYS_WAIT_ONE: u64 = 24;
pub const SYS_FPROC_START: u64 = 53;
pub const SYS_FPROC_SET_TLS: u64 = 54;
pub const SYS_FPROC_KILL: u64 = 55;
pub const SYS_FPROC_MAP: u64 = 49; // Daraja (AFR-0001 7.7): the bridge's calls on its foreign process
pub const SYS_FPROC_UNMAP: u64 = 50;
pub const SYS_FPROC_READ: u64 = 51;
pub const SYS_FPROC_WRITE: u64 = 52;
pub const SYS_FPROC_PROTECT: u64 = 56;
pub const SYS_VMO_UNMAP: u64 = 34; // (addr, len) -> 0; gives the frames back and closes the handle
// The cell window, AFR-0001 7.6 (S2): v0.5 service calls on a CELL handle, the desk's owner API as Nia's calls use it.
pub const SYS_CELL_OPEN: u64 = 64; // (h, min_cols, min_rows) -> 0
pub const SYS_CELL_PUT: u64 = 65; // (h, col, row, ptr, len) -> cells written
pub const SYS_CELL_CURSOR: u64 = 66; // (h, col, row, shape) -> 0
pub const SYS_CELL_SIZE: u64 = 67; // (h) -> cols << 14 | rows
pub const SYS_CELL_EVENT: u64 = 68; // (h, timeout_ms; negative = no limit) -> an event, or 0
/// The test rig only (fw_cfg opt/afrisovera/test-grants; EPERM otherwise): the kernel reads the caller's code page
/// raw, outside pc/uaccess.rs, for pc-userptr's AC test. SMAP must stop it even when the caller set RFLAGS.AC.
pub const SYS_TEST_RAWREAD: u64 = 127;
// the network (S3): one connection at a time, from a NET handle the kernel gave at spawn
pub const SYS_TCP_DIAL: u64 = 72; // (net h, IPv4 big-endian, port, timeout_ms) -> a CONN handle
pub const SYS_TCP_SEND: u64 = 73; // (conn h, ptr, len <= 512) -> bytes sent
pub const SYS_TCP_RECV: u64 = 74; // (conn h, ptr, cap, timeout_ms) -> bytes received; 0 = the peer closed
pub const SYS_TCP_CLOSE: u64 = 75; // (conn h, how: bit 0 = reset at once) -> 0
pub const SYS_DNS_A: u64 = 76; // (net h, name ptr, name len, timeout_ms) -> the IPv4 address, big-endian

/// v1's errors (AFR-0001 7.2), returned by the v0.5 calls only; the v0 calls keep EPERM.
pub(crate) const E_PERM: i64 = -1;
pub(crate) const E_BADHANDLE: i64 = -2;
pub(crate) const E_FAULT: i64 = -4;
pub(crate) const E_INVAL: i64 = -5;
pub(crate) const E_NOMEM: i64 = -6;
pub(crate) const E_WOULDBLOCK: i64 = -7;
pub(crate) const E_PEERCLOSED: i64 = -8;
pub(crate) const E_TOOBIG: i64 = -9;
const E_CANCELED: i64 = -10;
/// v0.5 (S4, for the architect's confirmation): no route on the local network (no address from the router, or the
/// next hop did not answer ARP), so a program can tell that from a silent server (E_PEERCLOSED).
const E_NOROUTE: i64 = -11;
pub(crate) const E_NOSYS: i64 = -3;
/// A v1 result: the value, or the error as its negative number (AFR-0001 7.2).
fn res(r: Result<u64, i64>) -> u64 {
    match r {
        Ok(v) => v,
        Err(e) => err(e),
    }
}
fn err(e: i64) -> u64 {
    e as u64
}

/// v1 rights (AFR-0001 7.4) a VMO handle carries, and the protection bits of `vmo_map`.
pub(crate) const R_READ: u8 = 1 << 0;
pub(crate) const R_WRITE: u8 = 1 << 1;
pub(crate) const R_MAP: u8 = 1 << 3;
pub(crate) const R_EXECUTE: u8 = 1 << 2;
pub(crate) const R_SIGNAL: u8 = 1 << 6;
pub(crate) const R_WAIT: u8 = 1 << 7;
const PROT_READ: u64 = 1;
const PROT_WRITE: u64 = 2;
const PROT_EXECUTE: u64 = 4;

/// The map area (AFR-0001 7.6): 64 MiB, 1 GiB above code, data and stack. First fit, with one unmapped
/// guard page after every mapping.
pub const MAP_LO: u64 = mmu::USER_BASE + 0x4000_0000;
pub const MAP_HI: u64 = mmu::USER_BASE + 0x4400_0000;
/// The caps: 32 MiB of objects per process, 128 MiB for all of them together (in pages).
const VMO_PROC_PAGES: u32 = (32 << 20) / 4096;
const VMO_ALL_PAGES: usize = (128 << 20) / 4096;
/// Pages reserved by every process's objects together (the 128 MiB cap). Frames are taken from the frame pool only
/// at `vmo_map`, never from a static.
static VMO_RESERVED: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
/// The last `vmo_map` times, for pc-perf's budget rows: (KiB, microseconds) of the last 1 MiB and 16 MiB maps.
static VMO_BENCH_1M: AtomicU64 = AtomicU64::new(0);
static VMO_BENCH_16M: AtomicU64 = AtomicU64::new(0);
pub const EPERM: u64 = u64::MAX;

/// An edit session (`edit`): while on, every printable key SYS_WIN_GETKEY hands out is also
/// kept here, so the kernel can save what was typed when the editor exits.
pub static mut EDITING: bool = false;
pub static mut EDIT_BUF: [u8; 256] = [0; 256];
pub static mut EDIT_LEN: usize = 0;

pub const RIGHT_SEND: u8 = 1;
pub const RIGHT_RECV: u8 = 2;

/// A process's personality (AFR-0001 7.7), fixed at spawn: whose system calls it makes. NATIVE is every program
/// before Daraja; its calls are the Mansa table above, by `int 0x80`. A foreign process's calls go to its bridge
/// (pc/foreign.rs) and never reach that table. 2 (Windows x86-64) and 3 (Darwin x86-64) are reserved and refused.
pub const PERS_NATIVE: u8 = 0;
pub const PERS_LINUX_X86_64: u8 = 1;

/// The process table's size, for tables kept per slot elsewhere (foreign.rs).
pub const SLOTS: usize = N;
const N: usize = 5; // ring-3 slots: five, as the ARM kernel (N_USER), so `ps` shows the same 6-slot table
/// Handles per process (AFR-0001 7.6: v0.5 widens v0's four to eight, each with an object kind).
const N_CAPS: usize = 8;
/// Endpoints a capability can name (0..N_EP); each holds one message of its own.
pub const N_EP: usize = 4;
const CODE_VA: u64 = mmu::USER_BASE;
/// Ring-3 layout: code from USER_BASE (up to CODE_PAGES pages), the data page at DATA_VA,
/// and STACK_PAGES of stack ending at STACK_TOP.
const CODE_PAGES: usize = 16; // 64 KiB: the Nia interpreter is ~20 KiB
/// A program spawned with grants (Dira) may have 128 KiB of code, up to the data page; the cap belongs to the grant,
/// not to every program (architect's S4 verdict, Q1: up to 256 KiB allowed; 128 fits below DATA_VA).
const CODE_PAGES_GRANTED: usize = 32;
/// Where `spawn_dira` maps a store file read-only for `dira file` (between the data page and the stack).
const FILE_VA: u64 = mmu::USER_BASE + 0x2_4000;
const FILE_MAX: usize = 16 * 1024;
const STACK_PAGES: usize = 16; // 64 KiB
const STACK_TOP: u64 = mmu::USER_BASE + 0x40_000;
/// Every process has a data page at the same address; what it holds is private.
pub const DATA_VA: u64 = mmu::USER_BASE + 0x20_000;

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Free,
    Runnable,
    BlockedSend,
    BlockedRecv,
    /// Daraja (AFR-0001 7.7): a foreign process made before its bridge's fproc_start
    Unstarted,
    /// Daraja: a foreign process waiting for its bridge's reply to a call
    InCall,
    /// Daraja: a bridge waiting in wait_one for its foreign process's next message
    WaitChan,
    Dead,
}

/// What a handle names (AFR-0001 7.6).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Endpoint,
    /// a memory object: its size in pages, and where it is mapped (0 = not mapped)
    Vmo { pages: u32, va: u64 },
    /// the desk's cell window, with the title the kernel gave it at spawn (never the process's own words)
    Cell { title: &'static [u8] },
    /// Daraja (AFR-0001 7.7): control of the foreign process in `slot`; only its bridge ever holds it
    Fproc { slot: usize },
    /// Daraja: the call channel of the foreign process in `slot` (the kernel is the other end)
    Chan { slot: usize },
    /// the network, with the address classes it may reach (bit 0 public, 1 private, 2 link-local, 3 127/8)
    Net { classes: u8 },
    /// a TCP connection made by `tcp_dial`; `id` tells it from a later one after it was reset
    Conn { id: u32 },
}

/// A handle only kernel code makes, given at spawn (AFR-0001 7.6's spawn table). Nothing a process does adds to it.
#[derive(Clone, Copy)]
pub enum Grant {
    /// the cell window, titled `title`: "Dira (built-in)" for the `dira` command, "celltest3 (test)" for the test rig
    Cell { title: &'static [u8] },
    /// Daraja (AFR-0001 7.7): control of the foreign process in `slot` (READ, WRITE, MAP, EXECUTE, SIGNAL). Only
    /// spawn_pair gives it, and it makes the receiver a bridge for its whole life.
    Fproc { slot: usize },
    /// Daraja: the call channel of the foreign process in `slot` (READ, WRITE, WAIT)
    Chan { slot: usize },
    /// Daraja: the bundle, read-only in the receiver's map area: the 48-byte header (AFR-0001 7.7: magic DARAJA02,
    /// the archive's length, 16 fresh random bytes for AT_RANDOM, the mapped page count, 8 reserved bytes), then
    /// `bytes` (a cpio "newc" archive)
    Bundle { bytes: &'static [u8] },
    /// the network, reaching the classes in `classes` (the test rig alone may add bit 3)
    Net { classes: u8 },
}

/// The bundle's header (AFR-0001 7.7, the architect's 48-byte layout of 15 Nov 2026):
/// 0..8 magic DARAJA02; 8..16 the archive's length (little-endian, from the bytes copied here, never from the archive);
/// 16..32 AT_RANDOM; 32..40 the mapped page count (from the frames mapped here); 40..48 reserved 0.
const BUNDLE_HDR: usize = 48;

/// The process that has the cell window open (one owner at a time, the desk's rule), so its end closes it.
static mut CELL_PID: Option<usize> = None;
/// The longest desk pass `preempt` ran with interrupts masked (us), for pc-cell3 (the architect's measure).
static TICK_MAX_US: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
struct Cap {
    valid: bool,
    kind: Kind,
    endpoint: usize,
    rights: u8,
}
const NO_CAP: Cap = Cap { valid: false, kind: Kind::Endpoint, endpoint: 0, rights: 0 };

#[derive(Clone, Copy)]
struct Proc {
    state: State,
    root: u64,
    frame: TrapFrame,
    caps: [Cap; N_CAPS],
    pending: u64, // a BlockedSend process's message
    wait_ep: usize, // the endpoint a BlockedSend/BlockedRecv process waits on
    slices: u64,  // timer interrupts taken while running
    vmo_pages: u32, // pages reserved by this process's memory objects (the 32 MiB cap)
    block: bool,    // the kernel wrote a start block in its data page (Dira, S4): stop records go there too
    personality: u8, // PERS_NATIVE, or a foreign personality (AFR-0001 7.7)
    fs_base: u64,    // a foreign process's FS and GS bases, saved and restored by the kernel on a switch
    gs_base: u64,
    /// Daraja: this process is a bridge, from its spawn to its end (set once, by the grants; the architect's term 1)
    bridge: bool,
}

const ZERO_FRAME: TrapFrame = TrapFrame {
    r15: 0, r14: 0, r13: 0, r12: 0, r11: 0, r10: 0, r9: 0, r8: 0, rdi: 0, rsi: 0, rbp: 0,
    rbx: 0, rdx: 0, rcx: 0, rax: 0, vector: 0, error: 0, rip: 0, cs: 0, rflags: 0, rsp: 0, ss: 0,
};
const FREE: Proc = Proc {
    state: State::Free, root: 0, frame: ZERO_FRAME, caps: [NO_CAP; N_CAPS], pending: 0, wait_ep: 0, slices: 0, vmo_pages: 0,
    personality: PERS_NATIVE, fs_base: 0, gs_base: 0, bridge: false, block: false,
};

/// One endpoint's single message slot.
#[derive(Clone, Copy)]
struct Endpoint {
    full: bool,
    value: u64,
}

static mut PROCS: [Proc; N] = [FREE; N];
/// The kernel stack of the process in each slot (its base; 0 = none). Every spawn takes a
/// new 16 KiB stack from the heap, as the ARM kernel's `process::spawn` does, so `stacks
/// served` counts spawns on both kernels. Since 13 Nov 2026 the PC gives each stack back
/// when its slot is cleared (`release_kstack`: `run_all`'s end and every failed spawn), so a
/// finished program leaves nothing behind. The ARM kernel does not yet; the performance
/// plan's S2 brings it to the same rule.
static mut KSTACKS: [usize; N] = [0; N];

/// A new kernel stack for `slot`; a refusal is named on the console, as on ARM.
fn kstack_top(slot: usize) -> Option<u64> {
    // Every path clears a slot's entry before the slot is used again; a path that forgot would
    // leak the old stack here, so the suites (debug builds) stop on it.
    // SAFETY: single core; only touched from the kernel with interrupts masked.
    debug_assert_eq!(unsafe { (*core::ptr::addr_of!(KSTACKS))[slot] }, 0, "kernel stack of slot {} not released", slot);
    match crate::heap::alloc_stack() {
        Ok(p) => {
            // SAFETY: single core; only touched from the kernel with interrupts masked.
            unsafe { (*core::ptr::addr_of_mut!(KSTACKS))[slot] = p as usize };
            Some((p as usize + crate::heap::STACK_SIZE) as u64)
        }
        Err(why) => {
            crate::kprintln!("[proc] P{} NOT spawned: no kernel stack ({})", slot, why);
            None
        }
    }
}

/// Give `slot`'s kernel stack back to the heap (performance plan, 13 Nov 2026: every spawn took a fresh 16 KiB
/// stack and none was ever returned, so each finished program left 16 KiB behind for good; the perf suites
/// measured it as `mem.apps_rerun_growth_kib`). Only for a slot whose program can no longer run on it.
fn release_kstack(slot: usize) {
    // SAFETY: single core, interrupts masked (as kstack_top); the slot's program is not running.
    let base = unsafe { core::mem::replace(&mut (*core::ptr::addr_of_mut!(KSTACKS))[slot], 0) };
    if base != 0 {
        if let Ok(layout) = core::alloc::Layout::from_size_align(crate::heap::STACK_SIZE, crate::heap::STACK_ALIGN) {
            // SAFETY: `base` came from heap::alloc_stack, which allocates exactly this layout from the global
            // allocator, and it is freed once (the slot's entry was cleared above).
            unsafe { alloc::alloc::dealloc(base as *mut u8, layout) };
        }
    }
}

/// Point ring 3's next trap at `slot`'s own kernel stack.
fn use_kstack(slot: usize) {
    // SAFETY: as kstack_top.
    let base = unsafe { (*core::ptr::addr_of!(KSTACKS))[slot] };
    crate::cpu::set_rsp0(if base == 0 { 0 } else { (base + crate::heap::STACK_SIZE) as u64 });
}
static mut ENDPOINTS: [Endpoint; N_EP] = [Endpoint { full: false, value: 0 }; N_EP];
static mut CURRENT: usize = 0;
static ACTIVE: AtomicBool = AtomicBool::new(false);
/// Timer interrupts that moved the CPU from one process to a different one.
pub static TIMER_SWITCHES: AtomicU64 = AtomicU64::new(0);
pub static REFUSALS: AtomicU64 = AtomicU64::new(0);
pub static FAULTS: AtomicU64 = AtomicU64::new(0);

pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

fn procs() -> &'static mut [Proc; N] {
    // SAFETY: single core; only touched from the kernel with interrupts masked (boot code
    // before `run_all`, trap handlers during it).
    unsafe { &mut *core::ptr::addr_of_mut!(PROCS) }
}

/// Load a program into a fresh address space: its code (read-only), a stack and a data
/// page, all private. `preemptible` lets the timer interrupt it in ring 3.
pub fn spawn(code: &[u8], caps: &[(usize, u8)], preemptible: bool) -> Option<usize> {
    spawn_with(code, caps, &[], preemptible)
}

/// `spawn`, plus handles only the kernel makes (`Grant`), placed after the endpoint capabilities. Only kernel code
/// builds the list: the `dira` command, and the test rig behind fw_cfg `opt/afrisovera/test-grants`.
pub fn spawn_with(code: &[u8], caps: &[(usize, u8)], grants: &[Grant], preemptible: bool) -> Option<usize> {
    spawn_as(code, caps, grants, preemptible, PERS_NATIVE, 0, 0, 0x202)
}

/// What the kernel hands Dira at spawn besides its handles (S4): the start block's mode and argument, and a store
/// file to map read-only.
pub struct Start<'a> {
    pub mode: u8,
    pub arg: &'a [u8],
    pub file: Option<&'a [u8]>,
}

/// The start block at the start of the data page (userland/dira/src/main.rs, `Block`, reads it).
const BLOCK_MAGIC: u32 = 0x4E50_5344; // "DSPN"
const BLOCK_STOP_TSC: u64 = DATA_VA + 24;
const BLOCK_STOP_KIND: u64 = DATA_VA + 32;
const STOP_CLOSE: u32 = 1;
const STOP_CTRLC: u32 = 2;

/// Fill the start block in the data frame `d` (1:1 mapped, not yet visible to ring 3).
fn write_block(d: usize, st: &Start, cell_h: u8, net_h: u8, file_len: u32) {
    let mut b = [0u8; 64 + 512];
    b[0..4].copy_from_slice(&BLOCK_MAGIC.to_le_bytes());
    b[4] = st.mode;
    b[5] = cell_h;
    b[6] = net_h;
    let n = st.arg.len().min(512);
    b[8..12].copy_from_slice(&(n as u32).to_le_bytes());
    b[12..16].copy_from_slice(&file_len.to_le_bytes());
    b[16..24].copy_from_slice(&crate::timer::hz().to_le_bytes());
    b[64..64 + n].copy_from_slice(&st.arg[..n]);
    // SAFETY: a fresh, zeroed data frame of 4 KiB, 1:1 mapped; the block is 576 bytes.
    unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), d as *mut u8, b.len()) }; // uaccess: a fresh frame by its 1:1 address
}


/// `spawn_with` with a personality (AFR-0001 7.7). A foreign process is given no capabilities and no grants at all
/// (a request for either is refused, nothing made), starts with `fs`/`gs` as its FS and GS bases, and with TF from
/// `rflags` if set (every other flag is the kernel's: IF as `preemptible` says). A reserved or unknown personality is
/// refused.
pub fn spawn_as(
    code: &[u8],
    caps: &[(usize, u8)],
    grants: &[Grant],
    preemptible: bool,
    personality: u8,
    fs: u64,
    gs: u64,
    rflags: u64,
) -> Option<usize> {
    spawn_full(code, caps, grants, preemptible, personality, fs, gs, rflags, None)
}

/// `spawn_as`, plus the start block only kernel code makes for Dira (S4): native only, refused before anything is
/// made for a foreign personality, like caps and grants.
#[allow(clippy::too_many_arguments)]
fn spawn_full(
    code: &[u8],
    caps: &[(usize, u8)],
    grants: &[Grant],
    preemptible: bool,
    personality: u8,
    fs: u64,
    gs: u64,
    rflags: u64,
    start: Option<&Start>,
) -> Option<usize> {
    if personality != PERS_NATIVE && personality != PERS_LINUX_X86_64 {
        crate::kprintln!("[daraja] NOT spawned: personality {} is reserved or unknown (AFR-0001 7.7)", personality);
        return None;
    }
    if personality != PERS_NATIVE && (!caps.is_empty() || !grants.is_empty()) {
        crate::kprintln!(
            "[daraja] NOT spawned: a foreign process holds no handles (AFR-0001 7.7), {} asked",
            caps.len() + grants.len()
        );
        return None;
    }
    if personality != PERS_NATIVE && start.is_some() {
        crate::kprintln!("[daraja] NOT spawned: a foreign process is given no start block (AFR-0001 7.7)");
        return None;
    }
    let slot = procs().iter().position(|p| p.state == State::Free)?;
    kstack_top(slot)?;
    let Some(root) = mmu::new_space() else {
        release_kstack(slot);
        return None;
    };
    // code: as many read-only pages as the program needs (filled with int3 past its end)
    let cap = if grants.is_empty() { CODE_PAGES } else { CODE_PAGES_GRANTED };
    let pages = code.len().div_ceil(4096).clamp(1, cap);
    let too_long = start.is_some_and(|st| st.arg.len() > 512 || st.file.is_some_and(|f| f.len() > FILE_MAX));
    if code.len() > cap * 4096 || too_long {
        crate::kprintln!(
            "[proc] P{} NOT spawned: program is {} bytes, more than {} KiB (or its start data is too long)",
            slot,
            code.len(),
            cap * 4
        );
        mmu::free_space(root);
        release_kstack(slot);
        return None;
    }
    let mut ok = true;
    for i in 0..pages {
        let Some(c) = frame::alloc() else { ok = false; break };
        let part = &code[(i * 4096).min(code.len())..((i + 1) * 4096).min(code.len())];
        // SAFETY: a fresh frame, 1:1 mapped, not yet visible to any ring-3 program.
        unsafe {
            core::ptr::write_bytes(c as *mut u8, 0xCC, 4096); // uaccess: a fresh frame by its 1:1 address
            core::ptr::copy_nonoverlapping(part.as_ptr(), c as *mut u8, part.len()); // uaccess: a fresh frame by its 1:1 address
        }
        ok &= mmu::map_user_in(root, CODE_VA + (i * 4096) as u64, c as u64, false);
    }
    for i in 0..STACK_PAGES {
        match frame::alloc() {
            Some(st) => ok &= mmu::map_user_in(root, STACK_TOP - ((i + 1) * 4096) as u64, st as u64, true),
            None => ok = false,
        }
    }
    let mut data = 0;
    match frame::alloc() {
        Some(d) => {
            data = d;
            ok &= mmu::map_user_in(root, DATA_VA, d as u64, true);
        }
        None => ok = false,
    }
    // `dira file`: the store file's bytes, read-only and never executable, at FILE_VA
    if let Some(f) = start.and_then(|st| st.file) {
        for i in 0..f.len().div_ceil(4096) {
            let Some(fr) = frame::alloc() else { ok = false; break };
            let part = &f[i * 4096..((i + 1) * 4096).min(f.len())];
            // SAFETY: a fresh frame (zeroed by frame::alloc), 1:1 mapped, not yet visible to ring 3.
            unsafe { core::ptr::copy_nonoverlapping(part.as_ptr(), fr as *mut u8, part.len()) }; // uaccess: a fresh frame by its 1:1 address
            ok &= mmu::map_user_ro_nx_in(root, FILE_VA + (i * 4096) as u64, fr as u64);
        }
    }
    if !ok {
        // a frame or table refused: give back what this space already took
        mmu::free_space(root);
        release_kstack(slot);
        return None;
    }
    let p = &mut procs()[slot];
    *p = FREE;
    p.root = root;
    for (h, &(endpoint, rights)) in caps.iter().enumerate().take(N_CAPS) {
        // a capability for an endpoint that does not exist is left empty (its use is refused)
        if endpoint < N_EP {
            p.caps[h] = Cap { valid: true, kind: Kind::Endpoint, endpoint, rights };
        }
    }
    let mut granted = true;
    for g in grants {
        if let Some(h) = p.caps.iter().position(|c| !c.valid) {
            p.caps[h] = match *g {
                Grant::Cell { title } => {
                    Cap { valid: true, kind: Kind::Cell { title }, endpoint: 0, rights: R_READ | R_WRITE | R_WAIT }
                }
                Grant::Fproc { slot } => {
                    p.bridge = true; // fixed here, at spawn, for the process's whole life (the architect's term 1)
                    let r = R_READ | R_WRITE | R_MAP | R_EXECUTE | R_SIGNAL;
                    Cap { valid: true, kind: Kind::Fproc { slot }, endpoint: 0, rights: r }
                }
                Grant::Chan { slot } => {
                    p.bridge = true;
                    Cap { valid: true, kind: Kind::Chan { slot }, endpoint: 0, rights: R_READ | R_WRITE | R_WAIT }
                }
                Grant::Bundle { bytes } => match map_bundle(root, bytes) {
                    Some(pages) => {
                        p.vmo_pages += pages;
                        Cap { valid: true, kind: Kind::Vmo { pages, va: MAP_LO }, endpoint: 0, rights: R_READ | R_MAP }
                    }
                    None => {
                        granted = false;
                        NO_CAP
                    }
                },
                Grant::Net { classes } => {
                    Cap { valid: true, kind: Kind::Net { classes }, endpoint: 0, rights: R_READ | R_WRITE | R_WAIT }
                }
            };
        } else {
            granted = false;
        }
    }
    if !granted {
        crate::kprintln!("[proc] NOT spawned: a grant could not be given (reason above), nothing kept");
        // a grant could not be given (no handle slot, no frames, no secret random bytes): nothing half made
        let pages = p.vmo_pages;
        VMO_RESERVED.fetch_sub(pages as usize, Ordering::Relaxed);
        *p = FREE;
        mmu::free_space(root);
        release_kstack(slot);
        return None;
    }
    if let Some(st) = start {
        // the handles Dira holds, by their place in its table (the grants fill the first free slots, in order)
        let cell_h = p.caps.iter().position(|c| c.valid && matches!(c.kind, Kind::Cell { .. })).map_or(0xFF, |h| h as u8);
        let net_h = p.caps.iter().position(|c| c.valid && matches!(c.kind, Kind::Net { .. })).map_or(0xFF, |h| h as u8);
        let file_len = st.file.map_or(u32::MAX, |f| f.len() as u32);
        write_block(data, st, cell_h, net_h, file_len);
        p.block = true;
    }
    p.frame = ZERO_FRAME;
    p.frame.rip = CODE_VA;
    p.frame.cs = USER_CS;
    // only TF may come from the caller (the selftest's single-step case); IOPL, NT, AC or VM here would reach ring 3
    // through iretq (IOPL 3 would open every I/O port), so the function masks them, not each caller
    p.frame.rflags = (rflags & 0x100) | if preemptible { 0x202 } else { 0x2 };
    p.personality = personality;
    p.fs_base = fs;
    p.gs_base = gs;
    p.frame.rsp = STACK_TOP;
    p.frame.ss = USER_DS;
    p.state = State::Runnable;
    Some(slot)
}

/// Map the bundle read-only (and never executable) at the start of the map area of the new space `root`: the 48-byte
/// header (the archive's length, 16 fresh secret random bytes for AT_RANDOM never reused across spawns, the page
/// count), then `bytes`. The pages are
/// reserved against the caps like any object. Returns the page count, or None (nothing left mapped or reserved).
fn map_bundle(root: u64, bytes: &[u8]) -> Option<u32> {
    let total = BUNDLE_HDR.checked_add(bytes.len())?;
    let pages = u32::try_from(total.div_ceil(4096)).ok()?;
    if pages > VMO_PROC_PAGES || !mmu::nx() {
        crate::kprintln!("[daraja] bundle NOT given: {} pages, or no NX on this CPU", pages);
        return None;
    }
    if VMO_RESERVED.load(Ordering::Relaxed) + pages as usize > VMO_ALL_PAGES {
        crate::kprintln!("[daraja] bundle NOT given: the total memory cap is reached");
        return None;
    }
    let mut hdr = [0u8; BUNDLE_HDR];
    hdr[0..8].copy_from_slice(b"DARAJA02");
    hdr[8..16].copy_from_slice(&(bytes.len() as u64).to_le_bytes()); // the bytes copied below, never the archive's word
    hdr[32..40].copy_from_slice(&(pages as u64).to_le_bytes()); // the frames mapped below (all or none: None on failure)
    if crate::rng::fill_secret(&mut hdr[16..32]).is_err() {
        // fail closed: no counted entropy on this machine (no RDSEED or RDRAND, no TPM), so no AT_RANDOM, no bridge
        crate::kprintln!("[daraja] bundle NOT given: no secret random bytes on this machine (fail closed)");
        return None;
    }
    for i in 0..pages as usize {
        let ok = match frame::alloc() {
            Some(fr) => {
                // SAFETY: a fresh frame, 1:1 mapped, not yet in any ring-3 space.
                unsafe {
                    let dst = fr as *mut u8;
                    core::ptr::write_bytes(dst, 0, 4096); // uaccess: a fresh frame by its 1:1 address
                    for k in 0..4096usize {
                        let at = i * 4096 + k;
                        let b = if at < BUNDLE_HDR {
                            hdr[at]
                        } else if at - BUNDLE_HDR < bytes.len() {
                            bytes[at - BUNDLE_HDR]
                        } else {
                            break;
                        };
                        *dst.add(k) = b;
                    }
                }
                mmu::map_user_prot_in(root, MAP_LO + (i * 4096) as u64, fr as u64, false, false) || {
                    frame::free(fr);
                    false
                }
            }
            None => false,
        };
        if !ok {
            for j in 0..i {
                if let Some(fr) = mmu::unmap_user_in(root, MAP_LO + (j * 4096) as u64) {
                    frame::free(fr as usize);
                }
            }
            return None;
        }
    }
    VMO_RESERVED.fetch_add(pages as usize, Ordering::Relaxed);
    Some(pages)
}

/// Daraja (AFR-0001 7.7): the foreign process in a new, empty address space (no code, no stack, not runnable until
/// its bridge calls fproc_start), then its bridge `bridge_code`, a native program given exactly FPROC, CHAN and the
/// bundle (handles 0, 1 and 2). Returns (bridge slot, foreign slot), or None with nothing left behind.
pub fn spawn_pair(bridge_code: &[u8], personality: u8, bundle: &'static [u8]) -> Option<(usize, usize)> {
    if personality != PERS_LINUX_X86_64 || !mmu::nx() {
        crate::kprintln!("[daraja] pair NOT spawned: personality {} or no NX on this CPU", personality);
        return None;
    }
    let fs = procs().iter().position(|p| p.state == State::Free)?;
    kstack_top(fs)?;
    let Some(froot) = mmu::new_space() else {
        release_kstack(fs);
        return None;
    };
    {
        let p = &mut procs()[fs];
        *p = FREE;
        p.root = froot;
        p.personality = personality;
        p.state = State::Unstarted; // claims the slot, so the bridge's spawn takes another
    }
    let grants = [Grant::Fproc { slot: fs }, Grant::Chan { slot: fs }, Grant::Bundle { bytes: bundle }];
    let Some(b) = spawn_with(bridge_code, &[], &grants, true) else {
        procs()[fs] = FREE;
        mmu::free_space(froot);
        release_kstack(fs);
        return None;
    };
    crate::foreign::pair_begin(b, fs);
    crate::kprintln!("[daraja] P{} personality {} with its bridge P{}", fs, personality, b);
    Some((b, fs))
}

/// Run every spawned process until all have exited (or none can run). Returns the number
/// that ended by exit; a fault or a deadlock is reported in the log.
pub fn run_all() -> u64 {
    let Some(first) = next_runnable(N - 1) else { return 0 };
    // The way back from ring 3 (user::back_to_kernel, from a trap) leaves interrupts masked, and the console reads
    // COM1 only through its interrupt: the caller's state is put back at the end, or the shell stops hearing its
    // keys (pc-cell3 on the cloud runner, 14 Nov: celltest3 returned to a deaf shell). Callers that mask and unmask around
    // run_all themselves (compat/process.rs) are unchanged by this.
    let was_on = crate::cpu::interrupts_on();
    crate::cpu::cli();
    // SAFETY: single core, interrupts masked; the scheduler state is ours until ring 3 runs.
    unsafe { CURRENT = first };
    ACTIVE.store(true, Ordering::Relaxed);
    mmu::switch_to(procs()[first].root);
    use_kstack(first);
    tls_load(first);
    let start = procs()[first].frame;
    let rc = user::enter_frame(&start);
    tls_clear();
    crate::cpu::set_rsp0(0);
    ACTIVE.store(false, Ordering::Relaxed);
    mmu::switch_to(mmu::root());
    // a process that still had the window when the run ended (a deadlock, a refusal): it closes now
    // SAFETY: single core, interrupts masked.
    if let Some(pid) = unsafe { CELL_PID } {
        cell_ended(pid);
    }
    // and one that still had the connection: it is reset
    // SAFETY: as above.
    if let Some((pid, _)) = unsafe { CONN_LIVE } {
        conn_ended(pid);
    }
    for (slot, p) in procs().iter_mut().enumerate() {
        if p.state != State::Free {
            // a reaped foreign slot (Daraja, proc::reap) has root 0: free_space returns at once and its vmo_pages is
            // already 0, so nothing is freed or subtracted twice. Keep free_space's root == 0 guard for this.
            mmu::free_space(p.root); // its pages, mapped objects included, and page tables back to the frame pool
            VMO_RESERVED.fetch_sub(p.vmo_pages as usize, Ordering::Relaxed); // and its objects' reservation
        }
        *p = FREE;
        release_kstack(slot); // and its kernel stack back to the heap
        crate::foreign::forget(slot); // Daraja: its foreign-memory table and the pair
    }
    if was_on {
        crate::cpu::sti();
    }
    rc
}

// ------------------------------------------------------ process memory (AFR-0001 7.6, S1) --

/// Pages counted on the foreign process whose bridge is `pid` (0 if `pid` is not a bridge).
fn partner_pages(pid: usize) -> u32 {
    crate::foreign::partner(pid).map_or(0, |s| procs()[s].vmo_pages)
}

/// Daraja: the foreign slot behind handle `h` of process `pid`, if that handle is an FPROC with all of `need`, and the
/// foreign process is still there.
pub(crate) fn fproc_of(pid: usize, h: u64, need: u8) -> Result<usize, i64> {
    let c = procs()[pid].caps.get(h as usize).copied().filter(|c| c.valid).ok_or(E_BADHANDLE)?;
    let Kind::Fproc { slot } = c.kind else { return Err(E_BADHANDLE) };
    if c.rights & need != need {
        return Err(E_PERM);
    }
    let f = &procs()[slot];
    if matches!(f.state, State::Free | State::Dead) || f.personality == PERS_NATIVE {
        return Err(E_PEERCLOSED);
    }
    Ok(slot)
}

/// Daraja: the call-channel slot behind handle `h` of `pid`, with all of `need`.
pub(crate) fn chan_of(pid: usize, h: u64, need: u8) -> Result<usize, i64> {
    let c = procs()[pid].caps.get(h as usize).copied().filter(|c| c.valid).ok_or(E_BADHANDLE)?;
    let Kind::Chan { slot } = c.kind else { return Err(E_BADHANDLE) };
    if c.rights & need != need {
        return Err(E_PERM);
    }
    Ok(slot)
}

/// Write `v` at the start of `slot`'s data page (DATA_VA), before the process runs (`daraja bench` gives each test
/// program its count this way). Through the kernel's 1:1 map of the page's frame; nothing if the page is not there.
pub(crate) fn poke_data_u32(slot: usize, v: u32) {
    if let Some(fr) = mmu::user_frame_in(procs()[slot].root, DATA_VA) {
        // SAFETY: the frame of `slot`'s own data page, 1:1 mapped; the process has not run yet, nothing else holds it.
        unsafe { core::ptr::write_unaligned(fr as *mut u32, v) }; // uaccess: the process's own data frame by its 1:1 address, before it runs (poke_data_u32)
    }
}

/// The page-table root of `slot`.
pub(crate) fn root_of(slot: usize) -> u64 {
    procs()[slot].root
}

/// Count `pages` of foreign memory on the foreign process `foreign` (AFR-0001 7.7): its bridge's objects plus its own
/// memory stay within one process's cap, and every process's together within the total. Nothing is counted on refusal.
pub(crate) fn charge_foreign(foreign: usize, bridge: usize, pages: u32) -> Result<(), i64> {
    let (fp, bp) = (procs()[foreign].vmo_pages, procs()[bridge].vmo_pages);
    if fp as u64 + bp as u64 + pages as u64 > VMO_PROC_PAGES as u64 {
        return Err(E_NOMEM);
    }
    // single core, interrupts masked in the trap handler: the check and the add cannot interleave
    if VMO_RESERVED.load(Ordering::Relaxed) + pages as usize > VMO_ALL_PAGES {
        return Err(E_NOMEM);
    }
    VMO_RESERVED.fetch_add(pages as usize, Ordering::Relaxed);
    procs()[foreign].vmo_pages += pages;
    Ok(())
}

/// Give back what charge_foreign counted (an unmap, or a map that failed half way).
pub(crate) fn uncharge(slot: usize, pages: u32) {
    let p = &mut procs()[slot];
    let n = pages.min(p.vmo_pages);
    p.vmo_pages -= n;
    VMO_RESERVED.fetch_sub(n as usize, Ordering::Relaxed);
}

/// [ptr, ptr+len) is inside `pid`'s own mapped ring-3 pages; `pid` must be the running process (its space loaded).
pub(crate) fn user_buffer_ok(pid: usize, ptr: u64, len: u64) -> bool {
    // SAFETY: single core, inside the caller's system call.
    pid == unsafe { CURRENT } && user_range(ptr, len)
}

/// `vmo_create(size)`: a memory object of `size` bytes (a whole number of pages). Only the reservation is made here
/// (the per-process and the total cap); the frames come at `vmo_map`.
fn vmo_create(pid: usize, size: u64) -> u64 {
    if size == 0 || size % 4096 != 0 || size > (VMO_PROC_PAGES as u64) * 4096 {
        return err(if size != 0 && size % 4096 == 0 { E_NOMEM } else { E_INVAL });
    }
    let pages = (size / 4096) as u32;
    let p = &mut procs()[pid];
    // the pair's cap (AFR-0001 7.7): a bridge's objects and its foreign process's memory share one 32 MiB
    if p.vmo_pages + pages + partner_pages(pid) > VMO_PROC_PAGES {
        return err(E_NOMEM);
    }
    let Some(h) = p.caps.iter().position(|c| !c.valid) else { return err(E_NOMEM) };
    // single core, interrupts masked in the trap handler: the check and the add cannot interleave
    if VMO_RESERVED.load(Ordering::Relaxed) + pages as usize > VMO_ALL_PAGES {
        return err(E_NOMEM);
    }
    VMO_RESERVED.fetch_add(pages as usize, Ordering::Relaxed);
    p.vmo_pages += pages;
    p.caps[h] = Cap { valid: true, kind: Kind::Vmo { pages, va: 0 }, endpoint: 0, rights: R_READ | R_WRITE | R_MAP };
    h as u64
}

/// The lowest address in the map area where `pages` fit, with a guard page after them, clear of every other
/// mapping of this process and its guard page.
fn map_area_fit(p: &Proc, pages: u32) -> Option<u64> {
    let len = pages as u64 * 4096 + 4096;
    let taken = |a: u64| {
        p.caps.iter().any(|c| match c.kind {
            Kind::Vmo { pages: n, va } if c.valid && va != 0 => a < va + n as u64 * 4096 + 4096 && va < a + len,
            _ => false,
        })
    };
    let mut starts = [MAP_LO; N_CAPS + 1];
    let mut k = 1;
    for c in p.caps.iter() {
        if let Kind::Vmo { pages: n, va } = c.kind {
            if c.valid && va != 0 {
                starts[k] = va + n as u64 * 4096 + 4096;
                k += 1;
            }
        }
    }
    let mut best: Option<u64> = None;
    for &a in &starts[..k] {
        if a + len <= MAP_HI && !taken(a) && best.map_or(true, |b| a < b) {
            best = Some(a);
        }
    }
    best
}

/// `vmo_map(h, offset, len, prot)`: map the whole object, read and write, never executable. Its frames are taken
/// from the frame pool zeroed (`frame::alloc`); if the pool runs dry, the ones taken go back and nothing is mapped.
fn vmo_map(pid: usize, h: u64, offset: u64, len: u64, prot: u64) -> u64 {
    let p = &mut procs()[pid];
    let Some(c) = p.caps.get(h as usize).copied().filter(|c| c.valid) else { return err(E_BADHANDLE) };
    let Kind::Vmo { pages, va } = c.kind else { return err(E_BADHANDLE) };
    if c.rights & R_MAP == 0 || prot & PROT_EXECUTE != 0 {
        return err(E_PERM);
    }
    if va != 0 || offset != 0 || len != pages as u64 * 4096 || prot != PROT_READ | PROT_WRITE {
        return err(E_INVAL);
    }
    if !mmu::nx() {
        // AFR-0001 7.6 promises pages that are never executable; without NX that cannot be kept, so no object is
        // mapped at all (every x86-64 CPU made since about 2004 has NX; the boot line says when one does not).
        // E_PERM, not E_NOMEM: no amount of freed memory helps, so a program must not retry (architect, 14 Nov)
        return err(E_PERM);
    }
    let Some(at) = map_area_fit(p, pages) else { return err(E_NOMEM) };
    let t0 = crate::timer::cycles();
    for i in 0..pages as u64 {
        let ok = match frame::alloc() {
            Some(fr) => mmu::map_user_in(p.root, at + i * 4096, fr as u64, true) || {
                frame::free(fr);
                false
            },
            None => false,
        };
        if !ok {
            for j in 0..i {
                if let Some(fr) = mmu::unmap_user_in(p.root, at + j * 4096) {
                    frame::free(fr as usize);
                }
            }
            return err(E_NOMEM);
        }
    }
    let us = crate::timer::cycles().saturating_sub(t0) * 1_000_000 / crate::timer::hz().max(1);
    let kib = pages as u64 * 4;
    if kib == 1024 {
        VMO_BENCH_1M.store(us, Ordering::Relaxed);
    } else if kib == 16 * 1024 {
        VMO_BENCH_16M.store(us, Ordering::Relaxed);
    }
    crate::kprintln!("[vmo] P{}: {} KiB mapped at {:#x}, zeroed, not executable ({} us)", pid, kib, at, us);
    p.caps[h as usize].kind = Kind::Vmo { pages, va: at };
    at
}

/// `vmo_unmap(addr, len)`: the whole mapping of one object; its frames go back to the pool and its handle closes.
fn vmo_unmap(pid: usize, addr: u64, len: u64) -> u64 {
    let p = &mut procs()[pid];
    let Some(h) = p.caps.iter().position(|c| {
        matches!(c.kind, Kind::Vmo { pages, va } if c.valid && va != 0 && va == addr && pages as u64 * 4096 == len)
    }) else {
        return err(E_INVAL);
    };
    let Kind::Vmo { pages, .. } = p.caps[h].kind else { return err(E_INVAL) };
    for i in 0..pages as u64 {
        if let Some(fr) = mmu::unmap_user_in(p.root, addr + i * 4096) {
            frame::free(fr as usize);
        }
    }
    p.caps[h] = NO_CAP;
    p.vmo_pages -= pages;
    VMO_RESERVED.fetch_sub(pages as usize, Ordering::Relaxed);
    0
}

// ------------------------------------------------------- the cell window (AFR-0001 7.6, S2) --

// ----------------------------------------------- the desk's own stack for the cell calls (S2) --

/// The desk's drawing is deep (glyphs, the flush, the plan step), and a timer interrupt can nest on top of it while
/// `cell_event` waits with interrupts open. A process's kernel stack is 16 KiB (heap::STACK_SIZE), so every desk
/// call made for a process runs here instead, as Nia's calls run on the boot stack (pc-cell3 on a cloud runner, 14 Nov: the
/// window drew, then the run hung with nothing more on the serial line).
const DESK_STACK: usize = 64 * 1024;
#[repr(C, align(16))]
struct DeskStack([u8; DESK_STACK]);
static mut DESK_STACK_MEM: DeskStack = DeskStack([0; DESK_STACK]);
/// The bottom bytes of the desk stack keep this pattern unless the stack overflowed into them.
const DESK_CANARY: u64 = 0xDE5C_57AC_CA9A_A1E5;
static mut DESK_ON: bool = false;
static mut DESK_JOB: Option<*mut dyn FnMut()> = None;

extern "C" fn desk_trampoline() {
    // SAFETY: set by `on_desk` just before the switch, to a closure that lives on the caller's stack until it returns.
    unsafe {
        if let Some(j) = DESK_JOB {
            (*j)();
        }
    }
}

/// Run `f` on the desk's stack (directly if already there). Single core; interrupts may be open inside `f` (cell_event)
/// and a nested interrupt uses this stack too, which is why it is large.
fn on_desk<R>(f: impl FnOnce() -> R) -> R {
    // SAFETY: single core; DESK_ON and DESK_JOB are touched only here.
    if unsafe { DESK_ON } {
        return f();
    }
    let mut f = Some(f);
    let mut r = None;
    let mut job = || {
        if let Some(g) = f.take() {
            r = Some(g());
        }
    };
    let p: *mut (dyn FnMut() + '_) = &mut job;
    // SAFETY: the pointer is used only inside the switch below, while `job` is alive on this stack.
    let p: *mut (dyn FnMut() + 'static) = unsafe { core::mem::transmute(p) };
    // SAFETY: the stack is ours alone (DESK_ON); r12 keeps the old stack pointer across the call (callee-saved).
    unsafe {
        let base = core::ptr::addr_of_mut!(DESK_STACK_MEM) as *mut u64;
        if *base != DESK_CANARY {
            core::ptr::write_volatile(base, DESK_CANARY); // uaccess: the kernel's own stack canary
        }
        DESK_ON = true;
        DESK_JOB = Some(p);
        let top = base as u64 + DESK_STACK as u64;
        core::arch::asm!(
            "mov r12, rsp",
            "mov rsp, {top}",
            "call {f}",
            "mov rsp, r12",
            top = in(reg) top,
            f = in(reg) desk_trampoline as extern "C" fn(),
            out("r12") _,
            clobber_abi("C"),
        );
        DESK_JOB = None;
        DESK_ON = false;
        if core::ptr::read_volatile(base) != DESK_CANARY { // uaccess: the kernel's own stack canary
            crate::kprintln!("[proc] the desk stack overflowed its {} KiB", DESK_STACK / 1024);
        }
    }
    r.expect("the desk job ran")
}

/// The CELL handle `h` of process `pid`, with its title; `opened` asks that this process holds the window now.
fn cell_handle(pid: usize, h: u64, opened: bool) -> Result<&'static [u8], i64> {
    let c = procs()[pid].caps.get(h as usize).copied().filter(|c| c.valid).ok_or(E_BADHANDLE)?;
    let Kind::Cell { title } = c.kind else { return Err(E_PERM) };
    // SAFETY: single core, trap handler.
    if opened && unsafe { CELL_PID } != Some(pid) {
        return Err(E_PEERCLOSED);
    }
    Ok(title)
}

/// A desk owner-API result as a v1 result (AFR-0001 7.6's mapping).
fn desk_result(r: i32) -> u64 {
    use crate::desk::cell_abi as abi;
    match r {
        r if r >= 0 => r as u64,
        abi::ERR_ALREADY_OPEN => err(E_WOULDBLOCK),
        abi::ERR_NO_DESK | abi::ERR_NO_WINDOW | abi::ERR_BAD_WINDOW => err(E_PEERCLOSED),
        _ => err(E_INVAL),
    }
}

fn cell_open(pid: usize, h: u64, min_cols: u64, min_rows: u64) -> u64 {
    let title = match cell_handle(pid, h, false) {
        Ok(t) => t,
        Err(e) => return err(e),
    };
    if (min_cols as i64) < 0 || (min_rows as i64) < 0 {
        return err(E_INVAL);
    }
    let r = on_desk(|| crate::desk::app_cell_open(title, true, min_cols as i32, min_rows as i32));
    if r >= 0 {
        // SAFETY: single core, trap handler.
        unsafe { CELL_PID = Some(pid) };
        return 0;
    }
    desk_result(r)
}

fn cell_put(pid: usize, h: u64, col: u64, row: u64, ptr: u64, len: u64) -> u64 {
    if let Err(e) = cell_handle(pid, h, true) {
        return err(e);
    }
    if len % 2 != 0 || len > 240 || (col as i64) < 0 || (row as i64) < 0 {
        return err(E_INVAL);
    }
    if !user_range(ptr, len) {
        return err(E_FAULT);
    }
    let mut buf = [0u8; 240];
    // G2 part 3: the one door for user memory (pc/uaccess.rs), all or nothing
    if let Err(e) = crate::uaccess::from_user(ptr, &mut buf[..len as usize]) {
        return err(e);
    }
    let cells = &buf[..len as usize];
    desk_result(on_desk(|| crate::desk::app_cell_put(crate::desk::cell_abi::WIN, col as i32, row as i32, cells)))
}

/// Wait for an event with interrupts open (as WIN_GETKEY), so the serial line and the timer go on; the desk runs while
/// it waits. A timer tick here interrupted the kernel, so `preempt` leaves it alone.
fn cell_event(pid: usize, h: u64, timeout_ms: i64) -> u64 {
    if let Err(e) = cell_handle(pid, h, true) {
        return err(e);
    }
    let t = if timeout_ms < 0 { -1 } else { timeout_ms.min(i32::MAX as i64) as i32 };
    crate::cpu::sti();
    let r = on_desk(|| crate::desk::app_cell_event(t));
    crate::cpu::cli();
    desk_result(r)
}

/// Process `pid` ended (exit, fault, Close anyway, the end of the run): if it had the window, it closes.
fn cell_ended(pid: usize) {
    // SAFETY: single core, interrupts masked.
    unsafe {
        if CELL_PID == Some(pid) {
            CELL_PID = None;
            on_desk(crate::desk::cell_app_ended);
        }
    }
}

// ------------------------------------------------------------ the network (AFR-0001 7.6, S3) --

/// The open connection's owner and id (one in all in v0.5; virtio.rs keeps the connection itself).
static mut CONN_LIVE: Option<(usize, u32)> = None;
static mut CONN_NEXT: u32 = 1;
/// The network wait in progress: the caller, its deadline (counter cycles; 0 = none), when the desk was last looked
/// at, whether a close request stopped it, and when that was seen.
static mut WAIT_PID: usize = 0;
static mut WAIT_UNTIL: u64 = 0;
static mut WAIT_LOOK: u64 = 0;
static mut WAIT_CANCELED: bool = false;
static mut WAIT_SEEN_AT: u64 = 0;
/// Which stop ended the wait: STOP_CLOSE (the window's close request) or STOP_CTRLC (Ctrl+C on the console).
static mut WAIT_KIND: u32 = 0;
/// The kernel's copy of the bytes a send or recv moves (a static, not the 16 KiB kernel stack, which the desk pass in
/// `net_stop` also uses; one call at a time, single core).
static mut NET_BUF: [u8; crate::virtio::CONN_SEG] = [0; crate::virtio::CONN_SEG];

/// An address's class (7.6): bit 0 public, 1 private networks, 2 link-local, 3 this machine (127/8). `None` for 0/8,
/// 224/3 and 255.255.255.255, which no handle reaches.
fn ip_class(ip: [u8; 4]) -> Option<u8> {
    match ip {
        [0, ..] | [224..=255, ..] => None,
        [127, ..] => Some(3),
        [10, ..] | [172, 16..=31, ..] | [192, 168, ..] => Some(1),
        [169, 254, ..] => Some(2),
        _ => Some(0),
    }
}
const CLASS_NAMES: [&str; 4] = ["public", "private network", "link-local", "this machine"];

/// The NET handle `h` of process `pid`: its classes.
fn net_handle(pid: usize, h: u64) -> Result<u8, i64> {
    let c = procs()[pid].caps.get(h as usize).copied().filter(|c| c.valid).ok_or(E_BADHANDLE)?;
    let Kind::Net { classes } = c.kind else { return Err(E_PERM) };
    Ok(classes)
}

/// The CONN handle `h` of process `pid`: whether its connection is still open (a reset leaves the handle, dead).
fn conn_handle(pid: usize, h: u64) -> Result<bool, i64> {
    let c = procs()[pid].caps.get(h as usize).copied().filter(|c| c.valid).ok_or(E_BADHANDLE)?;
    let Kind::Conn { id } = c.kind else { return Err(E_BADHANDLE) };
    // SAFETY: single core, trap handler.
    Ok(unsafe { CONN_LIVE } == Some((pid, id)))
}

/// `pid` holds the window and it has a close request waiting (or is gone): a network call returns E_CANCELED.
fn close_pending(pid: usize) -> bool {
    // SAFETY: single core, trap handler.
    (unsafe { CELL_PID }) == Some(pid) && crate::desk::cell_close_pending()
}

/// virtio.rs dropped the connection (a reset either way): the handle goes dead.
fn conn_sync() {
    if !crate::virtio::conn_open() {
        // SAFETY: single core, trap handler.
        unsafe { CONN_LIVE = None };
    }
}

/// The stop hook while a network call waits (virtio.rs asks it every 256 spins): the call's deadline, or, looked at
/// every 10 ms, a close request on the caller's window. While it looks, the serial line's bytes go to the desk and the
/// desk runs, as in `cell_event`'s wait (keys stay queued for `cell_event`).
fn net_stop() -> bool {
    // SAFETY: single core; the statics are set by `net_wait` around this call's wait only.
    unsafe {
        if WAIT_CANCELED {
            return true;
        }
        let now = crate::timer::cycles();
        if WAIT_UNTIL != 0 && now >= WAIT_UNTIL {
            return true;
        }
        if now.wrapping_sub(WAIT_LOOK) < crate::timer::hz().max(1) / 100 {
            return false;
        }
        WAIT_LOOK = now;
        if CELL_PID != Some(WAIT_PID) {
            // a process with no window: Ctrl+C on the console (the serial line the person types on, the same source
            // as the shell's own Ctrl+C) stops its wait, as a close request does a window's (architect's S4, Q3)
            while let Some(c) = crate::uart::getc() {
                if c == 3 {
                    WAIT_CANCELED = true;
                    WAIT_KIND = STOP_CTRLC;
                    WAIT_SEEN_AT = crate::timer::cycles();
                }
            }
            return WAIT_CANCELED;
        }
        on_desk(|| {
            while let Some(b) = crate::uart::getc() {
                crate::desk::cell_serial_byte(b);
            }
            crate::desk::app_tick();
        });
        if crate::desk::cell_close_pending() {
            WAIT_CANCELED = true;
            WAIT_KIND = STOP_CLOSE;
            WAIT_SEEN_AT = crate::timer::cycles();
        }
        WAIT_CANCELED
    }
}

/// Run `f` (a network wait) for process `pid` with interrupts open, as `cell_event` waits, under the stop hook;
/// `timeout_ms` negative = no limit. Returns its result and whether a close request stopped it.
fn net_wait<T>(pid: usize, timeout_ms: i64, f: impl FnOnce() -> T) -> (T, bool) {
    let hz = crate::timer::hz().max(1);
    // SAFETY: single core, inside the caller's system call; nothing else uses these.
    unsafe {
        WAIT_PID = pid;
        WAIT_CANCELED = false;
        WAIT_KIND = 0;
        WAIT_LOOK = 0;
        WAIT_UNTIL = if timeout_ms < 0 {
            0
        } else {
            crate::timer::cycles().saturating_add((timeout_ms.min(3_600_000) as u64 * hz / 1000).max(1))
        };
    }
    crate::virtio::set_net_abort(Some(net_stop));
    crate::cpu::sti();
    let r = f();
    crate::cpu::cli();
    crate::virtio::set_net_abort(None);
    // SAFETY: as above.
    (r, unsafe { WAIT_CANCELED })
}

/// A wait stopped by a close request: the connection is reset at once (7.6), with the line pc-net3 grades.
fn net_canceled(pid: usize, call: &str) -> u64 {
    // SAFETY: single core, trap handler.
    // SAFETY: single core, trap handler; a stop found before the wait began (close_pending) is seen now.
    let (seen, kind) = unsafe {
        if WAIT_KIND == 0 {
            WAIT_KIND = STOP_CLOSE;
            WAIT_SEEN_AT = crate::timer::cycles();
        }
        (WAIT_SEEN_AT, WAIT_KIND)
    };
    let ms = crate::timer::cycles().saturating_sub(seen) * 1000 / crate::timer::hz().max(1);
    let what = if kind == STOP_CTRLC { "Ctrl+C on the console" } else { "the close request" };
    crate::kprintln!("[net] P{} {}: E_CANCELED {} ms after {} was seen", pid, call, ms, what);
    if procs()[pid].block {
        // the process reads when the kernel saw the stop, and which (userland/dira: "N ms after it was asked")
        // inside pid's own system call its space is the current one and its data page is always mapped; the bytes
        // still go through the one door (G2 part 3: a raw write here would halt the PC under SMAP)
        let _ = crate::uaccess::to_user(BLOCK_STOP_TSC, &seen.to_le_bytes());
        let _ = crate::uaccess::to_user(BLOCK_STOP_KIND, &kind.to_le_bytes());
    }
    crate::virtio::conn_reset("a close request");
    conn_sync();
    err(E_CANCELED)
}

fn tcp_dial(pid: usize, h: u64, ip: u64, port: u64, timeout_ms: i64) -> u64 {
    let classes = match net_handle(pid, h) {
        Ok(c) => c,
        Err(e) => return err(e),
    };
    if ip > u32::MAX as u64 || port == 0 || port > 0xFFFF {
        return err(E_INVAL);
    }
    let a = (ip as u32).to_be_bytes();
    let class = ip_class(a);
    let allowed = class.is_some_and(|b| classes & (1 << b) != 0);
    crate::kprintln!(
        "[net] P{} dial {}.{}.{}.{}:{} ({}){}",
        pid, a[0], a[1], a[2], a[3], port,
        class.map_or("never allowed", |b| CLASS_NAMES[b as usize]),
        if allowed { "" } else { ": refused" }
    );
    if !allowed {
        return err(E_PERM);
    }
    // SAFETY: single core, trap handler.
    if unsafe { CONN_LIVE }.is_some() || crate::virtio::conn_open() {
        return err(E_WOULDBLOCK);
    }
    let Some(slot) = procs()[pid].caps.iter().position(|c| !c.valid) else { return err(E_NOMEM) };
    if close_pending(pid) {
        return err(E_CANCELED);
    }
    let (r, canceled) = net_wait(pid, timeout_ms, || crate::virtio::conn_dial(a, port as u16));
    use crate::virtio::ConnErr;
    match r {
        Ok(()) => {
            // SAFETY: single core, trap handler.
            let id = unsafe {
                let id = CONN_NEXT;
                CONN_NEXT = CONN_NEXT.wrapping_add(1);
                CONN_LIVE = Some((pid, id));
                id
            };
            procs()[pid].caps[slot] = Cap { valid: true, kind: Kind::Conn { id }, endpoint: 0, rights: R_READ | R_WRITE | R_WAIT };
            slot as u64
        }
        Err(_) if canceled => net_canceled(pid, "dial"),
        Err(ConnErr::Busy) => err(E_WOULDBLOCK),
        Err(ConnErr::NoRoute) => err(E_NOROUTE),
        Err(_) => err(E_PEERCLOSED),
    }
}

fn tcp_send(pid: usize, h: u64, ptr: u64, len: u64) -> u64 {
    let live = match conn_handle(pid, h) {
        Ok(l) => l,
        Err(e) => return err(e),
    };
    if len == 0 || len > crate::virtio::TCP_MAX_PAYLOAD as u64 {
        return err(E_INVAL);
    }
    if !user_range(ptr, len) {
        return err(E_FAULT);
    }
    if !live {
        return err(E_PEERCLOSED);
    }
    if close_pending(pid) {
        return net_canceled(pid, "send");
    }
    let n = len as usize;
    // SAFETY: single core; NET_BUF is this call's alone.
    let buf = unsafe { &mut *core::ptr::addr_of_mut!(NET_BUF) };
    // G2 part 3: the bytes come through the one door, all or nothing
    if let Err(e) = crate::uaccess::from_user(ptr, &mut buf[..n]) {
        return err(e);
    }
    let (r, canceled) = net_wait(pid, -1, || crate::virtio::conn_send(&buf[..n]));
    conn_sync();
    match r {
        Ok(k) => k as u64,
        Err(_) if canceled => net_canceled(pid, "send"),
        Err(crate::virtio::ConnErr::TooBig) => err(E_TOOBIG),
        Err(_) => err(E_PEERCLOSED),
    }
}

fn tcp_recv(pid: usize, h: u64, ptr: u64, cap: u64, timeout_ms: i64) -> u64 {
    let live = match conn_handle(pid, h) {
        Ok(l) => l,
        Err(e) => return err(e),
    };
    if cap == 0 {
        return err(E_INVAL);
    }
    let n = cap.min(crate::virtio::CONN_SEG as u64);
    // the whole buffer must be the caller's and writable for ring 3 before the wait (its code pages are inside
    // user_range but read-only: a kernel write there would fault in ring 0)
    if !user_range(ptr, n) || !crate::uaccess::user_pages_ok(ptr, n, true) {
        return err(E_FAULT);
    }
    if !live {
        return err(E_PEERCLOSED);
    }
    if close_pending(pid) {
        return net_canceled(pid, "recv");
    }
    // SAFETY: single core; NET_BUF is this call's alone.
    let buf = unsafe { &mut *core::ptr::addr_of_mut!(NET_BUF) };
    let (r, canceled) = net_wait(pid, timeout_ms, || crate::virtio::conn_recv(&mut buf[..n as usize]));
    conn_sync();
    use crate::virtio::ConnErr;
    match r {
        Ok(k) => {
            // checked writable before the wait (only this process changes its own mappings, and it is in this call);
            // the door walks it again and copies inside SMAP's window
            match crate::uaccess::to_user(ptr, &buf[..k]) {
                Ok(()) => k as u64,
                Err(e) => err(e),
            }
        }
        Err(ConnErr::Stopped) if canceled => net_canceled(pid, "recv"),
        Err(ConnErr::Stopped) => err(E_WOULDBLOCK),
        Err(ConnErr::TooBig) => {
            crate::kprintln!("[net] P{} recv: past 4 MiB on this connection (E_TOOBIG, reset)", pid);
            err(E_TOOBIG)
        }
        Err(_) => err(E_PEERCLOSED),
    }
}

/// `tcp_close(h, how)`: FIN and a short wait (at most 200 ms, then a reset); with `how` bit 0, a reset at once (a load
/// the person stopped must not wait on a server that says nothing; S4, the close box stopping Dira).
fn tcp_close(pid: usize, h: u64, how: u64) -> u64 {
    let live = match conn_handle(pid, h) {
        Ok(l) => l,
        Err(e) => return err(e),
    };
    if live {
        if how & 1 != 0 {
            crate::virtio::conn_reset("closed at once by its process");
        } else {
            crate::virtio::conn_close();
        }
        conn_sync();
    }
    procs()[pid].caps[h as usize] = NO_CAP;
    0
}

fn dns_a(pid: usize, h: u64, ptr: u64, len: u64, timeout_ms: i64) -> u64 {
    if let Err(e) = net_handle(pid, h) {
        return err(e);
    }
    if len == 0 || len > 253 {
        return err(E_INVAL);
    }
    if !user_range(ptr, len) {
        return err(E_FAULT);
    }
    let mut nb = [0u8; 253];
    if let Err(e) = crate::uaccess::from_user(ptr, &mut nb[..len as usize]) {
        return err(e);
    }
    let Ok(name) = core::str::from_utf8(&nb[..len as usize]) else { return err(E_INVAL) };
    let mut q = [0u8; 260];
    if crate::dnscheck::question(name, &mut q).is_none() {
        return err(E_INVAL);
    }
    // the name server is reached through its own route, and there is one route at a time
    // SAFETY: single core, trap handler.
    if unsafe { CONN_LIVE }.is_some() || crate::virtio::conn_open() {
        return err(E_WOULDBLOCK);
    }
    if close_pending(pid) {
        return err(E_CANCELED);
    }
    let (r, canceled) = net_wait(pid, timeout_ms, || {
        if !crate::virtio::lease_stoppable() {
            return Err(E_NOROUTE);
        }
        let _route = match crate::virtio::route_open(crate::virtio::dhcp_dns(), "[net]", "name server") {
            Ok(r) => r,
            Err(crate::virtio::RouteErr::Busy) => return Err(E_WOULDBLOCK),
            Err(_) => return Err(E_NOROUTE),
        };
        crate::virtio::dns_resolve_routed(name).ok_or(E_PEERCLOSED)
    });
    match r {
        Ok(a) => u32::from_be_bytes(a) as u64,
        Err(_) if canceled => net_canceled(pid, "dns_a"),
        Err(e) => err(e),
    }
}

/// Dira (S4, pc/dira_proc.rs): the program with a CELL handle titled "Dira (built-in)" in the window mode, and a NET
/// handle for classes 0 to 2 (3 too under the test rig's flag), preemptible (so Close anyway can stop it), with its
/// start block and, for `dira file`, the store file.
pub fn spawn_dira(code: &[u8], window: bool, arg: &[u8], file: Option<&[u8]>) -> Option<usize> {
    let mut classes = 0b0111;
    let rig = crate::qemu::fw_cfg_file(b"opt/afrisovera/test-grants");
    if rig {
        classes |= 0b1000;
    }
    let both = [Grant::Cell { title: b"Dira (built-in)" }, Grant::Net { classes }];
    let grants: &[Grant] = if window { &both } else { &both[1..] };
    // mode: bit 0 text, bit 1 the test rig (Dira's two test addresses work only then)
    let start = Start { mode: if window { 0 } else { 1 } | if rig { 2 } else { 0 }, arg, file };
    spawn_full(code, &[], grants, true, PERS_NATIVE, 0, 0, 0x202, Some(&start))
}

/// Process `pid` ended: if it had the connection, it is reset at once (7.6).
fn conn_ended(pid: usize) {
    // SAFETY: single core, interrupts masked.
    if let Some((p, _)) = unsafe { CONN_LIVE } {
        if p == pid {
            crate::virtio::conn_reset("its process ended");
            // SAFETY: as above.
            unsafe { CONN_LIVE = None };
        }
    }
}

/// Process `pid` ended (exit, fault, Close anyway): its connection is reset and its window closes.
fn ended(pid: usize) {
    conn_ended(pid);
    cell_ended(pid);
    bridge_gone(pid); // Daraja: a bridge's foreign process ends with it (AFR-0001 7.7, no orphans)
}

/// `nettest3` (suite pc-net3, the test rig only): the ring-3 programs `net3` (CELL and NET handles, a window titled
/// "nettest3 (test)") and `net3b` (NET only), against scripts/lib/dira-server.py at 192.168.50.1:18086 on QEMU's user
/// network. Prints the frames check.
pub fn net_test() {
    if !crate::qemu::fw_cfg_file(b"opt/afrisovera/test-grants") {
        crate::kprintln!("[net3] refused: only the test rig (fw_cfg opt/afrisovera/test-grants) may grant the network");
        return;
    }
    if !crate::desk::shown() {
        crate::kprintln!("[net3] the desk is not showing (open it with: desk)");
        return;
    }
    let frames0 = frame::used_frames();
    let net = Grant::Net { classes: 0b0111 };
    let a = spawn_with(code(Prog::Net3), &[], &[Grant::Cell { title: b"nettest3 (test)" }, net], false);
    let b = spawn_with(code(Prog::Net3b), &[], &[net], false);
    if a.is_none() || b.is_none() {
        crate::kprintln!("[net3] could not spawn the programs");
        let _ = run_all();
        return;
    }
    let ended = run_all();
    let frames1 = frame::used_frames();
    crate::kprintln!(
        "[net3] {} ended; frames in use {} before and {} after ({}); a connection open: {}",
        ended,
        frames0,
        frames1,
        if frames0 == frames1 { "ok" } else { "WRONG" },
        crate::virtio::conn_open()
    );
}

/// `celltest3` (suite pc-cell3, the test rig only): run the ring-3 `cell` program with a CELL handle titled
/// "celltest3 (test)", preemptible, with the LAPIC timer running; then a program with no CELL handle that calls
/// cell_open (E_PERM). Prints the longest desk pass the timer ran with interrupts masked, and the frames check.
pub fn cell_test() {
    if !crate::qemu::fw_cfg_file(b"opt/afrisovera/test-grants") {
        crate::kprintln!("[cell3] refused: only the test rig (fw_cfg opt/afrisovera/test-grants) may grant a cell window");
        return;
    }
    if !crate::desk::shown() {
        crate::kprintln!("[cell3] the desk is not showing (open it with: desk)");
        return;
    }
    let frames0 = frame::used_frames();
    TICK_MAX_US.store(0, Ordering::Relaxed);
    let a = spawn_with(code(Prog::Cell), &[], &[Grant::Cell { title: b"celltest3 (test)" }], true);
    let b = spawn(code(Prog::NoCell), &[(0, RIGHT_SEND)], false);
    if a.is_none() || b.is_none() {
        crate::kprintln!("[cell3] could not spawn the programs");
        let _ = run_all();
        return;
    }
    // the local-APIC timer is the shell's own tick (pc/main.rs) and keeps running: it preempts the program too
    let ended = run_all();
    let frames1 = frame::used_frames();
    crate::kprintln!(
        "[cell3] {} ended; frames in use {} before and {} after ({}); the longest desk pass in the timer (interrupts open): {} us",
        ended,
        frames0,
        frames1,
        if frames0 == frames1 { "ok" } else { "WRONG" },
        TICK_MAX_US.load(Ordering::Relaxed)
    );
}

/// `vmotest` (suite pc-vmo): run the ring-3 `vmo` program and check that every frame it took came back, that its
/// reservation is gone, and that the one fault is the jump into its not-executable object. Prints pc-perf's two
/// timing lines.
pub fn vmo_test() {
    let (frames0, faults0) = (frame::used_frames(), FAULTS.load(Ordering::Relaxed));
    if spawn(code(Prog::Vmo), &[], false).is_none() {
        crate::kprintln!("[vmo] test: could not spawn the program");
        let _ = run_all();
        return;
    }
    let exited = run_all();
    let (frames1, faults) = (frame::used_frames(), FAULTS.load(Ordering::Relaxed) - faults0);
    let (reserved, b1, b16) = vmo_stats();
    // run_all counts the processes that ended (exit or fault): the one program, stopped by its fault
    let ok = frames1 == frames0 && reserved == 0 && faults == 1 && exited == 1;
    crate::kprintln!(
        "[vmo] test: {} ended, {} fault(s), frames in use {} before and {} after, {} pages still reserved ({})",
        exited, faults, frames0, frames1, reserved, if ok { "ok" } else { "WRONG" }
    );
    crate::kprintln!("[vmo] bench: map 1024 KiB: {} us", b1);
    crate::kprintln!("[vmo] bench: map 16384 KiB: {} us", b16);
}

/// The kernel memory the `uptr` program aims at (suite pc-userptr, gap G2 on the PC). Every call it makes must be
/// refused, so these bytes stay as they are and are never spoken.
static mut UPTR_CANARY: [u8; 16] = *b"G2-CANARY-INTACT";

/// `smapprobe` (suite pc-userptr, the test rig only): the kernel reads a ring-3 page plainly, outside pc/uaccess.rs.
/// With SMAP on, the #PF handler names it and halts (that line is the proof SMAP is working); without SMAP the read
/// goes through and is reported, and the program (the `fault` one) then runs and ends by its own fault.
pub fn smap_probe() {
    if !crate::qemu::fw_cfg_file(b"opt/afrisovera/test-grants") {
        crate::kprintln!("[smap] refused: only the test rig (fw_cfg opt/afrisovera/test-grants) may run the probe");
        return;
    }
    let Some(slot) = spawn(code(Prog::Fault), &[], false) else {
        crate::kprintln!("[smap] probe: could not spawn a program");
        return;
    };
    let back = mmu::cr3();
    crate::kprintln!(
        "[smap] probe: a plain kernel read of P{}'s code page at {:#x}, SMAP {}",
        slot,
        CODE_VA,
        if crate::uaccess::smap_on() { "on" } else { "not in this CPU" }
    );
    mmu::switch_to(procs()[slot].root);
    // SAFETY: deliberate: the page is mapped (present, USER) in the space just made current; with SMAP on this read
    // faults, which is the point of the probe. // uaccess: the SMAP probe itself
    let v = unsafe { core::ptr::read_volatile(CODE_VA as *const u8) }; // uaccess: the SMAP probe itself
    mmu::switch_to(back);
    crate::kprintln!("[smap] probe read {:#x} from ring-3 memory: nothing stopped it", v);
    let _ = run_all();
}

/// `actest` (suite pc-userptr, the test rig only): the ring-3 `acset` program sets RFLAGS.AC itself, then asks for
/// SYS_TEST_RAWREAD. With the AC clear at every kernel entry, SMAP stops the read (named by the #PF handler, halted);
/// without it the read goes through and the program prints the byte.
pub fn ac_test() {
    if !crate::qemu::fw_cfg_file(b"opt/afrisovera/test-grants") {
        crate::kprintln!("[actest] refused: only the test rig (fw_cfg opt/afrisovera/test-grants) may run it");
        return;
    }
    let Some(slot) = spawn(code(Prog::AcSet), &[], false) else {
        crate::kprintln!("[actest] could not spawn the program");
        return;
    };
    crate::kprintln!("[actest] P{} sets RFLAGS.AC, then asks for a raw kernel read of its code page, SMAP {}", slot,
        if crate::uaccess::smap_on() { "on" } else { "not in this CPU" });
    let _ = run_all();
    crate::kprintln!("[actest] the raw read went through: nothing stopped it");
}

/// `uptrtest` (suite pc-userptr, the test rig only): run the ring-3 `uptr` program with the canary's kernel address
/// in rdi and a CELL handle (so CELL_PUT reaches its pointer check), then check the canary, that the program ended
/// without a fault, and that every frame came back.
pub fn uptr_test() {
    if !crate::qemu::fw_cfg_file(b"opt/afrisovera/test-grants") {
        crate::kprintln!("[uptr] refused: only the test rig (fw_cfg opt/afrisovera/test-grants) may grant a cell window");
        return;
    }
    if !crate::desk::shown() {
        crate::kprintln!("[uptr] the desk is not showing (open it with: desk)");
        return;
    }
    let canary = core::ptr::addr_of_mut!(UPTR_CANARY);
    // SAFETY: single core, shell context; nothing else touches the canary.
    unsafe { canary.write_volatile(*b"G2-CANARY-INTACT") }; // uaccess: the kernel's own canary
    let (frames0, faults0) = (frame::used_frames(), FAULTS.load(Ordering::Relaxed));
    let Some(slot) = spawn_with(code(Prog::Uptr), &[], &[Grant::Cell { title: b"uptrtest (test)" }, Grant::Net { classes: 1 }], false) else {
        crate::kprintln!("[uptr] test: could not spawn the program");
        let _ = run_all();
        return;
    };
    procs()[slot].frame.rdi = canary as u64;
    crate::kprintln!("[uptr] P{} aims at the kernel canary at {:#x}", slot, canary as u64);
    let exited = run_all();
    // SAFETY: as above; read through the pointer so the check sees memory, not a cached constant.
    let c = unsafe { canary.read_volatile() }; // uaccess: the kernel's own canary
    if &c == b"G2-CANARY-INTACT" {
        crate::kprintln!("[uptr] G2-CANARY intact");
    } else {
        crate::kprintln!("[uptr] G2-CANARY OVERWRITTEN by a ring-3 pointer: \"{}\"", core::str::from_utf8(&c).unwrap_or("?"));
    }
    let (frames1, faults) = (frame::used_frames(), FAULTS.load(Ordering::Relaxed) - faults0);
    let ok = frames1 == frames0 && faults == 0 && exited == 1;
    crate::kprintln!(
        "[uptr] test: {} ended, {} fault(s), frames in use {} before and {} after ({})",
        exited, faults, frames0, frames1, if ok { "ok" } else { "WRONG" }
    );
}

/// Pages every process's objects hold in reservation now, and the last 1 MiB and 16 MiB map times (us), for `ps`
/// and pc-perf.
pub fn vmo_stats() -> (usize, u64, u64) {
    (VMO_RESERVED.load(Ordering::Relaxed), VMO_BENCH_1M.load(Ordering::Relaxed), VMO_BENCH_16M.load(Ordering::Relaxed))
}

/// The next runnable process after `from`, round robin.
fn next_runnable(from: usize) -> Option<usize> {
    (1..=N).map(|k| (from + k) % N).find(|&i| procs()[i].state == State::Runnable)
}

/// Leave the current process (already marked as it should be) and resume another; when no
/// process can run, go back to the kernel.
fn switch(f: &mut TrapFrame) {
    // SAFETY: single core, called from a trap handler with interrupts masked.
    let cur = unsafe { CURRENT };
    procs()[cur].frame = *f;
    tls_save(cur);
    match next_runnable(cur) {
        Some(n) => {
            // SAFETY: as above.
            unsafe { CURRENT = n };
            if n != cur {
                tls_switch(cur, n);
            }
            *f = procs()[n].frame;
            mmu::switch_to(procs()[n].root);
            use_kstack(n);
        }
        None => {
            let alive = procs()
                .iter()
                .filter(|p| matches!(p.state, State::BlockedSend | State::BlockedRecv | State::Unstarted | State::InCall | State::WaitChan))
                .count();
            if alive > 0 {
                crate::kprintln!("[proc] {} process(es) blocked forever on IPC, a deadlock; stopping them", alive);
            }
            user::back_to_kernel(procs().iter().filter(|p| p.state == State::Dead).count() as u64);
        }
    }
}

/// The capability behind `handle` in the current process, if it grants `right`.
fn cap(handle: u64, right: u8) -> Option<usize> {
    // SAFETY: single core, trap handler.
    let p = &procs()[unsafe { CURRENT }];
    let c = p.caps.get(handle as usize)?;
    (c.valid && c.kind == Kind::Endpoint && c.rights & right != 0).then_some(c.endpoint)
}

/// True if [ptr, ptr+len) lies inside a process's ring-3 range (code, data, stack), so a
/// pointer a program passes cannot make the kernel read or write kernel memory.
/// Every page of the range must be mapped for ring 3 in the CALLER's space: the range has
/// holes (between code, data and stack), and a kernel copy into one would fault.
fn user_range(ptr: u64, len: u64) -> bool {
    let Some(end) = ptr.checked_add(len) else { return false };
    // either between USER_BASE and the stack top, or inside the map area (AFR-0001 7.6); the page walk below then
    // refuses a guard page or a hole
    let low = ptr >= mmu::USER_BASE && end <= STACK_TOP;
    let mapped = ptr >= MAP_LO && end <= MAP_HI;
    if !low && !mapped {
        return false;
    }
    // SAFETY: single core, inside the caller's system call.
    let root = procs()[unsafe { CURRENT }].root;
    let mut pg = ptr & !0xFFF;
    while pg < end.max(ptr + 1) {
        if !mmu::user_mapped_in(root, pg) {
            return false;
        }
        pg += 4096;
    }
    true
}

pub fn syscall(f: &mut TrapFrame) {
    // SAFETY: single core, trap handler.
    let pid = unsafe { CURRENT };
    if procs()[pid].personality != PERS_NATIVE {
        // AFR-0001 7.7: a foreign process never reaches the Mansa table, whatever it puts in rax
        crate::foreign::int80_refused(pid, f);
        procs()[pid].state = State::Dead;
        ended(pid); // one way out for every process (a foreign one holds no window or connection)
        let p = procs()[pid].personality;
        if let Some(b) = crate::foreign::ended_by_fault(pid, p, 0x80, 0, 0, f.rip, f.rsp) {
            wake_bridge(b);
        }
        switch(f);
        return;
    }
    // AFR-0001 7.7: for a bridge, 24 is wait_one (bridge_call) and the v0 window calls 20 to 23 do not exist; for every
    // other process nothing here changes. "Bridge" is fixed at spawn, never "has a partner now" (the architect's term 1)
    if procs()[pid].bridge {
        if bridge_call(pid, f) {
            return;
        }
        if (SYS_WIN_CREATE..SYS_WIN_GETKEY).contains(&f.rax) {
            f.rax = err(E_NOSYS);
            return;
        }
    }
    match f.rax {
        SYS_EXIT => {
            crate::kprintln!("[P{}] exit {}", pid, f.rdi);
            procs()[pid].state = State::Dead;
            ended(pid);
            switch(f);
        }
        SYS_CELL_OPEN => f.rax = cell_open(pid, f.rdi, f.rsi, f.rdx),
        SYS_CELL_PUT => f.rax = cell_put(pid, f.rdi, f.rsi, f.rdx, f.rcx, f.r8),
        SYS_CELL_CURSOR => {
            f.rax = match cell_handle(pid, f.rdi, true) {
                Err(e) => err(e),
                Ok(_) => desk_result(on_desk(|| {
                    crate::desk::app_cell_cursor(crate::desk::cell_abi::WIN, f.rsi as i32, f.rdx as i32, f.rcx as i32)
                })),
            }
        }
        SYS_CELL_SIZE => {
            f.rax = match cell_handle(pid, f.rdi, true) {
                Err(e) => err(e),
                Ok(_) => desk_result(on_desk(|| crate::desk::app_cell_size(crate::desk::cell_abi::WIN))),
            }
        }
        SYS_CELL_EVENT => f.rax = cell_event(pid, f.rdi, f.rsi as i64),
        SYS_TEST_RAWREAD => {
            f.rax = if crate::qemu::fw_cfg_file(b"opt/afrisovera/test-grants") {
                // SAFETY: deliberate, test rig only: the caller's own code page, present in its space; SMAP must
                // stop this read. // uaccess: the AC test itself (suite pc-userptr)
                unsafe { core::ptr::read_volatile(CODE_VA as *const u8) as u64 } // uaccess: the AC test itself
            } else {
                EPERM
            };
        }
        SYS_TCP_DIAL => f.rax = tcp_dial(pid, f.rdi, f.rsi, f.rdx, f.rcx as i64),
        SYS_TCP_SEND => f.rax = tcp_send(pid, f.rdi, f.rsi, f.rdx),
        SYS_TCP_RECV => f.rax = tcp_recv(pid, f.rdi, f.rsi, f.rdx, f.rcx as i64),
        SYS_TCP_CLOSE => f.rax = tcp_close(pid, f.rdi, f.rsi),
        SYS_DNS_A => f.rax = dns_a(pid, f.rdi, f.rsi, f.rdx, f.rcx as i64),
        SYS_PRINT => {
            crate::kprintln!("[P{}] {:#x}", pid, f.rdi);
            f.rax = 0;
        }
        SYS_PREEMPTED => f.rax = procs()[pid].slices,
        SYS_GETPID => f.rax = pid as u64,
        SYS_PUTC => {
            let c = f.rdi as u8;
            if c == b'\n' {
                crate::kprintln!();
            } else {
                crate::kprint!("{}", c as char);
            }
            f.rax = 0;
        }
        SYS_TIME => f.rax = crate::cmos::epoch(),
        SYS_AI => {
            // the prompt and the reply live in the CALLER's memory: only its own ring-3 range
            let (pp, pl, op, oc) = (f.rdi, f.rsi.min(128), f.rdx, f.rcx.min(256));
            // and every byte moves through the one door (pc/uaccess.rs): the whole reply buffer is checked writable
            // before the responder runs, so a refusal never depends on how long the reply is
            let mut pb = [0u8; 128];
            let mut ob = [0u8; 256];
            if !user_range(pp, pl)
                || !user_range(op, oc)
                || !crate::uaccess::user_pages_ok(op, oc, true)
                || crate::uaccess::from_user(pp, &mut pb[..pl as usize]).is_err()
            {
                f.rax = EPERM;
            } else {
                let n = crate::nia::respond(&pb[..pl as usize], &mut ob).min(oc as usize);
                f.rax = match crate::uaccess::to_user(op, &ob[..n]) {
                    Ok(()) => n as u64,
                    Err(_) => EPERM,
                };
            }
        }
        SYS_SPEAK => {
            let (tp, tl) = (f.rdi, f.rsi.min(160));
            let mut tb = [0u8; 160];
            if !user_range(tp, tl) || crate::uaccess::from_user(tp, &mut tb[..tl as usize]).is_err() {
                f.rax = EPERM;
            } else {
                crate::kprintln!(
                    "    [asili voice @EL0] speaking: \"{}\"",
                    core::str::from_utf8(&tb[..tl as usize]).unwrap_or("?")
                );
                f.rax = 0;
            }
        }
        SYS_WIN_CREATE => f.rax = crate::tonga::win_create(pid, f.rdi as usize, f.rsi as usize) as u64,
        SYS_WIN_FILL => {
            let (a, b, c) = (f.rdi as usize, f.rsi as usize, f.rdx as usize);
            crate::tonga::win_fill(a, b, c, f.rcx as usize, f.r8 as usize, f.r9 as u32);
            f.rax = 0;
        }
        SYS_WIN_PRESENT => {
            crate::tonga::win_present();
            crate::screen::mark();
            f.rax = 0;
        }
        SYS_WIN_TEXT => {
            let (a, b, c) = (f.rdi as usize, f.rsi as usize, f.rdx as usize);
            crate::tonga::win_text(a, b, c, f.rcx as u8, f.r8 as u32);
            f.rax = 0;
        }
        SYS_WIN_GETKEY => {
            // the only, focused app: wait for a key, sleeping on `hlt` with interrupts open
            // so COM1's interrupt can deliver it (the trap gate entered with them off)
            let c = loop {
                if let Some(c) = crate::uart::getc() {
                    break c;
                }
                crate::cpu::sti();
                crate::cpu::hlt();
                crate::cpu::cli();
            };
            // SAFETY: single core; interrupts are off again here.
            unsafe {
                if EDITING && (0x20..0x7E).contains(&c) && EDIT_LEN < EDIT_BUF.len() {
                    EDIT_BUF[EDIT_LEN] = c;
                    EDIT_LEN += 1;
                }
            }
            f.rax = c as u64;
        }
        SYS_SLEEP => {
            // block the caller ~rdi ms (clamped), measured on the TSC; returns the ms slept.
            // The trap gate has interrupts off, so this waits on the counter, as the ARM
            // kernel's SYS_SLEEP does at EL1. "@EL0" is the shared log's name for user mode
            // (ring 3 here); the registry's sleep-syscall grader reads that line.
            let ms = f.rdi.min(5000);
            let got = crate::timer::busywait_ms(ms);
            crate::kprintln!("    [P{} @EL0] slept {}ms (asked {}ms)", pid, got, ms);
            f.rax = got;
        }
        SYS_VMO_CREATE => f.rax = vmo_create(pid, f.rdi),
        SYS_VMO_MAP => f.rax = vmo_map(pid, f.rdi, f.rsi, f.rdx, f.rcx),
        SYS_VMO_UNMAP => f.rax = vmo_unmap(pid, f.rdi, f.rsi),
        SYS_FPROC_MAP => f.rax = res(crate::foreign::fproc_map(pid, f.rdi, f.rsi, f.rdx, f.rcx)),
        SYS_FPROC_UNMAP => f.rax = res(crate::foreign::fproc_unmap(pid, f.rdi, f.rsi, f.rdx)),
        SYS_FPROC_READ => f.rax = res(crate::foreign::fproc_read(pid, f.rdi, f.rsi, f.rdx, f.rcx)),
        SYS_FPROC_WRITE => f.rax = res(crate::foreign::fproc_write(pid, f.rdi, f.rsi, f.rdx, f.rcx)),
        SYS_FPROC_PROTECT => f.rax = res(crate::foreign::fproc_protect(pid, f.rdi, f.rsi, f.rdx, f.rcx)),
        SYS_SEND => match cap(f.rdi, RIGHT_SEND) {
            None => {
                REFUSALS.fetch_add(1, Ordering::Relaxed);
                crate::kprintln!("[P{}] SEND on handle {} refused: no SEND capability", pid, f.rdi);
                f.rax = EPERM;
            }
            Some(e) => {
                let v = f.rsi;
                // a receiver already waiting on THIS endpoint gets the message directly
                if let Some(r) = procs().iter().position(|p| p.state == State::BlockedRecv && p.wait_ep == e) {
                    procs()[r].frame.rax = v;
                    procs()[r].state = State::Runnable;
                    f.rax = 0;
                } else {
                    // SAFETY: single core, trap handler.
                    let ep = unsafe { &mut (*core::ptr::addr_of_mut!(ENDPOINTS))[e] };
                    if !ep.full {
                        *ep = Endpoint { full: true, value: v };
                        f.rax = 0;
                    } else {
                        f.rax = 0; // returned when the message is taken
                        procs()[pid].pending = v;
                        procs()[pid].wait_ep = e;
                        procs()[pid].state = State::BlockedSend;
                        switch(f);
                    }
                }
            }
        },
        SYS_RECV => match cap(f.rdi, RIGHT_RECV) {
            None => {
                REFUSALS.fetch_add(1, Ordering::Relaxed);
                crate::kprintln!("[P{}] RECV on handle {} refused: no RECV capability", pid, f.rdi);
                f.rax = EPERM;
            }
            Some(e) => {
                // SAFETY: single core, trap handler.
                let ep = unsafe { &mut (*core::ptr::addr_of_mut!(ENDPOINTS))[e] };
                if ep.full {
                    f.rax = ep.value;
                    ep.full = false;
                    // a sender blocked on THIS endpoint: its message moves into the slot it waited for
                    if let Some(s) = procs().iter().position(|p| p.state == State::BlockedSend && p.wait_ep == e) {
                        *ep = Endpoint { full: true, value: procs()[s].pending };
                        procs()[s].state = State::Runnable;
                    }
                } else {
                    procs()[pid].wait_ep = e;
                    procs()[pid].state = State::BlockedRecv;
                    switch(f); // the sender will put the value in our saved rax
                }
            }
        },
        _ => f.rax = EPERM,
    }
}

/// An entry by the `syscall` instruction (cpu.rs syscall_entry). For a native process it is what it was before
/// EFER.SCE: an invalid opcode, at the `syscall` instruction itself. A foreign process's call goes to its bridge.
pub fn syscall_insn(f: &mut TrapFrame) {
    // SAFETY: single core, trap handler.
    let pid = unsafe { CURRENT };
    if procs()[pid].personality == PERS_NATIVE {
        f.rip = f.rip.wrapping_sub(2); // `syscall` is 0F 05; rcx (and the frame's rip) holds the next instruction
        // (rcx and r11 are already overwritten by `syscall` here; harmless while a fault only ends the process, but a
        // future fault that delivers a signal to a native program must not trust them)
        fault(f, 6, 0);
        return;
    }
    match crate::foreign::call(pid, procs()[pid].personality, f) {
        crate::foreign::Outcome::Wait(bridge) => {
            // the CALL is queued: this process waits for the reply, its bridge wakes to read it
            procs()[pid].state = State::InCall;
            wake_bridge(bridge);
            switch(f);
        }
        crate::foreign::Outcome::End => {
            // no bridge (the step 1 selftest's programs): the process ends
            procs()[pid].state = State::Dead;
            ended(pid);
            switch(f);
        }
    }
}

/// Wake a bridge waiting in wait_one: the signals as they are now become its result.
fn wake_bridge(b: usize) {
    if procs()[b].state == State::WaitChan {
        procs()[b].frame.rax = crate::foreign::bridge_signals();
        procs()[b].state = State::Runnable;
    }
}

/// `pid` ended: if it was a bridge, its foreign process ends with it (AFR-0001 7.7, no orphans).
fn bridge_gone(pid: usize) {
    if let Some(fs) = crate::foreign::bridge_ended(pid) {
        if !matches!(procs()[fs].state, State::Free | State::Dead) {
            procs()[fs].state = State::Dead;
        }
    }
}

/// Daraja: give back a dead foreign process's memory at once (its frames, page tables and reservation), while its
/// bridge still runs, so nothing is held for the bridge's lifetime (the browser engineer's point, 14 Nov). Only for a
/// space that is not loaded; run_all's clean-up then finds root 0 and nothing to free. Its kernel stack and slot go
/// back at run_all's end, as for every process.
fn reap(slot: usize) {
    let p = &mut procs()[slot];
    if p.state != State::Dead || p.personality == PERS_NATIVE || p.root == 0 || mmu::cr3() & !0xFFF == p.root & !0xFFF {
        return;
    }
    let pages = p.vmo_pages;
    mmu::free_space(p.root);
    VMO_RESERVED.fetch_sub(pages as usize, Ordering::Relaxed);
    p.vmo_pages = 0;
    p.root = 0;
    crate::foreign::forget_memory(slot);
    crate::kprintln!("[daraja] P{} memory given back ({} pages) while its bridge runs", slot, pages);
}

/// The foreign process in `slot` waits for a reply.
pub(crate) fn in_call(slot: usize) -> bool {
    procs()[slot].state == State::InCall
}

/// The foreign process in `slot` has not been started yet.
pub(crate) fn unstarted(slot: usize) -> bool {
    procs()[slot].state == State::Unstarted
}

/// A bridge's Daraja calls (17, 18, 24, 53, 54, 55; 49 to 52 and 56 are in the main table). True if `f.rax` was one.
fn bridge_call(pid: usize, f: &mut TrapFrame) -> bool {
    use crate::foreign as d;
    if let Some(fs) = d::partner(pid) {
        reap(fs);
    }
    match f.rax {
        SYS_CHANNEL_READ => f.rax = res(d::channel_read(pid, f.rdi, f.rsi, f.rdx, f.rcx, f.r8)),
        SYS_CHANNEL_WRITE => {
            f.rax = match d::channel_write(pid, f.rdi, f.rsi, f.rdx, f.rcx, f.r8) {
                Ok(d::Reply::Result(s, v)) => {
                    procs()[s].frame.rax = v;
                    procs()[s].state = State::Runnable;
                    0
                }
                Ok(d::Reply::Exit(s, _)) => {
                    procs()[s].state = State::Dead;
                    reap(s);
                    0
                }
                Err(e) => err(e),
            }
        }
        SYS_WAIT_ONE => match d::wait_one(pid, f.rdi, f.rsi, f.rdx as i64) {
            Ok(d::Wait::Now(s)) => f.rax = s,
            Ok(d::Wait::Block) => {
                procs()[pid].state = State::WaitChan;
                switch(f); // wake_bridge puts the signals in rax
            }
            Err(e) => f.rax = err(e),
        },
        SYS_FPROC_START => {
            f.rax = match d::fproc_start(pid, f.rdi, f.rsi, f.rdx) {
                Ok((s, rip, rsp)) => {
                    let p = &mut procs()[s];
                    p.frame = ZERO_FRAME;
                    p.frame.rip = rip;
                    p.frame.rsp = rsp;
                    p.frame.cs = USER_CS;
                    p.frame.ss = USER_DS;
                    p.frame.rflags = 0x202;
                    p.state = State::Runnable;
                    0
                }
                Err(e) => err(e),
            }
        }
        SYS_FPROC_SET_TLS => {
            f.rax = match d::fproc_set_tls(pid, f.rdi, f.rsi, f.rdx) {
                Ok((s, which, v)) => {
                    if which == 0 {
                        procs()[s].fs_base = v;
                    } else {
                        procs()[s].gs_base = v;
                    }
                    0
                }
                Err(e) => err(e),
            }
        }
        SYS_FPROC_KILL => {
            f.rax = match d::fproc_kill(pid, f.rdi, f.rsi) {
                Ok(s) => {
                    procs()[s].state = State::Dead;
                    reap(s);
                    0
                }
                Err(e) => err(e),
            }
        }
        _ => return false,
    }
    true
}

// FS and GS bases (AFR-0001 7.7). Only foreign processes have their own; a native process runs with both at 0, as
// before. CR4.FSGSBASE is off (cpu.rs syscall_init), so ring 3 cannot change them except by loading a selector, and
// the kernel reads them back from the MSRs whenever a foreign process stops running.
fn tls_save(slot: usize) {
    let p = &mut procs()[slot];
    if p.personality != PERS_NATIVE {
        // SAFETY: architectural MSRs; the kernel itself never uses FS or GS.
        unsafe {
            p.fs_base = crate::cpu::rdmsr(crate::cpu::MSR_FS_BASE);
            p.gs_base = crate::cpu::rdmsr(crate::cpu::MSR_GS_BASE);
        }
    }
}
fn tls_write(fs: u64, gs: u64) {
    // SAFETY: as tls_save; the values are canonical (set by the kernel at spawn, or read back from the MSRs).
    unsafe {
        crate::cpu::wrmsr(crate::cpu::MSR_FS_BASE, fs);
        crate::cpu::wrmsr(crate::cpu::MSR_GS_BASE, gs);
    }
}
/// Before `slot` first runs: its bases if it is foreign. (Every base is 0 when run_all starts.)
fn tls_load(slot: usize) {
    let p = &procs()[slot];
    if p.personality != PERS_NATIVE {
        tls_write(p.fs_base, p.gs_base);
    }
}
/// From `from` to `to`: native to native costs nothing; anything foreign on either side sets `to`'s bases.
fn tls_switch(from: usize, to: usize) {
    // the two personality bytes only (no copy of a whole Proc): native to native stays two compares
    let (pa, pb) = (procs()[from].personality, procs()[to].personality);
    if pb != PERS_NATIVE {
        let b = &procs()[to];
        tls_write(b.fs_base, b.gs_base);
    } else if pa != PERS_NATIVE {
        tls_write(0, 0);
    }
}
/// Back to the kernel: both bases 0 again.
fn tls_clear() {
    tls_write(0, 0);
}
/// The FS and GS bases the CPU holds now (for `daraja selftest`: both 0 outside ring 3).
pub fn tls_now() -> (u64, u64) {
    // SAFETY: reads only.
    unsafe { (crate::cpu::rdmsr(crate::cpu::MSR_FS_BASE), crate::cpu::rdmsr(crate::cpu::MSR_GS_BASE)) }
}

/// A timer interrupt arrived while a process ran in ring 3: its slice is over.
pub fn preempt(f: &mut TrapFrame) {
    if !active() {
        return;
    }
    // Only a tick that interrupted ring 3 may switch or draw: a system call waiting with interrupts open (cell_event,
    // WIN_GETKEY) is kernel code in progress, and the desk's rule is that nothing draws over it (architect, S2).
    if f.cs & 3 != 3 {
        return;
    }
    // SAFETY: single core, trap handler.
    let cur = unsafe { CURRENT };
    // SAFETY: as above.
    if unsafe { CELL_PID } == Some(cur) {
        // the desk keeps drawing while the window's owner computes (at most every 10 ms, the desk's own pace), and
        // "Close anyway" stops it here, as a fault would
        let t0 = crate::timer::cycles();
        // the serial line's bytes go to the window as Nia's loop poll hands them over: keys, and Ctrl+\ (the close
        // request, then "Close anyway"), even while the owner computes without a call.
        // The desk pass runs with interrupts OPEN, as in mainline, before the process resumes (the architect's S2
        // condition: a full frame in debug took 51 ms, past the 20 ms interrupts may stay masked; pc-cell3 on the cloud runner,
        // 14 Nov). A tick that lands meanwhile interrupted ring 0, so it returns at once (above); the trap's EOI is
        // already sent; nothing else can enter here, and the desk's own stack takes the nesting.
        on_desk(|| {
            crate::cpu::sti();
            while let Some(b) = crate::uart::getc() {
                crate::desk::cell_serial_byte(b);
            }
            crate::desk::app_tick();
            crate::cpu::cli();
        });
        let us = crate::timer::cycles().saturating_sub(t0) * 1_000_000 / crate::timer::hz().max(1);
        TICK_MAX_US.fetch_max(us, Ordering::Relaxed);
        if crate::desk::cell_forced() {
            crate::kprintln!("[P{}] stopped: Close anyway (the window's owner did not answer its close request)", cur);
            procs()[cur].state = State::Dead;
            ended(cur);
            switch(f);
            return;
        }
    }
    procs()[cur].slices += 1;
    if next_runnable(cur).is_some_and(|n| n != cur) {
        TIMER_SWITCHES.fetch_add(1, Ordering::Relaxed);
        switch(f); // still Runnable: it goes to the back of the line
    }
}

/// A process faulted: it dies, the others go on.
pub fn fault(f: &mut TrapFrame, vector: u64, addr: u64) {
    // SAFETY: single core, trap handler.
    let pid = unsafe { CURRENT };
    FAULTS.fetch_add(1, Ordering::Relaxed);
    // the tail is the shared log's wording (ARM: "EL0 fault in P.. killing"), which the registry reads
    crate::kprintln!(
        "[P{}] fault: vector {} at RIP {:#x} (address {:#x}); process stopped (EL0 fault in P{}, ring 3: killing it)",
        pid, vector, f.rip, addr, pid
    );
    procs()[pid].state = State::Dead;
    ended(pid);
    let p = procs()[pid].personality;
    if p != PERS_NATIVE {
        if let Some(b) = crate::foreign::ended_by_fault(pid, p, vector, f.error, addr, f.rip, f.rsp) {
            wake_bridge(b);
        }
    }
    switch(f);
}

// ------------------------------------------------------------------- the programs --
global_asm!(
    r#"
// Receiver: holds a RECV capability (handle 0). Marks its own data page, waits for a
// message, then tries what it has no right to do.
.global proc_prog_recv
.global proc_prog_recv_end
proc_prog_recv:
    movabs rcx, {data}
    mov qword ptr [rcx], 0xEC7
    mov eax, 4              // RECV(0): nothing yet -> blocks until the sender sends
    xor edi, edi
    int 0x80
    mov rdi, rax
    mov eax, 1              // print what arrived
    int 0x80
    mov eax, 3              // SEND(0, 0x99): it holds only RECV -> refused
    xor edi, edi
    mov esi, 0x99
    int 0x80
    mov eax, 4              // RECV(2): no capability at that handle at all -> refused
    mov edi, 2
    int 0x80
    movabs rcx, {data}      // its data page: still its own mark, whatever the sender wrote
    mov rdi, [rcx]
    mov eax, 1
    int 0x80
    xor eax, eax            // exit(0)
    xor edi, edi
    int 0x80
proc_prog_recv_end:

// Sender: holds a SEND capability (handle 0). Marks its data page (same address as the
// receiver's), sends 0x42, then tries to receive on its send-only handle.
.global proc_prog_send
.global proc_prog_send_end
proc_prog_send:
    movabs rcx, {data}
    mov qword ptr [rcx], 0x5E4D
    mov eax, 3              // SEND(0, 0x42)
    xor edi, edi
    mov esi, 0x42
    int 0x80
    mov eax, 4              // RECV(0): it holds only SEND -> refused
    xor edi, edi
    int 0x80
    movabs rcx, {data}
    mov rdi, [rcx]
    mov eax, 1
    int 0x80
    xor eax, eax
    xor edi, edi
    int 0x80
proc_prog_send_end:

// Endpoint check (review R1-10): a receiver that waits on handle 0, prints what arrives
// (or the refusal's EPERM) and exits; two senders that each send their own value on
// handle 0. Which endpoint handle 0 names is the capability's business, not the program's.
.global proc_prog_eprecv
.global proc_prog_eprecv_end
proc_prog_eprecv:
    mov eax, 4              // RECV(0)
    xor edi, edi
    int 0x80
    mov rdi, rax
    mov eax, 1              // print what arrived
    int 0x80
    xor eax, eax
    xor edi, edi
    int 0x80
proc_prog_eprecv_end:
.global proc_prog_epsenda
.global proc_prog_epsenda_end
proc_prog_epsenda:
    mov eax, 3              // SEND(0, 0xA1)
    xor edi, edi
    mov esi, 0xA1
    int 0x80
    xor eax, eax
    xor edi, edi
    int 0x80
proc_prog_epsenda_end:
.global proc_prog_epsendb
.global proc_prog_epsendb_end
proc_prog_epsendb:
    mov eax, 3              // SEND(0, 0xB2)
    xor edi, edi
    mov esi, 0xB2
    int 0x80
    xor eax, eax
    xor edi, edi
    int 0x80
proc_prog_epsendb_end:

// Worker: spins until the timer has sliced it 3 times, then prints its count and exits.
// It never yields: only the timer can take the CPU from it.
.global proc_prog_worker
.global proc_prog_worker_end
proc_prog_worker:
2:
    mov eax, 5              // how many times have I been preempted?
    int 0x80
    cmp rax, 3
    jb 2b
    mov rdi, rax
    mov eax, 1
    int 0x80
    xor eax, eax
    xor edi, edi
    int 0x80
proc_prog_worker_end:

// sleep: say it started (0x51), ask the kernel for 300 ms, print what it measured, exit 0
.global proc_prog_sleep
.global proc_prog_sleep_end
proc_prog_sleep:
    mov eax, 1
    mov edi, 0x51
    int 0x80
    mov eax, 15
    mov edi, 300
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    xor eax, eax
    xor edi, edi
    int 0x80
proc_prog_sleep_end:

// gui: the ARM `user_gui` (the `wm` demo) in x86: a 200x100 window whose background
// colour depends on the pid, a gold header bar, a green and a red block, present, exit.
.global proc_prog_gui
.global proc_prog_gui_end
proc_prog_gui:
    mov eax, 8
    int 0x80
    mov r13, rax
    mov eax, 20
    mov edi, 200
    mov esi, 100
    int 0x80
    mov r12, rax
    mov eax, 0x081018
    imul eax, r13d
    add eax, 0x203040
    mov r9d, eax
    mov eax, 21
    mov rdi, r12
    xor esi, esi
    xor edx, edx
    mov ecx, 200
    mov r8d, 100
    int 0x80
    mov eax, 21
    mov rdi, r12
    xor esi, esi
    xor edx, edx
    mov ecx, 200
    mov r8d, 18
    mov r9d, 0xE0A000
    int 0x80
    mov eax, 21
    mov rdi, r12
    mov esi, 14
    mov edx, 30
    mov ecx, 78
    mov r8d, 52
    mov r9d, 0x27C93F
    int 0x80
    mov eax, 21
    mov rdi, r12
    mov esi, 104
    mov edx, 30
    mov ecx, 82
    mov r8d, 52
    mov r9d, 0xFF5F56
    int 0x80
    mov eax, 22
    int 0x80
    xor eax, eax
    xor edi, edi
    int 0x80
proc_prog_gui_end:

// edit: a 210x110 window (dark body, blue header); every key typed is drawn at the cursor
// until '~', then a last present and exit 0. The ARM `user_edit` program, in x86.
.global proc_prog_edit
.global proc_prog_edit_end
proc_prog_edit:
    mov eax, 20
    mov edi, 210
    mov esi, 110
    int 0x80
    mov r12, rax
    mov eax, 21
    mov rdi, r12
    xor esi, esi
    xor edx, edx
    mov ecx, 210
    mov r8d, 110
    mov r9d, 0x141A28
    int 0x80
    mov eax, 21
    mov rdi, r12
    xor esi, esi
    xor edx, edx
    mov ecx, 210
    mov r8d, 18
    mov r9d, 0x1B3A6B
    int 0x80
    mov eax, 22
    int 0x80
    mov r13d, 8
    mov r14d, 28
2:
    mov eax, 24
    int 0x80
    cmp rax, 0x7E
    je 4f
    mov r15, rax
    mov eax, 23
    mov rdi, r12
    mov rsi, r13
    mov rdx, r14
    mov rcx, r15
    mov r8d, 0xE0E6F0
    int 0x80
    add r13, 8
    cmp r13, 200
    jb 3f
    mov r13d, 8
    add r14, 10
3:
    mov eax, 22
    int 0x80
    jmp 2b
4:
    mov eax, 22
    int 0x80
    xor eax, eax
    xor edi, edi
    int 0x80
proc_prog_edit_end:

// vmo: process memory (AFR-0001 7.6, S1). Prints every result with SYS_PRINT, in this order, for pc-vmo's grader:
//  1. a 1 MiB object: its handle (0), its address (MAP_LO); the first and last words read 0 (zeroed); the last
//     word written and read back (0x5eed)
//  2. refusals: map with EXECUTE (E_PERM, -1); size 100 (E_INVAL, -5); 32 MiB more, past the per-process cap
//     (E_NOMEM, -6); map of an empty handle (E_BADHANDLE, -2)
//  3. a 4 KiB object mapped: its address, after the 1 MiB one and its guard page
//  4. a buffer across the 1 MiB object's guard page, handed to a v0 call: refused (EPERM)
//  5. unmap the 1 MiB object (0), again (-5); a 16 MiB object created and mapped: its address (first fit)
//  6. a 4 KiB object created and never mapped (its handle, 2): it keeps its reservation until the process ends;
//     then 4 KiB objects created and mapped until the handle table is full: each address, then -6
//  7. a jump into the 16 MiB object: it is not executable, so the CPU faults and the kernel stops the process
.global proc_prog_vmo
.global proc_prog_vmo_end
proc_prog_vmo:
    mov eax, 32
    mov edi, 0x100000
    int 0x80
    mov r12, rax
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov eax, 33
    mov rdi, r12
    xor esi, esi
    mov edx, 0x100000
    mov ecx, 3
    int 0x80
    mov r13, rax
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov rdi, qword ptr [r13]
    mov eax, 1
    int 0x80
    mov rdi, qword ptr [r13 + 0xFFFF8]
    mov eax, 1
    int 0x80
    mov eax, 0x5EED
    mov qword ptr [r13 + 0xFFFF8], rax
    mov rdi, qword ptr [r13 + 0xFFFF8]
    mov eax, 1
    int 0x80
    mov eax, 32
    mov edi, 0x1000
    int 0x80
    mov r14, rax
    mov eax, 33
    mov rdi, r14
    xor esi, esi
    mov edx, 0x1000
    mov ecx, 7
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov eax, 32
    mov edi, 100
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov eax, 32
    mov edi, 0x2000000
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov eax, 33
    mov edi, 7
    xor esi, esi
    mov edx, 0x1000
    mov ecx, 3
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov eax, 33
    mov rdi, r14
    xor esi, esi
    mov edx, 0x1000
    mov ecx, 3
    int 0x80
    mov r15, rax
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov eax, 14
    lea rdi, [r13 + 0xFFFF0]
    mov esi, 0x20
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov eax, 34
    mov rdi, r13
    mov esi, 0x100000
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov eax, 34
    mov rdi, r13
    mov esi, 0x100000
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov eax, 32
    mov edi, 0x1000000
    int 0x80
    mov rbx, rax
    mov eax, 33
    mov rdi, rbx
    xor esi, esi
    mov edx, 0x1000000
    mov ecx, 3
    int 0x80
    mov rbx, rax
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov eax, 32
    mov edi, 0x1000
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
2:
    mov eax, 32
    mov edi, 0x1000
    int 0x80
    test rax, rax
    js 3f
    mov rdi, rax
    mov eax, 33
    xor esi, esi
    mov edx, 0x1000
    mov ecx, 3
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    jmp 2b
3:
    mov rdi, rax
    mov eax, 1
    int 0x80
    jmp rbx
proc_prog_vmo_end:

// uptr (suite pc-userptr, gap G2 on the PC): every call that takes a user address, handed four bad ranges of 16
// bytes: the kernel's canary (its address arrives in rdi from uptr_test), a non-canonical address, a range from the
// top of the caller's own stack past the end of its ring-3 range, and one that wraps from the top of the address
// space round into the kernel's low memory. Handle 0 is a CELL (uptr_test grants it) and the window is opened, so
// CELL_PUT gets past its handle check to the pointer; a 4 KiB object is mapped, so VMO_UNMAP has a live mapping to
// match. Prints with SYS_PRINT: the object's address; cell_open (0); then for each range AI with it as the prompt,
// AI with it as the reply buffer, SPEAK with it as the text (each EPERM), VMO_UNMAP of it (E_INVAL) and CELL_PUT of
// 16 bytes from it (E_FAULT) and DNS_A with it as the name (handle 1, a NET grant; E_FAULT); then AI with its own read-only code page as the reply buffer (EPERM: inside its range,
// but not writable), the object's own unmap (0) and 0x5afe. Stack: [rsp] the four ranges, [rsp+32]
// the prompt "hi", [rsp+48] a reply buffer.
// RULE: a new system call that takes a user address adds its line here, and to scripts/pc-userptr-test.sh, in the
// same commit that adds the call.
.global proc_prog_uptr
.global proc_prog_uptr_end
proc_prog_uptr:
    mov r15, rdi
    sub rsp, 96
    mov word ptr [rsp + 32], 0x6968
    mov qword ptr [rsp], r15
    mov rax, 0x8000000000000000
    mov qword ptr [rsp + 8], rax
    mov rax, 0x000001000003FFF8
    mov qword ptr [rsp + 16], rax
    mov rax, 0xFFFFFFFFFFFFFFF8
    mov qword ptr [rsp + 24], rax
    mov eax, 32
    mov edi, 0x1000
    int 0x80
    mov rdi, rax
    mov eax, 33
    xor esi, esi
    mov edx, 0x1000
    mov ecx, 3
    int 0x80
    mov r13, rax
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov eax, 64
    xor edi, edi
    mov esi, 1
    mov edx, 1
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    xor r12d, r12d
proc_prog_uptr_next:
    mov r14, qword ptr [rsp + r12 * 8]
    mov rdi, r14
    mov esi, 16
    lea rdx, [rsp + 48]
    mov ecx, 16
    mov eax, 13
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    lea rdi, [rsp + 32]
    mov esi, 2
    mov rdx, r14
    mov ecx, 16
    mov eax, 13
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov rdi, r14
    mov esi, 16
    mov eax, 14
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov rdi, r14
    mov esi, 0x1000
    mov eax, 34
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    xor edi, edi
    xor esi, esi
    xor edx, edx
    mov rcx, r14
    mov r8d, 16
    mov eax, 65
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov edi, 1
    mov rsi, r14
    mov edx, 16
    xor ecx, ecx
    mov eax, 76
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    inc r12
    cmp r12, 4
    jb proc_prog_uptr_next
    lea rdi, [rsp + 32]
    mov esi, 2
    mov rdx, 0x0000010000000000
    mov ecx, 16
    mov eax, 13
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov rdi, r13
    mov esi, 0x1000
    mov eax, 34
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov edi, 0x5afe
    mov eax, 1
    int 0x80
    xor eax, eax
    xor edi, edi
    int 0x80
proc_prog_uptr_end:

// acset (suite pc-userptr, the AC test): set RFLAGS.AC from ring 3, then SYS_TEST_RAWREAD; print what came back.
.global proc_prog_acset
.global proc_prog_acset_end
proc_prog_acset:
    pushfq
    or qword ptr [rsp], 0x40000
    popfq
    mov eax, 127
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    xor eax, eax
    xor edi, edi
    int 0x80
proc_prog_acset_end:

// cell: the cell window (AFR-0001 7.6, S2), handle 0 a CELL. Prints, in order: cell_open (0); a second cell_open
// (E_WOULDBLOCK, the window is held); cell_size (cols << 14 | rows); a put of 4 cells from the data page (4); a put
// from address 0x1000 (E_FAULT); a put of 241 bytes (E_INVAL); a put on row 100 (0, clipped); cell_cursor (0). Then
// every event it gets: a close request (kind 3) makes it exit 7; the key 's' makes it spin with no calls (for Close
// anyway); a closed event (kind 4) or an error makes it exit 9.
.global proc_prog_cell
.global proc_prog_cell_end
proc_prog_cell:
    mov eax, 64
    xor edi, edi
    mov esi, 64
    mov edx, 24
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov eax, 64
    xor edi, edi
    mov esi, 64
    mov edx, 24
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov eax, 67
    xor edi, edi
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    movabs rcx, {data}
    mov rax, 0x0F440F690F720F41
    mov qword ptr [rcx], rax
    mov eax, 65
    xor edi, edi
    mov esi, 2
    mov edx, 1
    movabs rcx, {data}
    mov r8d, 8
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov eax, 65
    xor edi, edi
    xor esi, esi
    xor edx, edx
    mov ecx, 0x1000
    mov r8d, 8
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov eax, 65
    xor edi, edi
    xor esi, esi
    xor edx, edx
    movabs rcx, {data}
    mov r8d, 241
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov eax, 65
    xor edi, edi
    xor esi, esi
    mov edx, 100
    movabs rcx, {data}
    mov r8d, 8
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov eax, 66
    xor edi, edi
    mov esi, 6
    mov edx, 1
    mov ecx, 1
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
2:
    mov eax, 68
    xor edi, edi
    mov rsi, -1
    int 0x80
    mov r12, rax
    mov rdi, rax
    mov eax, 1
    int 0x80
    test r12, r12
    js 4f
    mov rax, r12
    shr rax, 28
    cmp rax, 3
    je 3f
    cmp rax, 4
    je 4f
    cmp r12, 0x10000073
    je 5f
    jmp 2b
3:
    xor eax, eax
    mov edi, 7
    int 0x80
4:
    xor eax, eax
    mov edi, 9
    int 0x80
5:
    jmp 5b
proc_prog_cell_end:

// nocell: no CELL handle (handle 0 is an endpoint): cell_open is refused (E_PERM); it prints that and exits
.global proc_prog_nocell
.global proc_prog_nocell_end
proc_prog_nocell:
    mov eax, 64
    xor edi, edi
    mov esi, 10
    mov edx, 5
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    xor eax, eax
    xor edi, edi
    int 0x80
proc_prog_nocell_end:

// net3: the network for ring 3 (AFR-0001 7.6, S3), handle 0 a CELL, handle 1 a NET (classes 0, 1, 2). Prints, in
// order: cell_open (0); dials to 224.0.0.1, 0.0.0.1 and 127.0.0.1 (E_PERM each); dns_a of "a..b" (E_INVAL); dns_a of
// "net3.invalid" (E_PEERCLOSED: no checked answer); a dial to 192.168.50.1:18086 (a CONN handle); a second dial
// (E_WOULDBLOCK); the send of GET /index.html (28); recv until it returns 0 (0, then the byte total); tcp_close (0);
// a send on the closed handle (E_BADHANDLE); a dial (a handle) and the send of GET /slow (22); recv with no limit,
// ended by the close request (E_CANCELED); cell_event (the close request, 0x30000000); recv again (E_PEERCLOSED,
// the connection was reset). Exits 7.
.global proc_prog_net3
.global proc_prog_net3_end
proc_prog_net3:
    mov eax, 64
    xor edi, edi
    mov esi, 40
    mov edx, 10
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov eax, 72
    mov edi, 1
    mov esi, 0xE0000001
    mov edx, 80
    mov ecx, 1000
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov eax, 72
    mov edi, 1
    mov esi, 0x00000001
    mov edx, 80
    mov ecx, 1000
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov eax, 72
    mov edi, 1
    mov esi, 0x7F000001
    mov edx, 80
    mov ecx, 1000
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov eax, 76
    mov edi, 1
    lea rsi, [rip + net3_bad]
    mov edx, 4
    mov ecx, 3000
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov eax, 76
    mov edi, 1
    lea rsi, [rip + net3_name]
    mov edx, 12
    mov ecx, 6000
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov eax, 72
    mov edi, 1
    mov esi, 0xC0A83201
    mov edx, 18086
    mov ecx, 5000
    int 0x80
    mov r13, rax
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov eax, 72
    mov edi, 1
    mov esi, 0xC0A83201
    mov edx, 18086
    mov ecx, 5000
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov eax, 73
    mov rdi, r13
    lea rsi, [rip + net3_get]
    mov edx, 28
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    xor r14d, r14d
2:
    mov eax, 74
    mov rdi, r13
    movabs rsi, {data}
    mov edx, 4096
    mov ecx, 5000
    int 0x80
    test rax, rax
    jle 3f
    add r14, rax
    jmp 2b
3:
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov rdi, r14
    mov eax, 1
    int 0x80
    mov eax, 75
    mov rdi, r13
    xor esi, esi
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov eax, 73
    mov rdi, r13
    lea rsi, [rip + net3_get]
    mov edx, 28
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov eax, 72
    mov edi, 1
    mov esi, 0xC0A83201
    mov edx, 18086
    mov ecx, 5000
    int 0x80
    mov r13, rax
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov eax, 73
    mov rdi, r13
    lea rsi, [rip + net3_slow]
    mov edx, 22
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov eax, 74
    mov rdi, r13
    movabs rsi, {data}
    mov edx, 4096
    mov rcx, -1
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov eax, 68
    xor edi, edi
    mov rsi, -1
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov eax, 74
    mov rdi, r13
    movabs rsi, {data}
    mov edx, 4096
    xor ecx, ecx
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    xor eax, eax
    mov edi, 7
    int 0x80
net3_get:
    .ascii "GET /index.html HTTP/1.0\r\n\r\n"
net3_slow:
    .ascii "GET /slow HTTP/1.0\r\n\r\n"
net3_name:
    .ascii "net3.invalid"
net3_bad:
    .ascii "a..b"
proc_prog_net3_end:

// net3b: handle 0 a NET (classes 0, 1, 2), no window. Dials 192.168.50.1:18086 (a handle), sends GET /huge (22),
// reads until a call returns 0 or less: E_TOOBIG past 4 MiB (printed), then the byte total; dials again (a handle:
// the reset freed the route) and sends GET /slow (22); exits 5 with that connection open (the kernel resets it).
.global proc_prog_net3b
.global proc_prog_net3b_end
proc_prog_net3b:
    call 6f
    mov eax, 73
    mov rdi, r13
    lea rsi, [rip + net3b_huge]
    mov edx, 22
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    xor r14d, r14d
2:
    mov eax, 74
    mov rdi, r13
    movabs rsi, {data}
    mov edx, 4096
    mov ecx, 5000
    int 0x80
    test rax, rax
    jle 3f
    add r14, rax
    jmp 2b
3:
    mov rdi, rax
    mov eax, 1
    int 0x80
    mov rdi, r14
    mov eax, 1
    int 0x80
    call 6f
    mov eax, 73
    mov rdi, r13
    lea rsi, [rip + net3b_slow]
    mov edx, 22
    int 0x80
    mov rdi, rax
    mov eax, 1
    int 0x80
    xor eax, eax
    mov edi, 5
    int 0x80
6:
    mov eax, 72
    xor edi, edi
    mov esi, 0xC0A83201
    mov edx, 18086
    mov ecx, 5000
    int 0x80
    mov r13, rax
    mov rdi, rax
    mov eax, 1
    int 0x80
    ret
net3b_huge:
    .ascii "GET /huge HTTP/1.0\r\n\r\n"
net3b_slow:
    .ascii "GET /slow HTTP/1.0\r\n\r\n"
proc_prog_net3b_end:

// fault: write to address 0, a kernel-only page, so the CPU raises a page fault from ring 3
.global proc_prog_fault
.global proc_prog_fault_end
proc_prog_fault:
    xor eax, eax
    mov qword ptr [rax], rax
2:
    jmp 2b
proc_prog_fault_end:
"#,
    data = const DATA_VA,
);

extern "C" {
    static proc_prog_recv: u8;
    static proc_prog_recv_end: u8;
    static proc_prog_send: u8;
    static proc_prog_send_end: u8;
    static proc_prog_eprecv: u8;
    static proc_prog_eprecv_end: u8;
    static proc_prog_epsenda: u8;
    static proc_prog_epsenda_end: u8;
    static proc_prog_epsendb: u8;
    static proc_prog_epsendb_end: u8;
    static proc_prog_worker: u8;
    static proc_prog_worker_end: u8;
    static proc_prog_sleep: u8;
    static proc_prog_sleep_end: u8;
    static proc_prog_gui: u8;
    static proc_prog_gui_end: u8;
    static proc_prog_edit: u8;
    static proc_prog_edit_end: u8;
    static proc_prog_fault: u8;
    static proc_prog_fault_end: u8;
    static proc_prog_vmo: u8;
    static proc_prog_vmo_end: u8;
    static proc_prog_uptr: u8;
    static proc_prog_uptr_end: u8;
    static proc_prog_acset: u8;
    static proc_prog_acset_end: u8;
    static proc_prog_cell: u8;
    static proc_prog_cell_end: u8;
    static proc_prog_nocell: u8;
    static proc_prog_nocell_end: u8;
    static proc_prog_net3: u8;
    static proc_prog_net3_end: u8;
    static proc_prog_net3b: u8;
    static proc_prog_net3b_end: u8;
}

pub enum Prog {
    Recv,
    Send,
    /// RECV on handle 0, print it, exit (the endpoint check)
    EpRecv,
    /// SEND 0xA1 / 0xB2 on handle 0, exit (the endpoint check)
    EpSendA,
    EpSendB,
    Worker,
    Sleep,
    Edit,
    Fault,
    /// process memory, AFR-0001 7.6 (the `vmotest` command, suite pc-vmo)
    Vmo,
    /// every pointer-taking call handed kernel and bad addresses (the `uptrtest` command, suite pc-userptr)
    Uptr,
    /// sets RFLAGS.AC, then SYS_TEST_RAWREAD (the `actest` command, suite pc-userptr)
    AcSet,
    /// the cell window for ring 3, AFR-0001 7.6 S2 (the `celltest3` command, suite pc-cell3)
    Cell,
    /// a program with no CELL handle calling cell_open
    NoCell,
    /// the network for ring 3, AFR-0001 7.6 S3 (the `nettest3` command, suite pc-net3): CELL and NET handles
    Net3,
    /// the same, NET only: the 4 MiB cap, and an exit with the connection open
    Net3b,
    /// the `wm` demo window (ARM user_gui)
    Gui,
    /// userland/rustapp built for x86-64 (scripts/build-userapp.sh): compiled Rust in ring 3
    RustApp,
    /// userland/niavm built for x86-64: the Nia WASM interpreter as a ring-3 process
    NiaVm,
}

/// The compiled-Rust ring-3 app: the ARM kernel's `rustapp` source, built for x86-64 and
/// linked at USER_BASE (userland/rustapp/linker-x86_64.ld), raw binary from llvm-objcopy.
static RUSTAPP: &[u8] = include_bytes!("rustapp-x86_64.bin");
/// The Nia WASM interpreter (userland/niavm) built for x86-64, same way as RUSTAPP.
static NIAVM: &[u8] = include_bytes!("niavm-x86_64.bin");

pub fn code(p: Prog) -> &'static [u8] {
    if let Prog::RustApp = p {
        return RUSTAPP;
    }
    if let Prog::NiaVm = p {
        return NIAVM;
    }
    // SAFETY: each pair of labels brackets one program's machine code in .text.
    unsafe {
        let (a, b) = match p {
            Prog::Recv => (&proc_prog_recv as *const u8, &proc_prog_recv_end as *const u8),
            Prog::Send => (&proc_prog_send as *const u8, &proc_prog_send_end as *const u8),
            Prog::EpRecv => (&proc_prog_eprecv as *const u8, &proc_prog_eprecv_end as *const u8),
            Prog::EpSendA => (&proc_prog_epsenda as *const u8, &proc_prog_epsenda_end as *const u8),
            Prog::EpSendB => (&proc_prog_epsendb as *const u8, &proc_prog_epsendb_end as *const u8),
            Prog::Worker => (&proc_prog_worker as *const u8, &proc_prog_worker_end as *const u8),
            Prog::Sleep => (&proc_prog_sleep as *const u8, &proc_prog_sleep_end as *const u8),
            Prog::Edit => (&proc_prog_edit as *const u8, &proc_prog_edit_end as *const u8),
            Prog::Fault => (&proc_prog_fault as *const u8, &proc_prog_fault_end as *const u8),
            Prog::Gui => (&proc_prog_gui as *const u8, &proc_prog_gui_end as *const u8),
            Prog::Vmo => (&proc_prog_vmo as *const u8, &proc_prog_vmo_end as *const u8),
            Prog::Uptr => (&proc_prog_uptr as *const u8, &proc_prog_uptr_end as *const u8),
            Prog::AcSet => (&proc_prog_acset as *const u8, &proc_prog_acset_end as *const u8),
            Prog::Cell => (&proc_prog_cell as *const u8, &proc_prog_cell_end as *const u8),
            Prog::NoCell => (&proc_prog_nocell as *const u8, &proc_prog_nocell_end as *const u8),
            Prog::Net3 => (&proc_prog_net3 as *const u8, &proc_prog_net3_end as *const u8),
            Prog::Net3b => (&proc_prog_net3b as *const u8, &proc_prog_net3b_end as *const u8),
            Prog::RustApp | Prog::NiaVm => unreachable!(),
        };
        core::slice::from_raw_parts(a, b as usize - a as usize) // uaccess: the kernel's own program code
    }
}

/// One read-only row per process slot, for the shell's `ps` (PC track step 2):
/// (state name, preemptible-by-the-timer was ever used, timer slices taken).
pub fn table() -> [(&'static str, u64); N] {
    let mut out = [("free", 0u64); N];
    for (o, p) in out.iter_mut().zip(procs().iter()) {
        let name = match p.state {
            State::Free => "free",
            State::Runnable => "runnable",
            State::BlockedSend => "blk-send",
            State::BlockedRecv => "blk-recv",
            State::Unstarted => "unstarted",
            State::InCall => "in-call",
            State::WaitChan => "wait-chan",
            State::Dead => "dead",
        };
        *o = (name, p.slices);
    }
    out
}
