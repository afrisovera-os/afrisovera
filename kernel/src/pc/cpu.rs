//! x86-64 processor setup for the PC kernel: port I/O, MSRs, the kernel's own GDT and
//! IDT, the trap entry stubs, and the local APIC timer.
//!
//! Counterpart of the ARM kernel's `cpu.rs` + `exceptions.rs` + `gic.rs` + `timer.rs`:
//! the GDT/IDT play the role of VBAR_EL1's vector table, the local APIC the role of the
//! GIC CPU interface, and the APIC timer the role of the generic timer.

use core::arch::{asm, global_asm};
use core::sync::atomic::{AtomicU64, Ordering};

pub unsafe fn outb(port: u16, v: u8) {
    asm!("out dx, al", in("dx") port, in("al") v, options(nomem, nostack, preserves_flags));
}
pub unsafe fn outw(port: u16, v: u16) {
    asm!("out dx, ax", in("dx") port, in("ax") v, options(nomem, nostack, preserves_flags));
}
pub unsafe fn inb(port: u16) -> u8 {
    let v: u8;
    asm!("in al, dx", out("al") v, in("dx") port, options(nomem, nostack, preserves_flags));
    v
}
/// G13 (pc/cpu_sidech.rs): this CPU's side-channel state, from CPUID, and from IA32_ARCH_CAPABILITIES only when
/// CPUID.7.0:EDX[29] says the MSR exists (reading it otherwise is a #GP).
pub fn side_channels() -> crate::cpu_sidech::Report {
    use crate::cpu_sidech::{assess, Vendor, EDX7_ARCH_CAPABILITIES, MSR_ARCH_CAPABILITIES};
    // SAFETY: CPUID exists on every x86-64 CPU; leaf 7 is read only when leaf 0 says it exists.
    let l0 = unsafe { core::arch::x86_64::__cpuid_count(0, 0) };
    let mut v = [0u8; 12];
    v[0..4].copy_from_slice(&l0.ebx.to_le_bytes());
    v[4..8].copy_from_slice(&l0.edx.to_le_bytes());
    v[8..12].copy_from_slice(&l0.ecx.to_le_bytes());
    let vendor = match &v {
        b"GenuineIntel" => Vendor::Intel,
        b"AuthenticAMD" => Vendor::Amd,
        _ => Vendor::Other,
    };
    let (edx7, ebx7) = if l0.eax >= 7 {
        // SAFETY: as above.
        let r = unsafe { core::arch::x86_64::__cpuid_count(7, 0) };
        (r.edx, r.ebx)
    } else {
        (0, 0)
    };
    // SAFETY: the MSR exists (CPUID.7.0:EDX[29]); reading it has no side effects.
    let caps = if edx7 & EDX7_ARCH_CAPABILITIES != 0 { Some(unsafe { rdmsr(MSR_ARCH_CAPABILITIES) }) } else { None };
    assess(vendor, edx7, ebx7, caps)
}

/// The one G13 line, for the boot log (so `report` carries it), `cpuinfo` and `hwscan`.
pub fn side_channel_line(prefix: &str) {
    let r = side_channels();
    // CPUID.1:ECX[31]: running under a hypervisor, whose CPUID answer is its own, not the hardware's (QEMU's TCG CPU
    // says AuthenticAMD, so a cloud boot reads "no"; that is not a fact about any real machine)
    // SAFETY: CPUID leaf 1 exists on every x86-64 CPU.
    let hv = unsafe { core::arch::x86_64::__cpuid_count(1, 0) }.ecx & (1 << 31) != 0;
    crate::kprintln!(
        "{}cpu side channels: {}; {}{}",
        prefix,
        r,
        if r.meltdown_affected() {
            "no KPTI yet, so downloaded foreign programs are refused (G13)"
        } else {
            "Meltdown ruled out by the CPU"
        },
        if hv { " (under a hypervisor: its answer)" } else { "" }
    );
}

