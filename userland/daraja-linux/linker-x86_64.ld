/* The Linux bridge (daraja-linux) on the PC (x86-64): linked at the PC user code address (0x100_0000_0000), where
 * the kernel maps a native process's code, so no relocation is needed at load. Code and read-only data only: the
 * bridge keeps no writable statics (its code region is read-only); all its state is on its stack.
 * Copied from userland/rustapp/linker-x86_64.ld. */
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
