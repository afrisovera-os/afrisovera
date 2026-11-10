# ADR 0008 - Counted entropy on PCs without RDSEED or RDRAND

- **Status:** Proposed (step 1 accepted for work; counting waits on the evidence in "Decision" point 4)
- **Date:** 2026-11-10
- **Deciders:** chief architect, on the OS lead's request (review follow-up F3 of the mesh window)
- **Related:** `kernel/src/drbg.rs` (SP 800-90A HMAC_DRBG, SP 800-90B health tests, fail-closed `fill_secret`),
  `kernel/src/pc/compat/rng.rs`, ADR 0006 (PCs are a full target), AFR-0009 (mesh link and bundles),
  ADR 0007 (crypto crates; cited in the tree, file not yet written)

## Context

Since the mesh window (merged 10 Nov 2026) the kernel has one generator, `drbg.rs`. Keys, nonces and ephemerals
come only from `fill_secret`. That call reseeds from **counted** hardware entropy for every request and refuses
(with a zeroed buffer) when it has none. Clock readings and timer jitter are mixed in, but never counted.

On the PC the counted sources are RDSEED and RDRAND. Many of the machines this project is for do not have them:

| CPU family | Typical machines | RDRAND | RDSEED |
|---|---|---|---|
| Intel Sandy Bridge (2011-12: Core i3/i5/i7-2xxx, Pentium B9xx, Celeron B8xx) | most budget laptops sold in 2012 | no | no |
| Intel Ivy Bridge (2012-13: Core 3xxx) | later 2012 laptops | yes | no |
| Intel Haswell (2013-14) | | yes | no |
| Intel Broadwell and later (2014+) | | yes | yes |
| AMD before Excavator/Zen (E-, A-series, Phenom, Athlon II) | many budget laptops up to 2015 | no | no |

On a Sandy Bridge or older AMD laptop, `fill_secret` refuses today, so that PC cannot make a mesh identity or
any other secret. This is correct (fail closed), but it locks out the 2012 laptop that ADR 0006 and the owner's
target list care about. We need a counted source that such a PC can trust. The two candidates are a CPU-timing
jitter source and the TPM's random number generator.

### What the PC kernel can do today

- **TSC:** `timer::rdtsc()` reads it, and `timer::hz()` gives a calibrated frequency. Sandy Bridge and later Intel
  CPUs, and AMD family 10h and later, have an invariant TSC with one-cycle resolution, the timer a jitter source
  needs. `drbg.rs` already hashes 256 TSC deltas per (re)seed as uncounted jitter.
- **ACPI:** `pc/acpi.rs` walks the XSDT, but only for `FACP` and `APIC`. It does not look for `TCPA` (TPM 1.2) or
  `TPM2` (TPM 2.0) yet.
- **TPM:** no driver. The PC kernel has never touched a TPM.
- **Generator:** `Platform::STRONG` allows up to 3 sources (`MAX_SOURCES`), but the health-test cutoffs are global
  constants for an assumed H = 4 bits per byte. A jitter source has a much lower, measured H and needs its own
  cutoffs.

## Decision

1. **Both sources, in this order of trust:**
   a. **The TPM's RNG** (`TPM_GetRandom` on TPM 1.2, `TPM2_GetRandom` on TPM 2.0, over the TIS FIFO at
      `0xFED40000`, or CRB on firmware TPMs), when the ACPI `TCPA` or `TPM2` table says one is there and enabled.
      It is a counted strong source like RDRAND: the same byte health tests and the same assumed H = 4 bits per
      byte (its output already passed the TPM's own DRBG). Many 2012 business laptops (EliteBook, ProBook,
      ThinkPad, Latitude) carry a discrete TPM 1.2. Consumer laptops of that age mostly do not, and the TPM is
      often switched off in the firmware setup.
   b. **A CPU-timing jitter source** as a non-physical noise source in the sense of SP 800-90B. It times a short,
      memory-touching loop with the TSC, health-tests the raw deltas, and conditions them with SHA-256 (a vetted
      conditioning function, 90B section 3.1.5.1.1). The same design is in the jitterentropy library, which has
      90B entropy-source certificates and is the counted fallback source in Linux. It works on every PC with an
      invariant TSC, so it covers the consumer 2012 laptop without a TPM.
2. **Per-source entropy and cutoffs.** `drbg::Platform` gives each strong source its own claimed H (in 1/64 bits
   per sample) and its own sample type, and the RCT and APT cutoffs come from that H (90B sections 4.4.1 and
   4.4.2) instead of the global constants. Sources keep their existing H = 4 bits per byte. A reseed counts bits,
   not bytes: it collects until it has 256 counted bits from the sources that are healthy.
