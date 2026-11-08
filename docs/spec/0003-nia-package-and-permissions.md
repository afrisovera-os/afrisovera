# AFR-0003 — Nia app package format and permissions

| | |
|---|---|
| **Status** | **v0 Implemented** (bare `.wasm`, 10 permission bits, SHA-256 gate); **v1 Designed** (`.nia` signed package with manifest) |
| **Version** | 0.1 (describes code at `485b863`, 2026-11-08) |
| **Editor** | Chief architect |
| **Code** | `kernel/src/nia.rs`, `kernel/src/integrity.rs`, `kernel/src/ed25519.rs`, `kernel/src/store.rs`, `kernel/src/vfs.rs`, `userland/niavm`, `scripts/sign-app.py` |
| **Vectors** | `docs/spec/vectors/afr0003_vectors.py`, `docs/spec/vectors/hello.nia` |
| **Depends on** | AFR-0001 (v1 channels carry permissions), AFR-0005 (distribution), ADR 0003, ADR 0004 |
| **Licence** | Apache-2.0 |

RFC 2119 key words apply.

---

## 1. Motivation

Nia apps are the native app class (ADR 0003). An app today is a bare WebAssembly module that receives
every permission when launched from a file, and the only integrity check compares its hash against what
was on disk at boot. For an ecosystem we need: a package that says who made it and what it wants; a
signature anyone can verify offline; permissions the user sees before install and can withdraw after;
and one format that every store, sideload path and AfriShare transfer uses.

## 2. Status summary

| Element | v0 (built) | v1 (designed) |
|---|---|---|
| Container | bare WASM 1.0 module, `*.wasm` | `.nia` container (§5) holding manifest + module + signature |
| Identity of the app | file name | `id` in the manifest (reverse-DNS) + publisher key |
| Permissions | 10-bit mask chosen by the launcher; `run_file` grants all | declared in the manifest, granted by the user, stored per app |
| Integrity | SHA-256 compared with the hash recorded at boot | Ed25519 signature over the package digest |
| Authorship | one hard-coded demo key, one signature (`sum.wasm`) | publisher keys with key ids; trust policy (§8) |
| Import namespace | module name ignored, field matched | module MUST be `afri` |
| Size | 512 bytes (VFS file limit) | up to 16 MiB per package |

## 3. Terminology

- **Module** — a WebAssembly 1.0 binary.
- **Package** — a `.nia` file (v1).
- **Publisher key** — an Ed25519 key pair; its **key id** is the first 8 bytes of SHA-256 of the 32-byte
  public key.
- **Permission** — a named right to a host-function group; in v1 backed by a capability (ADR 0004).

## 4. v0 — the module as run today

### 4.1 Module acceptance

A module MUST begin with `00 61 73 6D 01 00 00 00` (`\0asm`, version 1). Sections 1 (type), 2 (import),
3 (function), 6 (global), 7 (export), 10 (code), 11 (data) are processed; others are skipped; a section
that runs past the end is rejected (`truncated section`). The module MUST export a function named `run`.

### 4.2 Limits (kernel interpreter)

| Limit | Value |
|---|---|
| types / functions / imports / globals | 16 / 16 / 8 / 16 |
| operand stack / locals / control depth | 256 / 32 / 32 |
| linear memory | 65 536 bytes (one page), not growable |

### 4.3 Host functions and permission bits

The import's **field** name selects the host function; the module name is ignored in v0.

| Bit | Permission | Host functions (signature) |
|---|---|---|
| 0 | `print` | `print(i32)`, `print64(i64)` |
| 1 | `putc` | `putc(i32)` |
| 2 | `draw` | `win_create(w,h)->i32`, `win_fill(wid,x,y,w,h,color)`, `win_present()`, `win_text(wid,x,y,ch,color)` |
| 3 | `file` | `file_write(nptr,nlen,dptr,dlen)->i32`, `file_read(nptr,nlen,bptr,cap)->i32` (−1 on error) |
| 4 | `time` | `time()->i32` |
| 5 | `input` | `getkey()->i32` |
| 6 | `ai` | `ai(pptr,plen,optr,ocap)->i32` |
| 7 | `voice` | `speak(ptr,len)` |
| 8 | `net` | `net(op)->i32` |
| 9 | `random` | `random()->i32` |

Check, on every call: `host == UNKNOWN || (caps & required) == 0` → the app traps with
`capability denied` and is stopped. Importing an unknown name is allowed; calling it traps.

**Grants in v0:** `nia::run_file` (every app started from the VFS or the store screen) passes `CAP_ALL`.
Narrower masks exist only in built-in demos. There is no consent prompt, no stored grant, no revocation.

The EL0/ring-3 interpreter `userland/niavm` supports only `print`, `putc`, `time`, `ai`, `speak` and
forwards them to syscalls without a permission check (AFR-0001 §8.2).

### 4.4 Storage

Apps are files in the AfriSovera VFS (`AFRIVFS1`): at most 24 files, 24-byte names, **512 bytes of data
per file** (longer writes are truncated silently — see docs/review). The store screen lists files ending
`.wasm` (up to 12).

