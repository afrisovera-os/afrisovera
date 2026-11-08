# AfriSovera specifications (AFR series)

Normative, RFC-style specifications of AfriSovera's interfaces: system calls, capabilities, IPC, the driver
ABI, file and package formats, and network protocols. Each spec describes **the code as built** (with file
references and the commit it describes) and, separately, **the next version as designed**. Nothing designed
is written in the present tense.

Decisions behind the specs: [`../adr/`](../adr/README.md).
Review findings: [`../review/`](../review/). Licence: Apache-2.0 (ADR 0005) — anyone may implement them.

## Status words

| Word | Meaning |
|---|---|
| **Implemented** | the code does what the section says; suites named in the spec exercise it |
| **Partly implemented** | some sections are built; each section says which |
| **Designed** | agreed shape, no code yet; conformance tests listed but not written |
| **Draft** | not yet agreed; open for comment |

Every spec here is open for comment for at least 7 days from its date (docs/policy/GOVERNANCE.md).

## Index

| Spec | Title | Status | Built today | Comment until |
|---|---|---|---|---|
| [AFR-0001](0001-mansa-syscall-capability-abi.md) | Mansa system-call and capability ABI | v0 Implemented (two dialects) · v1 Designed | `svc #0` / `int 0x80`, 18 + 14 calls, 4-slot caps, one endpoint | 2026-11-15 |
| [AFR-0002](0002-asili-driver-interface.md) | Asili driver interface | v0 Implemented (in-kernel conventions) · v1 Designed | registry, block/NIC/audio function sets, ~20 in-kernel drivers | 2026-11-15 |
| [AFR-0003](0003-nia-package-and-permissions.md) | Nia app package format and permissions | v0 Implemented · v1 Designed | bare `.wasm`, 10 permission bits, SHA-256 gate | 2026-11-15 |
| [AFR-0004](0004-afriidl.md) | AfriIDL language and wire encoding | v0 Implemented · v1 Designed | host compiler, scalar messages, Rust output (not used by the kernel) | 2026-11-15 |
| [AFR-0005](0005-afristore-federation.md) | AfriStore federation protocol | Designed | store screen over local files only | 2026-11-15 |
| [AFR-0006](0006-afripay.md) | AfriPay payment-rail protocol | Designed (regulated: no live provider without owner + counsel) | nothing | 2026-11-15 |
| [AFR-0007](0007-afriid.md) | AfriID identity protocol | Designed | nothing | 2026-11-15 |
| [AFR-0008](0008-pc-boot-and-install.md) | PC boot and install chain (GPT, ESP, boot entries) | Implemented · Secure Boot, A/B Designed | UEFI app, `installdisk`, `installpart`, `bootcopy`, `bootentry`, `bootnext` | 2026-11-15 |

Test-vector files: [`vectors/`](vectors/). All use the public RFC 8032 test key 1 — never a real key.

## Registries

**AfriIDL protocol ordinals** (AFR-0004 §4; allocate here before use, never reuse):

| Range / ordinal | Owner |
|---|---|
| `0x0001`–`0x00FF` | tests and examples (`0x0001` Greeter, AFR-0004 §8.2) |
| `0x0101` block · `0x0102` net · `0x0103` audio out · `0x0104` input | Asili device classes, AFR-0002 §7 |
| `0x0201` Payments · `0x0202` RailAdapter | AfriPay, AFR-0006 |
| `0x0301` Identity | AfriID, AFR-0007 |
| `0x0400`–`0x04FF` | reserved: Tonga (surfaces, input focus, trusted overlay) |
| `0x0500`–`0x05FF` | reserved: VFS / storage service |
| `0x0600`–`0x06FF` | reserved: netstack |

**Signature context strings** (AFR-0003 §5.3 block; every new signed format takes a new string):
`AFRISOVERA-NIA-SIG-V1\0`, `AFRISOVERA-STORE-DESCRIPTOR-V1\0`, `AFRISOVERA-STORE-CATALOG-V1\0`,
`AFRISOVERA-STORE-REVOKED-V1\0`, `AFRISOVERA-PAY-REQUEST-V1\0`.

**Permission names** (AFR-0003 §6; v0 bit in brackets): `print`(0) `putc`(1) `draw`(2) `file`(3) `time`(4)
`input`(5) `ai`(6) `voice`(7) `net`(8) `random`(9) `pay`(10, reserved) `identity`(11, reserved).

**GPT partition type GUIDs** (AFR-0008): `A5F1C0DE-0B5E-4A50-9AF1-50FE2A000001` AfriSovera data (built);
`…000002` system slot A, `…000003` system slot B (designed).

## Numbering

`AFR-NNNN`, four digits, never reused. A spec that replaces another gets a new number and the old one is
marked *Superseded by AFR-NNNN*. Minor corrections keep the number and bump the spec's own `Version`.
