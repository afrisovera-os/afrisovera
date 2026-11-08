# 10 — Security Model: Why Malware Can't Breathe

> Windows is plagued by malware; macOS/iOS far less. The usual explanation is
> market share, but the deeper reason is **architecture**: sandboxing, code
> signing, and less ambient authority. AfriSovera pushes that architecture to its
> logical end. This doc explains, honestly, why the malware classes that dominate
> Windows are **structurally impossible** here — and where real risk remains (we
> will not claim "unhackable").

## The one idea: no ambient authority

On Windows, a process runs with **the user's full authority** by default — it can
read your files, reach the network, touch other processes, and often escalate to
admin. Malware thrives because *authority is ambient*: get code running and you
inherit power.

On AfriSovera, a process starts with **zero authority.** It can compute, and
nothing else. Every access — a file, a socket, the camera, another service — is an
**unforgeable capability** the process must be explicitly handed ([`01`](01-kernel-mansa.md)). A virus
that lands on the device and starts executing has **no capabilities**, so it can:

- read your files? ❌ no file capability
- reach the network to exfiltrate or spread? ❌ no network capability
- infect other apps? ❌ can't even see them; no capability to their storage
- escalate to root? ❌ **there is no root** — no `setuid`, no ambient admin
- persist by editing system files? ❌ no capability to them; verified boot rejects tampering

It runs, it has no oxygen, it dies useless. You already saw the mechanism in
miniature: in the Phase 0 demo a process without a SEND capability had its
operation **refused by the kernel**. That same refusal is the whole security story.

## The layers (defense in depth)

| Layer | What it kills |
|---|---|
| **Capability security (kernel)** | Ambient-authority abuse; privilege escalation; lateral movement |
| **WASM sandbox (app runtime)** | Arbitrary native code execution; memory-safety exploits; buffer overflows escaping the app |
| **Microkernel isolation** | A compromised driver/service taking down the system or reaching others (it's a boxed userspace process) |
| **No root / no setuid** | The entire Windows "escalate then own the box" attack class |
| **Signed apps + verified boot** | Tampered binaries; boot-time rootkits; supply-chain swaps (with reproducible builds you can *verify* the image) |
| **App integrity gate (built, 2.34)** | Tampered/corrupted app bytes — the launcher recomputes each app's SHA-256 and refuses a mismatch *before* running it (a real, load-time check today; asymmetric **authorship** signing via Ed25519 is the next layer) |
| **Rust kernel + drivers** | ~70% of OS CVEs (use-after-free, buffer overflow) — eliminated at compile time |
| **Per-app network capability** | Silent data exfiltration; command-and-control (no cap → the app physically cannot open a socket) |

Classic virus behavior — infect executables, escalate privilege, spread over the
network, persist in system files — requires powers that **do not exist by default**
on AfriSovera. There is nothing to abuse.

## Why this beats "antivirus"

Windows bolts on antivirus that *scans for known-bad code after the fact*.
AfriSovera makes bad code **powerless by construction** — a fundamentally stronger
position than a blocklist that's always one step behind. There's no signature
database to keep current because there's no ambient authority to protect.

## The honest limits (we do not claim "unhackable")

A responsible security model states its failure modes:

1. **User-granted capabilities.** If you *grant* a malicious app a file or network
   capability, it can misuse that specific grant. Mitigations: capabilities are
   **narrow, explicit, and revocable**, and the OS shows exactly what an app is
   asking for (no vague "allow everything"). Social engineering is a real risk —
   architecture reduces it, it can't erase it.
2. **Kernel/TCB bugs.** The trusted computing base (kernel + a few services) must be
   correct. We shrink it deliberately (tiny microkernel), write it in Rust, and aim
   at **seL4-style formal verification** of the core ([`01`](01-kernel-mansa.md)) — a bar no mainstream
   phone OS meets.
3. **Bridged foreign apps.** An Android/Windows app run via Daraja ([`06`](06-universal-app-compatibility.md)) is boxed
   and capability-gated like everything else, but it carries its own platform's
   weaknesses *inside its box*. The box contains the blast radius; it doesn't make
   a bad Android app good.
4. **Hardware / side channels.** Out of scope for software guarantees; mitigated by
   partner-certified hardware.

## Bottom line

Malware "not breathing" on AfriSovera isn't luck or obscurity — it's that an app
has **no authority to abuse** until you grant it, the runtime **can't run rogue
native code**, and there's **no root to escalate to**. The dominant Windows attack
classes are designed out. What remains — misused grants, TCB bugs — we shrink,
verify, and make honest about. That's a security posture you can actually reason
about, which is the point.
