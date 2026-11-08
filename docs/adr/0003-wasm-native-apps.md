# ADR 0003 — Native apps are WebAssembly modules (Nia)

- **Status:** Accepted
- **Date:** 2026-08-05 (Phase 2.0–2.3, commits `e6d5dc4`, `8d874a8`, `3fc19cb`)
- **Deciders:** owner; recorded by the chief architect 2026-11-08
- **Related:** ADR 0004 (capabilities), spec [AFR-0003](../spec/0003-nia-package-and-permissions.md),
  docs/03-app-runtime-and-sdk.md, docs/06-universal-app-compatibility.md

## Context

A new OS dies without apps (Symbian, BlackBerry 10, Windows Phone, Firefox OS). Developers will not learn
a mandated language for a platform with no users. The same apps must run on ARM phones and on x86-64 PCs
(ADR 0006) without two builds. Apps must be sandboxed so strongly that a malicious app holds no authority
it was not given (ADR 0004). Target devices have 1–2 GB of RAM and costly data, so binaries must be small.

## Decision

The native application format is a **WebAssembly 1.0 module**, executed by **Nia**, AfriSovera's own WASM
engine inside the system. An app reaches the system only through **imported host functions** (matched by
field name today; the import module is fixed as `afri` from package format v1); each host function is
gated by a permission the app was granted (spec AFR-0003). There is
no other way out of the sandbox: no raw syscalls, no shared memory with the system, no native code.

The same `.wasm` bytes run unchanged on AArch64 and x86-64. Web/PWA apps and the Daraja Android bridge are
the other two app classes (docs/03); they are out of scope of this ADR.

## Consequences

Good:
- Architecture independence: one artefact for phone and PC.
- Language independence: Rust, C/C++, Zig, AssemblyScript, TinyGo, … all target WASM.
- The sandbox is structural: linear memory is bounds-checked, control flow is validated, imports are the
  entire attack surface, and that surface is a short, reviewable list.
- Small binaries (the bundled demo apps are a few hundred bytes to a few KiB).

Bad / accepted costs:
- Performance: Nia is an interpreter today; the AOT-on-install compiler in docs/03 is designed, not built.
- WASI is not implemented; the host API is AfriSovera's own imports, so off-the-shelf WASI programs
  do not run yet. A WASI adapter is a future layer.
- Today's VFS stores at most 512 bytes per file, so an installed app is limited to 512 bytes until the
  storage format grows (AFR-0003 §4.4).
- Threads, SIMD and GC proposals are not supported.
- Nia runs in the kernel's context today (not as a separate EL0/ring-3 process); the WASM validator and
  interpreter are therefore part of the TCB until the runtime moves to user space.

## Evidence in the tree

- `kernel/src/nia.rs` — decoder, validator, interpreter, host imports, permission gate.
- `kernel/src/niavm.bin`, `kernel/src/pc/niavm-x86_64.bin` — the Nia VM as a user program.
- `kernel/src/store.rs` — install/launch of `.wasm` apps from the VFS with integrity check.

## Alternatives considered

| Option | Why not |
|---|---|
| Native ELF apps per architecture | Two builds, and native code needs an MMU sandbox per app plus a vetted ABI — more TCB, weaker default. |
| JVM/Dalvik-style bytecode | Ties apps to one language family and a GC runtime. |
| JavaScript only (KaiOS model) | Kept as the Web/PWA class, but too slow and too large for system-grade apps on 1 GB devices. |
| eBPF-style restricted bytecode | Not general enough for applications. |

## Revisit if

- Interpreter speed blocks a real app class (games, media) before AOT exists — then prioritise AOT, not a
  native-code escape hatch.
- The WASM component model stabilises in a way that would let AfriIDL be expressed as WIT (spec AFR-0004
  keeps that door open).
