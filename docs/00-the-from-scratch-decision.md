# 00 — The From-Scratch Decision

## What "from scratch" actually means here

We are building, as new code we own:

- a **kernel** (Mansa),
- a **driver framework + drivers** (Asili),
- **system services** — display server, network stack, filesystem, power, telephony,
- an **application runtime** (Nia / WASM) + a **UI framework** (Sankofa UI),
- an **SDK, IDL, and build toolchain**,
- the **sovereign services** (AfriCore).

We are **not** rewriting: the Rust compiler, LLVM, the Vulkan spec, the WebAssembly spec, TLS/crypto primitives, or a web rendering engine. Reusing *open standards and battle-tested libraries* is not "forking Android" — it's not reinventing mathematics. A from-scratch OS still stands on compilers and specs. That distinction keeps the project finishable.

## Why even attempt it (vs. the Android fork)

A fork inherits Google's app model, Google's update cadence, Google's security decisions, and Google's architectural debt (a 2008 design carrying 17 years of compatibility weight). A clean-room OS lets us bank four structural wins a fork can never have:

1. **Capability security from byte zero** — no root, no ambient authority, every access is an unforgeable token. Retrofitting this onto Android is impossible; designing it in is free.
2. **A microkernel** — drivers and services in userspace, isolated. A crashing GPU driver doesn't panic the phone. The kernel is small enough to *formally verify* (the seL4 precedent).
3. **A modern app model** — WASM, not a 2008 Java/Dalvik design. Language-agnostic, sandboxed, portable, tiny.
4. **True sovereignty & longevity** — we own the entire pipeline; nothing structurally tethers a user or a nation to a foreign company.

## The two gaps that kill new OSes — and our answers

This is the most important page in the entire design. If these answers are weak, **stop the project.**

### Gap 1 — The App Gap
> *No apps → no users → no developers → no apps.* This killed Windows Phone (despite Microsoft's money) and webOS (despite great engineering).

**Our three-on-ramp answer:**

| On-ramp | What it gives day one | Precedent |
|---|---|---|
| **Web / PWA** (first-class) | Thousands of existing web apps run immediately; any web dev ships with zero new toolchain | **KaiOS** did exactly this and shipped on **100M+** phones in India/Africa |
| **Nia (WASM/WASI)** | Any language (Rust, C, Go, Kotlin, Swift, JS, Python) compiles to one portable, sandboxed app format | WASM is the genuinely new, forward bet |
| **Daraja** (Android bridge) | Real, existing Android apps run in a contained translation layer during the transition years | Anbox/Waydroid prove Android runtimes embed in other OSes |

The strategy: **the Web + Daraja make the OS useful on day one; Nia + a great SDK make native development attractive so the ecosystem migrates over time.** You don't ask the world to rewrite first — you give them an OS that already runs their stuff, then make native better.

### Gap 2 — The Driver Gap
> SoC vendors ship closed drivers **for Android's interfaces only.** A new OS gets nothing for free. This is why no one ships new phone OSes.

**Our answer (two-pronged):**

1. **Asili stable driver ABI** — a clean, versioned userspace driver interface (the Fuchsia lesson). Drivers are normal sandboxed processes; a driver compiled once keeps working across OS updates. This makes writing/maintaining drivers *cheap*, so community + partners can sustain them.
2. **Android-driver shim for bring-up** — to light up real silicon without re-begging Qualcomm/MediaTek, a compatibility layer can host **existing Android kernel driver blobs / HALs** behind the Asili ABI (similar to how `libhybris` bridged Android drivers into Sailfish/Ubuntu Touch). This buys real hardware support *now*, while native Asili drivers are written over time.

**Honest device strategy:** start where we control the board — **QEMU/virtual → a single reference dev phone (e.g. a Pixel-class device with an open-ish GPU) → SBCs (Raspberry Pi / PINE64) → partner-certified handsets.** We do **not** promise "runs on any phone." We promise "runs excellently on a known, growing list, and here's exactly which."

## The cost — stated plainly
This is a **multi-year, well-funded effort**, not a weekend ROM. A serious v1.0-on-a-real-phone is on the order of **dozens of engineers over several years**, and survival depends on a funding model (sovereignty partners — telcos/states/universities — plus AfriStore/AfriPay revenue) and on hitting *useful* milestones early (a bootable, app-running dev kit long before "ships on a phone"). If you want this, the roadmap in [`05-roadmap.md`](05-roadmap.md) is built to deliver value at every phase rather than betting everything on a distant 1.0.