pub unsafe fn rdmsr(msr: u32) -> u64 {
    let (lo, hi): (u32, u32);
    asm!("rdmsr", in("ecx") msr, out("eax") lo, out("edx") hi, options(nomem, nostack));
    ((hi as u64) << 32) | lo as u64
}
pub unsafe fn wrmsr(msr: u32, v: u64) {
    asm!("wrmsr", in("ecx") msr, in("eax") v as u32, in("edx") (v >> 32) as u32,
         options(nomem, nostack));
}
pub fn cli() {
    // SAFETY: masks interrupts on this core.
    unsafe { asm!("cli", options(nomem, nostack)) }
}
pub fn sti() {
    // SAFETY: unmasks interrupts on this core; the IDT is loaded before any caller.
    unsafe { asm!("sti", options(nomem, nostack)) }
}
pub fn hlt() {
    // SAFETY: waits for the next interrupt.
    unsafe { asm!("hlt", options(nomem, nostack)) }
}
/// The ARM shell's idle instruction (`wfi`), on the PC: `hlt` until the next interrupt
/// (the local APIC timer runs while the shell does). If interrupts happen to be masked,
/// `hlt` would never return, so it only pauses instead.
/// True if maskable interrupts are on (RFLAGS.IF).
pub fn interrupts_on() -> bool {
    let flags: u64;
    // SAFETY: reads RFLAGS through the stack; no other effect.
    unsafe { asm!("pushfq", "pop {}", out(reg) flags, options(nomem, preserves_flags)) };
    flags & (1 << 9) != 0
}
pub fn wfi() {
    crate::xhci::service(); // USB devices plugged in or out since the last idle moment
    crate::uvc::idle(); // a camera's new picture onto the screen, or a failed stream stopped
    crate::ps2::service(); // the PS/2 keyboard's Caps Lock light, when Caps Lock changed
    if crate::acpi::power_button_pressed() {
        crate::acpi::power_off_pressed(); // files saved, then off
    }
    let flags: u64;
    // SAFETY: reads RFLAGS through the stack; no other effect.
    unsafe { asm!("pushfq", "pop {}", out(reg) flags, options(nomem, preserves_flags)) };
    if flags & (1 << 9) != 0 {
        hlt();
    } else {
        core::hint::spin_loop();
    }
}
/// Sleep until the next interrupt unless `ready()` already holds, without losing a wake-up: interrupts are
/// masked while `ready` is checked, and `sti; hlt` run back to back (an interrupt can only be taken after
/// the instruction that follows `sti`, so one that arrives in between still ends the `hlt`). With
/// interrupts off on entry it only checks once (a `hlt` would never end). Returns `ready()`'s answer.
pub fn idle_until(mut ready: impl FnMut() -> bool) -> bool {
    if !interrupts_on() {
        return ready();
    }
    cli();
    let r = ready();
    if r {
        sti();
    } else {
        // SAFETY: re-enables interrupts and waits for the next one; the timer bounds the wait.
        unsafe { asm!("sti", "hlt", options(nomem, nostack)) }
    }
    r
}
pub fn halt_forever() -> ! {
    loop {
        cli();
        hlt();
    }
}

// ------------------------------------------------------------------------------ GDT --
// The firmware's GDT lives in memory the kernel now owns and may reuse, so the kernel
// installs its own: null, 64-bit kernel code (0x08), kernel data (0x10), user data (0x18)
// and 64-bit user code (0x20), both privilege level 3, and the TSS (0x28, two slots).
// The TSS slot is filled at load time; the CPU writes its busy bit, so the table is mutable.
static mut GDT: [u64; 7] = [
    0,
    0x00AF_9A00_0000_FFFF,
    0x00CF_9200_0000_FFFF,
    0x00CF_F200_0000_FFFF,
    0x00AF_FA00_0000_FFFF,
    0,
    0,
];
pub const KERNEL_CS: u16 = 0x08;
/// Selectors a ring-3 program runs with (index | requested privilege level 3).
pub const USER_DS: u64 = 0x18 | 3;
pub const USER_CS: u64 = 0x20 | 3;
const TSS_SEL: u16 = 0x28;

/// The 64-bit task-state segment. Its only job here: RSP0, the stack the CPU switches to
/// when an interrupt or fault arrives while a ring-3 program runs (a user stack can never be
/// trusted to receive the kernel's trap frame).
#[repr(C, packed)]
struct Tss {
    _r0: u32,
    rsp0: u64,
    _rsp12: [u64; 2],
    _r1: u64,
    _ist: [u64; 7],
    _r2: u64,
    _r3: u16,
    iomap: u16,
}
static mut TSS: Tss = Tss { _r0: 0, rsp0: 0, _rsp12: [0; 2], _r1: 0, _ist: [0; 7], _r2: 0, _r3: 0, iomap: 104 };

/// The kernel stack for traps out of ring 3.
#[repr(align(16))]
#[allow(dead_code)] // only its address is used
struct TrapStack([u8; 16 * 1024]);
static mut TRAP_STACK: TrapStack = TrapStack([0; 16 * 1024]);

#[repr(C, packed)]
struct DescPtr {
    limit: u16,
    base: u64,
}

