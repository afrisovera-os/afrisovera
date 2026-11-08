# 11 — Driver Acquisition & Manufacturer Adaptation

> "How does the OS find and install a driver for a device — automatically — and
> how do manufacturers make hardware that works with AfriSovera?" This is the
> driver gap ([`00`](00-the-from-scratch-decision.md), [`02`](02-drivers-and-services-asili.md)) at the practical level. Here is the real pipeline,
> and an honest line between what's engineerable and what isn't.

## The auto-driver pipeline

```
  device present ──▶ Device Manager reads hardware IDs
                     (DT `compatible`, PCI/USB IDs, SoC model, EDID…)
        │
        ▼
  query the sovereign Driver Registry with those IDs
        │
        ├─ match found ─▶ download SIGNED Asili driver ─▶ verify signature
        │                                                   │
        │                                                   ▼
        │                                    load as a SANDBOXED userspace
        │                                    process (holds only its HW caps)
        │
        └─ no native match ─▶ fall back to the Android-driver shim (bring-up)
                              or prompt for a community/vendor driver
```

Every step is capability-gated: a freshly-downloaded driver is a **sandboxed
process** that holds capabilities for **only its hardware** (a specific MMIO range,
IRQ line, DMA region) — a malicious or buggy "driver" can't touch the rest of the
system, unlike a Windows kernel-mode driver that owns the machine.

### 1. Hardware-ID awareness (manufacturer/model detection)
The Device Manager ([`asili`](../kernel/src/asili.rs), today a registry — tomorrow the enumerator) reads the
identifiers every bus exposes: Device Tree `compatible` strings, PCI/USB
vendor:product IDs, the SoC model, display EDID, etc. That *is* manufacturer and
model awareness — the same identity mechanism Windows/Linux use to pick a driver.

### 2. Fetch + verify + load (the "download a driver" part)
Those IDs are looked up in a **sovereign, signed Driver Registry** (an
AfriStore-for-drivers, self-hostable by a partner). A matching **signed Asili
driver** is downloaded, its signature verified, and it's loaded as a sandboxed
process. Conceptually like Windows Update's driver service — but **open, signed,
sandboxed, and not owned by one company.**

### 3. "Reconstruct the driver to run on our OS" — the honest line
You **cannot** magically decompile an arbitrary closed Windows/Android driver
binary and rebuild it for a new OS. Pretending otherwise is fantasy. What is real:

| Source of driver | How it runs on AfriSovera | Reality |
|---|---|---|
| **Native Asili driver** | Written/compiled against the stable Asili ABI | The clean long-term path |
| **Open-source driver** | **Recompiled** against the Asili ABI | Very doable; community-scalable |
| **Vendor Android blob / HAL** | **Android-driver shim** wraps it behind the Asili ABI (libhybris precedent) | Buys real GPU/modem/sensor support *now* |
| **Closed Windows/other binary** | ❌ not portable | Would require the vendor to ship an Asili build |

So "reconstruction" = **recompile (open) or shim (Android blob)**, not
binary-decompile-and-rebuild. The shim is the bring-up shortcut; native Asili
drivers replace shims over time, device by device.

## Why the stable ABI changes the economics
The reason Symbian/webOS couldn't sustain drivers: every OS update broke them, so
maintenance was ruinous. Asili's **versioned, stable driver ABI** means a driver
compiled once **keeps working across OS updates** — drivers become *cheap to write
and maintain*, which is what lets a community + partners actually sustain a driver
ecosystem.

## How manufacturers adapt (and why they'd want to)
- **Write once, low upkeep.** Target the documented Asili ABI + SDK; it keeps
  working across releases. No per-release driver rewrites.
- **Certification program.** A partner-certified device tier ([`05`](05-roadmap.md)) with a trust
  badge — "AfriSovera Certified" means an untampered, well-supported device.
- **Near-free bring-up.** A vendor already shipping Android drivers gets initial
  support via the shim, then can invest in native Asili drivers for a better,
  lighter, capability-secure result.
- **Market access.** Sovereignty partners (telcos/states/universities) procuring
  AfriSovera devices is the commercial pull that makes adapting worth it.
- **Reference hardware ladder.** We meet them partway: QEMU → SBCs → a reference
  dev phone → certified handsets, controlling the board at each rung.

## The bootstrap problem: no drivers, no internet, fresh install

"How does a raw machine with no drivers and no internet get online to fetch
drivers?" You never download the thing you need to download with. The network
driver is **never** fetched over the network. Resolution, in order:

1. **In-box baseline drivers (bundled in the image).** Like every real installer
   (Windows/Linux ISOs), the AfriSovera image ships with essential drivers baked
   in — display, input, storage, **and network (WiFi/Ethernet) for common
   chipsets**. A fresh install already boots, shows a screen, and can get online.
   Only *optional/peripheral* drivers are fetched later, once connected.
2. **Reuse on-device blobs (Android shim).** Converting an existing Android phone:
   the vendor driver blobs are already on its partitions; the shim reuses them
   behind the Asili ABI — hardware lights up **fully offline, zero download**.
3. **Offline / P2P install (AfriShare).** First-class in data-scarce markets: the
   OS and driver packs install from another phone, USB stick, SD card, or repair
   kiosk over Wi-Fi Direct / Bluetooth / USB — the internet is never required.
4. **Generic class drivers.** Standard-conformant hardware (USB HID, USB mass
   storage, standard framebuffer, virtio, mac80211-class WiFi) works with generic
   in-box drivers, no device-specific download.
5. **Per-device / certified images.** Certified handsets get an image pre-matched
   to that exact hardware — nothing to fetch.

**Honest limit:** we can't magically support *unknown* hardware offline. An exotic
WiFi chip with no in-box driver and no reusable blob needs a sideloaded driver pack
(SD/USB/another machine) — the same reality every OS faces. Hence the device-support
**tier list**: "excellent on a known, growing list," not "any silicon, offline,
unmodified." The bootstrap driver set always ships on the install medium; the
registry is only for optional drivers fetched *after* you already have connectivity.

## Honest status
Today the Device Manager is a **registry** (it records what bring-up brought
online). The **enumerate → query registry → download → verify → sandbox-load**
pipeline, the signed Driver Registry, and the Android shim are **designed, not yet
built** — they're Phase 1–3 work. This doc is the architecture we build toward; it
is deliberately clear about the difference between what runs today and what is
planned, and about the one thing that is *not* possible (rebuilding arbitrary
closed binaries).
