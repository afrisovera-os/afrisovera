# AFR-0009 — Sovereign Mesh wire format (link, bundle, tags, pairing)

| | |
|---|---|
| **Status** | **Draft — not built.** No mesh, radio-host or sealing code exists |
| **Version** | 0.1 (2026-11-09) |
| **Editor** | Chief architect, with the mesh protocol developer |
| **Vectors** | `docs/spec/vectors/afr0009_vectors.py` (public RFC 8032 / RFC 7748 test keys only) |
| **Depends on** | AFR-0001 v1 (channels), AFR-0002 v1 (class protocol shape), AFR-0003 (`mesh` permission), AFR-0004 (AfriIDL) |
| **Design** | `docs/design/mesh-engineering-plan.md` (plan of record, branch `claude/mesh-protocol-dcb5a4` at `8a7181f8`); docs/17 |
| **Licence** | Apache-2.0 |

RFC 2119 key words apply. Integers are little-endian unless marked BE (big-endian, used only inside
cryptographic derivations, as their source RFCs do).

---

## 1. Motivation

docs/17 promises a device-to-device network in which **every message stays sealed from everyone but its
recipient** and no company can track or bill it. The engineering plan splits this into radios (Layer A),
security and identity (Layer B), and gossip relaying with store-and-forward (Layer C). This spec fixes the
**bytes** of Layers B and C, so that:
- two implementations interoperate;
- Phase 0 (two QEMU machines over TCP) and Phase 1 (BLE) carry the same frames;
- Phase 2 can add multi-hop relaying without a format change.

## 2. Scope and status summary

| Part | Status |
|---|---|
| Identity keys and identity record (§5.2) | Designed |
| Pairing: Noise XX + short authentication string or QR (§4.3) | Designed |
| Link layer: Noise framing and records (§4) | Designed |
| Bundle: end-to-end sealed, padded, tagged (§6) | Designed |
| Recipient tags, beacon tags (§5.4, §5.5) | Designed |
| Relay rules: dedup, TTL, expiry, budgets (§7) | Designed (Phase 2) |
| Transport bindings: TCP (Phase 0), BLE L2CAP (Phase 1) | Designed (§8); WiFi and bridge bindings later |
| Introductions, ratchet, proof-of-work | **Not in this version** (§11) |

Prerequisites that do not exist yet:
- a hardened DRBG (review of the plan, M4);
- strict Ed25519 (`S < L`, review 1, R1-6) plus signing;
- X25519, ChaCha20-Poly1305 and HKDF;
- the on-device keystore (review 2, R2-6).

Whether those primitives come from audited `no_std` crates or are hand-built is an open owner decision
(plan §10.3). This spec works the same either way.

## 3. Terminology and cryptographic suite

- **Node**: a device running the mesh service. **Identity**: a node's long-term key pair (§5.1).
- **Contact**: a node whose identity record this node has accepted, by pairing (§4.3) or later by introduction.
- **Neighbour**: a node at the other end of a currently established link (§4).
- **Link**: a Noise session over one transport between two neighbours, hop by hop (layer B1).
- **Bundle**: one end-to-end sealed message from a sender to a recipient (layer B2). Relays carry it and
  cannot open it.
- **Period**: `floor(unix_seconds / 3600)` for recipient tags, `floor(unix_seconds / 900)` for beacon tags.

**Suite v1**:
- Noise `25519_ChaChaPoly_SHA256`;
- HPKE (RFC 9180) with `DHKEM(X25519, HKDF-SHA256)` `0x0020`, `HKDF-SHA256` `0x0001` and `ChaCha20Poly1305`
  `0x0003`, in **mode_auth (`0x02`)**;
- Ed25519 strict (RFC 8032 with `S < L` and small-order public keys rejected);
- HMAC-SHA256 and HKDF-SHA256 (RFC 5869).

Context strings, all ASCII with a trailing NUL, and all registered in `docs/spec/README.md`:

| String | Used in |
|---|---|
| `AFRISOVERA-MESH-LINK-V1\0` | Noise prologue (§4.1) |
| `AFRISOVERA-MESH-SAS-V1\0` | short authentication string (§4.3) |
| `AFRISOVERA-MESH-IDENTITY-V1\0` | identity record signature (§5.2) |
| `AFRISOVERA-MESH-CONTACT-V1\0` | contact root key (§5.3) |
| `AFRISOVERA-MESH-TAG-V1\0` | recipient tag (§5.4) |
| `AFRISOVERA-MESH-BEACON-V1\0` | beacon tag (§5.5) |
| `AFRISOVERA-MESH-BUNDLE-V1\0` | HPKE `info` for bundles (§6) |

