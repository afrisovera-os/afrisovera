# Architecture Decision Records

An Architecture Decision Record (ADR) writes down one significant decision: the forces at the time, what
was chosen, what was given up, and how we would know it was wrong. ADRs are **append-only**. A decision is
changed by writing a new ADR that supersedes the old one, never by editing the old one's Decision section
(typos and links may be fixed).

Format: Michael Nygard's, plus two sections this project needs — **Evidence in the tree** (where the code
shows the decision is real, so a reader can check it) and **Revisit if** (the observations that would
reopen it).

| ADR | Title | Status | Date |
|---|---|---|---|
| [0001](0001-microkernel.md) | Mansa is a capability microkernel | Accepted (partly realised) | 2026-08-05 |
| [0002](0002-rust.md) | Rust for the kernel, drivers and services | Accepted | 2026-08-05 |
| [0003](0003-wasm-native-apps.md) | Native apps are WebAssembly modules (Nia) | Accepted | 2026-08-05 |
| [0004](0004-capability-security.md) | Capability security, no ambient authority | Accepted (partly realised) | 2026-08-05 |
| [0005](0005-apache-2-license.md) | Apache-2.0 for the platform code | Accepted | 2026-08-05, public copy 2026-11-08 |
| [0006](0006-pc-track.md) | x86-64 PCs and laptops are a full target | Accepted | 2026-11-03 |
| [0007](0007-crypto-crate-dependencies.md) | Third-party crates only for cryptographic primitives, pinned, vendored and audited | Accepted | 2026-11-10, written 2026-11-13 |
| [0008](0008-pc-entropy-without-rdrand.md) | Counted entropy on PCs without RDSEED or RDRAND | Proposed (step 1 accepted) | 2026-11-10 |

Status words: **Proposed**, **Accepted**, **Superseded by NNNN**, **Deprecated**. "Partly realised" means
the decision stands but the code does not yet carry all of it; the ADR says exactly which part.

New ADR: copy the section headings of 0001, take the next number, add a row above.