pub fn load_gdt() {
    // SAFETY: single core, interrupts masked; GDT, TSS and TRAP_STACK are 'static and only
    // written here, before anything can use them.
    unsafe {
        TSS.rsp0 = core::ptr::addr_of!(TRAP_STACK) as u64 + core::mem::size_of::<TrapStack>() as u64;
        // F8: IST 1 = the #DF stack (gate 8 uses it): a kernel-stack overrun faults with RSP in the guard, where
        // no frame can be pushed; the CPU then raises #DF on this stack instead, so it can still be reported.
        // (The TSS is packed: the whole array is assigned, never a reference into it.)
        let df_top = (core::ptr::addr_of!(crate::stackinfo::DF_STACK) as u64
            + core::mem::size_of::<crate::stackinfo::DfStack>() as u64)
            & !15;
        // Daraja (AFR-0001 7.7): IST 2, 3 and 4 for NMI, #DB and #MC. `syscall` does not change the stack pointer,
        // so for the few instructions before syscall_entry loads the kernel stack the CPU is on a stack ring 3
        // chose; an NMI, a debug trap or a machine check there must never be pushed onto it.
        TSS._ist = [df_top, ist_top(core::ptr::addr_of!(NMI_STACK)), ist_top(core::ptr::addr_of!(DB_STACK)), ist_top(core::ptr::addr_of!(MC_STACK)), 0, 0, 0];
        SYSCALL_KSTACK = TSS.rsp0;
        let base = core::ptr::addr_of!(TSS) as u64;
        let limit = (core::mem::size_of::<Tss>() - 1) as u64;
        GDT[5] = (limit & 0xFFFF)
            | (base & 0xFF_FFFF) << 16
            | 0x89 << 40 // present, available 64-bit TSS
            | ((limit >> 16) & 0xF) << 48
            | ((base >> 24) & 0xFF) << 56;
        GDT[6] = base >> 32;
    }
    let p = DescPtr {
        limit: (core::mem::size_of::<[u64; 7]>() - 1) as u16,
        base: core::ptr::addr_of!(GDT) as u64,
    };
    // SAFETY: GDT is 'static; CS is reloaded with a far return, the data segments
    // with the new data selector, then the task register with the TSS.
    unsafe {
        asm!(
            "lgdt [{p}]",
            p = in(reg) &p,
            options(readonly, nostack, preserves_flags)
        );
        asm!(
            "pushq $0x08",
            "leaq 2f(%rip), %rax",
            "pushq %rax",
            "lretq",
            "2:",
            "movw $0x10, %ax",
            "movw %ax, %ds",
            "movw %ax, %es",
            "movw %ax, %ss",
            "movw %ax, %fs",
            "movw %ax, %gs",
            out("rax") _,
            options(att_syntax)
        );
        asm!("ltr {0:x}", in(reg) TSS_SEL, options(nostack, preserves_flags));
    }
}

/// The stack ring 3's next trap lands on: the running process's own kernel stack, or the
/// boot trap stack when `top` is 0.
pub fn set_rsp0(top: u64) {
    // SAFETY: single core, interrupts masked (the scheduler's callers); the CPU reads RSP0
    // only on the next privilege change, which cannot happen until this store is done.
    unsafe {
        TSS.rsp0 = if top != 0 {
            top
        } else {
            core::ptr::addr_of!(TRAP_STACK) as u64 + core::mem::size_of::<TrapStack>() as u64
        };
        // the `syscall` entry switches to the same stack an `int 0x80` would land on
        SYSCALL_KSTACK = TSS.rsp0;
    }
}

/// One IST stack (AFR-0001 7.7: NMI, #DB, #MC each have their own).
#[repr(align(16))]
#[allow(dead_code)] // only its address is used
struct IstStack([u8; 8 * 1024]);
static mut NMI_STACK: IstStack = IstStack([0; 8 * 1024]);
static mut DB_STACK: IstStack = IstStack([0; 8 * 1024]);
static mut MC_STACK: IstStack = IstStack([0; 8 * 1024]);

fn ist_top(s: *const IstStack) -> u64 {
    (s as u64 + core::mem::size_of::<IstStack>() as u64) & !15
}

/// The IST entries Daraja relies on, as the CPU will read them: (TSS IST 2, 3, 4 set, the boot core's NMI, #DB and
/// #MC gates on IST 2, 3, 4, the second core's gates on none). For `daraja selftest`.
pub fn ist_state() -> (bool, bool, bool) {
    // SAFETY: plain reads of tables written once at boot.
    unsafe {
        let ist = TSS._ist;
        let tss = ist[1] != 0 && ist[2] != 0 && ist[3] != 0;
        let bsp = IDT[2].ist == 2 && IDT[1].ist == 3 && IDT[18].ist == 4;
        let ap = IDT_AP[2].ist == 0 && IDT_AP[1].ist == 0 && IDT_AP[18].ist == 0;
        (tss, bsp, ap)
    }
}

// ----------------------------------------------------------------------- SYSCALL --
// Daraja (AFR-0001 7.7): the `syscall` instruction. Native programs still enter only by `int 0x80`; their `syscall`
// is answered as an invalid opcode (proc.rs), exactly as before SCE was set. A foreign process's `syscall` goes to
// its bridge. Written from the AMD64 APM vol. 2 (SYSCALL, STAR/LSTAR/SFMASK) and Intel SDM vol. 3 (IST).
//
// The entry runs on the boot core only: EFER.SCE is set on it alone (core 1 never runs ring 3, and its `syscall`
// stays an invalid opcode), so the static scratch slot below is never shared. pc_trap checks it again on every
// entry. If ring 3 ever runs on core 1, the scratch moves to per-CPU data reached with `swapgs`, and that change
// goes to the architect (AFR-0001 7.7). The return is always isr_common's `iretq`, never SYSRET.
const MSR_EFER: u32 = 0xC000_0080;
const MSR_STAR: u32 = 0xC000_0081;
const MSR_LSTAR: u32 = 0xC000_0082;
const MSR_SFMASK: u32 = 0xC000_0084;
pub const MSR_FS_BASE: u32 = 0xC000_0100;
pub const MSR_GS_BASE: u32 = 0xC000_0101;
/// The trap frame's vector for an entry by `syscall` (not an IDT vector).
pub const SYSCALL_INSN_VECTOR: u64 = 0x100;
/// #DB traps taken at syscall_entry itself (a `syscall` run with TF set), absorbed there.
pub static DB_AT_ENTRY: AtomicU64 = AtomicU64::new(0);

