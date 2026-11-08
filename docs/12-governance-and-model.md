# 12 — Governance, Monetization & the Inventor's Stake

> The founder's questions, answered honestly: who controls AfriSovera, how it
> stays sovereign *and* fundable, how developers (and the inventor) make money
> without becoming the gatekeeper we set out to abolish, and the truthful limits
> on "install AfriSovera on any device." This is strategy, not code — but it
> constrains the architecture, so it belongs in the design set.

## Open source, but you keep control (the Linux/Android lesson)

The **code** is open ([`08`](08-sovereignty-and-openness.md)); **control** is retained through levers a software
license cannot touch:

1. **Trademark + certification mark.** "AfriSovera" and "AfriSovera Certified" are
   owned, not licensed away. Anyone may fork the code; only certified, untampered
   builds may carry the name. (Firefox/Mozilla, Chromium/Chrome, Linux/"Linux®".)
2. **The Foundation.** A non-profit the inventor founds and stewards holds the
   canonical roadmap and reference implementation. Contributors are welcome; the
   Foundation sets direction.
3. **The reference services.** Everyone federates against a reference AfriStore /
   AfriID / AfriPay / AfriPush — which the founder's entity operates.

Open code is also a **legal shield**: a foundation and a distributed community ship
the OS, not the founder personally. Going fully private would destroy the sovereign,
no-gatekeeper promise that is the entire reason to build this.

## The gatekeeper is the people, not a corporation

AfriStore is **federated** — *a* store, never *the* store:

- **Anyone can run a store.** The founder runs the reference one; nations, telcos,
  co-ops, communities run theirs. No single entity can deplatform an app or a dev.
