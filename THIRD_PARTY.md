# Third-party code in the tree

> The list of third-party code, fonts and data AfriSovera carries, with licences, and the place ADR 0007 records
> the kernel's cryptographic crate dependencies. Every licence below is compatible with distributing the combined
> work under Apache-2.0 (ADR 0005).

## Vendored source already in the tree

| Code | Origin | Licence |
|---|---|---|
| `kernel/src/ed25519.rs` | TweetNaCl-derived Ed25519 verification | Public domain (TweetNaCl) |

TweetNaCl is public domain; the file keeps its own in-source notice. (Signing is being added as a kernel crate
dependency below, not by extending this file.)

## Kernel cryptographic crate dependencies (ADR 0007)

Per ADR 0007, the kernel depends on third-party crates **only for cryptographic primitives**, only inside
`kernel/src/mesh/`. Every other module stays crate-free; the protocol logic (Noise state machine, HPKE key
schedule, AFR-0009 framing, netstack, drivers) and the hardened DRBG stay the project's own code, and the kernel
never uses a crate's random number generator. The authoritative, complete set with versions and checksums is the
committed `Cargo.lock`; this table is the human-readable summary, reconciled against that lock when the mesh
wires in.

### Direct crates (exact pins, `default-features = false`)

| Crate | Version | Licence | Use |
|---|---|---|---|
| `x25519-dalek` | =2.0.1 | BSD-3-Clause | X25519 key agreement |
| `ed25519-dalek` | =2.1.1 | BSD-3-Clause | Ed25519 signing + strict verification |
| `chacha20poly1305` | =0.10.1 | Apache-2.0 OR MIT | ChaCha20-Poly1305 AEAD |
| `zeroize` | =1.9.1 | Apache-2.0 OR MIT | Wipe secrets on drop (F1, review S2). Already in the tree through `chacha20poly1305`; named directly by `mesh-noise` and `mesh-hpke` since 10 Nov 2026, with the dalek crates' `zeroize` feature on |

### Transitive crates (reconciled with the committed `Cargo.lock`, 10 Nov 2026)

How they arrive: the kernel names only our own path crates (`mesh-crypto`, `mesh-noise`, `mesh-hpke`), which
hold the three exact pins above. Resolving that gave exactly this set (the `Adding ...` lines when the lock was
written):

| Crate (version in the lock) | Licence | In the kernel image? |
|---|---|---|
| `curve25519-dalek` 4.1.3 | BSD-3-Clause | yes |
| `curve25519-dalek-derive` 0.1.1 | MIT OR Apache-2.0 | no (build-time macro) |
| `subtle` 2.6.1 | BSD-3-Clause | yes |
| `chacha20` 0.9.1, `poly1305` 0.8.0, `aead` 0.5.2, `cipher` 0.4.4, `crypto-common` 0.1.7, `universal-hash` 0.5.1, `inout` 0.1.4, `typenum` 1.20.1, `opaque-debug` 0.3.1 | Apache-2.0 OR MIT | yes |
| `generic-array` 0.14.7 | MIT | yes |
| `ed25519` 2.2.3, `signature` 2.2.0, `sha2` 0.10.9, `digest` 0.10.7, `block-buffer` 0.10.4, `rand_core` 0.6.4, `zeroize` 1.9.1, `cfg-if` 1.0.5, `cpufeatures` 0.2.17 | Apache-2.0 OR MIT | yes (rand_core: traits only; the kernel never uses a crate RNG) |
| `fiat-crypto` 0.2.9 | MIT OR Apache-2.0 OR BSD-1-Clause | only if the fiat backend is selected (it is not on aarch64-unknown-none by default) |
| `libc` 0.2.191 | MIT OR Apache-2.0 | no (cpufeatures uses it on hosted targets only) |
| `rustc_version` 0.4.1, `semver` 1.0.28, `version_check` 0.9.5, `proc-macro2` 1.0.107, `quote` 1.0.47, `syn` 2.0.119 | MIT OR Apache-2.0 | no (build scripts and proc macros) |
| `zeroize_derive` 1.5.0 | Apache-2.0 OR MIT | no (build-time macro; arrives with the x25519-dalek `zeroize` feature, F1, 10 Nov 2026) |
| `unicode-ident` 1.0.27 | (MIT OR Apache-2.0) AND Unicode-3.0 | no (proc-macro dependency) |

