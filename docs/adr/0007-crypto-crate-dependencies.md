# ADR 0007 - Third-party crates only for cryptographic primitives, pinned, vendored and audited

- **Status:** Accepted (points 1 and 2 in force since the mesh window, 10 Nov 2026; points 3 and 4 done by F2)
- **Date:** 2026-11-13 (written down after the fact: THIRD_PARTY.md has cited it since 10 Nov 2026)
- **Deciders:** chief architect, with the OS lead (mesh window, review follow-up F2)
- **Related:** `docs/policy/THIRD_PARTY.md` (the crate list and licences), ADR 0002 (Rust), ADR 0005 (Apache-2.0),
  AFR-0009 (mesh wire format), `docs/design-f2-vendor-audit.md`, ADR 0008 (the kernel's own DRBG)

## Context

Until the mesh window the kernel used no crates at all. The mesh needs X25519, Ed25519 and ChaCha20-Poly1305.
Writing these ourselves would be the riskiest code in the tree: constant-time arithmetic is easy to get subtly
wrong, and our own tests would not catch a timing leak. The dalek and RustCrypto crates are widely reviewed,
permissively licensed and work without the standard library.

Taking crates brings three new risks: a crate pulls in more than primitives, a version changes under us, and a
crate or its download is tampered with. This ADR sets the rules that keep those risks small.

## Decision

1. **Primitives only, mesh only.** The kernel depends on third-party crates only for cryptographic primitives,
   and only inside `kernel/src/mesh/` (through our own path crates `mesh-crypto`, `mesh-noise`, `mesh-hpke`). The
   protocol logic stays our own code: Noise state machine, HPKE key schedule, AFR-0009 framing. So do the netstack,
   the drivers and the hardened DRBG. The kernel never uses a crate's random number generator. Any new crate
   anywhere in the kernel needs the architect's verdict.
2. **Exact pins.** Direct crates use exact versions (`=x.y.z`) with `default-features = false`. Every workspace's
   `Cargo.lock` is committed and is the authoritative list, with checksums. THIRD_PARTY.md is the readable summary
   with licences, reconciled against the locks.
3. **Vendored.** Every crate of every workspace lives in `vendor/` with its own LICENSE files. `.cargo/config.toml`
   replaces crates.io with it, and CI builds with `CARGO_NET_OFFLINE=true`, so no build needs crates.io. A change
   to `vendor/**`, any `Cargo.lock` or `.cargo/config.toml` needs the architect's verdict before it merges.
4. **Audited.** `cargo audit` runs on every lockfile, together with a check that `vendor/` equals a fresh
   `cargo vendor --locked`. It runs by hand and on any push to master that changes a lockfile, manifest or
   `vendor/`. It has no schedule (the Actions-minutes ruling of 11 Nov 2026). A new advisory against an unchanged
   lock is seen at the next run. The review gate in point 3 is the real check; this run on master is a second
   check after the merge.
5. **Licences.** Only licences compatible with distributing the combined work under Apache-2.0 (ADR 0005): MIT,
   Apache-2.0, BSD, ISC, Zlib, Unicode, public domain or similar. No GPL, LGPL or MPL, and no unknown licence.
   Host-only crates count too, because `vendor/` is published with the source.

## Amendment: Wi-Fi primitives (14 Nov 2026, architect's verdict on WIFI_PLAN.md, Q1)

Point 1 gains a second place: the PC kernel's Wi-Fi.
- **Crates:** `sha1` =0.10.6, `hmac` =0.12.1, `pbkdf2` =0.12.2, `aes` =0.8.4, `ccm` =0.5.0, `aes-kw` =0.2.1,
  `cmac` =0.7.2 and `subtle` =2.6.1. Exact pins, no default features, through one path crate `wlan-crypto`, a
  dependency of the kernel under `[target.'cfg(target_arch = "x86_64")'.dependencies]`. One version per crate in
  the locks (`hmac` and `cipher` are the versions already there). Licences MIT/Apache-2.0 (`subtle` BSD-3-Clause).
- **Confined:** only `kernel/src/wlan/crypto.rs` names `wlan_crypto`, and nothing else in the kernel names the crates
  (suite `wifi-crypto`). The protocol logic (EAPOL, key hierarchy, handshakes, CCMP framing, nonces) is ours.
- **No SIMD and no CPU-feature paths in the PC kernel**, which saves no SIMD state: `aes` with `--cfg
  aes_force_soft` (`.cargo/config.toml`, `[target.x86_64-unknown-uefi]`; wlan-crypto refuses to build for UEFI
  without it), `sha1` with `force-soft`. Suite `pc-wifikat` disassembles the image and fails on any xmm, ymm or
  zmm register, AES-NI, SHA-NI or PCLMULQDQ instruction, and on `cpuid` in the Wi-Fi crypto's object code.
- **Known answers:** every vector parsed from the published files (RFC 2202, 6070, 3394, 4493, 3610; IEEE 802.11
  Annexes J.3 and J.4 through hostapd's tests), pinned by sha256 (`scripts/wifi-kat-fetch.sh`), never typed by hand.
  `wifi selftest` runs them in the kernel; Wi-Fi makes no key in a boot where they have not passed.
- **Not in this row:** `p256` for WPA3-SAE comes with its own plan (W9).

## Consequences

- The kernel's crypto rests on reviewed code, and the parts that are ours stay small enough to read.
- The repo grows by about 31 MB (61 crates, with fuzz-host's). The ARM image's panic strings change their path once, when `vendor/`
  arrives. Remapping the path is measured but not done (F2 design).
- Updating a crate is a deliberate act: a new pin, a fresh vendor, an audit run and an architect's verdict.
- Not chosen: writing the primitives ourselves (risk), or a cloud cache of crates instead of `vendor/` (builds
  would still depend on a network service, and the public source would not carry the licences).
