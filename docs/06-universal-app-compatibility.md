# 06 — Universal App Compatibility (Android + iOS + Windows)

> *"Run apps from every platform"* is the most seductive idea in OS design — and the most dangerous. This doc gives the real architecture, the honest feasibility per platform, and the strategic trap that killed BlackBerry. Read the warning at the end before believing the dream.

## The honest feasibility matrix

| Platform | Binary apps (download & run as-is) | How | Reality |
|---|---|---|---|
| **Android** | ✅ **Yes** | ART/AOSP runtime embedded (Waydroid/Anbox precedent) | Mature, legal (Apache AOSP), proven. The easy one. |
| **Windows (Win32)** | ⚠️ **Technically yes** | Wine-style API translation + x86→ARM CPU emulation (Box64/FEX) | Clean-room legal; **heavy**, desktop-UX, x86 emulation tax. Fine on a desktop/convergence mode, awkward on a 2 GB phone. |
| **iOS (App Store)** | ❌ **No — legally & practically blocked** | Would need to defeat FairPlay DRM + reimplement all Apple frameworks | DRM-encrypted, code-signed, Apple-SDK-forbidden. **Cannot ship App Store binaries.** Only source-recompile or clean-room reimplementation of *unencrypted* apps is possible — and limited. |

So the truthful headline is: **Android: yes. Windows: yes-but-heavy. iOS App Store apps: no.** Anything that claims transparent iOS App Store support on a non-Apple phone is either lying or describing piracy that will get sued out of existence.

## Daraja — the unified foreign-app bridge framework

All foreign-platform support lives under **Daraja** (Swahili: *bridge*). Each foreign app runs **boxed**: it's a sandboxed AfriSovera process, capability-gated by the Mansa kernel exactly like a native app, with its world translated to AfriSovera services (display, input, network, files). No foreign runtime gets ambient power.

```
   Android app      iOS-built app      Windows .exe
        │                 │                  │
        ▼                 ▼                  ▼
 ┌────────────┐   ┌──────────────┐   ┌──────────────────┐
 │ Daraja·    │   │ Daraja·Tofaa │   │ Daraja·Dirisha   │
 │ Android    │   │ (iOS, ltd.)  │   │ (Windows/Win32)  │
 │ ART+AOSP   │   │ Darwin+UIKit │   │ Wine + Box64/FEX │
 │ runtime    │   │ reimpl.      │   │ x86→ARM          │
 └─────┬──────┘   └──────┬───────┘   └────────┬─────────┘
       └─────────────────┼────────────────────┘
                         ▼
        Daraja translation layer → AfriSovera services
        (Tonga display · input · netstack · VFS) — capability-gated
                         ▼
                 Mansa microkernel
```

Each bridge sandboxes the foreign app and translates its system calls onto AfriSovera's native services — the foreign app thinks it's home; AfriSovera keeps full control.

---

## Daraja·Android (the achievable one)
- Embed an **AOSP/ART runtime** as an AfriSovera service (Waydroid/Anbox-class). Android apps run in a container, surfaces composited by Tonga, input/network/files translated.
- **microG-compatible** Play-services shim (reuses AfriCore: AfriPush for notifications, AfriID for sign-in) so apps that expect Google services work without Google.
- Legal: AOSP is Apache-2.0. Clean.
- **This alone closes ~90% of the practical "I need my apps" problem in our markets.**

## Daraja·Tofaa (iOS — the hard truth)
"Tofaa" = *apple* in Swahili. Here is exactly why iOS is the hard wall:

1. **FairPlay DRM** — apps downloaded from the App Store are **encrypted**, decrypted at runtime by Apple's kernel using a hardware/account-bound key. On non-Apple hardware you have no key. You **cannot** legally run an App Store `.ipa`. Full stop.
2. **Code signing** — iOS demands Apple-issued signatures verified by the Secure Enclave. We can't mint them.
3. **Proprietary frameworks** — iOS apps link Apple's UIKit/SwiftUI/Foundation/Metal/CoreData… To run them we'd have to **clean-room reimplement Apple's entire framework stack** — a massive, perpetually-moving target. Precedents exist but are *limited*: **Darling** (macOS-on-Linux, Wine-style, no mature GUI apps) and **touchHLE** (runs a handful of *old 32-bit* iOS games via reimplemented frameworks). Nothing runs modern App Store apps.
4. **Apple's SDK license** explicitly forbids running iOS apps off Apple hardware; redistributing reimplemented frameworks invites legal action.

