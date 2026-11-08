/* Link at the user code VA (0x8000_0000) — where the kernel maps process code —
 * so no PIC is needed. Code + rodata only (no writable statics; the app uses
 * only the stack for the interpreter's value stack and locals). */
ENTRY(_start)

SECTIONS
{
    . = 0x80000000;

    .text : {
        KEEP(*(.text.entry))
        *(.text .text.*)
    }
    .rodata : { *(.rodata .rodata.*) }

    /DISCARD/ : {
        *(.data*) *(.bss*) *(.got*) *(.comment) *(.note*) *(.eh_frame*) *(.ARM.*)
    }
}
