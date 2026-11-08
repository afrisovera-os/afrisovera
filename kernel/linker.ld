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

    /* Boot stack: grows down from __stack_top. 512 KiB is plenty for Phase 0. */
    . = ALIGN(16);
    . += 0x80000;
    __stack_top = .;

    /* Per-core stack for a secondary CPU brought up via PSCI (SMP). */
    . = ALIGN(16);
    . += 0x10000;
    __stack_top_secondary = .;

    /* First free physical address after the kernel image — the physical frame
     * allocator manages RAM from here up to the end of DRAM. */
    . = ALIGN(0x1000);
    __kernel_end = .;

    /* Total in-memory footprint, for the arm64 Image header's image_size field. */
    __image_size = __kernel_end - RAM_START;

    /DISCARD/ : { *(.comment) *(.note.*) *(.eh_frame*) }
}
