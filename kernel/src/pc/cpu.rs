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
    }
}

/// A second core: the same GDT and IDT as the boot core (no task register: core 1 never
/// runs ring 3, so it needs no RSP0).
pub fn load_gdt_ap() {
    let p = DescPtr {
        limit: (core::mem::size_of::<[u64; 7]>() - 1) as u16,
        base: core::ptr::addr_of!(GDT) as u64,
    };
    let i = DescPtr {
        limit: (core::mem::size_of::<[Gate; 256]>() - 1) as u16,
        base: core::ptr::addr_of!(IDT) as u64,
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

pub const TIMER_VECTOR: u64 = 32;
pub const SPURIOUS_VECTOR: u64 = 255;
/// COM1's receive interrupt (ISA IRQ 4), routed here through the IOAPIC.
pub const COM1_VECTOR: u64 = 36;

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
ISR_NOERR 64
ISR_NOERR 128
ISR_NOERR 255

isr_common:
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
    .quad isr32, isr255, isr128, isr36, isr64
.text
"#
);

extern "C" {
    static isr_table: [u64; 37];
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
        SYSCALL_VECTOR if crate::proc::active() => crate::proc::syscall(f),
        SYSCALL_VECTOR => crate::user::syscall(f),
        // a page fault the kernel's own tables can serve (the demand window): retry
        14 if crate::mmu::page_fault(cr2(), f.error) => {}
        // any other fault raised by a ring-3 program stops that program, not the machine
        v if f.cs & 3 == 3 && crate::proc::active() => crate::proc::fault(f, v, cr2()),
        v if f.cs & 3 == 3 => crate::user::fault(v, f.error, f.rip, cr2()),
        v => {
            let cr2: u64;
            // SAFETY: reads the faulting address register (meaningful for #PF).
            unsafe { asm!("mov {}, cr2", out(reg) cr2, options(nomem, nostack)) };
            crate::kprintln!(
                "[pc] EXCEPTION vector {} ({}) error {:#x} at RIP {:#x} (CR2 {:#x}) -- halted",
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
            let mut n = 0;
            while lapic_read(REG_ICR_LO) & (1 << 12) != 0 && n < 1_000_000 {
                n += 1; // delivery pending
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
