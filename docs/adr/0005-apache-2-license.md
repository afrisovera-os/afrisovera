# ADR 0005 — Apache-2.0 for the platform code

- **Status:** Accepted
- **Date:** 2026-08-05 (`license = "Apache-2.0"` in `kernel/Cargo.toml` since Phase 0); public copy chosen
  by the owner 2026-11-08
- **Deciders:** owner; recorded by the chief architect 2026-11-08
- **Related:** docs/08-sovereignty-and-openness.md §Licensing, docs/12-governance-and-model.md

## Context

The project's survival bet is an ecosystem: OEMs, driver writers and app developers must be able to adopt
the platform without legal friction. Hardware vendors' legal teams routinely refuse GPL kernels for
proprietary-adjacent work and routinely accept Apache-2.0 (AOSP precedent). An explicit patent grant
matters to OEMs more than any other clause. At the same time the *sovereign services* (store, identity,
payments, push) must not be recapturable by a company that forks them, closes them and gatekeeps again.

On 2026-11-08 the owner chose to keep the private repository and publish a **clean public copy** under
Apache-2.0, made by the release engineer.

## Decision

1. The **platform** — Mansa kernel, Asili drivers, Nia runtime, SDK, AfriIDL and its tools, Tonga, shell,
   libraries, and these specifications — is licensed **Apache-2.0**.
2. The specifications in `docs/spec/` are Apache-2.0 as well, so anyone may implement them (an
   independent AfriStore mirror, a compatible Nia runtime) without permission.
3. The **server sides** of the AfriCore sovereign services (AfriStore, AfriID, AfriPay, AfriPush) are
   *not built*. docs/08 recommends a copyleft licence (AGPL-3.0 or MPL-2.0) for them. That remains a
   **separate, open decision**, to be taken in its own ADR before the first server-side line is written,
   with counsel. Nothing in this ADR licenses those servers.
4. The name "AfriSovera" and any certification mark are trademarks, not licensed by Apache-2.0 (§6 of the
   licence already excludes trademarks).

## Consequences

Good:
- Lowest-friction adoption for OEMs and app developers; explicit patent grant and patent-retaliation clause.
- Specifications can be implemented by anyone, which is what makes federation (AFR-0005) real.

Bad / accepted costs:
- A company may ship a closed derivative of the platform. Accepted: the defence is the trademark and the
  certification programme, not the code licence.
- Third-party code brought in must be Apache-2.0-compatible (MIT, BSD, ISC, Apache). GPL-only code may not
  be copied into the platform; Linux is used as a *reference* for hardware behaviour, never copied.
- Every public file needs a clean provenance; the release engineer's public copy must pass a secrets and
  provenance audit before each push.

## Evidence in the tree

- `kernel/Cargo.toml` line 6: `license = "Apache-2.0"`.
- Drivers cite Linux behaviour in comments (e.g. "as Linux does") but contain no copied Linux code; reviewers
  should keep it that way (see docs/review/).

## Revisit if

- A hardware partner can only release a driver under GPL — then it lives as a separate user-space driver
  process (the Asili model makes that a clean boundary), not inside the platform tree.
- Counsel advises a different licence for the specs (e.g. CC-BY-4.0) before 1.0.
