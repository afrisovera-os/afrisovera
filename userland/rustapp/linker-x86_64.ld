/* PC (x86-64): link the ring-3 app at the PC user code VA (0x100_0000_0000) — exactly where the kernel
 * maps a process's code — so no PIC is needed. Code + rodata only; the app uses
 * no writable statics (the RX user region can't be written), only the stack. */
ENTRY(_start)

SECTIONS
{
    . = 0x10000000000;  /* the PC user code VA, mmu::USER_BASE */

    .text : {
        KEEP(*(.text.entry))   /* _start must be first (at USER_BASE) */
        *(.text .text.*)
    }
    .rodata : { *(.rodata .rodata.*) *(.data.rel.ro .data.rel.ro.*) }
    /* position-independent code reaches its constants through the GOT; linked with
     * --apply-dynamic-relocs the entries already hold their final addresses (the app is
     * only ever loaded at USER_BASE), so no loader has to relocate anything */
    .got : { *(.got .got.plt) }
    .dynamic : { *(.dynamic) }
    .rela.dyn : { *(.rela.dyn .rela.*) }
    .dynsym : { *(.dynsym) }
    .dynstr : { *(.dynstr) }
    .hash : { *(.hash) }
    .gnu.hash : { *(.gnu.hash) }

    /DISCARD/ : {
        *(.data) *(.data.[!r]*) *(.bss*) *(.comment) *(.note*) *(.eh_frame*) *(.ARM.*)
    }
}
