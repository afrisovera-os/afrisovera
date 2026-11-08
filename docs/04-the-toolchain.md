# 04 — The Toolchain ("the best tools")

The brief asked to build "using the best tools." Here is the deliberate technology selection, with the reasoning, because tool choice *is* architecture for an OS.

## Languages

| Layer | Language | Why |
|---|---|---|
| Kernel (Mansa) | **Rust** | Memory safety with no GC, predictable latency, fearless concurrency; kills ~70% of OS CVE classes. Proven viable by Redox + Rust-in-Linux/Android/Windows. |
| Drivers & services (Asili, AfriCore) | **Rust** | Same safety guarantees where a crash = a broken phone; clean async IPC. |
| App layer (Nia) | **Any language → WASM** | Don't lock developers into one language (the mistake of Symbian/WP). Rust/C/Go/Kotlin/Swift/JS/Python all target WASM. |
| Build glue / tooling | **Rust + a little Python** | One toolchain for contributors; Python only for scripts. |

## Core platform technologies

- **WebAssembly + WASI** — the universal app binary format. Portable across CPU arch, sandboxed by construction, tiny binaries (good for cheap storage/data). The genuinely modern app substrate.
- **Vulkan** — the graphics API under the Sankofa UI compositor and rendering engine. Low-overhead, explicit, runs on the mobile GPUs we target.
- **AfriIDL** — our **interface definition language** (heritage: Fuchsia FIDL, Cap'n Proto). Every IPC interface — kernel objects, driver ABI, service APIs — is declared once in AfriIDL and code-generated into every language, giving **typed, versioned, evolvable contracts**. This is what makes a microkernel-everything-is-IPC system maintainable.
- **A copy-on-write filesystem** — integrity, cheap snapshots, atomic A/B OS updates with rollback.

## Reference / design lineage (stand on giants, don't reinvent specs)

| We learn from | What we take |
|---|---|
| **seL4** | Capability model, fast IPC, formal-verification path |
| **Zircon / Fuchsia** | Userspace drivers, stable driver ABI, handle/object model, FIDL→AfriIDL |
| **Redox OS** | Proof a Rust microkernel OS is buildable; userland patterns |
| **KaiOS / Firefox OS** | Web-as-apps shipping at scale in our exact markets |
| **Waydroid / Anbox / libhybris** | Embedding an Android runtime & reusing Android drivers |
| **Flutter / Skia / vello** | Declarative UI + portable GPU rendering engine |

We reuse open **standards and libraries** (LLVM, the Rust toolchain, a web engine, crypto/TLS libraries, the Vulkan loader). Rewriting those would make the project unfinishable and buys nothing. "From scratch" means **the OS is ours** — kernel, drivers, services, runtime, UI, SDK — not that we re-derive compilers.

## Build, CI & dev infrastructure

- **Cargo workspaces** + a thin meta-build orchestrating kernel + userland + images.
- **Reproducible, hermetic builds** producing signed, A/B-updatable OS images.
- **Emulator-first CI**: every commit boots the full OS in **QEMU** (AArch64 + x86-64) and runs an automated smoke/test suite. Developers — and app developers — never wait on physical hardware.
- **Target hardware ladder:** QEMU → SBCs (Raspberry Pi / PINE64 / PinePhone-class) → one reference dev phone → partner-certified handsets. We control the board at each rung before climbing.
- **Open source core** with a public AfriIDL, SDK, and emulator image, so community device-enablement and app development can start early.

## One-line tech summary
**Rust microkernel + userspace Rust drivers/services + WASM/WASI apps + a Vulkan-rendered declarative UI + an FIDL-style IDL — the stack you'd choose in 2026 if you got to start clean, biased throughout toward safety, light footprint, and a non-empty app store on day one.**
