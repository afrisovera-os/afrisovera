# AFR-0007 — AfriID identity protocol

| | |
|---|---|
| **Status** | **Designed — not built.** No identity code exists |
| **Version** | 0.1 (2026-11-08) |
| **Editor** | Chief architect |
| **Vectors** | `docs/spec/vectors/afr0005_0007_vectors.py` |
| **Depends on** | AFR-0001 v1 (channels), AFR-0003 (`identity` permission), AFR-0004 (AfriIDL) |
| **Licence** | Apache-2.0 (this spec); identity-provider server licence open (ADR 0005 §3) |

RFC 2119 key words apply.

---

## 1. Motivation

Signing in today means a foreign platform account (Google, Apple, Facebook) that the user does not control
and that tracks them across apps. docs/02 specifies AfriID as standards-based (OIDC/OAuth2) and
**self-hostable** by a telco, a state or a university, with nothing forcing one mothership. The project's
assistant text also promises **self-owned, on-device keys**. Those two statements are reconciled here:

> **The device holds the keys; providers vouch for them.** Authentication to any service is by a key that
> never leaves the device (passkey model). Federation to apps and websites is plain OpenID Connect, through
> an identity provider the user chooses — or, offline, through the device itself (self-issued).

## 2. Scope and status summary

| Part | Status |
|---|---|
| On-device identity service and key store | Designed (§5) |
| OIDC federation with self-hostable providers | Designed (§6) — uses OIDC as published, adds only profile rules |
| Pairwise identifiers per app | Designed (§6.3) |
| Self-issued (offline) identity | Designed (§7) |
| Account recovery | Designed, outline only (§8) |
| Microg-compatible shim for Android apps (docs/06) | Not in scope (Daraja spec) |
| National ID / KYC integration | Not in scope; a provider's business, never the OS's |

## 3. Terminology

- **Identity provider (IdP)** — an OIDC OpenID Provider (a telco, state, university, community, company).
- **Relying party (RP)** — an app or website that wants to know who the user is.
- **Account** — the user's account at one IdP. A device may hold several.
- **Device key** — a P-256 or Ed25519 key generated on the device, per (account, IdP), non-exportable when a
  hardware keystore exists.

## 4. Principles (normative)

1. No IdP is built in or preferred; the user adds IdPs by URL or QR code. The OS ships with none enabled.
2. An app never sees the user's IdP password or device keys.
3. An app gets a **different subject identifier** from every other app (pairwise), so apps cannot join their
   records without the user.
4. Every release of identity data to an app passes a system consent sheet listing exactly the claims.
5. The device works with no IdP at all (self-issued identity, §7).

## 5. On-device service (AfriIDL, ordinal 0x0301)

```
protocol Identity @0x0301 {
    sign_in @1 (client_id: string:128, scopes: vector<string:32>:16, nonce: string:64) -> SignInResult;
    sign_out @2 (client_id: string:128) -> i32;
}
struct SignInResult { status: i32; id_token: string:4096?; }   // status 0 ok, -1 cancelled, -2 no account, -3 denied
```