/// The kernel stack the next `syscall` lands on (kept equal to TSS.rsp0 by load_gdt and set_rsp0).
static mut SYSCALL_KSTACK: u64 = 0;
/// The user stack pointer, for the two instructions between `syscall` and the push of the frame.
static mut SYSCALL_USER_RSP: u64 = 0;

global_asm!(
    r#"
.global syscall_entry
syscall_entry:
    mov [rip + {ursp}], rsp
    mov rsp, [rip + {kstack}]
    push {uds}
    push qword ptr [rip + {ursp}]
    push r11
    push {ucs}
    push rcx
    push 0
    push {vec}
    jmp isr_common
"#,
    ursp = sym SYSCALL_USER_RSP,
    kstack = sym SYSCALL_KSTACK,
    uds = const USER_DS,
    ucs = const USER_CS,
    vec = const SYSCALL_INSN_VECTOR,
);
extern "C" {
    static syscall_entry: u8;
}

fn syscall_entry_addr() -> u64 {
    // SAFETY: only the label's address is taken.
    unsafe { &syscall_entry as *const u8 as u64 }
}

/// Switch the `syscall` instruction on, on this (the boot) core. SFMASK clears IF, TF, DF and AC on entry, and
/// CR4.FSGSBASE is cleared so ring 3 can never set its own FS or GS base (only the kernel does, per process).
pub fn syscall_init() {
    // SAFETY: architectural MSRs and CR4 of an x86-64 CPU; interrupts are masked (boot), nothing uses `syscall` yet.
    unsafe {
        wrmsr(MSR_STAR, (0x10u64 << 48) | ((KERNEL_CS as u64) << 32)); // SYSRET bases unused (iretq), set anyway
        wrmsr(MSR_LSTAR, syscall_entry_addr());
        wrmsr(MSR_SFMASK, (1 << 9) | (1 << 8) | (1 << 10) | (1 << 18)); // IF, TF, DF, AC
        let mut cr4: u64;
        asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack, preserves_flags));
        if cr4 & (1 << 16) != 0 {
            cr4 &= !(1 << 16);
            asm!("mov cr4, {}", in(reg) cr4, options(nomem, nostack, preserves_flags));
        }
        wrmsr(MSR_EFER, rdmsr(MSR_EFER) | 1); // SCE
    }
}

/// (EFER.SCE on, CR4.FSGSBASE off, LSTAR is syscall_entry, SFMASK as set), for `daraja selftest`.
pub fn syscall_state() -> (bool, bool, bool, bool) {
    // SAFETY: reads only.
    unsafe {
        let cr4: u64;
        asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack, preserves_flags));
        (
            rdmsr(MSR_EFER) & 1 != 0,
            cr4 & (1 << 16) == 0,
            rdmsr(MSR_LSTAR) == syscall_entry_addr(),
            rdmsr(MSR_SFMASK) & 0x4_0700 == 0x4_0700,
        )
    }
}

/// A second core: the same GDT as the boot core and its IDT without IST entries (IDT_AP; no task register: core 1
/// never runs ring 3, so it needs no RSP0).
pub fn load_gdt_ap() {
    let p = DescPtr {
        limit: (core::mem::size_of::<[u64; 7]>() - 1) as u16,
        base: core::ptr::addr_of!(GDT) as u64,
    };
    let i = DescPtr {
        limit: (core::mem::size_of::<[Gate; 256]>() - 1) as u16,
        base: core::ptr::addr_of!(IDT_AP) as u64,
    };
    // SAFETY: the boot core's tables, complete and 'static; CS and the data segments are
    // reloaded exactly as `load_gdt` does.
    unsafe {
        asm!("lgdt [{p}]", p = in(reg) &p, options(readonly, nostack, preserves_flags));
        asm!(
            "pushq $0x08",
            "leaq 2f(%rip), %rax",
            "pushq %rax",
            "lretq",
            "2:",
            "movw $0x10, %ax",
            "movw %ax, %ds",
            "movw %ax, %es",
            "movw %ax, %ss",
            "movw %ax, %fs",
            "movw %ax, %gs",
            out("rax") _,
            options(att_syntax)
        );
        asm!("lidt [{i}]", i = in(reg) &i, options(readonly, nostack, preserves_flags));
    }
}

/// The selector currently in CS, so the boot log can show the reload took.
pub fn cs() -> u16 {
    let v: u16;
    // SAFETY: reads a segment register.
    unsafe { asm!("mov {0:x}, cs", out(reg) v, options(nomem, nostack, preserves_flags)) };
    v
}

// ------------------------------------------------------------------------------ IDT --
#[repr(C)]
#[derive(Clone, Copy)]
struct Gate {
    off_lo: u16,
    sel: u16,
    ist: u8,
    attr: u8,
    off_mid: u16,
    off_hi: u32,
    zero: u32,
}
const EMPTY: Gate = Gate { off_lo: 0, sel: 0, ist: 0, attr: 0, off_mid: 0, off_hi: 0, zero: 0 };
static mut IDT: [Gate; 256] = [EMPTY; 256];
/// Core 1's IDT: the boot core's, but with no IST for NMI, #DB and #MC (core 1 has no task register, so it has no
/// IST stacks to switch to; it never runs ring 3, so it never needs them).
static mut IDT_AP: [Gate; 256] = [EMPTY; 256];

