# ADR 0001 — Mansa is a capability microkernel

- **Status:** Accepted — **partly realised** (see "Where the code is today")
- **Date:** 2026-08-05 (Phase 0 complete, commit `29f18b3`)
- **Deciders:** owner; recorded by the chief architect 2026-11-08
- **Related:** ADR 0002 (Rust), ADR 0004 (capabilities), specs AFR-0001 and AFR-0002, docs/01, docs/02, docs/11

## Context

The target user runs cheap phones and old PCs, often with no repair shop nearby; a crashing camera or
modem driver must not take the device down. The project's only chance against the driver gap is a
**stable driver ABI** that lets one driver binary survive OS updates, and a way to host vendor drivers
in a box. A security story of "no ambient authority" (ADR 0004) needs a kernel small enough to audit
and, later, to verify formally (seL4 precedent). A monolithic kernel gives none of these: every driver
shares the kernel's address space, ABI and fate.

## Decision

Mansa is a **microkernel**. In its final form the kernel provides only: address spaces and virtual memory,
threads and scheduling, IPC (channels, call/reply, shared-memory objects), the capability table, interrupt
delivery to user-space handlers, and low-level timers. **Drivers, file systems, the network stack, the
display server and every AfriCore service run as user-space processes**, reachable only through
capabilities and AfriIDL-typed IPC.

Development is **staged**: subsystems may be built inside the kernel first, to prove the hardware and the
protocol, and are then moved out behind the interfaces of AFR-0001 v1 and AFR-0002. Moving them out is
the plan of record, not an option.

## Where the code is today (2026-11-08)

| Microkernel property | Status |
|---|---|
| User processes in separate address spaces (EL0 / ring 3) | **Built** on both architectures |
| Capability-checked IPC between processes | **Built**, minimal: 4 caps per process, one endpoint, one word per message (AFR-0001 §6) |
| Fault containment of user processes | **Built** (`fault`, `pc-ring3`) |
| Drivers in user space | **Not built** — every driver (virtio, NVMe, AHCI, xHCI, e1000, RTL8169, HDA, SDHCI, …) is a kernel module |
| File system, network stack, compositor, shell in user space | **Not built** — `vfs.rs`, the TCP/IP stack, `tonga.rs` and `shell.rs` run in the kernel |
| WASM apps out of the kernel | **Partly** — `niavm` runs in EL0/ring 3; the full Nia interpreter still runs in the kernel |

Honest summary: Mansa today is a **kernel with microkernel mechanisms proven, running a monolithic
payload**. That is acceptable for bring-up; it is not acceptable as a 1.0 architecture, and no document
may describe the system as a running microkernel without this qualification.

## Consequences

Good:
- Fault isolation and least privilege for drivers once moved out.
- A stable ABI boundary (AFR-0002) that vendors can target.
- A TCB small enough for formal verification as a stretch goal.

Bad / accepted costs:
- IPC cost on every driver interaction; IPC must be designed for speed (call/reply fast path, shared
  memory for bulk data).
- More engineering before the first user-visible feature, which is why staging is allowed.
- The longer drivers stay in the kernel, the more their code grows assumptions (direct calls, shared
  statics, 1:1 physical addresses) that the move must undo. **Every new driver SHOULD keep its hardware
  access behind a narrow set of functions** (map BAR, read/write config, allocate DMA, wait for interrupt)
  so the move is mechanical — see AFR-0002 §6.

## Alternatives considered

| Option | Why not |
|---|---|
| Monolithic kernel (Linux model) | No fault isolation, no stable driver ABI; the driver gap stays unsolved. |
| Linux kernel + new userland (most "new" phone OSes) | Inherits Linux's driver model and GPL constraints; not "ours"; does not deliver capability security. |
| Hybrid (XNU/NT) | Keeps drivers in kernel for speed; loses the isolation we need most on cheap hardware. |
| seL4 as the kernel | Strongest option on paper; rejected for ownership, a C code base, and the effort of a verified-kernel userland. Its model is borrowed (AFR-0001). |

## Revisit if

- IPC round-trip cost on the reference phone exceeds ~2 µs after optimisation — then reconsider which
  services may share an address space, not the microkernel itself.
- By the end of Phase 3, no driver has moved to user space: the decision is then not being carried out and
  must be re-affirmed or replaced explicitly.
