//! PC stand-in for the ARM `process.rs` (EL0 processes, the ARM syscall ABI, the EL0 demo
//! apps). The PC kernel has its own ring-3 processes (`pc/proc.rs`, proved at boot by the
//! ring-3, IPC and scheduler lines); the ARM demo programs are AArch64 machine code and
//! do not run here. Each entry point says so instead of pretending.
#![allow(dead_code)]

fn absent(what: &str) {
    crate::kprintln!(
        "[pc] {}: not available on this PC yet, because the ARM EL0 programs are AArch64 code; the PC's own ring-3 processes ran at boot (see the [pc] ring 3 / IPC / scheduler lines)",
        what
    );
}

/// The ARM `ps` layout over the PC's own table: slot P0 is the kernel context hosting the
/// shell, P1.. are the ring-3 slots of `pc/proc.rs` (free or dead once the boot proof ran).
pub fn dump_table() {
    let rows = crate::proc::table();
    crate::kprintln!("  SLOT  STATE     KIND    PREEMPT");
    crate::kprintln!("  P0    runnable  idle    no");
    let (mut runnable, mut blocked, mut dead) = (1u32, 0u32, 0u32);
    for (i, (state, slices)) in rows.iter().enumerate() {
        match *state {
            "runnable" => runnable += 1,
            "blk-send" | "blk-recv" => blocked += 1,
            _ => dead += 1,
        }
        crate::kprintln!("  P{}    {:8}  {:6}  {}", i + 1, state, "ring3", if *slices > 0 { "yes" } else { "no" });
    }
    crate::kprintln!(
        "ps: {} slots, {} runnable, {} blocked, {} dead",
        rows.len() + 1, runnable, blocked, dead
    );
    // the ARM row: every spawn costs a 16 KiB kernel stack from the heap (pc/proc.rs)
    crate::kprintln!(
        "    stacks: {} served since boot, room for {} more at {} KiB (see `heap`)",
        crate::heap::stacks_served(),
        crate::heap::stacks_left(),
        crate::heap::STACK_SIZE / 1024
    );
}

/// `rustapp`: ordinary compiled Rust (userland/rustapp, the ARM app's own source built for
/// x86-64) runs in ring 3 and reaches the kernel only by `int 0x80`: it prints 0x37 (the
/// sum 1..=10) and the marker 0xc0de, then exits.
pub fn run_rustapp() {
    let Some(pid) = crate::proc::spawn(crate::proc::code(crate::proc::Prog::RustApp), &[], false) else {
        crate::kprintln!("[pc] rustapp: could not create the process");
        return;
    };
    crate::cpu::cli();
    let exited = crate::proc::run_all();
    crate::cpu::sti();
    if exited == 1 {
        crate::kprintln!("    [P{} @EL0] compiled-Rust app exited (ring 3, code 0)", pid);
    } else {
        crate::kprintln!("[pc] rustapp: the process did not exit cleanly");
    }
}
/// `edit`: a ring-3 editor (pc/proc.rs proc_prog_edit) draws what is typed in a window
/// until '~'; the kernel then saves the typed text to edited.txt, as the ARM kernel does.
pub fn run_edit_app() {
    crate::screen::set_wm_view(true);
    crate::tonga::wm_reset();
    // SAFETY: single core, shell context: nothing else touches the edit session.
    unsafe {
        crate::proc::EDIT_LEN = 0;
        crate::proc::EDITING = true;
    }
    let Some(_) = crate::proc::spawn(crate::proc::code(crate::proc::Prog::Edit), &[], false) else {
        // SAFETY: as above.
        unsafe { crate::proc::EDITING = false };
        crate::kprintln!("[pc] edit: could not create the process");
        return;
    };
    crate::cpu::cli();
    crate::proc::run_all();
    crate::cpu::sti();
    // SAFETY: as above; the editor has exited.
    let (buf, n) = unsafe {
        crate::proc::EDITING = false;
        (&*core::ptr::addr_of!(crate::proc::EDIT_BUF), crate::proc::EDIT_LEN)
    };
    if n > 0 {
        if crate::vfs::write("edited.txt", &buf[..n]) {
            crate::kprintln!("[edit] saved {} bytes to edited.txt", n);
        } else {
            crate::kprintln!("[edit] save FAILED (filesystem full?)");
        }
    } else {
        crate::kprintln!("[edit] nothing typed; edited.txt unchanged");
    }
}
/// `faultdemo`: a ring-3 program writes to address 0 (a kernel-only page); the page fault
/// stops that program alone (pc/proc.rs `fault`) and the shell carries on.
pub fn run_fault_demo() {
    if crate::proc::spawn(crate::proc::code(crate::proc::Prog::Fault), &[], false).is_none() {
        crate::kprintln!("[pc] faultdemo: could not create the process");
        return;
    }
    crate::cpu::cli();
    crate::proc::run_all();
    crate::cpu::sti();
}
/// `wm`: three ring-3 programs each create, draw and present their own window through the
/// shared Tonga compositor (the ARM `user_gui`, in x86), then exit.
pub fn run_gui_demo() {
    crate::tonga::wm_reset();
    crate::tonga::win_order_reset();
    crate::screen::set_wm_view(true);
    let mut windows = 0;
    for _ in 0..3 {
        if crate::proc::spawn(crate::proc::code(crate::proc::Prog::Gui), &[], false).is_some() {
            windows += 1;
        }
    }
    if windows == 0 {
        crate::kprintln!("[pc] wm: could not create the window programs");
        return;
    }
    crate::cpu::cli();
    let exited = crate::proc::run_all();
    crate::cpu::sti();
    crate::kprintln!("    [wm] {} ring-3 window program(s) drew and exited ({} clean)", windows, exited);
}
/// `niavm`: the Nia WASM interpreter (userland/niavm, the ARM app's own source built for
/// x86-64) runs a WASM module in ring 3; its host calls (print, putc, time, ai, speak) are
/// system calls.
pub fn run_niavm() {
    let Some(pid) = crate::proc::spawn(crate::proc::code(crate::proc::Prog::NiaVm), &[], false) else {
        crate::kprintln!("[pc] niavm: could not create the process");
        return;
    };
    crate::cpu::cli();
    let exited = crate::proc::run_all();
    crate::cpu::sti();
    if exited == 1 {
        crate::kprintln!("    [P{} @EL0] Nia interpreter exited (ring 3, code 0)", pid);
    } else {
        crate::kprintln!("[pc] niavm: the process did not exit cleanly");
    }
}
pub fn run_rtc_demo() { absent("userspace RTC driver (EL0)") }
pub fn run_sgi_el0_probe() { absent("SGI probe from EL0") }
/// `syssleep`: a ring-3 process asks the kernel, by system call, to block it ~300 ms
/// (pc/proc.rs SYS_SLEEP), prints the measured time and exits; the shell resumes after.
pub fn run_sleep_demo() {
    let Some(pid) = crate::proc::spawn(crate::proc::code(crate::proc::Prog::Sleep), &[], false) else {
        crate::kprintln!("[pc] syssleep: could not create the process");
        return;
    };
    crate::cpu::cli();
    let exited = crate::proc::run_all();
    crate::cpu::sti();
    if exited == 1 {
        crate::kprintln!("    [P{} @EL0] exit (ring 3, code 0)", pid);
    } else {
        crate::kprintln!("[pc] syssleep: the process did not exit cleanly");
    }
}
