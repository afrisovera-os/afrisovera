# 01 — Mansa: the Microkernel

**Mansa** (after Mansa Musa — sovereignty, wealth, governance) is AfriSovera's kernel: a **capability-based microkernel written in Rust.**

## Why a microkernel, and why Rust

**Microkernel** — the kernel does the absolute minimum: address spaces, threads/scheduling, inter-process communication (IPC), and capabilities. *Everything else* — drivers, filesystems, network stack, display — runs as isolated userspace processes. Benefits that matter for a phone in a tough environment:

- **Fault isolation** — a buggy camera or modem driver crashes its own process and gets restarted; it cannot panic the device.
- **Security surface** — a tiny kernel (tens of thousands of lines, not tens of millions) is auditable and, like **seL4**, potentially *formally verifiable*.
- **Updatability** — services and drivers update independently without rebuilding the kernel.

**Rust** — memory safety without a garbage collector, fearless concurrency, zero-cost abstractions, predictable latency. It eliminates the entire class of bugs (use-after-free, buffer overflow) that account for ~70% of OS CVEs. This is *the* modern choice for a new OS; **Redox OS** proves a Rust microkernel OS is buildable, and the broader industry (Android, Windows, Linux) is already moving kernel code to Rust.

**Design heritage we deliberately borrow from:**
- **seL4** — capability model + formal verification ambition + blistering IPC.
- **Zircon (Fuchsia)** — object/handle model, userspace drivers, stable driver ABI.
- **Redox** — Rust microkernel ergonomics and userland.

## The capability model (the security foundation)

There is **no root, no `setuid`, no ambient authority.** A process can do *only* what it holds capabilities for. A capability is an **unforgeable, kernel-managed token** (a handle) granting a specific right to a specific object — "may write to this file region," "may map this MMIO range," "may send to this service."

- Capabilities are passed **explicitly** over IPC (you can only widen another process's power by handing it a token you hold).
- The launcher/app-manager hands an app exactly the capabilities it's granted — nothing more. Permissions become *real*, not advisory.
- This makes **sandbox-by-default** the natural state, not a bolted-on afterthought. An app with no network capability physically cannot touch the network.

## What lives in the kernel (and what doesn't)

| In Mansa (kernel mode) | In userspace (just a process) |
|---|---|
| Address spaces / virtual memory (MMU) | Drivers (GPU, modem, sensors, storage…) |
| Threads & scheduler | Filesystems / VFS |
| **IPC** (synchronous + async channels) | Network stack |
| Capability table & objects | Display compositor |
| Interrupt routing to userspace handlers | Power & telephony managers |
| Low-level timers | AfriCore services, app runtimes |

## IPC — the heart of the system

In a microkernel, *everything* is IPC, so it must be fast and safe. Mansa provides:

- **Channels** — bidirectional, capability-carrying message pipes (you can send data *and* handles).
- **Fast synchronous call/reply** for the common "ask a service and wait" pattern (seL4-style, optimized).
- **Shared-memory VMOs** (virtual memory objects) for bulk/zero-copy data (frame buffers, media).
- Interfaces are defined in **AfriIDL** (see [`04-the-toolchain.md`](04-the-toolchain.md)) and code-generated, so services and clients share a typed, versioned contract.

## Scheduling & power (tuned for our user)

- **Energy-aware scheduling** with explicit power/perf profiles, including a **"ration mode"** that aggressively parks background work to stretch standby through grid outages.
- Deadline/priority classes so the UI thread is never starved — perceived smoothness on weak hardware is a first-class goal.
- Memory: tight footprint targets (design center **1–2 GB RAM**), compressed-RAM swap built into the memory manager, and an OOM policy that protects the foreground app and the compositor above all.

## Boot & portability

- **Architecture:** AArch64 (ARM64) for phones, SBCs and M-series Macs, **and x86-64 for PCs and laptops as a full target** (owner's decision, 3 Nov 2026 — it was emulator/dev-path only before; see the PC track in [`05`](05-roadmap.md)).
- **Boot:** UEFI where available (SBCs, emulator, some devices); per-SoC bring-up (e.g. via a minimal bootloader like LK/U-Boot) on real handsets. Boot is the one genuinely per-device chunk and is isolated behind a thin board-support layer so the rest of the OS stays portable.

## Verification ambition
The kernel is deliberately small enough that, post-1.0, a **seL4-style formal verification** of core IPC and capability paths is a realistic stretch goal — a security claim no mainstream phone OS can make.
