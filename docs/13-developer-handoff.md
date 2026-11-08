# 13 — Developer Handoff (read this first to take over)

> Onboarding for the next developer — human or AI — to continue AfriSovera cold.
> If you read one doc before touching code, read this, then [`07`](07-phase-0-plan.md) (the living build
> log with every milestone). Architecture: [`00`](00-the-from-scratch-decision.md)–[`06`](06-universal-app-compatibility.md). Strategy/sovereignty: [`08`](08-sovereignty-and-openness.md)–[`12`](12-governance-and-model.md).
> Voice: [`14`](14-asili-voice.md). Real-hardware bring-up: [`15`](15-hardware-bringup.md).

## What exists right now (Phase 0 → 2.71, ~51 milestones)

A **bootable, capability-secure Rust microkernel OS** for AArch64, verified in QEMU
(on the `virt` machine and `raspi3b`). Every milestone is boot-verified and guarded by
one of **12 regression suites** (see below); the living log is [`07`](07-phase-0-plan.md). Highlights:

- **Kernel (Mansa):** boot (arm64 Image header, EL2→EL1 drop) → MMU/identity map →
  frame allocator → heap → GICv2 + generic timer → exceptions → preemptive scheduler →
  **DTB parse** over a **board peripheral-map abstraction**.
- **Userspace:** EL0 processes, per-process address spaces, SVC syscalls, a
  capability table, IPC endpoints; **userspace drivers** (RTC via an MMIO capability).
- **SMP (multi-core):** a **second CPU core online via PSCI**, the secondary
  **enables the MMU** with the primary's tables, drains a **shared spinlock job
  queue**, runs its **own per-core preemptive scheduler**, and exchanges **cross-core
  SGIs + a round-trip IPC** with the primary. A cross-core console spinlock serializes
  both cores' output. `-smp 1` is unaffected.
- **Two boards:** QEMU `virt` and **Raspberry Pi** `raspi3b` from one codebase; the Pi
  boots MMU + heap + framebuffer and takes **interrupt-driven timer ticks** via the
  **BCM local interrupt controller** (no GIC) driving a **preemptive scheduler**.
- **Networking:** a **virtio-net** driver with ARP/IPv4/ICMP/UDP/DNS, a **DHCP client**
  (leases 10.0.2.15 from SLIRP), and **TCP**: a 3-way handshake to ESTABLISHED and a
  **request/response on it** — `net tcp send <port> [text]` carries a payload to a
  peer and reads the peer's answer back (milestone 2.752).
- **Storage + config:** **virtio-blk** + a **VFS-on-disk**; files and a **system
  config** (hostname, DHCP lease, boot counter) **survive reboots**.
- **Graphics/UI:** `ramfb` framebuffer, 8×8 font, the **Tonga** compositor (app-owned
  windows + input routing) with a **live system-status panel**, **AfriStore** (crypto
  **trust badges**).
- **Nia runtime:** a WASM interpreter (control flow, calls, linear memory, a
  capability-gated host ABI incl. `random`) — runs in-kernel and **as an EL0 process**
  (`niavm`); **integrity verify-on-launch** + a **DoS-hardened parser** (malformed
  modules are rejected, never crash the kernel). Plus **Nia Web** (`.webapp` renderer).
- **Crypto + RNG:** SHA-256/512 + **Ed25519** (all vector-validated), integrity
  manifest + verify-before-launch + signed-app pipeline; a user `hash` command; a
  **hardware-seeded PRNG** (`rand`) also gated behind `CAP_RANDOM` for WASM apps.
- **Asili AI/Voice:** on-device rule-based `ai()` (honest stub for a real model) and a
  real formant **voice synthesizer** through a **virtio-sound** driver.
- **Daraja:** an Android permission→capability translation bridge (scaffold).

Committed on branch `master` (no remote). **Emulator-verified, not hardware-verified** —
full real-device bring-up is Phase 3 ([`15`](15-hardware-bringup.md)).

## Environment / prerequisites

- **Rust stable** (`rustup`), target `aarch64-unknown-none`, components `rust-src`
  + `llvm-tools` (for `llvm-objcopy`). `cargo build` from the repo root builds the
  kernel for the bare-metal target (workspace `default-members = [kernel]`).
- **QEMU** (`qemu-system-aarch64`). On this dev machine it lives at
  `C:\Program Files\qemu` (not on PATH in old shells — add it, or the scripts do).
- **python3** with the `cryptography` package (for `sign-app.py`) and stdlib sockets
  (for the shell-driver harness).

## Build / run / test

```bash
cargo build -p mansa-kernel                 # debug kernel ELF
cargo build -p mansa-kernel --release       # size-optimized
bash scripts/smoke-test.sh                  # THE regression suite (also the CI gate)
```