// Interrupt vector map (architect, 12 Nov 2026; pick a new vector from here):
//   0x00-0x1F  CPU exceptions
//   0x20-0x2F  legacy lines, 0x20 + IRQ 0-15 (0x20 the local-APIC timer, 0x24 COM1 = IRQ 4, 0x2C PS/2 aux = IRQ 12;
//              0x2E would be IRQ 14, the primary IDE line)
//   0x40       the inter-processor interrupt (smp.rs)
//   0x50-0x6F  MSI and MSI-X devices, one per driver: 0x50 NVMe
//   0x80       system call
//   0xFF       spurious
pub const TIMER_VECTOR: u64 = 32;
pub const SPURIOUS_VECTOR: u64 = 255;
/// COM1's receive interrupt (ISA IRQ 4), routed here through the IOAPIC.
pub const COM1_VECTOR: u64 = 36;
/// The i8042's aux port (ISA IRQ 12: the PS/2 mouse or touchpad), routed through the IOAPIC.
pub const PS2_VECTOR: u64 = 44;
/// The NVMe controller's MSI-X completions (nvme.rs), the first MSI vector.
pub const NVME_VECTOR: u64 = 0x50;

// One stub per vector: push a dummy error code where the CPU does not push one, push
// the vector number, and join the common path, which saves every general register,
// calls `pc_trap` with a pointer to the frame, and returns with `iretq`.
global_asm!(
    r#"
.macro ISR_NOERR n
isr\n:
    push 0
    push \n
    jmp isr_common
.endm
.macro ISR_ERR n
isr\n:
    push \n
    jmp isr_common
.endm
ISR_NOERR 0
ISR_NOERR 1
ISR_NOERR 2
ISR_NOERR 3
ISR_NOERR 4
ISR_NOERR 5
ISR_NOERR 6
ISR_NOERR 7
ISR_ERR   8
ISR_NOERR 9
ISR_ERR   10
ISR_ERR   11
ISR_ERR   12
ISR_ERR   13
ISR_ERR   14
ISR_NOERR 15
ISR_NOERR 16
ISR_ERR   17
ISR_NOERR 18
ISR_NOERR 19
ISR_NOERR 20
ISR_ERR   21
ISR_NOERR 22
ISR_NOERR 23
ISR_NOERR 24
ISR_NOERR 25
ISR_NOERR 26
ISR_NOERR 27
ISR_NOERR 28
ISR_ERR   29
ISR_ERR   30
ISR_NOERR 31
ISR_NOERR 32
ISR_NOERR 36
ISR_NOERR 44
ISR_NOERR 64
ISR_NOERR 80
ISR_NOERR 128
ISR_NOERR 255

isr_common:
    // G2 part 3: ring 3 can set RFLAGS.AC itself (popfq), and neither int 0x80 nor an interrupt or exception gate
    // clears it, so a trap would run with SMAP off. Every entry from ring 3 (int 0x80, the timer and devices, the
    // exceptions, syscall_entry) comes through here before any kernel code runs; the stubs before it only push to
    // the kernel's own stack. pushfq/popfq, not clac: clac is #UD on a CPU without SMAP. iretq restores the
    // program's own flags.
    pushfq
    and qword ptr [rsp], -0x40001
    popfq
    push rax
    push rcx
    push rdx
    push rbx
    push rbp
    push rsi
    push rdi
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15
    mov rdi, rsp
    cld
    call pc_trap
// Entered directly (rsp = a saved frame) to start a process: see user_enter_frame.
.global isr_restore
isr_restore:
    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rdi
    pop rsi
    pop rbp
    pop rbx
    pop rdx
    pop rcx
    pop rax
    add rsp, 16
    iretq

.data
.balign 8
.global isr_table
isr_table:
    .quad isr0, isr1, isr2, isr3, isr4, isr5, isr6, isr7
    .quad isr8, isr9, isr10, isr11, isr12, isr13, isr14, isr15
    .quad isr16, isr17, isr18, isr19, isr20, isr21, isr22, isr23
    .quad isr24, isr25, isr26, isr27, isr28, isr29, isr30, isr31
    .quad isr32, isr255, isr128, isr36, isr64, isr44, isr80
.text
"#
);

extern "C" {
    static isr_table: [u64; 39];
}

/// The frame `isr_common` builds: the saved registers, then what the stub and the CPU
/// pushed.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TrapFrame {
    pub r15: u64, pub r14: u64, pub r13: u64, pub r12: u64,
    pub r11: u64, pub r10: u64, pub r9: u64, pub r8: u64,
    pub rdi: u64, pub rsi: u64, pub rbp: u64, pub rbx: u64,
    pub rdx: u64, pub rcx: u64, pub rax: u64,
    pub vector: u64, pub error: u64,
    pub rip: u64, pub cs: u64, pub rflags: u64, pub rsp: u64, pub ss: u64,
}

/// The system-call vector: the one gate a ring-3 program may raise with `int`.
pub const SYSCALL_VECTOR: u64 = 0x80;

