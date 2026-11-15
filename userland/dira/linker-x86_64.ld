/* Dira on the PC (ring 3): linked at the PC user code VA (mmu::USER_BASE), where the kernel maps a process's code.
 * Code and read-only data only, at most 128 KiB (the code cap for a program spawned with grants, below the data page
 * at USER_BASE + 0x20000). Dira has no writable statics: its buffers live in a memory object it maps at start. The
 * ASSERTs make a writable static a link error instead of silently discarded bytes. */
ENTRY(_start)

SECTIONS
{
    . = 0x10000000000;

    .text : {
        KEEP(*(.text.entry))
        *(.text .text.*)
    }
    .rodata : { *(.rodata .rodata.*) *(.data.rel.ro .data.rel.ro.*) }
    .got : { *(.got .got.plt) }
    .dynamic : { *(.dynamic) }
    .rela.dyn : { *(.rela.dyn .rela.*) }
    .dynsym : { *(.dynsym) }
    .dynstr : { *(.dynstr) }
    .hash : { *(.hash) }
    .gnu.hash : { *(.gnu.hash) }
    .data : { *(.data .data.*) }
    .bss : { *(.bss .bss.* COMMON) }

    ASSERT(SIZEOF(.data) == 0, "Dira has a writable static (.data): it would be lost; use the memory object")
    ASSERT(SIZEOF(.bss) == 0, "Dira has a writable static (.bss): it would be lost; use the memory object")
    ASSERT(. <= 0x10000020000, "Dira is past its 128 KiB code cap")

    /DISCARD/ : { *(.comment) *(.note*) *(.eh_frame*) }
}
