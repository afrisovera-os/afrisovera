# AFR-0005 — AfriStore federation protocol

| | |
|---|---|
| **Status** | **Designed — not built.** v0 code is a local store *screen* only |
| **Version** | 0.1 (2026-11-08; v0 description at commit `485b863`) |
| **Editor** | Chief architect |
| **Code (v0)** | `kernel/src/store.rs` |
| **Vectors** | `docs/spec/vectors/afr0005_0007_vectors.py` |
| **Depends on** | AFR-0003 (`.nia` packages, publisher keys), AFR-0006 (paid apps) |
| **Licence** | Apache-2.0 (this spec); the reference server's licence is open (ADR 0005 §3) |

RFC 2119 key words apply.

---

## 1. Motivation

docs/08 makes distribution a right: sideloading is permanent, anyone may run a store, AfriStore is *a*
store and never *the* store, and signing proves authorship without being a permission to publish. That only
holds if stores share one open protocol, so that a device can follow several stores, a country or a school
can run its own, a mirror can carry any store's apps through a slow or censored link, and apps can travel
offline over AfriShare and still be verified. This spec defines that protocol.

## 2. Scope and status summary

| Part | Status |
|---|---|
| Store screen listing local `.wasm` files, launch on 1–9, trust badge per file | **v0 Implemented** (`store.rs`); search box, categories, "GET" and ratings are decoration with no logic |
| Store descriptor, keys, catalogs, mirrors, revocation, offline bundles | **Designed** (this spec) |
| Paid apps (offers, receipts) | **Designed** here, payment itself in AFR-0006 |
| Reviews, ratings, reputation | **Not in scope** (a later spec) |
| Moderation policy of any particular store | **Not in scope** (each store's own) |

## 3. Terminology

- **Store** — an operator that publishes signed catalogs. Identified by a DNS name (`store.example.org`).
- **Root key** — the store's long-lived Ed25519 key, kept offline; signs only descriptors.
- **Catalog key** — an online Ed25519 key that signs catalogs; listed in the descriptor.
- **Catalog** — a signed list of app entries with a sequence number and an expiry.
- **Mirror** — any server (or USB stick, or phone over AfriShare) that serves a store's files verbatim. A
  mirror is never trusted; everything it serves is verified.
- **Following** — a device's configured set of stores, each pinned to its root key.

## 4. Objects and encoding

All documents are UTF-8 JSON in **canonical form**: RFC 8785 (JCS) — keys sorted, no insignificant
whitespace, integers only (no floats), strings NFC-normalised. Every signed document travels with a
detached signature file, `<name>.sig`, which is the 108-byte signature block of AFR-0003 §5.3 with a
per-document context string, computed over `SHA-256(canonical bytes)`:

| Document | Context string |
|---|---|
| Store descriptor | `AFRISOVERA-STORE-DESCRIPTOR-V1\0` |
| Catalog | `AFRISOVERA-STORE-CATALOG-V1\0` |
| Revocation list | `AFRISOVERA-STORE-REVOKED-V1\0` |

A client MUST re-canonicalise before verifying and MUST reject a document whose received bytes are not
already canonical (no second interpretation of the same signature).

### 4.1 Store descriptor (`/.well-known/afristore.json`)

```json
{"type":"afristore.descriptor","v":1,"store":"store.example.org","name":"Example Store",
 "version":3,"root":"<hex Ed25519 public key>",
 "catalog_keys":[{"key":"<hex>","not_after":"2027-11-08T00:00:00Z"}],
 "mirrors":["https://store.example.org/","https://mirror.example.net/s/"],
 "follows":[{"store":"apps.uni.example","root":"<hex>"}],
 "policy":"https://store.example.org/policy"}
```

Signed by `root`. A descriptor with a new `root` MUST be signed by **both** the old and the new root (two
`.sig` files: `afristore.json.sig` and `afristore.json.sig2`). `version` MUST increase with each change.

### 4.2 Catalog (`/catalog/<seq>.json`, latest pointer `/catalog/latest` = decimal seq)

```json
{"type":"afristore.catalog","v":1,"store":"store.example.org","seq":7,
 "issued":"2026-11-08T00:00:00Z","expires":"2026-11-15T00:00:00Z",
 "apps":[{"id":"org.example.hello","version":"1.0.0","publisher":"21fe31dfa154a261",
          "sha256":"<hex of the .nia file>","size":339,"permissions":["print"],
          "urls":["pkg/<sha256>.nia"],
          "offer":{"amount":0,"currency":"KES"},
          "via":"apps.uni.example"}]}
```

- `seq` strictly increases per store. `expires` ≤ 31 days after `issued`.
- `publisher` is the AFR-0003 key id; `permissions` MUST equal the package manifest's.
- `urls` are relative to any mirror, or absolute. Package files are **content-addressed** by SHA-256 and
  immutable.
- `offer` absent or amount 0 = free; otherwise AFR-0006.
- `via` names a followed store the entry was taken from (federation, §5.3); absent = this store's own.
- A catalog lists the **latest** version of each app; older versions MAY stay downloadable by hash.

### 4.3 Revocation list (`/revoked.json`)

```json
{"type":"afristore.revoked","v":1,"store":"store.example.org","seq":12,"issued":"…",
 "publishers":[{"key":"<key id>","reason":"compromised","after":"2026-10-01T00:00:00Z"}],
 "packages":[{"sha256":"<hex>","reason":"malware"}]}
```

Signed by a catalog key. Revocation by one store applies to installs **from that store**; the device shows
it for any installed app with that hash or key and asks the user (it does not delete apps on its own).

## 5. Protocol

### 5.1 Transport

HTTPS (TLS 1.2+) for online use; plain HTTP, a USB stick, an SD card or AfriShare are equally valid
**transports**, because trust comes from signatures, not from the channel. Paths are relative to a mirror
base URL. Responses are static files (a store can be a static web host).

### 5.2 Client update state machine (per followed store)

```
 Idle ──timer (≤ 24 h) or user──► FetchLatest ──► FetchCatalog(seq)
   ▲                                 │ network error: try next mirror; all fail → Stale
   │                                 ▼
   │                         Verify: signature by a catalog key listed in the pinned descriptor
   │                                 store name matches · seq > last_seen_seq · now < expires
   │                                 │ fail → Rejected (keep previous catalog, log reason)
   │                                 ▼
   └─────────── Accepted: last_seen_seq := seq, catalog cached
 Stale: catalog past `expires` → shown "out of date since <date>", installs still possible with a warning
```

Descriptor refresh: weekly or on a catalog signed by an unknown key; a descriptor is accepted only if signed
by the pinned root (or by old and new root on rotation) and `version` > the pinned version.

### 5.3 Federation

A store MAY include entries from stores it `follows`: it copies the entry, adds `via`, and re-signs its own
catalog. A client that follows both stores sees the entry twice and MUST de-duplicate by (`id`,
`publisher`, `sha256`). An entry with the same `id` but a different `publisher` is a **different app** and
is shown with the publisher key id; a device never lets one publisher's update replace another's app
(AFR-0003 §5.4).

### 5.4 Install

1. Pick an entry from a verified catalog.
2. Download `sha256` from any mirror (or take it from local storage/AfriShare); check size, then SHA-256.
3. Verify the package itself (AFR-0003 §5.3) and check `publisher` and `permissions` match the entry.
4. Continue with the AFR-0003 install state machine (consent screen).

### 5.5 Offline bundle (AfriShare, USB)

A bundle is a directory or zip: `afristore.json` + `.sig`, `catalog/<seq>.json` + `.sig`, `revoked.json` +
`.sig`, and `pkg/<sha256>.nia`. A receiving device verifies it exactly as online. A bundle from a store the
device does not follow can be installed only as a sideload (AFR-0003 §5.4, unsigned-or-unknown path), with the
store's name and root key fingerprint shown.

## 6. Security considerations

Threat model after The Update Framework (TUF), simplified:

| Threat | Defence |
|---|---|
| Malicious mirror serves a modified package | content hash in a signed catalog; package signature |
| Rollback to an older catalog with a vulnerable app | `seq` must increase; `last_seen_seq` persisted |
| Freeze (mirror keeps serving an old, valid catalog) | `expires` ≤ 31 days; Stale warning |
| Mix-and-match across stores | catalog names its `store`; entries de-duplicated by publisher |
| Catalog key compromise | short-lived catalog keys (`not_after`); root re-signs descriptor without it |
| Root key compromise | not recoverable inside the protocol: the user must re-pin (shown as a new store) |
| Publisher key compromise | revocation list; install pinning per app |
| Store censors an app | other stores, mirrors and sideloading (docs/08); not a protocol failure |
| Tracking of users by the store | catalogs are public static files; no device identifier is sent; downloads by hash |

- Clients MUST NOT send device identifiers, accounts or install lists when fetching catalogs or packages.
- A store's moderation decision (delisting) is not a revocation and MUST NOT remove an installed app.
- Signature verification MUST be strict RFC 8032 (AFR-0003 §5.3).

## 7. Versioning

Every document carries `"v":1`. A client MUST ignore unknown fields and MUST reject an unknown `v`. Context
strings carry the version (`…-V1`), so a v2 signature can never verify as v1.

## 8. Test vectors

Catalog for the AFR-0003 `hello.nia` (SHA-256 `ca84e810…07edf`), canonical, 415 bytes:

```
{"apps":[{"id":"org.example.hello","permissions":["print"],"publisher":"21fe31dfa154a261","sha256":"ca84e810f56c7ea77d501220783aed924d0cda74f1a78427165dad4c1db07edf","size":339,"urls":["pkg/ca84e810f56c7ea77d501220783aed924d0cda74f1a78427165dad4c1db07edf.nia"],"version":"1.0.0"}],"expires":"2026-11-15T00:00:00Z","issued":"2026-11-08T00:00:00Z","seq":7,"store":"store.example.org","type":"afristore.catalog","v":1}
```

```
SHA-256    4a5fa89d3fd10016594d12c2598ca032574f571e1c06ac87143e8371a9ecc569
signature  df77911f18002270c40cc0f3cd7a442ffd5eb5a65df9cfab7960ef1f501a630d
           b92690b81a347a5e628a433dbc65bb588b4bd852fffe4fbab3ccbd4d2f13dc06
           (RFC 8032 test key 1, context AFRISOVERA-STORE-CATALOG-V1\0)
```

Reject vectors: the same bytes with a space after the first `{` (not canonical); `seq` 6 after 7 was seen
(rollback); `expires` in the past (Stale, not Rejected); the signature checked with context `…-V2\0`.

Conformance tests to write with the implementation: a static-file test store in the cloud suite, a mirror
that flips one byte of a package, and a replayed old catalog.

## 9. References

- docs/02 (AfriStore), docs/08 §app store, docs/12; AFR-0003; AFR-0006.
- The Update Framework specification (roles, rollback and freeze attacks); RFC 8785 (JSON Canonicalization
  Scheme); RFC 8032.
- F-Droid repository index (precedent for signed static catalogs and mirrors).