## 4. Link layer (B1)

### 4.1 Handshake

| Situation | Pattern | Who initiates |
|---|---|---|
| Two nodes meeting to pair (pairing flag set in a beacon, §5.5, or a QR code scanned) | `Noise_XX_25519_ChaChaPoly_SHA256` | the scanner / connector |
| Two contacts (beacon tag recognised, §5.5) | `Noise_KK_25519_ChaChaPoly_SHA256` | the scanner / connector |

- **Prologue:** `AFRISOVERA-MESH-LINK-V1\0` followed by one transport byte (`0x01` BLE L2CAP, `0x02` WiFi
  direct, `0x03` TCP (Phase 0), `0x04` internet bridge).
- **Static key:** the node's X25519 identity key (§5.1).

In KK, a responder whose static key does not belong to the contact the initiator expected fails the handshake
(Noise's own behaviour). The initiator MUST then close and MUST NOT retry KK with that link for the rest of the
beacon period.

### 4.2 Framing and records

Every Noise message on a stream transport is preceded by a **u16 length** (LE) and is at most 65 535 bytes
(the Noise limit). After the handshake, every transport message decrypts to one **record**:

```
 0  1  type
 1  …  body (type-specific)
```

| Type | Name | Body | Allowed in |
|---|---|---|---|
| `0x01` | BUNDLE | one bundle (§6), 256 / 1 024 / 4 096 / 16 384 bytes | KK links; XX links after PAIR_CONFIRM by both sides |
| `0x02` | OFFER | `n` u8 (1…64), then `n` × 16-byte bundle ids | KK |
| `0x03` | WANT | same as OFFER: the ids the sender asks to receive | KK |
| `0x04` | IDENTITY | identity record (128 bytes, §5.2) followed by beacon key (32 bytes) | XX only, once each way |
| `0x05` | PAIR_CONFIRM | u8: `1` = the user accepted, `0` = rejected | XX only, once each way |
| `0x06` | CLOSE | u8 reason (§9) | any |
| `0x07` | PING | empty | any |

- A record with an unknown type MUST be ignored. One that is malformed or not allowed in this link's state
  MUST cause CLOSE `0x02`.
- Each side calls Noise `Rekey()` on its sending key after 10 000 messages or 1 hour, whichever comes first.
  The receiver rekeys in step at the same count. Links are short-lived in practice; this bounds key reuse.

### 4.3 Pairing (in person)

1. Both users put their devices in pairing mode (beacon pairing flag set, §5.5), or one shows a QR code.
2. The connector runs XX. Both sides now know each other's static X25519 key and the handshake hash `h`.
3. Each side sends IDENTITY. The receiver MUST check three things:
   - the record's signature (§5.2);
   - that the record's X25519 key equals the Noise remote static key;
   - with a QR code, that both match the scanned commitment.
   If any check fails, it sends CLOSE `0x03`.
4. Each device shows the **short authentication string**:
   `SAS = u32be(SHA-256("AFRISOVERA-MESH-SAS-V1\0" ‖ h)[0..4]) mod 1 000 000`, as six digits with leading
   zeros. The two users compare the digits aloud. A man in the middle has two separate handshakes with two
   different `h`, so the digits differ unless they are guessed (a 10⁻⁶ chance per attempt). Implementations
   MUST NOT let an attempt be repeated silently: show each failed attempt to the user.
5. Each user taps accept or reject. The device sends PAIR_CONFIRM. Pairing completes only when both sides have
   sent and received `1`. Each side then stores the contact (identity record, beacon key, contact root, §5.3)
   in the keystore.

**QR form** (a camera replaces step 4): `afrimesh:1:` followed by base64url, no padding, of
`ed25519_pub(32) ‖ x25519_pub(32) ‖ SHA-256(beacon_key)[0..8]`. Step 3 checks both keys against it. The SAS may
then be skipped, because the QR code itself is the out-of-band channel.

**State machine (per link)**:
```
 Idle ─connect─► Handshaking ─XX ok─► Identifying ─IDENTITY ok both ways─► Confirming ─PAIR_CONFIRM 1/1─► Paired ─► Established
                     │ KK ok ─────────────────────────────────────────────────────────────────────────────────────► Established
                     │ fail → Closed          │ bad record → CLOSE 0x03          │ reject or 5 min timeout → CLOSE 0x04
 Established ─OFFER/WANT/BUNDLE…─► Established ─CLOSE, transport loss, or idle 120 s─► Closed
```

## 5. Identity, contacts and tags

### 5.1 Identity

Each identity has two key pairs: an Ed25519 key that signs, and an X25519 key that agrees keys and is the
Noise static key. Both are generated from the hardened DRBG and stored in the keystore.

The mesh identity MUST NOT be, or be derived from, an AfriID device key (AFR-0007). A user MAY hold several
mesh identities. They share nothing, so they cannot be linked to each other.

Each identity also has a random 32-byte **beacon key** (§5.5). It is given only to contacts, and it MUST be
regenerated if the user removes a contact who should no longer be able to recognise them.

### 5.2 Identity record (128 bytes)

```
  0 32  ed25519_pub
 32 32  x25519_pub
 64 64  signature   Ed25519(ed25519_sk, "AFRISOVERA-MESH-IDENTITY-V1\0" ‖ SHA-256(bytes 0..64))
```

### 5.3 Contact root

Both sides compute the same 32-byte secret, whether the contact came from pairing or from an introduction:

```
 lo, hi = the two x25519_pub values in byte order
 root   = HKDF-SHA256(salt = lo ‖ hi, ikm = X25519(own_sk, other_pub), info = "AFRISOVERA-MESH-CONTACT-V1\0", L = 32)
```

### 5.4 Recipient tag (16 bytes)

```
 dir = 0x00 if sender x25519_pub < recipient x25519_pub (bytewise) else 0x01
 tag = HMAC-SHA256(root, "AFRISOVERA-MESH-TAG-V1\0" ‖ dir ‖ u64be(period of the bundle's creation))[0..16]
```

- **Relays** see a tag that changes every hour and differs in each direction, so they cannot read the sender
  or the recipient from it.
- **The recipient** keeps a table of the tags each contact could use for every period in the last 72 hours,
  the maximum bundle lifetime (§7). That is 72 × 16 bytes per contact, about 115 KiB for 100 contacts. It looks
  each tag up to find the sender, so the sender's identity needs no space on the wire.

### 5.5 Beacon tag (8 bytes) and advertising

```
 beacon_tag = HMAC-SHA256(beacon_key, "AFRISOVERA-MESH-BEACON-V1\0" ‖ u64be(floor(t / 900)))[0..8]
```

- A node advertises its current beacon tag. Its contacts recognise it and start KK; strangers see 8 random
  bytes that change every 15 minutes.
- The BLE advertising address MUST be a new **non-resolvable private address** every period (HCI
  `LE Set Random Address`), and the WiFi MAC MUST be random per session. Otherwise the radio address links the
  rotating tags together.
- In pairing mode the beacon sets flag bit 0 and carries 8 fresh random bytes instead of the tag.

BLE advertising payload: service data for the mesh service UUID `a5f1c0de-0b5e-4a50-9af1-50fe2a00a1e5`:
`flags u8 ‖ tag (8)`, where flags bit 0 = pairing mode and the other bits are reserved and set to 0.

**Honest limit:** the service UUID itself says "an AfriSovera mesh node is here". Hiding that the mesh is in
use at all is not a goal of v1.

## 6. Bundle (B2)

A bundle is exactly one of the **size buckets** 256, 1 024, 4 096 or 16 384 bytes. Larger content is split by
the app layer into several bundles; that is not specified here.

```
  0  1  version      1
  1  1  flags        0 (reserved)
  2  1  ttl          hops remaining; the sender sets ≤ 16 (default 8); each relay decrements
  3  1  bucket       0..3 → 256, 1 024, 4 096, 16 384
  4 16  bundle_id    16 random bytes from the DRBG; the duplicate-suppression key
 20 16  tag          recipient tag (§5.4)
 36  8  expires      u64 unix seconds
 44 32  enc          HPKE encapsulated key (sender's ephemeral X25519 public key)
 76  …  ciphertext   ChaCha20-Poly1305 output: (bucket size − 92) plaintext bytes + 16-byte tag
```

**Sealing.** HPKE mode_auth, single shot (sequence 0):
- `pkR` = the recipient's X25519 key, `skS` = the sender's X25519 identity key;
- `info = "AFRISOVERA-MESH-BUNDLE-V1\0"`;
- `aad` = header bytes 0..44 with the `ttl` byte set to 0, so relays can decrement it without breaking the
  seal while every other header field stays bound.

**Plaintext** (`bucket − 92` bytes):
```
  0  1  type         1 text (UTF-8) · 2 content chunk · 3 delivery receipt · other values reserved
  1  1  reserved     0
  2  2  length       body length
  4  8  created      u64 unix seconds, as the sender's clock read it
 12  …  body         then zero padding to the end
```
The largest body per bucket is 152, 920, 3 992 or 16 280 bytes.

**Opening (recipient):**
1. Find the tag in the table (§5.4). If it is not there, the bundle is not for this node: keep relaying it
   (§7).
2. HPKE AuthDecap with the matched contact's X25519 key as `pkS`, then open. A failure is a forgery or
   corruption: drop the bundle silently, and still mark the id as seen.
3. Check that the period of `created` equals the tag's period, and that `expires − created` ≤ 72 h. Otherwise
   drop.
4. Check that the padding is all zero. Otherwise drop.

**Why mode_auth.** The recipient learns the sender from the tag, so it can demand that exactly that contact
sealed the bundle. That gives sender authentication with no signature inside the bundle, so it is deniable
toward third parties. The cost is stated in §10 (key-compromise impersonation).

## 7. Relay rules (Phase 2; Phase 1 sends one hop only)

1. Accept BUNDLE only on a KK link (an authenticated neighbour). The XX case after pairing is the one
   exception, so the first message can travel at pairing time.
2. Drop silently if any of these hold:
   - `version` ≠ 1;
   - `bucket` > 3 or the length does not match the bucket;
   - `expires` ≤ now, or `expires` > now + 72 h;
   - `ttl` = 0;
   - the `bundle_id` is in the seen-cache.
3. Put the id in the seen-cache until `expires`.
4. If the tag is ours, open it (§6). Otherwise decrement `ttl`, store the bundle within this neighbour's budget,
   and offer it (OFFER/WANT) to other neighbours until it expires.
5. **Budgets (normative minimum):**
   - a node MUST enforce a per-neighbour cap on stored bytes and on bundles accepted per hour, and MUST stop
     relaying when the user has not granted the relay capability or the battery policy says no;
   - recommended defaults: 4 MiB stored and 64 bundles per hour per neighbour, and 32 MiB in total;
   - a neighbour that exceeds its budget gets CLOSE `0x05`, and its later bundles are refused for that period.
6. A relay MUST NOT reorder or modify any byte except `ttl`. It MUST NOT log tags or ids beyond the
   seen-cache.

## 8. Transport bindings

| Binding | Carrier | Notes |
|---|---|---|
| `0x03` TCP (Phase 0) | TCP port **4401** on the existing netstack (`net tcp` paths) | used between two QEMU machines in the cloud suite. The plan said UDP; TCP is used because it is already green and has the same framing as the bridge. |
| `0x01` BLE (Phase 1) | L2CAP LE credit-based channel, SPSM `0x00A1`, MTU ≥ 512 | one Noise message per SDU (≤ 65 535). No SMP and no ATT/GATT pairing: security comes from §4 only. |
| `0x02` WiFi direct | open IBSS or soft-AP, TCP 4401 on top, random MAC | plan R2: no WPA2, because B1 already seals every frame. |
| `0x04` bridge | TCP over the internet | Phase 3. A later version will specify how bridges find each other. |

## 9. Errors (CLOSE reasons)

| Code | Meaning |
|---|---|
| `0x00` | normal close |
| `0x01` | Noise failure (decrypt or handshake) |
| `0x02` | malformed record, or a record not allowed in this state |
| `0x03` | bad identity record or commitment mismatch |
| `0x04` | pairing rejected or timed out |
| `0x05` | budget exceeded |
| `0x06` | version not supported |

Bundles never produce a CLOSE: bad bundles are dropped silently (§6, §7). This avoids giving an attacker an
oracle that tells which bundles a node can open.

## 10. Security considerations

- **Relays are hostile by assumption.** A relay sees a padded, fixed-size bucket, an hourly-rotating tag, a
  hop count and an expiry, and nothing else. Content and sender are sealed by B2, and the neighbour is
  authenticated by B1.
- **Linkability within an hour.** Bundles from A to B in the same period carry the same tag, so a relay can
  tell that they share a sender and recipient for that hour. This is the price of a recipient look-up table. A
  per-bundle tag would need trial decryption of every bundle against every contact.
- **Key-compromise impersonation (mode_auth).** Whoever steals B's X25519 key can forge bundles to B that
  appear to come from any of B's contacts. Theft of B's key already exposes all of B's traffic, so this is
  accepted. The keystore (R2-6) is the defence.
- **Forward secrecy.** B1 links have it (Noise ephemeral keys). B2 bundles have it only for the sender's
  ephemeral key: a later theft of B's static key opens stored bundles. A ratchet between frequent contacts is
  future work (§11).
- **SAS.** Six digits give 10⁻⁶ per attempt. Repeated failures MUST be visible to the user (§4.3).
- **Sybil and flooding.** Bundles enter only through authenticated neighbours under budgets (§7). An attacker
  who pairs with a victim sits inside that victim's trust and is limited by the victim's budget. A node with no
  paired neighbour in range cannot inject anything: this is deliberate, and documented in the plan (§5.4).
- **Traffic analysis** across many relays (timing, volume) is made harder, not defeated. The project makes no
  anonymity claim.
- **Randomness.** Every key, ephemeral key, `bundle_id` and pairing nonce depends on the hardened DRBG. This
  spec MUST NOT be implemented on top of the unhardened generator.
- **No telemetry.** Nothing in this protocol reaches any AfriSovera server. There is none.

## 11. Versioning and future work

- `version` byte in bundles, `…-V1` context strings, and the prologue string: a v2 bundle or link can never be
  accepted as v1.
- Unknown record types are ignored (§4.2). Unknown bundle versions are dropped, not relayed: a v1 relay cannot
  check a v2 bundle's budget rules.
- **Next, in later versions:**
  - an introduction record (inner type 4: a contact vouches for another's identity record; both must accept);
  - a ratchet for frequent contacts;
  - a content-chunk format for files;
  - optional proof-of-work for open injection;
  - bridge discovery;
  - the AfriIDL service `Mesh @0x0701` that apps reach with the `mesh` permission (bit 12).

## 12. Test vectors

`docs/spec/vectors/afr0009_vectors.py`.

**Keys:**
- Alice: Ed25519 RFC 8032 TEST 1 seed, X25519 RFC 7748 §6.1 Alice private.
- Bob: Ed25519 TEST 2 seed, X25519 RFC 7748 §6.1 Bob private.
- Time `T = 1794225600` (2026-11-09T12:00:00Z), recipient-tag period 498396.

**Self-checks in the script:**
- the RFC 7748 shared secret;
- the hand-written HPKE key schedule (base mode), cross-checked against the `cryptography` library's HPKE;
- a relayed bundle (with `ttl` changed) still opens;
- the wrong sender key is rejected.

| Item | Value |
|---|---|
| Identity record (Alice) | `d75a9801…f707511a` ‖ `8520f009…aa9b4e6a` ‖ sig `6ef2a9ea…3ee6f0b` (128 bytes; full hex in the script output) |
| Contact root (Alice, Bob) | `14af56d88b42b1fad7ad534fc8a312b8f8e4f2137694e6c3cfe85a3dc4844e00` |
| Tag A→B, period 498396 | `5a19f125776210e2961e5d3b64a82271` |
| Tag B→A, period 498396 | `75c08454c83e4397069d1aed9fa2375c` |
| Tag A→B, period 498397 | `4415e70ef6962494f882d0a68923a35a` |
| Beacon tag, key 32 × `42`, at T | `304197d51d6135f7` |
| SAS for `h = SHA-256("example Noise handshake hash")` | `546787` |
| Bundle A→B, bucket 0, ttl 8, id `00…0f`, expires T + 24 h, text `Habari, Bob!`, ephemeral private 32 × `11` | 256 bytes, SHA-256 `fbff0cc1a6fbe3838eda2a58b06adaa25a7b15f3df5a71cf84969783248d4bb4` |

**Reject vectors:**
- the bundle opened with Bob's own key as the sender key (`InvalidTag`);
- any header byte other than `ttl` changed;
- `expires` more than 72 h ahead;
- a non-zero padding byte;
- a bucket-1 length with bucket byte 0;
- an IDENTITY record whose X25519 key differs from the Noise remote static key.

**Conformance tests to write with the implementation (cloud suite, Phase 0):**
- two QEMU machines pair over TCP 4401 using a scripted SAS accept, exchange a bundle, and both print the
  vector values;
- a man-in-the-middle relay produces mismatched SAS digits;
- a third machine relays a bundle it cannot open, and its log shows no plaintext;
- a duplicate bundle is delivered once;
- an over-budget neighbour is closed with `0x05`.

## 13. References

- docs/17; `docs/design/mesh-engineering-plan.md`; review notes of 2026-11-09 (M1–M5, R1–R7); AFR-0003,
  AFR-0004, AFR-0007.
- The Noise Protocol Framework (rev. 34); RFC 9180 (HPKE); RFC 7748 (X25519); RFC 8032 (Ed25519); RFC 5869
  (HKDF); RFC 8439 (ChaCha20-Poly1305); Bluetooth Core 5.x Vol 3 Part A (LE credit-based channels), Vol 6
  Part B §1.3 (private addresses).
- Briar: BTP (rotating tags), BQP (QR key agreement), BHP. The tag idea is borrowed from them; multi-hop
  relaying through third parties is this project's own extension.
