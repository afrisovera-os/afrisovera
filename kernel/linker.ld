/* Mansa kernel image layout for QEMU `-M virt` (AArch64).
 * virt places DRAM at 0x4000_0000; QEMU's `-kernel <elf>` honors these
 * LOAD addresses and jumps to ENTRY. Keep `.text.boot` first so `_start`
 * is the very first instruction the CPU executes. */
ENTRY(_start)

/* QEMU `virt` reserves the first 2 MiB of DRAM (0x4000_0000) for the device tree
 * and loads a raw arm64 `Image` at 0x4020_0000. We link there so BOTH the ELF
 * (-kernel <elf>, honors LOAD addrs) and the raw Image boot at the same address. */
RAM_START = 0x40200000;

SECTIONS
{
    . = RAM_START;

    .text : {
        KEEP(*(.text.boot))   /* _start must lead the image */
        *(.text .text.*)
    }

    .rodata : { *(.rodata .rodata.*) }
    .data   : { *(.data .data.*) }

    . = ALIGN(16);
    __bss_start = .;
    .bss : {
        *(.bss .bss.*)
        *(COMMON)
    }
    . = ALIGN(16);
    __bss_end = .;

    /* The stack area (F7, docs/design-f7-stack-guard.md): one 2 MiB-aligned region holding both boot
     * stacks, each with an UNMAPPED 64 KiB guard below it (paging::init leaves the guard pages invalid),
     * and one 16 KiB emergency stack per core that the sync vector switches to when SP is in a guard.
     * Boot stack 1 MiB since 6e00ee98: the debug-build shell::exec frame alone is 595 KiB
     * (docs/evidence-f7-stackwater.log). Everything here must stay inside one 2 MiB region (one L3 table);
     * the ASSERT below holds it to that. */
    . = ALIGN(0x200000);
    __stack_area = .;
    __boot_guard_lo = .;
    . += 0x10000;
    __boot_guard_hi = .;
    __stack_bottom = .;
    . += 0x100000;
    __stack_top = .;
    __sec_guard_lo = .;
    . += 0x10000;
    __sec_guard_hi = .;
    . += 0x10000;
    __stack_top_secondary = .;
    . += 0x4000;
    __emerg_top0 = .;
    . += 0x4000;
    __emerg_top1 = .;
    . = ALIGN(0x1000);
    __stack_area_end = .;
    ASSERT(__stack_area_end - __stack_area <= 0x200000, "F7: the stack area must fit one 2 MiB region (one L3 table)")

    /* First free physical address after the kernel image — the physical frame
     * allocator manages RAM from here up to the end of DRAM. */
    . = ALIGN(0x1000);
    __kernel_end = .;

    /* Total in-memory footprint, for the arm64 Image header's image_size field. */
    __image_size = __kernel_end - RAM_START;

    /DISCARD/ : { *(.comment) *(.note.*) *(.eh_frame*) }
}
