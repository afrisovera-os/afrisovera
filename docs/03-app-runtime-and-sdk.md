# 03 — App Runtime, UI Framework & SDK

This layer decides whether the OS lives or dies, because it decides whether the OS has apps. AfriSovera runs **three classes of app**, deliberately, to defeat the app gap.

## The three app classes

```
   ┌──────────────┐   ┌──────────────┐   ┌───────────────────┐
   │  Web / PWA   │   │  Nia (WASM)  │   │  Daraja (Android) │
   │  apps        │   │  native apps │   │  compat apps      │
   └──────┬───────┘   └──────┬───────┘   └─────────┬─────────┘
          │                  │                     │
          ▼                  ▼                     ▼
   ┌─────────────────────────────────────────────────────────┐
   │            Sankofa UI  +  AfriCore SDK                    │
   │     (capability-secured access to system services)       │
   └─────────────────────────────────────────────────────────┘
```

### 1. Web / PWA — instant breadth (the KaiOS playbook)
A first-class web runtime (built on an existing open engine — we don't write a browser engine from scratch) makes **thousands of existing web apps work on day one**, and lets any web developer ship to AfriSovera with **zero new toolchain**. KaiOS proved this model ships at 100M+ scale in exactly our target markets. PWAs install from AfriStore as tiny, data-light "Lite Apps."

### 2. Nia (WASM/WASI) — the native, forward bet
**Nia** (Swahili: *purpose*) is the flagship native runtime. **Every native app is a WebAssembly module**, sandboxed, capability-gated by the kernel.

- **Language-agnostic:** Rust, C/C++, Go, Kotlin, Swift, JavaScript, Python, AssemblyScript — anything that targets WASM. Developers don't have to learn one mandated language (Symbian's C++ and WP's C# lock-in hurt them).
- **Tiny & portable:** WASM binaries are small (great for cheap storage / costly downloads) and CPU-architecture-independent (one build runs on ARM and x86 dev kits alike).
- **Secure:** WASM is sandboxed by construction; combined with Mansa capabilities, an app reaches *only* the system services it was granted. WASI gives a clean, portable system interface; AfriCore APIs extend it with payments, push, P2P, identity, sensors.
- **Fast enough:** AOT-compiled to native on install (no interpreter tax on the hot path).

### 3. Daraja — the Android compatibility bridge (the on-ramp that buys time)
**Daraja** (Swahili: *bridge*) runs **real, existing Android apps** in a contained translation layer — an Android runtime environment hosted as an AfriSovera service, drawing on the proven Anbox/Waydroid approach. This is **not** the OS's identity; it's the bridge that keeps users from being stranded while the native (Nia/Web) ecosystem grows. Android apps run boxed and capability-gated like everything else, and can be quietly retired as native equivalents appear.

> Strategy in one line: **Web + Daraja make the OS useful on day one; Nia + a first-class SDK make native the better place to be, so the ecosystem migrates.**

## Sankofa UI — the native UI framework

**Sankofa UI** is a **declarative, reactive, GPU-accelerated** UI framework — the developer-facing toolkit and the rendering engine.

- **Declarative & reactive** (the SwiftUI / Jetpack Compose / Flutter model): you describe the UI as a function of state; the framework diffs and redraws. This is the modern standard developers already expect.
- **Own rendering engine on Vulkan** (heritage: Flutter/Skia, Rust's `vello`): consistent pixels on every device, not dependent on per-OEM widget toolkits — solving the design-inconsistency that plagued Android.
- **Tuned for weak GPUs and 60 Hz on cheap panels**, with a strict frame budget and a **Lite Mode** that drops effects on 1–2 GB devices while staying fluid.
- **One design language** enforced across all system + first-party apps (the iOS consistency lesson).
- Bindable from any Nia language and exposed to Web apps via a component layer.

## The SDK & developer experience

- **AfriSovera SDK**: Sankofa UI + AfriCore APIs (AfriPay, AfriPush, AfriShare, AfriID, sensors, multi-SIM, on-device AI), typed via **AfriIDL**, available for every WASM-targeting language.
- **Emulator-first**: the OS runs in QEMU on a developer's Windows/Mac/Linux machine, so you can build and test AfriSovera apps **before any phone exists**. This is non-negotiable for bootstrapping a developer base.
- **On-device development** (the emerging-market reality where the phone is the only computer): a built-in editor + on-device WASM toolchain for Web/Lite apps, publish-to-AfriStore from the phone.
- **Asili (on-device AI assistant)** for coding help, plus translation/TTS/STT for low-literacy users — offline-capable, cloud optional.

## Why this beats every dead predecessor's app strategy
- Symbian/BB/WP each forced **one language + one toolchain + one store** and started app-less. AfriSovera starts with **the entire Web + real Android apps already running**, then offers a **language-agnostic** native path. The developer is never told "rewrite everything in our language to ship anything" — the exact wall the others died on.
