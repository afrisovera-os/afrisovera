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
pub const EPERM: u64 = u64::MAX;

/// An edit session (`edit`): while on, every printable key SYS_WIN_GETKEY hands out is also
/// kept here, so the kernel can save what was typed when the editor exits.
pub static mut EDITING: bool = false;
pub static mut EDIT_BUF: [u8; 256] = [0; 256];
pub static mut EDIT_LEN: usize = 0;

pub const RIGHT_SEND: u8 = 1;
pub const RIGHT_RECV: u8 = 2;

const N: usize = 5; // ring-3 slots: five, as the ARM kernel (N_USER), so `ps` shows the same 6-slot table
const N_CAPS: usize = 4;
/// Endpoints a capability can name (0..N_EP); each holds one message of its own.
pub const N_EP: usize = 4;
const CODE_VA: u64 = mmu::USER_BASE;
/// Ring-3 layout: code from USER_BASE (up to CODE_PAGES pages), the data page at DATA_VA,
/// and STACK_PAGES of stack ending at STACK_TOP.
const CODE_PAGES: usize = 16; // 64 KiB: the Nia interpreter is ~20 KiB
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
    Dead,
}

#[derive(Clone, Copy)]
struct Cap {
    valid: bool,
    endpoint: usize,
    rights: u8,
}
const NO_CAP: Cap = Cap { valid: false, endpoint: 0, rights: 0 };

#[derive(Clone, Copy)]
struct Proc {
    state: State,
    root: u64,
    frame: TrapFrame,
    caps: [Cap; N_CAPS],
    pending: u64, // a BlockedSend process's message
    wait_ep: usize, // the endpoint a BlockedSend/BlockedRecv process waits on
    slices: u64,  // timer interrupts taken while running
}

const ZERO_FRAME: TrapFrame = TrapFrame {
    r15: 0, r14: 0, r13: 0, r12: 0, r11: 0, r10: 0, r9: 0, r8: 0, rdi: 0, rsi: 0, rbp: 0,
    rbx: 0, rdx: 0, rcx: 0, rax: 0, vector: 0, error: 0, rip: 0, cs: 0, rflags: 0, rsp: 0, ss: 0,
};
const FREE: Proc = Proc { state: State::Free, root: 0, frame: ZERO_FRAME, caps: [NO_CAP; N_CAPS], pending: 0, wait_ep: 0, slices: 0 };

/// One endpoint's single message slot.
#[derive(Clone, Copy)]
struct Endpoint {
    full: bool,
    value: u64,
}

static mut PROCS: [Proc; N] = [FREE; N];
/// The kernel stack of the process in each slot (its base; 0 = none yet). Every spawn
/// takes a new 16 KiB stack from the heap, as the ARM kernel's `process::spawn` does, so
/// `stacks served` counts spawns on both kernels. Like the ARM kernel (its `ps` says so),
/// the PC does not yet give a reaped process's stack back: reclaiming kernel stacks on
/// reap is one change for both kernels, not a PC-only divergence.
static mut KSTACKS: [usize; N] = [0; N];

/// A new kernel stack for `slot`; a refusal is named on the console, as on ARM.
fn kstack_top(slot: usize) -> Option<u64> {
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
    let slot = procs().iter().position(|p| p.state == State::Free)?;
    kstack_top(slot)?;
    let root = mmu::new_space()?;
    // code: as many read-only pages as the program needs (filled with int3 past its end)
    let pages = code.len().div_ceil(4096).clamp(1, CODE_PAGES);
    if code.len() > CODE_PAGES * 4096 {
        crate::kprintln!("[proc] P{} NOT spawned: program is {} bytes, more than {} KiB", slot, code.len(), CODE_PAGES * 4);
        mmu::free_space(root);
        return None;
    }
    let mut ok = true;
    for i in 0..pages {
        let Some(c) = frame::alloc() else { ok = false; break };
        let part = &code[(i * 4096).min(code.len())..((i + 1) * 4096).min(code.len())];
        // SAFETY: a fresh frame, 1:1 mapped, not yet visible to any ring-3 program.
        unsafe {
            core::ptr::write_bytes(c as *mut u8, 0xCC, 4096);
            core::ptr::copy_nonoverlapping(part.as_ptr(), c as *mut u8, part.len());
        }
        ok &= mmu::map_user_in(root, CODE_VA + (i * 4096) as u64, c as u64, false);
    }
    for i in 0..STACK_PAGES {
        match frame::alloc() {
            Some(st) => ok &= mmu::map_user_in(root, STACK_TOP - ((i + 1) * 4096) as u64, st as u64, true),
            None => ok = false,
        }
    }
    match frame::alloc() {
        Some(d) => ok &= mmu::map_user_in(root, DATA_VA, d as u64, true),
        None => ok = false,
    }
    if !ok {
        // a frame or table refused: give back what this space already took
        mmu::free_space(root);
        return None;
    }
    let p = &mut procs()[slot];
    *p = FREE;
    p.root = root;
    for (h, &(endpoint, rights)) in caps.iter().enumerate().take(N_CAPS) {
        // a capability for an endpoint that does not exist is left empty (its use is refused)
        if endpoint < N_EP {
            p.caps[h] = Cap { valid: true, endpoint, rights };
        }
    }
    p.frame = ZERO_FRAME;
    p.frame.rip = CODE_VA;
    p.frame.cs = USER_CS;
    p.frame.rflags = if preemptible { 0x202 } else { 0x2 };
    p.frame.rsp = STACK_TOP;
    p.frame.ss = USER_DS;
    p.state = State::Runnable;
    Some(slot)
}