**What is actually possible (and what we'd build):**
- A **clean-room Darwin + UIKit-subset runtime** (Tofaa) that runs **unencrypted, developer-provided iOS builds** — i.e., a developer who *wants* to support AfriSovera ships a build, like targeting another platform.
- Better: an **iOS-source-compatibility SDK** — let developers **recompile** their iOS app for AfriSovera (Swift compiles to WASM/native; we provide UIKit-compatible APIs over Sankofa UI). This targets the *developer's source*, which is legal and far more tractable than binary emulation.
- **No App Store binary support.** We say so plainly. Pretending otherwise is the fastest way to a lawsuit and broken-app reviews.

## Daraja·Dirisha (Windows — feasible but mismatched)
"Dirisha" = *window* in Swahili.
- **Win32 apps** run via a **Wine-style clean-room reimplementation** of the Windows API (legal — Wine has done this for 20+ years) **plus x86/x64→ARM64 CPU translation** (Box64/FEX-Emu-class), since phones are ARM and most Windows apps are x86.
- **Two real costs:** (1) **performance** — API translation *and* CPU emulation stacked is heavy; rough on a 2 GB ARM phone. (2) **UX mismatch** — Win32 apps assume mouse, keyboard, resizable windows, big screens. They're miserable on a 6-inch touchscreen.
- **Where it makes sense:** AfriSovera's **desktop/convergence mode** (phone → external monitor + keyboard, the "your phone is your PC" play that's genuinely valuable in markets where the phone is the only computer). On that mode, Dirisha lets a user run a needed legacy Windows tool. As a phone-touch experience, deprioritize it.

---

## Performance & battery reality (don't forget the 2 GB phone)
Our design center is a cheap, low-RAM, battery-constrained device. Compatibility layers cost RAM, CPU, storage, and battery:
- **Android**: moderate cost; acceptable.
- **Windows (Wine + x86 emulation)**: high cost; realistic only on better hardware / desktop mode.
- **iOS framework reimplementation**: high cost *and* incomplete.

So compatibility is **tiered by device capability**: every AfriSovera phone runs native + Web + Android; Windows/iOS bridges are opt-in modules for capable devices and convergence mode, not baseline promises on a $40 handset.

## ⚠️ The strategic trap (the most important paragraph here)

**Running everyone else's apps can kill your OS.** BlackBerry 10 shipped an Android runtime — and it *accelerated* BB10's death, because developers said *"why build a native BlackBerry app? Ours already runs (badly) via the Android layer."* The compatibility layer removed the reason to ever go native, so the native ecosystem never formed, and users got second-class versions of everything. An OS whose apps all come from other platforms is a permanent **second-class emulator** of those platforms — slower, less integrated, always a step behind.

**The rule that keeps Daraja healthy:** *compatibility is a bridge, not a destination.*
- Bridges exist to make the OS **useful on day one** and to **buy time** while the native (Nia/WASM + Web) ecosystem grows — exactly the role they play in [`docs/00`](00-the-from-scratch-decision.md).
- Native apps must always be **first-class and visibly better** (faster, AfriPay/AfriShare-integrated, capability-secure, battery-light). The OS should gently steer users and devs toward native equivalents and let bridged apps fade.
- We measure success by **native ecosystem growth**, not by how many foreign apps run. If bridge usage stays 100% forever, the OS has failed even if every app "works."

## Bottom line
- **Android — yes**, and it's enough to solve the day-one app problem.
- **Windows — yes via Wine + x86→ARM**, but treat it as a convergence/desktop feature for capable devices, not a phone-touch promise.
- **iOS — no binary/App Store support is legally or practically possible.** Offer a **source-recompile SDK** and a limited clean-room runtime for developer-provided builds, and be honest that App Store apps will not run.
- And above all: **keep the bridges a bridge.** The day developers stop building native AfriSovera apps because "Android/Windows already runs," the OS is dead — it just doesn't know it yet.