Running & driving it:

- `scripts/run-graphics.sh [out.png]` — boot with a framebuffer, screenshot via QMP.
- `scripts/shell-shot.sh <out.png> <cmds...>` — boot, drive the shell over **TCP
  serial**, screenshot. A `:X` arg sends raw keys (for interactive apps).
- `scripts/audio-shot.sh <out.png> <cmds...>` — same, with a `virtio-sound` device +
  a `wav` audiodev (so `say`/`audio tone` produce a capturable WAV).
- `scripts/run-image.sh [MEM]` — build a **raw arm64 Image** and boot it so QEMU
  passes a **live DTB** in x0 (watch for `device tree (live)`).
- `scripts/run-raspi.sh` — build the **Raspberry Pi** variant and boot it on
  `-M raspi3b` (UART banner).
- `scripts/build-userapp.sh` — rebuild the EL0 userland crates → `kernel/src/*.bin`.
- `scripts/build-image.sh` — objcopy the kernel ELF → `target/mansa-kernel.img`.
- `scripts/gen-wasm.py` — the Python WASM assembler (edit + regenerate the embedded
  `WASM_*` modules in `nia.rs`; **watch the export func index = num_imports +
  local_index**).
- `scripts/sign-app.py WASM_SUM` — off-device Ed25519 signer (publisher toolchain).
- `scripts/gen-idl.sh` — regenerate `generated/ipc.rs` from `idl/*.afriidl`.

### Regression test suites (12) — the safety net

Each boots QEMU and asserts real behavior. **Running all 12 sequentially exceeds a
2-minute timeout** — run them backgrounded (redirect to a file, then read it) or in
subsets. Kill stray QEMU first (`taskkill //F //IM qemu-system-aarch64.exe`).

| Script | Proves |
|---|---|
| `smoke-test.sh` | boot + MMU + capability IPC + crypto self-tests + shell subsystems (the CI gate) |
| `smp-test.sh` | `-smp 2`: 2nd core online, MMU-on, drains job queue, per-core scheduler, SGI + IPC round-trip |
| `pi-test.sh` | `raspi3b`: MMU + heap + framebuffer + interrupt-driven preemptive scheduler |
| `dhcp-test.sh` | DHCP DISCOVER→OFFER→REQUEST→ACK, lease 10.0.2.15 acquired |
| `tcp-test.sh` | TCP SYN→SYN/ACK→ACK to ESTABLISHED, checksums verified in the pcap |
| `tcp-data-test.sh` | segment builder 10/10 + **data on the connection**: bytes delivered to a real host peer (which reports what it received), its reply read and ACKed, checksums verified in the pcap |
| `persist-test.sh` | a file written+synced in boot 1 is restored from disk in boot 2 |
| `config-test.sh` | system config (+ boot counter 1→2) survives a reboot |
| `hash-test.sh` | kernel SHA-256 + SHA-512 of `"abc"` match Python `hashlib` |
| `rand-test.sh` | the hardware-seeded PRNG produces varied, non-degenerate output |
| `rand-cap-test.sh` | capability-gated `random()`: granted app draws values, un-capable app denied |
| `dice-test.sh` | a WASM dice app rolls valid 1..6 via `CAP_RANDOM` + `i32.rem_u` |
| `wasm-fuzz-test.sh` | malformed WASM modules are rejected — no kernel panic, shell stays alive |

(`net-test.sh` and `disk-test.sh` also exist as targeted virtio-net / virtio-blk checks.)

### Gotchas the hard way (do not relearn these)

- **Run the 12 suites backgrounded**, not in one foreground command (they exceed the
  2-min tool timeout). Under heavy host load (many QEMU runs) TCP-serial capture goes
  flaky — kill stray QEMU, wait, retry; prefer the `*-test.sh` scripts (reliable
  structure) over ad-hoc `shell-demo.py` calls.
- **`kprintln!` must be ASCII-only.** The 8×8 font can't render an em-dash, and it
  emits a **non-UTF-8 byte** on the serial line — which then breaks any test that
  embeds the captured serial into a Python heredoc. Use `-`, not `—`. Tests that parse
  serial should read it as bytes and `.decode("latin1")` (and `grep -a`).
- **Graphics screenshots:** boot with **`-netdev` + `-device ramfb` only** — adding
  `-drive` too (three virtio-mmio devices) slows boot past the shell-demo capture
  window and the screenshot comes back blank. On this machine `/tmp` maps to
  `%TEMP%`.
- **`smp-test.sh` needs a generous sample window** (`sleep 12`): two emulated cores
  under TCG on a loaded host run the full bring-up + job-queue + SGI + IPC sequence
  slowly; a short window kills QEMU mid-sequence and reads as a spurious failure.

