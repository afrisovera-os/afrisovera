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

    /* Boot stack: grows down from __stack_top. */
    . = ALIGN(16);
    . += 0x80000;
    __stack_top = .;

    /* Per-core stack for a secondary CPU (SMP; defined for every board, used
     * only where the boot path brings a secondary online). */
    . = ALIGN(16);
    . += 0x10000;
    __stack_top_secondary = .;

    . = ALIGN(0x1000);
    __kernel_end = .;

    /* Total in-memory footprint, for the arm64 Image header's image_size field. */
    __image_size = __kernel_end - RAM_START;

    /DISCARD/ : { *(.comment) *(.note.*) *(.eh_frame*) }
}
