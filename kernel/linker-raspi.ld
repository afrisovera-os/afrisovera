/* Mansa kernel image layout for Raspberry Pi (BCM2837 / QEMU `-M raspi3b`).
 * DRAM starts at 0x0. With an arm64 Image header (text_offset=0), QEMU loads the
 * Image at RAM base + 2 MiB = 0x20_0000 (the same 2 MiB DTB reservation as virt),
 * so we link there. Keep `.text.boot` first so `_start` runs first. */
ENTRY(_start)

RAM_START = 0x200000;

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

    /* The stack area (F7, docs/design-f7-stack-guard.md; the same block as kernel/linker.ld so
     * the shared vector table links. NOTE: the board-raspi paging path keeps its own L2 and does NOT unmap
     * the guards; only the virt-path boards are guarded): one 2 MiB-aligned region holding both boot
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

    . = ALIGN(0x1000);
    __kernel_end = .;

    /* Total in-memory footprint, for the arm64 Image header's image_size field. */
    __image_size = __kernel_end - RAM_START;

    /DISCARD/ : { *(.comment) *(.note.*) *(.eh_frame*) }
}