fn set_gate(v: usize, handler: u64) {
    // present, 64-bit interrupt gate; DPL 0 (ring 3 `int n` faults) except the syscall gate
    let attr = if v as u64 == SYSCALL_VECTOR { 0xEE } else { 0x8E };
    // SAFETY: single core, interrupts masked while the IDT is being built.
    unsafe {
        IDT[v] = Gate {
            off_lo: handler as u16,
            sel: KERNEL_CS,
            ist: 0,
            attr,
            off_mid: (handler >> 16) as u16,
            off_hi: (handler >> 32) as u32,
            zero: 0,
        };
    }
}

pub fn load_idt() {
    // SAFETY: isr_table is filled by the assembler above; IDT is 'static.
    unsafe {
        let t = &*core::ptr::addr_of!(isr_table);
        for v in 0..33 {
            set_gate(v, t[v]);
        }
        set_gate(SPURIOUS_VECTOR as usize, t[33]);
        set_gate(SYSCALL_VECTOR as usize, t[34]);
        set_gate(COM1_VECTOR as usize, t[35]);
        set_gate(crate::smp::IPI_VECTOR as usize, t[36]);
        set_gate(PS2_VECTOR as usize, t[37]);
        set_gate(NVME_VECTOR as usize, t[38]);
        // F8: the double fault runs on IST 1 (TSS._ist[0], set in the GDT/TSS set-up), never on the stack that
        // overflowed.
        IDT[8].ist = 1;
        IDT_AP = IDT;
        IDT_AP[2].ist = 0;
        IDT_AP[1].ist = 0;
        IDT_AP[18].ist = 0;
        // Daraja (AFR-0001 7.7): on the boot core, NMI, #DB and #MC run on IST 2, 3, 4 (load_gdt sets the stacks).
        IDT[2].ist = 2;
        IDT[1].ist = 3;
        IDT[18].ist = 4;
        let p = DescPtr {
            limit: (core::mem::size_of::<[Gate; 256]>() - 1) as u16,
            base: core::ptr::addr_of!(IDT) as u64,
        };
        asm!("lidt [{p}]", p = in(reg) &p, options(readonly, nostack, preserves_flags));
    }
}

pub static BREAKPOINTS: AtomicU64 = AtomicU64::new(0);
pub static TICKS: AtomicU64 = AtomicU64::new(0);

const NAMES: [&str; 32] = [
    "divide error", "debug", "NMI", "breakpoint", "overflow", "bound range", "invalid opcode",
    "device not available", "double fault", "coprocessor overrun", "invalid TSS",
    "segment not present", "stack fault", "general protection", "page fault", "reserved",
    "x87 FP", "alignment check", "machine check", "SIMD FP", "virtualization",
    "control protection", "reserved", "reserved", "reserved", "reserved", "reserved",
    "reserved", "hypervisor injection", "VMM communication", "security", "reserved",
];

fn cr2() -> u64 {
    let v: u64;
    // SAFETY: reads the faulting address register; no side effects.
    unsafe { asm!("mov {}, cr2", out(reg) v, options(nomem, nostack)) };
    v
}