### 4.5 Integrity and authorship (v0)

- At boot, after the VFS is loaded, the SHA-256 of nine named bundled apps is recorded in RAM. At launch:
  hash differs → `Tampered`, refused; equal → runs; not recorded → `Unverified`, still runs.
  **Limitation:** the reference hash is taken from the disk contents at boot, so a file altered on disk
  while the system is off becomes the new reference. This is a corruption check within one boot, not a
  tamper check.
- Ed25519 (`ed25519.rs`, TweetNaCl-derived, verify only) checks one signature: message = the 32-byte
  SHA-256 digest of the module, key = the demo `PUBLISHER_PUBKEY`. The verifier does not reject `S ≥ L`
  (non-canonical S); v1 MUST (RFC 8032 §5.1.7).

## 5. v1 — the `.nia` package (Designed)

### 5.1 Layout

All integers little-endian. Offsets are from the start of the file.

```
Header (16 bytes)
  0   4  magic           "NIA\0"  = 4E 49 41 00
  4   2  format_version  = 1
  6   2  flags           bit 0: signed (MUST be 1 for store and AfriShare distribution); others 0
  8   4  section_count   n, 2 ≤ n ≤ 16
 12   4  reserved        MUST be 0
Section table (n × 24 bytes, at offset 16)
  +0  4  type
  +4  4  flags           reserved, MUST be 0
  +8  8  offset
 +16  8  length
Section bodies (in table order, contiguous, no gaps, no overlap)
```

| Type | Section | Rules |
|---|---|---|
| 1 | manifest | REQUIRED, exactly one, first; UTF-8 (§5.2) |
| 2 | module | REQUIRED, exactly one; a WASM 1.0 module |
| 3 | resources | OPTIONAL; a flat archive defined later |
| 4 | icon | OPTIONAL; PNG, ≤ 64 KiB, square |
| 15 | signature | REQUIRED when flags bit 0 is set; MUST be the last section and end at end of file |

A reader MUST reject: wrong magic; `format_version` it does not know; a section past end of file; gaps
or overlaps; a duplicate or missing required section; unknown types below 0x8000 (types ≥ 0x8000 are
vendor sections and MAY be ignored); total size above 16 MiB.

### 5.2 Manifest

A UTF-8 text of `key = value` lines (a strict TOML subset: strings in double quotes, arrays of strings in
brackets, no tables, no multi-line strings). Unknown keys MUST be preserved and ignored.

| Key | Required | Meaning |
|---|---|---|
| `id` | yes | reverse-DNS app id, `[a-z0-9.-]`, ≤ 128 bytes, e.g. `org.example.hello` |
| `name` | yes | display name, ≤ 64 bytes |
| `version` | yes | semantic version `MAJOR.MINOR.PATCH` |
| `abi` | yes | `"nia/1"` |
| `entry` | no | exported function to call, default `"run"` |
| `permissions` | yes | array of permission names (§6), may be empty |
| `min_os` | no | lowest AfriSovera version |
| `description`, `lang` | no | text; ISO 639 code list |

### 5.3 Signature section (108 bytes)

```
 0  2  algorithm   1 = Ed25519 (the only value in v1)
 2  2  reserved    0
 4  8  key_id      SHA-256(public_key)[0..8]
12 32  public_key
44 64  signature   Ed25519(sk, "AFRISOVERA-NIA-SIG-V1\0" || D)
```

where **D = SHA-256(file bytes [0, offset of the signature section))** — header, the whole section table
(including the signature's own table entry) and every other section. The 22-byte context string (21 ASCII
characters and a NUL) separates Nia signatures from any other use of the same key.

Verification MUST: recompute D; verify with a strict RFC 8032 verifier (reject non-canonical `S`, reject
small-order `A`); and check `key_id` matches `public_key`. The *trust* decision is separate (§8).

### 5.4 Install and launch state machine

```
 received ──parse──► Parsed ──verify sig──► Verified ──show name, publisher, permissions──► Consent
   │ bad format         │ bad signature                    │ user declines
   ▼                    ▼                                  ▼
 Rejected            Rejected (never runs)              Cancelled
 Consent ──user accepts──► Installed { id, version, key_id, granted ⊆ requested }
 Installed ──launch──► Running (holds exactly `granted`) ──exit/trap──► Installed
 Installed ──update with same id──► (same key_id: Consent only for NEW permissions; other key_id: refused
                                     unless the user removes the old app first)
 Installed ──user revokes a permission──► Installed (granted shrinks; takes effect at next host call)
```

Unsigned packages (flags bit 0 = 0) MAY be installed by explicit user choice (sideloading is a right,
docs/08); they are shown as "unsigned — no publisher" and can never be updated silently.

## 6. Permissions (v1)

