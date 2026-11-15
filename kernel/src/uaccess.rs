//! The ARM kernel's one door for EL0 memory (gap G2 part 2, docs/security-model.md; SECURITY_G2_PAN_PLAN.md).
//!
//! Every system call that reads or writes an EL0 buffer goes through `from_user` or `to_user`. They move bytes with
//! the UNPRIVILEGED load and store, LDTRB and STTRB, which the CPU checks with EL0's permissions. So a kernel
//! address (an EL1-only block or page) faults even if a call's own range check (process.rs `user_range`) is wrong
//! or missing. ARMv8.0 has no PAN; this works on the Cortex-A72 we test on and on the Raspberry Pi 4.
//!
//! The fault table (the architect's condition 3) is fixed at link time: one entry per helper, holding the address
//! of its own LDTRB or STTRB and of its fixup. The EL1 synchronous entry (exceptions.rs, in assembly, before any Rust runs) resumes at the fixup only
//! on a data abort taken at EL1 (EC 0x25) whose ELR is exactly one of those addresses; every other EL1 fault stays
//! fatal. A copy that faults part way is refused as a whole (condition 4): the helper returns -1, the caller
//! discards its buffer and returns EPERM, never a short length.
//!
//! UAO (ARMv8.2) would make LDTR/STTR act as EL1 (condition 2). This kernel never sets it; `init` clears it where
//! the CPU has it, and the `pan` suite (step 2) shows that a helper copy from a kernel address still faults there.

use core::arch::{asm, global_asm};

global_asm!(
    r#"
.section .text
.balign 4
// uaccess_from_user(dst: kernel *mut u8 = x0, src: EL0 address = x1, len = x2) -> 0, or -1 if a byte faulted
.global uaccess_from_user
uaccess_from_user:
    cbz x2, 2f
1:
.global uaccess_ld_insn
uaccess_ld_insn:
    ldtrb w3, [x1]
    strb w3, [x0], #1
    add x1, x1, #1
    subs x2, x2, #1
    b.ne 1b
2:  mov x0, #0
    ret
.global uaccess_ld_fix
uaccess_ld_fix:
    mov x0, #-1
    ret

// uaccess_to_user(dst: EL0 address = x0, src: kernel *const u8 = x1, len = x2) -> 0, or -1 if a byte faulted
.global uaccess_to_user
uaccess_to_user:
    cbz x2, 2f
1:  ldrb w3, [x1], #1
.global uaccess_st_insn
uaccess_st_insn:
    sttrb w3, [x0]
    add x0, x0, #1
    subs x2, x2, #1
    b.ne 1b
2:  mov x0, #0
    ret
.global uaccess_st_fix
uaccess_st_fix:
    mov x0, #-1
    ret

// The fault table: (faulting instruction, resume address) pairs, fixed at link time.
.section .rodata
.balign 8
.global uaccess_extable
uaccess_extable:
    .quad uaccess_ld_insn, uaccess_ld_fix
    .quad uaccess_st_insn, uaccess_st_fix
.global uaccess_extable_end
uaccess_extable_end:
"#
);

extern "C" {
    fn uaccess_from_user(dst: *mut u8, src: u64, len: usize) -> i64;
    fn uaccess_to_user(dst: u64, src: *const u8, len: usize) -> i64;
}

/// Read `dst.len()` bytes from EL0 memory at `ptr` (the running process's TTBR0 is loaded): all of them, or `Err(())`
/// with `dst` to be discarded by the caller.
pub fn from_user(ptr: u64, dst: &mut [u8]) -> Result<(), ()> {
    // SAFETY: `dst` is ours; every load of `ptr..ptr+len` is an LDTRB checked with EL0's permissions, and a fault
    // there resumes at the helper's fixup (exceptions.rs) instead of taking the kernel down.
    match unsafe { uaccess_from_user(dst.as_mut_ptr(), ptr, dst.len()) } {
        0 => Ok(()),
        _ => Err(()),
    }
}

/// Write `src` into EL0 memory at `ptr`: all of it, or `Err(())`. A fault part way may have written a prefix into
/// the caller's own memory (never anywhere else); the call is refused as a whole either way.
pub fn to_user(ptr: u64, src: &[u8]) -> Result<(), ()> {
    // SAFETY: as from_user, with STTRB.
    match unsafe { uaccess_to_user(ptr, src.as_ptr(), src.len()) } {
        0 => Ok(()),
        _ => Err(()),
    }
}

/// UAO off where the CPU has it (ID_AA64MMFR2_EL1.UAO, bits 7:4): with UAO set, LDTR and STTR would be checked as
/// EL1 and the door would let kernel addresses through. Says which.
pub fn init() {
    let mmfr2: u64;
    // SAFETY: reading ID_AA64MMFR2_EL1 (by its encoding, so no v8.2 assembler feature is needed) at EL1.
    unsafe { asm!("mrs {}, s3_0_c0_c7_2", out(reg) mmfr2, options(nomem, nostack)) };
    if (mmfr2 >> 4) & 0xF != 0 {
        // SAFETY: the CPU implements UAO (checked); clearing it keeps LDTR/STTR unprivileged.
        // `msr uao, #0`, encoded: no v8.2 assembler feature needed.
        unsafe { asm!(".inst 0xd500407f", options(nomem, nostack)) };
        crate::kprintln!("[cpu] uaccess: EL0 memory only through LDTR/STTR (UAO present, kept off)");
    } else {
        crate::kprintln!("[cpu] uaccess: EL0 memory only through LDTR/STTR (no UAO in this CPU)");
    }
}