- **stdin to native QEMU is broken under MSYS/Git-Bash** (delivers ~1 byte). Drive
  the shell over **TCP serial** via `shell-demo.py` (that's what the `*-shot.sh`
  scripts do), never a stdin pipe.
- **`shell-demo.py` drains until the shell prompt** returns, scoped to bytes sent
  *after* each command — because the prompt (`afrisovera> `) has a trailing space that
  otherwise leaks into the next command's read and truncates capture. Don't "simplify"
  it back to a fixed sleep.
- **QMP screendump filename must be JSON-encoded** (Windows backslashes break raw
  interpolation) — `shell-demo.py`/`qmp-screendump.py` use `json.dumps`.
- **arm64 boot / load address:** QEMU loads a raw `Image` at **RAM base + 2 MiB**
  (it reserves the first 2 MiB for the DTB). The `virt` kernel is linked at
  `0x4020_0000`, the Pi kernel at `0x0020_0000`; both carry the 64-byte arm64 Image
  header. Link address mismatch → runs at the wrong PA → undefined-instruction loop.
- **Enable FP/SIMD before any code runs** (`cpu::enable_fp_simd()`): the compiler
  emits NEON for `memcpy`/`memset`/`fmt`; without it the first such instruction is an
  FP-access trap (ESR EC `0x7`). This bit the Pi bring-up.

## kernel/src module map

| Module | Purpose |
|---|---|
| `main.rs` | Boot asm (Image header, EL drop, BSS clear), `kernel_main` (virt + a minimal `board-raspi` variant), module wiring, boot self-tests |
| `cpu.rs` | EL query, FP/SIMD enable, IRQ mask, `wfi`/`wfe` |
| `paging.rs` | MMU on; identity map (device 0–1 GiB, Normal DRAM 1 GiB block) |
| `frame.rs` | Physical frame allocator (bitmap) over RAM above the kernel |
| `heap.rs` | 1 MiB bump global allocator (`alloc`: `Box`/`Vec`) |
| `exceptions.rs` | Vector table (2 KiB aligned), IRQ/SVC frames |
| `gic.rs` | GICv2 (distributor + CPU interface) |
| `timer.rs` | EL1 physical/generic timer tick |
| `uart.rs` | PL011 driver; `_print` tees to UART **and** the framebuffer console |
| `framebuffer.rs` | ramfb setup, console, `fill_rect`/`draw_string`/`draw_char` |
| `font.rs` | 8×8 bitmap font (binary-literal glyphs) |
| `fwcfg.rs` | QEMU `fw_cfg` (MMIO + DMA) — used to configure ramfb |
| `board.rs` | **Peripheral MMIO base map**, build-time board select (virt / raspi) |
| `fdt.rs` | Flattened-device-tree parser (RAM + UART/GIC discovery); bundled reference DTB |
| `asili.rs` | Device Manager (registry the `devices` command lists) |
| `sched.rs` | Kernel thread scheduler (context switch, round-robin) |
| `process.rs` | EL0 processes, address spaces, SVC dispatch, capabilities, IPC, userspace drivers; syscalls DEBUG=0…SPEAK=14 |
| `vfs.rs` | Flat in-RAM filesystem (leak-free static table) |
| `tonga.rs` | Compositor / window server (app-owned surfaces, input) |
| `shell.rs` | AfriSh — the bring-up REPL (see command list below) |
| `store.rs` | AfriStore graphical catalog + crypto **trust badges** |
| `nia.rs` | WASM interpreter (control flow, memory, host ABI incl. `random`) + **verify-on-launch** + **hardened parser** (`nia::fuzz`) + Asili AI + embedded `WASM_*` modules |
| `virtio.rs` | virtio-mmio bus + drivers: **-sound** (PCM), **-net** (ARP/IPv4/ICMP/UDP/DNS/**DHCP**/**TCP handshake**), **-blk** (block storage) |
| `voice.rs` | Asili Voice synthesizer (glottal source + biquad formant resonators + coarticulation) |
| `web.rs` | Nia Web runtime — parses a `.webapp` HTML subset → DOM → renders it |
| `daraja.rs` | Android permission → AfriSovera capability translation (bridge scaffold) |
| `sha256.rs` / `sha512.rs` | FIPS 180-4 hashes (self-tested against the `"abc"` vectors) |
| `ed25519.rs` | Ed25519 signature **verification** (TweetNaCl-derived; RFC 8032 self-test) |
| `integrity.rs` | Integrity manifest, verify-before-launch, publisher key + signatures, `trust()` |
| `rng.rs` | Hardware-seeded PRNG (SplitMix64 seeded from `CNTPCT`/board timer via SHA-256); `next_u64/u32/fill` |
| `config.rs` | Persistent system config (`afrisovera.conf`: hostname/DHCP lease/boot counter) via the VFS + disk |