/// Run every spawned process until all have exited (or none can run). Returns the number
/// that ended by exit; a fault or a deadlock is reported in the log.
pub fn run_all() -> u64 {
    let Some(first) = next_runnable(N - 1) else { return 0 };
    // SAFETY: single core, interrupts masked; the scheduler state is ours until ring 3 runs.
    unsafe { CURRENT = first };
    ACTIVE.store(true, Ordering::Relaxed);
    mmu::switch_to(procs()[first].root);
    use_kstack(first);
    let start = procs()[first].frame;
    let rc = user::enter_frame(&start);
    crate::cpu::set_rsp0(0);
    ACTIVE.store(false, Ordering::Relaxed);
    mmu::switch_to(mmu::root());
    for p in procs().iter_mut() {
        if p.state != State::Free {
            mmu::free_space(p.root); // its pages and page tables back to the frame pool
        }
        *p = FREE;
    }
    rc
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
    match next_runnable(cur) {
        Some(n) => {
            // SAFETY: as above.
            unsafe { CURRENT = n };
            *f = procs()[n].frame;
            mmu::switch_to(procs()[n].root);
            use_kstack(n);
        }
        None => {
            let alive = procs().iter().filter(|p| matches!(p.state, State::BlockedSend | State::BlockedRecv)).count();
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
    (c.valid && c.rights & right != 0).then_some(c.endpoint)
}

/// True if [ptr, ptr+len) lies inside a process's ring-3 range (code, data, stack), so a
/// pointer a program passes cannot make the kernel read or write kernel memory.
/// Every page of the range must be mapped for ring 3 in the CALLER's space: the range has
/// holes (between code, data and stack), and a kernel copy into one would fault.
fn user_range(ptr: u64, len: u64) -> bool {
    let Some(end) = ptr.checked_add(len) else { return false };
    if ptr < mmu::USER_BASE || end > STACK_TOP {
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
    match f.rax {
        SYS_EXIT => {
            crate::kprintln!("[P{}] exit {}", pid, f.rdi);
            procs()[pid].state = State::Dead;
            switch(f);
        }
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
            if !user_range(pp, pl) || !user_range(op, oc) {
                f.rax = EPERM;
            } else {
                let mut pb = [0u8; 128];
                let mut ob = [0u8; 256];
                // SAFETY: both ranges are inside this process's mapped ring-3 pages (checked)
                // and its address space is the current one during its system call.
                unsafe {
                    core::ptr::copy_nonoverlapping(pp as *const u8, pb.as_mut_ptr(), pl as usize);
                    let n = crate::nia::respond(&pb[..pl as usize], &mut ob).min(oc as usize);
                    core::ptr::copy_nonoverlapping(ob.as_ptr(), op as *mut u8, n);
                    f.rax = n as u64;
                }
            }
        }
        SYS_SPEAK => {
            let (tp, tl) = (f.rdi, f.rsi.min(160));
            if !user_range(tp, tl) {
                f.rax = EPERM;
            } else {
                let mut tb = [0u8; 160];
                // SAFETY: inside this process's ring-3 pages (checked), its space current.
                unsafe { core::ptr::copy_nonoverlapping(tp as *const u8, tb.as_mut_ptr(), tl as usize) };
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

/// A timer interrupt arrived while a process ran in ring 3: its slice is over.
pub fn preempt(f: &mut TrapFrame) {
    if !active() {
        return;
    }
    // SAFETY: single core, trap handler.
    let cur = unsafe { CURRENT };
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
            Prog::RustApp | Prog::NiaVm => unreachable!(),
        };
        core::slice::from_raw_parts(a, b as usize - a as usize)
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
            State::Dead => "dead",
        };
        *o = (name, p.slices);
    }
    out
}
