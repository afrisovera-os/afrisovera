# ADR 0004 — Capability security, no ambient authority

- **Status:** Accepted — **partly realised**
- **Date:** 2026-08-05 (Phase 0 demo: a process without SEND is refused by the kernel)
- **Deciders:** owner; recorded by the chief architect 2026-11-08
- **Related:** ADR 0001, ADR 0003; specs AFR-0001 (kernel capabilities), AFR-0003 (app permissions);
  docs/10-security-model.md

## Context

The dominant malware classes on desktop systems work because authority is ambient: any running code
inherits the user's full power. Permission prompts layered on top of ambient authority (classic Android,
Windows UAC) are advisory and routinely bypassed. The project's users are often first-time smartphone
owners, poorly served by vague "allow everything" dialogs. We want a model in which code holds nothing it
was not explicitly given, and in which the OS can show precisely what was given.

## Decision

1. **No ambient authority.** A process starts with no rights except to compute on its own memory. There
   is no root user, no `setuid`, no privileged UID.
2. **Every authority is a capability**: an unforgeable kernel-managed reference to one object with a
   rights mask, named by a per-process handle (AFR-0001).
3. **Capabilities are passed explicitly** — granted at creation by the parent, or moved over IPC. The only
   way to widen another process's power is to hand it something you hold. Rights can only be narrowed
   when duplicated.
4. **App permissions are capabilities**, not flags in a policy database: an app granted "network" holds a
   channel to the network service; an app not granted it holds nothing to call (AFR-0003 §6).
5. **The user grants, the OS shows**: the permissions an app requests are declared in its package, shown
   before install, and revocable afterwards.

## Where the code is today (2026-11-08)

| Element | Status |
|---|---|
| Kernel capability check on IPC | **Built**: 4-slot table, SEND/RECV rights (AFR-0001 §6) |
| Grant at spawn | **Built** (kernel-only) |
| Transfer over IPC, duplicate with narrower rights, revoke | **Not built** |
| Non-IPC syscalls (window, AI, speech, time) capability-gated | **Not built** — ambient today (AFR-0001 §8.2) |
| Nia host-function permission bits | **Built**: 10-bit mask checked on every host call |
| Per-app grants | **Not built** — every app launched from a file gets all ten bits (AFR-0003 §6.3) |
| Package-declared permissions, install-time display, revocation | **Not built** |

## Consequences

Good:
- Malware that runs holds nothing; the security argument is structural rather than signature-based.
- Permissions are exact and explainable ("this app can reach the network service"), which suits
  low-literacy UI.
- Composes with the microkernel: a driver's capabilities *are* its sandbox.

Bad / accepted costs:
- Every service interface must be designed around handles; there is no "just open a path" shortcut.
- Users can still grant a malicious app a real capability; the model narrows abuse, it does not end
  social engineering (docs/10 "honest limits").
- Revocation needs care (AFR-0001 §7.4 chooses revocation by closing an intermediary channel).

## Alternatives considered

| Option | Why not |
|---|---|
| UNIX users/groups + ACLs | Ambient by design; root exists. |
| Android-style install-time permission flags enforced by a system server | Enforcement outside the kernel; history of bypasses; still has a root. |
| Mandatory access control (SELinux) | Powerful, but policy is global, complex and opaque to users; layered over ambient authority. |

## Revisit if

- A required compatibility layer (Daraja for Android apps) cannot be expressed in capabilities without a
  broad ambient grant — then the box gets the broad grant, not the system.
