# 08 — Sovereignty & Openness (the open-source commitment)

> This is a **first-class design document**, not a footnote. AfriSovera's reason
> to exist is that the current mobile world is controlled by two companies and
> the gatekept stores they run. An OS that merely copies that control with
> different branding is pointless. The openness commitments here are load-bearing
> — remove them and the project has no reason to exist.

## The commitment, in one paragraph

**AfriSovera is open source. You own the device, the OS, and the right to run,
build, and distribute software on it without asking permission from any company
or government.** No mandatory app store. No locked bootloader you can't unlock.
No permission-to-publish. No foreign mothership your identity, payments, or
notifications are forced through. The system's security comes from **capabilities
you grant**, not from a vendor deciding what you're allowed to do with your own
hardware.

## Why openness is *architectural*, not ideological

The rest of this design already encodes it — this doc just makes it explicit:

- **Capability security ([`01`](01-kernel-mansa.md))** means authority flows from
  the *user*, not from a privileged vendor daemon. There is no root, no ambient
  authority, no OEM backdoor with powers you don't have. That is user sovereignty
  expressed in the kernel.
- **A stable, documented driver ABI ([`02`](02-drivers-and-services-asili.md))**
  means anyone — not just the OEM — can write and ship a driver. Hardware support
  isn't a vendor gift that expires when they drop support.
- **WASM + Web apps ([`03`](03-app-runtime-and-sdk.md))** mean apps are portable
  and not locked to one store's binary format or one company's signing key.
- **Self-hostable AfriCore ([`02`](02-drivers-and-services-asili.md))** means a
  telco, a state, a university, or a co-op can run its *own* identity/push/store
  and not depend on ours — or anyone's.

You cannot bolt sovereignty onto a closed OS later. It has to be the grain of the
wood. It is.

## Licensing — the recommendation and the reasoning

Tool/spec reuse ([`04`](04-the-toolchain.md)) already stands on open licenses
(Rust/LLVM, Vulkan, WASM, AOSP-Apache for the Android bridge). For *our own* code:

| Component | Recommended license | Why |
|---|---|---|
| **Kernel, drivers, SDK, AfriIDL, UI framework, libraries** | **Apache-2.0** | Permissive + explicit **patent grant**; lets anyone (incl. OEMs and commercial app devs) adopt and ship without legal friction. Maximizes reach — the app/driver ecosystem is the whole survival bet. Matches the AOSP precedent and the `license` field already in `kernel/Cargo.toml`. |
| **AfriCore sovereign services** (AfriID/AfriPay/AfriPush/AfriStore server sides) | **Copyleft (AGPL-3.0 or MPL-2.0)** — *deliberately stronger* | These are the anti-gatekeeper layer. Copyleft here means a company **cannot take the sovereign store/identity/payments, close it, and re-gatekeep it**. Improvements to the sovereign core stay open. AGPL closes the "run-as-a-hosted-service without sharing changes" loophole. |
| **Trademarks / certification marks** ("AfriSovera", the device-tier badge) | **Reserved** | The *code* is free; the *name and the "certified compatible" mark* are protected so "AfriSovera" always means an un-tampered, un-spyware'd build. Openness of code, integrity of identity — both at once. |

**One-line rule:** *permissive where we want maximum adoption (the platform),
copyleft where we must prevent recapture (the sovereign services), protected marks
so "AfriSovera" keeps meaning something.* This is a recommendation to ratify, not a
final legal decision — a real project confirms it with counsel before v1.0.

## The app store: distribution as a right, not a privilege

The single most important anti-gatekeeper stance is here. AfriStore is **a** store,
never **the** store:

1. **Sideloading is a first-class, permanent right.** Installing an app from a
   file, a link, a P2P transfer (**AfriShare**), or a competing store is normal and
   supported — not a scary "unknown sources" dark path. The OS will *warn* about
   unsigned or over-permissioned apps (capabilities make that honest and specific),
   but it will **never forbid you from running your own device.**
2. **Multiple stores, by design.** Anyone can run a store — a country, a co-op, a
   FOSS community, a company. AfriStore is the reference one, federatable and
   self-hostable, not a monopoly.
3. **No permission-to-publish.** Signing proves *who* made an app and that it wasn't
   tampered with. It is **not** a gate where we decide whether you're *allowed* to
   ship. No arbitrary review board, no 30% toll as the price of existing.
4. **Capability-based safety instead of gatekeeper "review."** Safety comes from the
   kernel: an app gets only the capabilities the user grants, so a malicious app is
   *contained by construction*, not by our hope that a reviewer caught it.
5. **Local-currency payouts, offline install.** Developers in our markets get paid in
   their own currency via **AfriPay**, and apps install **offline over P2P** — the
   store works where the gatekept ones assume infrastructure that isn't there.

## Openness vs. security — the honest tension, resolved

Open must not mean insecure. The resolution is **user-controlled trust**, not
vendor-controlled lockdown:

- **Verified boot exists — with *your* keys.** The chain from [`02`](02-drivers-and-services-asili.md)
  protects against tampering, but the bootloader is **unlockable by the device
  owner** (the Android/Pixel model, done right). Verified boot defends the user; it
  is not a lock *against* the user. A locked-forever bootloader is a gatekeeper move
  and we reject it.
- **Reproducible builds ([`04`](04-the-toolchain.md))** so anyone can verify the
  shipped image matches the public source — openness you can *check*, not just
  trust.
- **Sandbox-by-default** means "run anything" is safe: capabilities box every app,
  native or bridged, so permissionless distribution doesn't mean permissionless
  access.

## Governance — not one company, not one state

- **Public specs first**: AfriIDL, the driver ABI, the SDK, the store protocol, and
  the emulator image are public so community device-enablement and app development
  start early ([`04`](04-the-toolchain.md), [`05`](05-roadmap.md) Phase 0–2).
- **Federation over centralization**: sovereignty partners (telcos/states/
  universities) run their own AfriID/AfriStore instances; no single instance —
  including ours — is the mandatory center.
- **Open contribution**: the core is developed in the open; the funding model
  ([`00`](00-the-from-scratch-decision.md)) is sovereignty partners + AfriStore/
  AfriPay revenue, **not** surveillance, ads, or data brokerage. The user is the
  customer, never the product.

## What AfriSovera will *not* be

- Not a walled garden. Not a mandatory single store. Not a locked bootloader.
- Not telemetry-by-default; no selling user data; the user is not the product.
- Not "open" in name while the real control (store, keys, identity) stays closed —
  the sovereign layer is copyleft precisely to prevent that bait-and-switch.

## Bottom line

Every dead predecessor OS was *someone's product*. AfriSovera is meant to be
*infrastructure a community and its partners own* — open from the kernel up, with a
distribution model no company or government can gate. The code is free (Apache), the
sovereign services stay free (copyleft), the name stays trustworthy (protected mark),
and the right to run, build, and share software on your own device is **not
negotiable**. That is the whole point of building this instead of forking someone
else's.