Reached only with the `identity` permission (AFR-0003 §6). `client_id` MUST equal the app's package `id`
for Nia apps (the service fills it in; an app cannot claim another app's id). Websites in the web runtime
use the standard browser OIDC flow instead, with the same consent sheet.

## 6. Federation with an IdP (OIDC profile)

### 6.1 Adding an IdP

The device fetches `https://<issuer>/.well-known/openid-configuration` (OIDC Discovery 1.0), requires
`authorization_code` + PKCE `S256`, and registers itself as a public native client (dynamic registration,
RFC 7591) **once per device**, not per app.

### 6.2 Device enrolment

The user signs in to the IdP once in the system sheet (password, SMS one-time code, or the IdP's own method).
The device then creates a device key and registers it with the IdP as a WebAuthn/passkey credential, so later
sign-ins need no password on this device. IdPs that do not support passkeys keep working with their own
method; the device stores only the refresh token, in the platform keystore.

### 6.3 Sign-in for an app

1. App calls `sign_in(client_id = its package id, scopes, nonce)`.
2. Consent sheet: app name and publisher, IdP chosen, claims requested (`openid` always; `profile`, `phone`,
   `email` only if requested and ticked).
3. The identity service obtains an ID token from the IdP for audience **`afri:app:<package id>`** using the
   device's refresh token / passkey (token exchange, RFC 8693), with the app's `nonce`.
4. The token's `sub` is **pairwise** (OIDC Core §8.1):
   `sub = base64url(SHA-256(sector_id ‖ 0x00 ‖ local_account_id ‖ 0x00 ‖ salt))`, `sector_id` = the RP's
   package id or web origin host, `salt` = 16 random bytes kept by the IdP.
5. The app receives the ID token (JWS, `EdDSA` or `ES256`) and verifies it against the IdP's JWKS like any
   OIDC RP, or sends it to its own server to do so.

Apps that want an account on their own backend verify the token there; the OS adds nothing proprietary.

## 7. Self-issued identity (offline, no IdP)

For apps that only need "the same user as last time" (games, notes, local co-ops), the identity service acts
as its own issuer, following OpenID Connect Self-Issued OP v2: per-RP key pair generated on the device;
`sub` = the JWK thumbprint (RFC 7638) of that key; token signed by that key. Nothing leaves the device, and
the RP learns nothing that links the user to any other RP. Lost device = lost self-issued identity unless the
user backs up the key store (§8).

## 8. Recovery (outline)

- IdP accounts: the IdP's own recovery (it is the IdP's job); after recovery, the old device key is revoked at
  the IdP and a new one enrolled.
- Self-issued keys: an encrypted key-store backup (user passphrase, Argon2id, XChaCha20-Poly1305) the user
  may put on a USB stick, SD card or another of their devices via AfriShare. Never to an AfriSovera server —
  there is none.
- Social recovery (k-of-n trusted contacts) is a later spec.

## 9. Security considerations

- **Phishing-resistance** comes from passkeys bound to the IdP's origin; password entry happens only in the
  system sheet, never in an app.
- **Tracking**: pairwise `sub`, no device identifier in tokens, no OS telemetry on sign-ins. An IdP can see
  which RPs a user signs in to (inherent in OIDC); self-issued mode avoids that.
- **Coercion / state IdPs**: since any IdP can be used and none is built in, the user can choose a provider
  they trust; the OS must never make a state IdP mandatory (docs/policy/GOVERNANCE.md principle 4).
- **Stolen device**: device keys require the device unlock; the IdP can revoke the device key remotely.
- **Token leakage**: ID tokens are audience-bound to one app; access tokens for IdP APIs never reach apps.

## 10. Versioning

Protocol ordinal 0x0301 under AfriIDL rules. The OIDC profile follows the published OIDC specs; a change of
profile (e.g. requiring DPoP) is a new spec version with a capability flag in the IdP's discovery document
(`afri_profile_versions_supported`).

## 11. Test vectors

| Input | Output |
|---|---|
| PKCE verifier `dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk` (RFC 7636 App. B) | S256 challenge `E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM` |
| pairwise `sub`: sector `app.example.com`, account `user-0001`, salt 16 × `00` | `bUGgxqSTB-59PmKolHXjncRH2bXhVoHZx3kayqip5UU` |

Reject vectors: an ID token whose `aud` is another app's `afri:app:` id; a `nonce` mismatch; an IdP whose
discovery lacks `S256` in `code_challenge_methods_supported`.

Conformance tests to write: a test IdP (an open-source OIDC server run in the cloud suite), two apps receiving
different `sub` for one account, and a consent-cancel path.

## 12. References

- docs/02 (AfriID), docs/05 Phase 4–5, docs/06; AFR-0003; AFR-0004.
- OpenID Connect Core 1.0 (§8.1 pairwise identifiers), OIDC Discovery 1.0, Self-Issued OP v2;
  RFC 6749, RFC 7591, RFC 7636 (PKCE), RFC 7638, RFC 8693; W3C WebAuthn Level 2.