3. **Conservative credit for jitter.** The jitter source never claims more than the lower of: (a) the minimum
   entropy estimate from NIST's `ea_non_iid` tool (SP 800-90B section 6.3) on raw deltas from that CPU class,
   divided by 2; and (b) 1/8 bit per delta. At 1/8 bit, 256 bits need 2,048 deltas. A delta is about 1-5 us on a
   2012 CPU, so one secret costs roughly 2-10 ms. That is fine for keys and handshakes.
4. **Counting only after evidence.** The jitter source is counted on a CPU class only after raw samples from at
   least one real machine of that class have been through `ea_non_iid` and the restart test (90B section 3.1.4),
   with the result stored in `docs/` with the data's hash. Until then it is mixed in, as today, and not counted.
   Under virtual machines (QEMU TCG, and KVM where the TSC is trapped or scaled) the timing is not the physical
   CPU's. The jitter source then refuses to count: it detects the hypervisor bit (CPUID.1:ECX bit 31) and stays
   uncounted. This keeps QEMU suites honest: `fill_secret` keeps refusing on a QEMU CPU without RDRAND.
5. **What stays out.** A seed file saved at shutdown is mixed in but **not** counted. A live USB stick is copied to
   many machines, so its seed is not secret, and the disk may be writable by an attacker. Microphone noise,
   network or disk timing and keyboard timing are not counted either: they are hard to assess, and an attacker can
   influence them.
6. **The fail-closed rule does not change.** When no counted source is present and healthy, `fill_secret` refuses
   and the mesh says so in plain words. There is no "weak mode".

## The smallest first step (PC only, ARM image unchanged)

Diagnostics, no counting change:

- `rng sources` on the PC lists the CPUID flags (RDRAND, RDSEED, invariant TSC, hypervisor bit) and whether ACPI
  has a `TCPA` or `TPM2` table (add those two signatures to the XSDT walk in `pc/acpi.rs`, read-only).
- `rng jitter dump <n>` prints `n` raw TSC deltas of the jitter loop over the serial console in hex, in the
  `ea_non_iid` input format (8-bit samples: the low byte of each delta, plus the full deltas for a second run).
- A suite `pc-rngsources` checks under QEMU that both commands run and that the hypervisor bit is reported.

Then, outside the team's own work: the owner runs `rng jitter dump 1000000` once on his 2012 laptop and saves the
serial log (a real-hardware test, so it goes to him; see TEAM_PROTOCOL.md section 2). The cloud runs `ea_non_iid`
on it. Step 2 (per-source H in `drbg.rs`, a kernel-wide crypto change) is reviewed by the architect against that
number. Step 3 is the TPM driver, reviewed separately.

## Consequences

- A 2012 laptop without RDRAND gets mesh identities from a measured, health-tested source instead of being locked
  out. When a TPM is present, the TPM is used in addition.
- `drbg.rs` gains per-source H. This is shared with the ARM kernel, so the ARM image moves once, in step 2, with
  the architect's agreement. ARM keeps its sources and cutoffs.
- Secrets cost a few milliseconds more on jitter-only machines.
- The jitter source's claim rests on an assessment of the CPU class, not on each machine. The 1/8-bit cap and the
  continuous health tests are the guard against a machine that behaves worse than the assessed one.

## Alternatives considered

- **Keep refusing on these PCs.** It is safe, but it shuts out the machines the PC track exists for.
- **Count the existing 256-delta jitter hash as it is.** Rejected: no assessment, no health tests on raw samples,
  and the credit would be a guess.
- **TPM only.** Rejected: most consumer laptops of that age have no TPM, or have it switched off, and switching it
  on is a firmware setting the owner would have to change.
- **Port jitterentropy (C) as it is.** Rejected for now: it would bring a C build into a Rust kernel. We take its
  design (and later its test vectors), written in Rust over the kernel's own SHA-256.

## Revisit if

- `ea_non_iid` gives less than 1/8 bit per delta on a target CPU class: lower the cap for that class, or do not
  count jitter there.
- A published attack shows that TSC jitter on these CPUs can be predicted or steered from user space or a
  co-resident VM.
- RDRAND or RDSEED become available on every machine we target (then jitter stays only as an extra).
- The TPM driver shows TPM 1.2 RNG faults in the field (stuck output caught by the RCT): then the TPM drops to
  mixed-in, not counted.