- **Signature, not permission.** Apps are signed (proving *who* made it and that
  it's untampered) — not gated by a corporate reviewer deciding what may exist.
  - *Built (milestones 2.32–2.36):* **both halves of the crypto core now exist.**
    Integrity — a real FIPS-validated **SHA-256** (`kernel/src/sha256.rs`) with an
    install-time manifest, a `verify <app>` command, and a load-time gate that
    refuses a tampered app before it runs. Authorship — a dependency-free
    **Ed25519** signature verifier (`kernel/src/ed25519.rs`, TweetNaCl-derived,
    validated against the RFC 8032 test vector) plus its **SHA-512** primitive, so
    the device can check *who* signed an app (publishers sign off-device; the device
    only verifies).
  - *Wired end-to-end (milestone 2.37):* `scripts/sign-app.py` signs an app's SHA-256
    digest with an Ed25519 publisher key off-device; the device holds the publisher
    key + signature and `verify-sig <app>` runs the on-device Ed25519 verifier —
    `signature valid` for the genuine app, `INVALID` for a tampered one, `unsigned`
    when there's no signature. Remaining for production: a real publisher-key format,
    per-app signature files shipped with the app, and a key-distribution/trust policy
    (which keys a given store or user trusts).
- **Reputation + web-of-trust + community moderation** replace the review board.
- **Capability sandbox is the safety net** ([`10`](10-security-model.md)): a bad app is contained by
  construction, so permissionless distribution is still safe.

## Developer monetization (not Apple's 30%)

- The developer **sets the price**; payment flows **directly** to them via **AfriPay
  / mobile-money / crypto**, with a **small protocol fee (low single digits)** — or
  free / donation / pay-what-you-want.
- Because stores are federated, **no one can tax or deplatform** a developer.
- Sell the app, sell a licence, subscription, or in-app via AfriPay — the dev owns
  the relationship with the customer.

## What the inventor gets (open source ≠ nothing — the Red Hat model)

| Stake | How it earns |
|---|---|
| **Trademark / certification** | Licence "AfriSovera Certified" to OEMs; certification program fees |
| **Foundation stewardship** | Direction, grants, sponsorship; convening power |
| **Reference services company** | **AfriPay rails take a fee on payment volume** (the real revenue); AfriStore hosting; AfriID/AfriPush hosting; consented cloud-AI offload |
| **OEM / enterprise** | Support contracts, custom builds for telcos/states/universities |
| **Equity** | Own the company built *around* the open OS (Red Hat, Canonical, Automattic) |

You profit from **services, brand, and rails** — never from gatekeeping apps. That
alignment is the point: you win when the ecosystem wins.

## Device ownership: the "third OS" — with honest limits

The pitch: **you bought the device; you should command all of it.** Manufacturers
ship hardware, then software-lock features to sell you a near-identical next model.
AfriSovera lets an owner **back up the factory image and flash a new OS** that opens
the hardware fully. Per-device reality, stated plainly:

| Device class | Reflash to AfriSovera? | Why |
|---|---|---|
| Android with **unlockable bootloader** | ✅ Yes | Bootloader unlock + vendor blobs on-device |
| **Apple-Silicon Macs** (M-series) | ✅ Feasible | Apple's *permissive* boot policy — Asahi Linux already boots on them |
| **iPhone A11 & older** (≈ iPhone X / 2017 and earlier) | ⚠️ Possible, tethered | `checkm8` — an **unpatchable BootROM exploit** (read-only silicon). checkra1n / Project Sandcastle (Linux-on-iPhone) prove it |
| **iPhone A12+** (iPhone XS / 2018 → today) | ❌ Not currently | Hardened BootROM; **no public unpatchable exploit**. Not physics — cryptography Apple actively patches |
| Android with **locked bootloader** | ⚠️ Only if unlockable / exploitable | Carrier/OEM lock |
| SBCs (Raspberry Pi / PINE64) | ✅ Yes | Open boot |
| **x86-64 PCs and laptops** (Intel/AMD) | ✅ Yes (PC track, added 3 Nov 2026) | UEFI boot from USB; owner switches Secure Boot off (or a signed shim later) |

**The honest framing:** it is **not an absolute "no."** The barrier is a hardware
root of trust + Apple's signing keys — cryptography, not a law of nature. It has
already been beaten on older iPhones (checkm8) and all Apple-Silicon Macs. On
**modern iPhones** it requires a BootROM/crypto break that **isn't public today**
and that we do **not** manufacture on demand (that's specialized offensive-security
research; AfriSovera does not build exploits). Our commitment: **support every
device where the door is already open** (Macs, checkm8 iPhones, unlockable/
exploitable Android, SBCs), and **architect so that the moment a new unlock or
exploit is published, AfriSovera can use it.** We leverage the legitimate
right-to-repair / jailbreak community's work; we do not promise iPhone support that
doesn't yet technically exist. Owners have DMCA jailbreak exemptions for their *own*
devices; distributing circumvention tools is the fraught part (mitigated by the
Foundation + decentralized hosting).

## The driver-learning backup (the founder's idea, made precise)

During a backup we **read the vendor driver blobs / firmware already present on the
device's partitions** and wrap them behind the stable Asili ABI (the automated
libhybris/Waydroid approach). The rebuilt AfriSovera image then drives the hardware
using the manufacturer's own drivers — **no re-begging the SoC vendor.**

- Works because on **Android** those blobs sit accessible on the device.
- Does **not** work on iPhone (encrypted, signed) — nothing to reuse.
- Legal note: reusing/redistributing vendor blobs raises copyright questions —
  handled by keeping blobs **on the user's device** (extract-and-wrap locally, don't
  redistribute) where possible, and native Asili drivers replacing them over time.

## What's genuinely new (and the honest ceiling)

New vs. the incumbents: **true device ownership**, **write-once WASM apps**, a
**federated no-gatekeeper store**, **capability security**, **on-device private
sovereign AI**, and **nation/telco self-hosting**.

Honest ceiling: **big tech will not "adapt to us"** — they have no incentive and
every reason not to. We win the **underserved** (Global South, privacy/repair/
sovereignty movements) and grow a movement that pressures them *indirectly* — the way
Linux never took the Windows desktop yet owns the server and made open source
unavoidable. Aim for that, not for forcing Apple.

## Legal reality (the part no one else will tell you)

Decentralization aids deniability; it does **not** make anyone untouchable. Real
exposure remains: **copyright/DMCA** (driver blobs), **patents**, **export controls**,
and **illegal content** in a store. Mitigation is structural — Foundation +
decentralized hosting + you holding the *brand*, not the *content* — which
**minimizes, not erases**, exposure. Build with counsel, not on the hope that "it's
decentralized so no law applies."