| Name | v0 bit | Grant policy | v1 backing capability |
|---|---|---|---|
| `print` | 0 | automatic | debug channel |
| `putc` | 1 | automatic | console channel |
| `draw` | 2 | automatic (own windows only) | Tonga surface channel |
| `time` | 4 | automatic | — (clock is not a secret) |
| `random` | 9 | automatic | — |
| `input` | 5 | automatic, only while focused | Tonga input events |
| `file` | 3 | automatic for the app's own private folder; **ask** for shared folders | VFS directory handle |
| `net` | 8 | **ask** | netstack socket-factory channel |
| `ai` | 6 | **ask** (on-device; data stays local) | Asili AI channel |
| `voice` | 7 | automatic for output | Asili voice channel |
| `pay` | — | **per transaction**, through the system payment sheet | AFR-0006 |
| `identity` | — | **ask**, per relying party | AFR-0007 |

Rules:
- An app MUST NOT receive a permission it did not request.
- A permission removed by the user MUST make the next call to its host functions fail with the
  `capability denied` trap (v0 behaviour) — in v1 the backing channel is closed.
- The grant screen MUST name each permission in plain words in the user's language and MUST NOT offer an
  "allow all".

## 7. Versioning

- `format_version` covers the container; `abi` covers the host-function set. Both are independent.
- A v1 reader MUST refuse `format_version > 1`.
- `nia/1` host functions are frozen once 1.0 ships; additions go to `nia/1.x` and are only importable by
  apps whose `abi` names that minor version or later.
- Permission names are never reused for a different meaning.

## 8. Security considerations

- **Trust policy.** A valid signature proves that the holder of `key_id` produced these bytes. Whether to
  *trust* a publisher is policy: the device keeps a list of trusted publisher keys (system, the user's
  stores per AFR-0005, keys the user added). A valid signature from an unknown key is shown as
  "signed by an unknown publisher <key id>", never as "verified".
- **Update pinning** (§5.4) prevents a different key from replacing an installed app (first-use trust).
- **Key compromise** is handled by the store revocation list (AFR-0005 §6.4).
- **The interpreter is part of the TCB** while it runs in the kernel; malformed modules MUST be rejected
  by the validator before execution. The `wasm-fuzz` suite exercises this.
- **v0 issues** carried to docs/review: all-permission grant from `run_file`; boot-time reference hashes;
  silent truncation of files over 512 bytes; non-strict Ed25519; module name not checked.

## 9. Test vectors

### 9.1 Minimal module (33 bytes)

```
00 61 73 6D 01 00 00 00  01 04 01 60 00 00  03 02 01 00
07 07 01 03 72 75 6E 00 00  0A 04 01 02 00 0B
SHA-256 = f7fb20ef0be78ab4dec443a627d3f9aa4e184bb52456b0acbb01685b97409636
```

It exports `run` with an empty body: v0 MUST load and run it with mask 0 and print nothing.

### 9.2 v0 permission check

| Module imports | Calls | Mask | Expected |
|---|---|---|---|
| `putc` | `putc(65)` | `CAP_PUTC` | prints `A` |
| `putc` | `putc(65)` | 0 | trap `capability denied` |
| `nosuch` | never | any | loads and runs |
| `nosuch` | `nosuch()` | `CAP_ALL` | trap `capability denied` |

### 9.3 v0 signature (RFC 8032 test 1 key)

```
secret key  9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60
public key  d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a
message     SHA-256(module 9.1) = f7fb20ef…09636 (32 bytes)
signature   9fd7bc3d055dd956933f3809e6475d1275e5a2017f9b1fa9a2a60cccad15bd53
            02ceff68a9c37e0dc541b5b9061dd5684eb6dba21e78a46f9140445dbdb06902
```

### 9.4 v1 package `hello.nia` (339 bytes, in `docs/spec/vectors/`)

Manifest (110 bytes):

```
id = "org.example.hello"
name = "Hello"
version = "1.0.0"
abi = "nia/1"
entry = "run"
permissions = ["print"]
```

Header + section table (88 bytes):

```
4E 49 41 00 01 00 01 00 03 00 00 00 00 00 00 00      magic, v1, signed, 3 sections
01 00 00 00 00 00 00 00 58 00 00 00 00 00 00 00 6E 00 00 00 00 00 00 00   manifest @0x58 len 110
02 00 00 00 00 00 00 00 C6 00 00 00 00 00 00 00 21 00 00 00 00 00 00 00   module   @0xC6 len 33
0F 00 00 00 00 00 00 00 E7 00 00 00 00 00 00 00 6C 00 00 00 00 00 00 00   signature @0xE7 len 108
```

```
D          = cef3ce3a2a84c023b8f1b0ea40f4d2836ba95b05cd40aea3ab13df50ca076a94
key_id     = 21fe31dfa154a261
signature  = 0918e1e1f8a24dcea72f596d15d923b5f9d6c6f637d6a8eaadd827a9b1143788
             c6e82743cd6118a263675efbc33cfd017487a1b307381f409fde352f812b3c0d
```

Negative vectors a reader MUST reject: flip any byte in [0, 0xE7) → signature fails; set
`section_count = 17` → format error; change the module length to 34 → section past signature / overlap.

## 10. References

- WebAssembly Core Specification 1.0; RFC 8032 (Ed25519); RFC 2119.
- docs/03, docs/08 (sideloading as a right), docs/10, docs/12; ADR 0003, ADR 0004.
