/* Link this EL0 app at the user code VA (0x8000_0000) — exactly where the kernel
 * maps a process's code — so no PIC is needed. Code + rodata only; the app uses
 * no writable statics (the RX user region can't be written), only the stack. */
ENTRY(_start)

SECTIONS
{
    . = 0x80000000;

    .text : {
        KEEP(*(.text.entry))   /* _start must be first (at 0x8000_0000) */
        *(.text .text.*)
    }
    .rodata : { *(.rodata .rodata.*) }

    /DISCARD/ : {
        *(.data*) *(.bss*) *(.got*) *(.comment) *(.note*) *(.eh_frame*) *(.ARM.*)
    }
}