#[no_mangle]
extern "sysv64" fn pc_trap(f: &mut TrapFrame) {
    match f.vector {
        3 => {
            BREAKPOINTS.fetch_add(1, Ordering::Relaxed);
        }
        // core 1's own timer drives its two threads (smp.rs); the rest is the boot core's
        TIMER_VECTOR if crate::smp::on_core1() => {
            lapic_eoi();
            crate::smp::core1_tick(f);
        }
        TIMER_VECTOR => {
            TICKS.fetch_add(1, Ordering::Relaxed);
            lapic_eoi();
            crate::virtio::emu_poll(); // frames the Intel NIC received -> the driver's RX ring
            crate::xhci::poll(); // USB keyboard reports -> console input
            crate::ps2::poll(); // PS/2 (built-in laptop) keyboard -> console input
            crate::screen::tick(TICKS.load(Ordering::Relaxed)); // redraw a changed screen
            if f.cs & 3 == 3 {
                crate::proc::preempt(f); // a ring-3 time slice ended
            }
        }
        SPURIOUS_VECTOR => {}
        crate::smp::IPI_VECTOR => {
            crate::smp::ipi();
            lapic_eoi();
        }
        COM1_VECTOR => {
            crate::uart::rx_irq();
            lapic_eoi();
        }
        PS2_VECTOR => {
            crate::ps2::aux_irq(); // a touchpad byte: taken as it lands, not at the next tick
            lapic_eoi();
        }
        NVME_VECTOR => {
            crate::nvme::irq(); // counts it; the waiting command reads its completion itself
            lapic_eoi();
        }
        // Daraja (AFR-0001 7.7): an entry by the `syscall` instruction (syscall_entry), boot core only
        SYSCALL_INSN_VECTOR => {
            if crate::smp::on_core1() {
                crate::uart::emergency_print(format_args!(
                    "\n[pc] syscall_entry ran on core 1, whose scratch slot it does not own (AFR-0001 7.7); halted\n"
                ));
                halt_forever();
            }
            if crate::proc::active() {
                crate::proc::syscall_insn(f);
            } else {
                // the single-task ring 3 of user.rs has no foreign programs: an invalid opcode, as before SCE
                crate::user::fault(6, 0, f.rip.wrapping_sub(2), 0);
            }
        }
        // a `syscall` run with TF set: the single-step trap lands on syscall_entry's first instruction, in ring 0, on
        // IST 3. SFMASK has already cleared TF in the flags it saved, so returning resumes the entry normally and the
        // kernel is not stepped.
        1 if f.cs & 3 == 0 && f.rip == syscall_entry_addr() => {
            DB_AT_ENTRY.fetch_add(1, Ordering::Relaxed);
        }
        SYSCALL_VECTOR if crate::proc::active() => crate::proc::syscall(f),
        SYSCALL_VECTOR => crate::user::syscall(f),
        // a page fault the kernel's own tables can serve (the demand window): retry
        14 if crate::mmu::page_fault(cr2(), f.error) => {}
        // F8: the kernel stack ran into its guard. Usually a #DF (the #PF frame could not be pushed with RSP in
        // the guard; gate 8 runs on IST 1), sometimes the #PF itself (RSP above the guard, a store below it).
        // Reported lock-free: the fault may have landed inside a print that holds PRINT_LOCK.
        8 | 14 if f.cs & 3 == 0 && crate::stackinfo::in_guard(cr2()) => {
            let (lo, hi) = crate::stackinfo::guard();
            crate::uart::emergency_print(format_args!(
                "\n[pc] STACK OVERFLOW: kernel stack ran into its guard {:#x}..{:#x} (vector {} {}, CR2 {:#x}, RIP {:#x}); halted\n",
                lo,
                hi,
                f.vector,
                NAMES[f.vector as usize],
                cr2(),
                f.rip
            ));
            halt_forever();
        }
        // G2 part 3: SMAP stopped the kernel touching a ring-3 page outside pc/uaccess.rs. Named, then halted: the
        // code that did it skipped the one door for user memory.
        14 if crate::uaccess::smap_violation(f.cs, f.error, cr2()) => {
            crate::kprintln!(
                "[pc] SMAP stopped a kernel access to ring-3 memory at {:#x} (RIP {:#x}): only pc/uaccess.rs may touch it; halted",
                cr2(),
                f.rip
            );
            halt_forever();
        }
        // any other fault raised by a ring-3 program stops that program, not the machine
        v if f.cs & 3 == 3 && crate::proc::active() => crate::proc::fault(f, v, cr2()),
        v if f.cs & 3 == 3 => crate::user::fault(v, f.error, f.rip, cr2()),
        v => {
            let cr2: u64;
            // SAFETY: reads the faulting address register (meaningful for #PF).
            unsafe { asm!("mov {}, cr2", out(reg) cr2, options(nomem, nostack)) };
            crate::kprintln!(
                "[pc] EXCEPTION vector {} ({}) error {:#x} at RIP {:#x} (CR2 {:#x}); halted",
                v,
                NAMES.get(v as usize).copied().unwrap_or("interrupt"),
                f.error,
                f.rip,
                cr2
            );
            halt_forever();
        }
    }
}

// ---------------------------------------------------------------------- local APIC --
// xAPIC (MMIO at the base in IA32_APIC_BASE) or x2APIC (MSRs from 0x800), whichever
// mode the firmware left the core in: QEMU's OVMF leaves xAPIC, many real PCs x2APIC.
const IA32_APIC_BASE: u32 = 0x1B;
const REG_EOI: u32 = 0xB0;
const REG_SVR: u32 = 0xF0;
const REG_LVT_TIMER: u32 = 0x320;
const REG_TIMER_INIT: u32 = 0x380;
const REG_TIMER_DIV: u32 = 0x3E0;

static mut APIC_MMIO: u64 = 0;
static mut X2APIC: bool = false;

fn lapic_write(reg: u32, v: u32) {
    // SAFETY: APIC registers; the mode was read from IA32_APIC_BASE by `lapic_init`.
    unsafe {
        if X2APIC {
            wrmsr(0x800 + (reg >> 4), v as u64);
        } else {
            core::ptr::write_volatile((APIC_MMIO + reg as u64) as *mut u32, v);
        }
    }
}

fn lapic_read(reg: u32) -> u32 {
    // SAFETY: as lapic_write.
    unsafe {
        if X2APIC {
            rdmsr(0x800 + (reg >> 4)) as u32
        } else {
            core::ptr::read_volatile((APIC_MMIO + reg as u64) as *const u32)
        }
    }
}

/// This core's local-APIC id (the IOAPIC's destination for its interrupts).
pub fn lapic_id() -> u32 {
    // SAFETY: reads only.
    if unsafe { X2APIC } { lapic_read(0x20) } else { lapic_read(0x20) >> 24 }
}

// ------------------------------------------------------------------------- IOAPIC --
// QEMU q35 (and PCs generally) put the IOAPIC at 0xFEC00000, inside the uncached APIC
// block the kernel maps. ISA IRQ 4 (COM1) arrives on its input 4.
const IOAPIC_DEFAULT: u64 = 0xFEC0_0000; // when there is no MADT (acpi.rs gives the real one)

fn ioapic_write(reg: u32, v: u32) {
    // SAFETY: the IOAPIC's index/data window.
    unsafe {
        let io = match crate::acpi::ioapic() {
            0 => IOAPIC_DEFAULT,
            a => a,
        };
        core::ptr::write_volatile(io as *mut u32, reg);
        core::ptr::write_volatile((io + 0x10) as *mut u32, v);
    }
}