The licence column is the crates' published licence fields (checked against `cargo metadata` for F2, 13 Nov
2026). No licence in this set is incompatible with
Apache-2.0.

### Wi-Fi primitives (ADR 0007 row "Wi-Fi primitives", architect's verdict 14 Nov 2026)

The PC kernel's Wi-Fi (WPA2-Personal) takes its primitives from these crates, through our path crate
`wlan-crypto`, used only by `kernel/src/wlan/crypto.rs` (suite `wifi-crypto` fails if anything else names them).
x86_64 only: the ARM kernel links none of them. `aes` is built with `--cfg aes_force_soft` and `sha1` with its
`force-soft` feature, so no AES-NI, SHA-NI or CPU-feature detection is compiled in (suite `pc-wifikat`
disassembles the image). The protocol (EAPOL, key hierarchy, handshakes, CCMP framing) is ours.

| Crate | Version | Licence | Use |
|---|---|---|---|
| `sha1` | =0.10.6 | MIT OR Apache-2.0 | SHA-1 under HMAC-SHA1 (PBKDF2, the 802.11 PRF, the EAPOL MIC) |
| `hmac` | =0.12.1 | MIT OR Apache-2.0 | HMAC-SHA1 (the same version the vendored tree already had) |
| `pbkdf2` | =0.12.2 | MIT OR Apache-2.0 | passphrase to PMK, 4096 rounds |
| `aes` | =0.8.4 | MIT OR Apache-2.0 | AES-128, software backend only |
| `ccm` | =0.5.0 | Apache-2.0 OR MIT | AES-CCM with an 8-byte tag (CCMP) |
| `aes-kw` | =0.2.1 | MIT OR Apache-2.0 | AES key wrap (RFC 3394) for the group key |
| `cmac` | =0.7.2 | MIT OR Apache-2.0 | AES-CMAC (key descriptor version 3, protected management frames later) |
| `subtle` | =2.6.1 | BSD-3-Clause | constant-time comparison of MICs and tags |

New transitive crates: `dbl` 0.3.2 (MIT OR Apache-2.0, under `cmac`). The rest (`cipher`, `ctr`, `aead`, `digest`,
`crypto-common`, `inout`, `generic-array`, `typenum`, `block-buffer`, `cpufeatures`, `cfg-if`) are the versions
already in the lock. No crate has a second version.

## The vendored tree (ADR 0007 points 3 and 4, F2, 13 Nov 2026)

`vendor/` holds every crate of the committed lockfiles: the root `Cargo.lock` above, plus the locks of the
host-only test crates `mesh-crypto`, `mesh-noise`, `mesh-hpke`, `mesh-link`, `drbg-host` and `wlan-crypto` (its
host tests, 14 Nov 2026).


`.cargo/config.toml` replaces crates.io with `vendor/` for every cargo command in the repository, and CI and the
cloud test job build with `CARGO_NET_OFFLINE=true`, so a build never needs crates.io. Each crate keeps its own
`LICENSE` files; a release checks those copies.

The host-only crates (never in the kernel image; they build the test programs) add this set to the table above:

| Crate | Licence |
|---|---|
| `snow` 0.9.6 (Noise test vectors) | Apache-2.0 OR MIT |
| `aes` 0.8.4, `aes-gcm` 0.10.3, `ctr` 0.9.2, `ghash` 0.5.1, `polyval` 0.6.2, `blake2` 0.10.6, `hmac` 0.12.1, `getrandom` 0.2.17 | Apache-2.0 OR MIT |
| `serde` 1.0.229, `serde_core` 1.0.229, `serde_derive` 1.0.229, `serde_json` 1.0.128, `itoa` 1.0.19 | Apache-2.0 OR MIT |
| `ryu` 1.0.23 | Apache-2.0 OR BSL-1.0 |
| `memchr` 2.8.3 | Unlicense OR MIT |
| `wasi` 0.11.1 (+wasi-snapshot-preview1) | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| `sha2` 0.10.8 and `syn` 3.0.10, beside the root lock's 0.10.9 and 2.0.119 (a test crate's lock resolves another version) | Apache-2.0 OR MIT |

