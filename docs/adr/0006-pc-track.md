# ADR 0006 — x86-64 PCs and laptops are a full target

- **Status:** Accepted
- **Date:** 2026-11-03 (owner's decision, commit `aad8fc1`; first boot `589c848`)
- **Deciders:** owner; recorded by the chief architect 2026-11-08
- **Related:** ADR 0003 (WASM apps), spec [AFR-0008](../spec/0008-pc-boot-and-install.md),
  docs/05-roadmap.md "The PC track", docs/01-kernel-mansa.md §Boot & portability

## Context

Until 2026-11-03, x86-64 was an emulator/development path only; the product was ARM phones. Device
ownership — the project's core value — applies to every device a person owns, and the PC is where lock-in
is felt most. A PC is also the most open real machine available: UEFI firmware, publicly documented parts
(AHCI, NVMe, xHCI, HD Audio, e1000, SDHCI), and it can boot from a USB stick without touching the installed
system. Phones need per-SoC bring-up and closed blobs (Phase 3), which is years of work.

## Decision

x86-64 PCs and laptops are a **first-class target beside ARM**, developed as a **parallel track** now, not
after Phase 3. Specifically:

1. One source tree, two kernels: `kernel/src/main.rs` (AArch64) and `kernel/src/pc/main.rs` (x86-64, a
   UEFI application). Shared subsystems (shell, VFS, Nia, Tonga, store, network stack, crypto) are compiled
   into both by path; architecture code lives behind the board-support seam (`shell_arch.rs`,
   `kernel/src/pc/compat/`).
2. **The ARM kernel image stays byte-identical** when PC work lands (md5 recorded in the task brief and in
   SESSION_PLAN.md). A PC change that alters the ARM binary is a bug unless it is deliberate and recorded.
3. The same Nia (`.wasm`) apps run unchanged on both architectures (ADR 0003).
4. Install is **non-destructive by default**: read-only inspection first, a backup of the partition table
   to removable media before any write, an owner-typed confirmation code tied to the table's current
   contents, and a way back for every write (spec AFR-0008).
5. Both architectures are graded by the same test registry; ARM-only suites are listed with reasons in
   `scripts/lib/pc-arch/arm-only.txt`.

## Consequences

Good:
- A real machine the owner already has can run AfriSovera now; credibility does not wait on phone silicon.
- Daraja·Dirisha (Windows apps) runs at native speed on x86 later, without binary translation.
- Commodity drivers (NVMe, AHCI, xHCI, e1000/igb, RTL8169, HDA, SDHCI) exercise the Asili model on real,
  documented hardware.

Bad / accepted costs:
- A second architecture doubles the arch-specific surface: paging, interrupts (LAPIC/IOAPIC), SMP
  (INIT/SIPI), syscall entry, context switch.
- Engineering time spent on PC drivers is not spent on phone bring-up.
- Secure Boot: AfriSovera is not signed by Microsoft's UEFI CA; the owner must switch Secure Boot off or a
  shim must be signed later. This is stated to the user by `fwinfo`.
- Today PC drivers run inside the kernel (as ARM drivers do), not as user-space Asili processes — see
  ADR 0001's "partly realised" note.

## Evidence in the tree

- `kernel/src/pc/` (≈30 modules: `uefi.rs`, `gpt.rs`, `esp.rs`, `bootent.rs`, `nvme.rs`, `ahci.rs`,
  `xhci.rs`, `e1000.rs`, `rtl8169.rs`, `hda.rs`, `sdhci.rs`, `smp.rs`, `user.rs`, …).
- Suites 691–744 (`pc-*`), `scripts/run-all-tests-pc.sh`, cloud walks recorded in SESSION_PLAN.md.

## Revisit if

- Phone bring-up (Phase 3) is starved for more than one quarter because of PC work.
- Keeping the ARM image byte-identical starts forcing duplicated code rather than a clean seam.
