/* Mansa kernel image layout for Raspberry Pi 4 (BCM2711 / real hardware).
 *
 * This is docs/15-hardware-bringup.md "Suggested order" item 1's flat-binary
 * half: RPi4 DRAM starts at 0x0 (see kernel/src/board.rs BCM2711.ram_base), and
 * the VideoCore firmware (start4.elf, reading config.txt) loads a flat
 * `kernel8.img` at a FIXED physical address rather than honoring the arm64
 * Image header's `text_offset` the way U-Boot/Linux do — the default load
 * address for a 64-bit `kernel8.img` is 0x8_0000, which this table's own prose
 * already names ("loads `kernel8.img` (a flat binary) to `0x8_0000`").
 * Linking here means an `objcopy -O binary` of this ELF lands at the address
 * the firmware actually jumps to, with no relocation step in between.
 *
 * This link address is necessary but not sufficient for a real boot: GIC-400
 * init (item 3) and the mailbox framebuffer (item 4) are still open, and this
 * image has never been tried on a real board or in QEMU (no QEMU rpi4 UART
 * path has been established in this tree — see docs/15). What this script
 * buys is a flat binary shaped like what the firmware expects to load, checked
 * at compile time (entry point, image size) — not a claim of a real boot.
 */
ENTRY(_start)

RAM_START = 0x80000;

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

    /* The stack area (F7, docs/design-f7-stack-guard.md; the same block as kernel/linker.ld,
     * because board-rpi4 shares paging::init): one 2 MiB-aligned region holding both boot
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