Every crate directory carries a licence file. The project's CI runs `cargo audit` over the locks and checks
`vendor/` equals a fresh `cargo vendor` of them whenever a lock, a manifest or `vendor/` changes, and the public
copy is published only from a commit that check has passed.

## Device firmware

Files some chips need from their vendor before they work (most laptop Bluetooth, many Wi-Fi chips). They run on
the chip, never on our CPU: AfriSovera only reads one and sends it to the device it is for. The owner allowed
shipping them on 13 Nov 2026; the rules are the architect's verdict of that day, set out below.

- **The list is `firmware/MANIFEST`**: each file's name, size, sha256, licence, vendor, version and pinned source
  (for the vendors' own files, the `linux-firmware` collection at a fixed commit; it holds vendor files and their
  licences, no Linux kernel code). The licence text of each file is kept whole in `firmware/licences/`.
- **Only files whose licence allows giving them out in binary form, unchanged.** A row and its licence go in
  together, with the chief architect's verdict before the merge; the release engineer checks the licence before a
  public sync.
- **The binaries are not in git.** The image build fetches each one (`scripts/fw/fwtool.py fetch`) and stops on any
  wrong size or sha256; the files and their licences go into `\EFI\afrisovera\firmware\` on the boot stick.
- **The kernel trusts only its own table** (`kernel/src/pc/fwdb.rs`, checked against the MANIFEST by
  `fwtool.py check` in the pc-firmware suite): size and sha256 must match before a byte is sent, and only to a
  device the row names.

Today the list holds only `fwtest.bin`, AfriSovera's own test file (Apache-2.0), for the loader's tests.

## Fonts

Fonts are data, not code, so ADR 0007 (crates) does not cover them. The SIL Open Font License 1.1 is allowed for
fonts only, never for code (the architect, 13 Nov 2026; a line in ADR 0005's notes).

| File | What it is | Source, pinned | Licence |
|---|---|---|---|
| `kernel/src/afriui/outline_data.rs` | The desk's outline font, shown in the OS as "AfriSovera Sans" (afriui.md section 26): the outlines of 381 characters taken from Noto Sans Regular by `tools/face/outline_src.py` (in the cloud) and written as tables. A Modified Version of Noto Sans under the OFL. | Noto Sans v2.015, `NotoSans/unhinted/ttf/NotoSans-Regular.ttf` from the notofonts release `NotoSans-v2.015` (latin-greek-cyrillic), sha256 `f3961a9cde016d41a4879aecda1474d3a36d6bf54fa0e4643de029cc2248b0e8`, also in the tool. Copyright 2022 The Noto Project Authors. No Reserved Font Name. | OFL-1.1, whole text in `fonts/OFL.txt`; the file stays under the OFL, not Apache-2.0 |
| `kernel/src/afriui/shape_data.rs` (face 1 and the shaper's tables) | The desk's Arabic face and its layout rules: a subset of Noto Sans Arabic (145 glyphs for 63 characters), its outlines, its GSUB substitutions and GPOS mark anchors compiled into flat tables by `tools/face/shape_src.py` (in the cloud). A Modified Version of Noto Sans Arabic under the OFL. The same file also holds the Unicode tables below. | Noto Sans Arabic v2.013, `NotoSansArabic/unhinted/ttf/NotoSansArabic-Regular.ttf` from the notofonts release `NotoSansArabic-v2.013` (github.com/notofonts/arabic), sha256 `bd86ca02f087d7f3c3788ba458fb6b73744c7639ed276b8d870dba6def6c40d0`, also in the tool. Copyright 2022 The Noto Project Authors. | OFL-1.1, whole text in `fonts/OFL.txt` |

`fontTools`, which reads the font, is a tool run in the cloud and is not shipped.

## Translation drafts

The desk's words in languages other than English and Twi started as machine drafts,
made in the cloud by `tools/lang/draft.py` with Helsinki-NLP Opus-MT models, which are not shipped: only the short
drafted strings are, in `kernel/src/afriui/lang.rs`, every one marked Draft until a fluent speaker checks it. The tool
uses a model only when its model card states Apache-2.0, MIT, BSD-3-Clause or CC-BY-4.0, and records it here.
NLLB-200 is never used (CC-BY-NC). The CC-BY-4.0 models are credited: Opus-MT by Jörg Tiedemann and the Language
Technology Research Group at the University of Helsinki (OPUS-MT, EAMT 2020), models as listed.

| Language | Model and revision | Licence |
|---|---|---|
| es | `Helsinki-NLP/opus-mt-en-es` at `5bc4493d463c` | apache-2.0 |
| fr | `Helsinki-NLP/opus-mt-en-fr` at `dd7f6540a7a4` | apache-2.0 |
| pt | `Helsinki-NLP/opus-mt-tc-big-en-pt` at `9f2863d807ec` | cc-by-4.0 |
| id | `Helsinki-NLP/opus-mt-en-id` at `6e4c52d61a6b` | apache-2.0 |
| de | `Helsinki-NLP/opus-mt-en-de` at `6183067f769a` | cc-by-4.0 |
| tr | `Helsinki-NLP/opus-mt-tc-big-en-tr` at `e539fc16a8a1` | cc-by-4.0 |

## Unicode data

| File | What it is | Source, pinned | Licence |
|---|---|---|---|
| `kernel/src/afriui/shape_data.rs` (BIDI, JOIN, MIRROR, BRACKET) | Bidi classes, Arabic joining types, mirrored characters and paired brackets, as range tables made by `tools/face/shape_src.py` | Unicode Character Database 16.0.0: `extracted/DerivedBidiClass.txt`, `extracted/DerivedJoiningType.txt`, `BidiMirroring.txt`, `BidiBrackets.txt`, each pinned by sha256 in the tool | Unicode License v3 (allowed, ADR 0007 point 5) |

`BidiCharacterTest.txt` (same release) is fetched by the cloud job for fuzz-host's conformance test and is not in the
tree. HarfBuzz (MIT, through uharfbuzz 0.57.0, Apache-2.0) and fontTools 4.67.0 (MIT) are tools run in the cloud
only: the first is the test oracle (`tools/face/shape_expected.txt`), the second reads and subsets the font. Neither
is shipped or vendored.

## Test tooling

Changes to other projects' code that only the CI applies when it builds its test machines. They are never built
into, or shipped with, AfriSovera.

| File | What it changes | Licence |
|---|---|---|
| `scripts/ci/qemu-usbnet-ncm.patch` | A CDC-NCM mode for QEMU's USB network device (`hw/usb/dev-network.c`), applied by the CI when it builds QEMU, to test the CDC-NCM driver (13 Nov 2026) | GPL-2.0-or-later, as the QEMU file it changes |
| `scripts/ci/qemu-uvc-test.patch` | A new QEMU test device, a USB Video Class camera (`hw/usb/dev-uvc-test.c`, and one line in `hw/usb/meson.build`), applied the same way, to test the webcam driver (13 Nov 2026) | GPL-2.0-or-later, as QEMU |
| `scripts/ci/qemu-hci-test.patch` | A new QEMU test device, a USB Bluetooth HCI controller (`hw/usb/dev-hci-test.c`, and one line in `hw/usb/meson.build`), applied the same way, to test the Bluetooth driver; test equipment only, never shipped (14 Nov 2026) | GPL-2.0-or-later, as QEMU |

## Compatibility and notices

BSD-3-Clause and the Apache-2.0 / MIT dual licences are permissive and compatible with distributing the combined
work under Apache-2.0. BSD-3-Clause and Apache-2.0 both require preserving their notices; this file plus the
vendored crates' own `LICENSE` files (kept under the vendor directory) carry them, and the public repository
includes the vendored licences. `cargo audit` runs in CI on any change to a lockfile (ADR 0007). Dependencies are vendored so a build never needs crates.io reachable.