SMP lives in `main.rs` (secondary bring-up, per-core scheduler state via `sched.rs`'s
`SCHED1`, the shared job queue, and the cross-core SGI/IPC path); `cpu::core_id()` reads
the MPIDR; `paging::enable_mmu_secondary()` turns on translation for core 1.

**Userland** (`userland/`, own crates, linked at their board base, objcopy'd to raw
binaries the kernel `include_bytes!`s): `rustapp` (compiled-Rust EL0 demo), `niavm`
(the Nia WASM interpreter running as an EL0 process; host calls = SVC syscalls).

## Shell commands (AfriSh)

`help about mem uptime echo clear caps devices board dtb ls cat write touch rm gui wm
edit rtc rustapp niavm nia store audio say web daraja verify verify-sig`

Security-relevant ones: `verify <app>` (SHA-256 vs manifest, `tamper` to demo a
mismatch), `verify-sig <app>` (Ed25519 publisher signature), `store` (badges),
`daraja <file>` (permission translation), `caps` (capability model).

## The crypto / signing story (milestones 2.32–2.38)

Publishers sign **off-device** (`sign-app.py` signs an app's SHA-256 digest with an
Ed25519 key). The device only **verifies**: `ed25519.rs` (validated vs RFC 8032),
`integrity.rs` holds the publisher key + per-app signatures, `verify-before-launch`
refuses a tampered app, and the store shows **SIGNED** (Ed25519) / **VERIFIED**
(SHA-256 integrity) badges. Honest boundary: this is integrity + authorship, **not**
the safety net — the capability sandbox is what contains a malicious app ([`10`](10-security-model.md)).

## The two-board story (milestones 2.27–2.31)

`board.rs` centralizes MMIO bases; drivers read from it. `fdt.rs` parses the live DTB
(from a raw `Image` boot). `--features board-raspi` selects the BCM2837 map + a
board-specific linker script + a minimal MMU-off UART bring-up; `run-raspi.sh` boots
it on `-M raspi3b`. See [`15`](15-hardware-bringup.md) for the full peripheral-map table and what real
hardware still needs (A64 UART driver, mailbox framebuffer, DTB-driven map).

## What is real vs stub, and what's next

**Real + verified:** the whole kernel/userspace/IPC/capability core; the WASM runtime
(in-kernel and EL0); the compositor + store UI; the voice **audio path** and formant
synthesis; the virtio-sound driver; device-tree discovery; two-board boot; the full
crypto/signing stack.

**Honest stubs / scaffolds:** Asili **AI** is rule-based (placeholder for a real
quantized model, [`09`](09-asili-ai.md)); **Daraja** translates permissions but does **not** run Android
bytecode; **Nia Web** is a DOM-subset renderer, not a browser; voice is robotic
(formant synthesis, not neural words).

**What's next (candidates):**
- **Piper neural TTS** — replace the formant synth's PCM with a real open TTS model
  over the same audio path (needs the model + an inference runtime; deferred).
- **Networking** — the virtio-net driver and the IP/UDP/DNS/DHCP/TCP paths exist and
  are exercised by suites; a connection is now opened, carries bytes both ways, and is
  **actively closed** (FIN_WAIT_1 → FIN_WAIT_2 → TIME_WAIT, suite `tcp-close`). What
  TCP still lacks is **retransmission, window and congestion control, out-of-order
  reassembly, a listening side, a real 2 MSL TIME_WAIT dwell, and more than one
  connection at a time** — i.e. a state machine rather than a request/response.
- **Fuller Pi bring-up** — BCM2837 paging + its legacy interrupt controller so more
  than the UART runs on `raspi3b`.
- **Store packaging** — a real publisher-key format, per-app signature files, and a
  key-trust policy (current signing is a built-in demo key + one signed app).
- **Real Android/iOS device reflash** — the device-ownership pitch ([`12`](12-governance-and-model.md)); honest limits apply.

## How to extend (recipes)

- **New WASM host function:** add a host id + capability in `nia.rs`, dispatch it in
  `host_call`; for the EL0 interpreter add an SVC syscall in `process.rs` and map the
  import name in `userland/niavm`.
- **New shell command:** add a match arm in `shell.rs` and a `help` line.
- **New WASM app:** add a `build_*` in `gen-wasm.py`, `emit()` it, paste the bytes
  into `nia.rs`, seed it in `main.rs`, (optionally) sign it with `sign-app.py`.
- **New board:** add its map to `board.rs` behind a feature, a linker script, and a
  `kernel_main` variant; wire `build.rs` to pick the script.
- **Always** re-run `scripts/smoke-test.sh` and add an assertion for anything new.