/// Route ISA IRQ `irq` (edge, active high) to `vector` on this core. Call after `lapic_init`.
pub fn ioapic_route(irq: u32, vector: u64) {
    ioapic_write(0x10 + 2 * irq + 1, lapic_id() << 24);
    ioapic_write(0x10 + 2 * irq, vector as u32); // fixed delivery, physical, unmasked
}

fn lapic_eoi() {
    lapic_write(REG_EOI, 0);
}

/// Mask the legacy 8259 PICs, enable the local APIC, and start its timer in periodic
/// mode on `TIMER_VECTOR`. Returns (x2apic?, base) for the boot log.
pub fn lapic_init(initial_count: u32) -> (bool, u64) {
    // SAFETY: port writes to the 8259 mask registers; MSR read of the APIC base.
    unsafe {
        outb(0x21, 0xFF);
        outb(0xA1, 0xFF);
        let base = rdmsr(IA32_APIC_BASE);
        X2APIC = base & (1 << 10) != 0;
        APIC_MMIO = base & 0xF_FFFF_F000;
        // Make sure the APIC is globally enabled (bit 11), keeping the mode as found.
        wrmsr(IA32_APIC_BASE, base | (1 << 11));
    }
    lapic_write(REG_SVR, 0x100 | SPURIOUS_VECTOR as u32); // software-enable
    lapic_write(REG_TIMER_DIV, 0x3); //                     divide by 16
    lapic_write(REG_LVT_TIMER, TIMER_VECTOR as u32 | (1 << 17)); // periodic
    lapic_write(REG_TIMER_INIT, initial_count);
    // SAFETY: plain reads of values set just above.
    unsafe { (X2APIC, APIC_MMIO) }
}

/// Enable this core's local APIC in the mode the boot core found (`lapic_init` ran first).
pub fn lapic_enable_here() {
    // SAFETY: this core's APIC-base MSR; the mode bit copied from the boot core's.
    unsafe {
        let base = rdmsr(IA32_APIC_BASE);
        let x2 = if X2APIC { 1 << 10 } else { 0 };
        wrmsr(IA32_APIC_BASE, base | (1 << 11) | x2);
    }
    lapic_write(REG_SVR, 0x100 | SPURIOUS_VECTOR as u32);
}

/// Start this core's local-APIC timer, periodic, on `TIMER_VECTOR`.
pub fn lapic_timer_start(initial_count: u32) {
    lapic_write(REG_TIMER_DIV, 0x3);
    lapic_write(REG_LVT_TIMER, TIMER_VECTOR as u32 | (1 << 17));
    lapic_write(REG_TIMER_INIT, initial_count);
}

const REG_ICR_LO: u32 = 0x300;
const REG_ICR_HI: u32 = 0x310;

/// IPIs whose delivery was still pending after 10 ms (xAPIC only; x2APIC has no pending bit).
pub static ICR_STUCK: AtomicU64 = AtomicU64::new(0);

/// Send an inter-processor interrupt: `lo` is the ICR's low word, `dest` the target APIC id.
fn icr(dest: u32, lo: u32) {
    // SAFETY: the interrupt-command register; xAPIC takes the destination in the high word
    // first, x2APIC the whole command in one MSR write.
    unsafe {
        if X2APIC {
            wrmsr(0x830, (dest as u64) << 32 | lo as u64);
        } else {
            lapic_write(REG_ICR_HI, dest << 24);
            lapic_write(REG_ICR_LO, lo);
            // delivery pending (bit 12): waited by the clock (10 ms), not by a count of reads
            // (before 12 Nov: a million reads, so the time allowed depended on the CPU and on
            // how slow its APIC reads were). An IPI still pending then is counted, not waited on.
            let (t0, hz) = (crate::timer::cycles(), crate::timer::hz().max(1));
            while lapic_read(REG_ICR_LO) & (1 << 12) != 0 {
                if crate::timer::cycles().wrapping_sub(t0) > hz / 100 {
                    ICR_STUCK.fetch_add(1, Ordering::Relaxed);
                    break;
                }
                core::hint::spin_loop();
            }
        }
    }
}

/// Wake every other core: INIT, 10 ms, then STARTUP twice (the Intel MP sequence), each
/// starting the core in real mode at `page` (below 1 MiB, 4 KiB aligned).
pub fn send_init_sipi(page: u64, dest: u32) {
    // ONE core, by its local-APIC id (acpi.rs): a broadcast would start every core of a
    // real many-core machine on the same start-up stack at once
    icr(dest, 0x4500); // INIT, assert
    crate::timer::busywait_ms(10);
    for _ in 0..2 {
        icr(dest, 0x4600 | (page >> 12) as u32); // STARTUP
        crate::timer::busywait_ms(1);
    }
}

/// A fixed interrupt `vector` to the core whose local-APIC id is `dest`.
pub fn send_ipi(dest: u32, vector: u64) {
    icr(dest, vector as u32 | 1 << 14);
}

pub fn lapic_timer_stop() {
    lapic_write(REG_LVT_TIMER, 1 << 16); // masked
    lapic_write(REG_TIMER_INIT, 0);
}
