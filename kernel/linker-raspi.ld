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

    /* Boot stack: grows down from __stack_top. */
    . = ALIGN(16);
    . += 0x80000;
    __stack_top = .;

    /* Per-core stack for a secondary CPU (SMP; used on virt, defined for both). */
    . = ALIGN(16);
    . += 0x10000;
    __stack_top_secondary = .;

    . = ALIGN(0x1000);
    __kernel_end = .;

    /* Total in-memory footprint, for the arm64 Image header's image_size field. */
    __image_size = __kernel_end - RAM_START;

    /DISCARD/ : { *(.comment) *(.note.*) *(.eh_frame*) }
}
