# AFR-0006 — AfriPay payment-rail protocol

| | |
|---|---|
| **Status** | **Designed — not built.** No payment code exists (the only mention in code is a canned assistant answer in `nia.rs`) |
| **Version** | 0.1 (2026-11-08) |
| **Editor** | Chief architect |
| **Vectors** | `docs/spec/vectors/afr0005_0007_vectors.py` |
| **Depends on** | AFR-0001 v1 (channels), AFR-0003 (`pay` permission), AFR-0004 (AfriIDL) |
| **Licence** | Apache-2.0 (this spec). Server-side licence and every regulatory question: open, owner + counsel |

RFC 2119 key words apply.

> **Regulatory notice.** Moving money is licensed per country (e.g. Kenya's National Payment System Act,
> Nigeria's CBN licensing, Ghana's Payment Systems and Services Act). AfriSovera, the OS, **never holds,
> routes or settles funds**: it carries a payment request from an app to a licensed provider chosen by the
> user, and carries the provider's receipt back. Nothing in this spec may be built against a live provider,
> and no provider account may be opened, without the owner's yes and legal advice (docs/05 Phase 4).

---

## 1. Motivation

In the target markets most people pay with mobile money (M-Pesa, MTN MoMo, Airtel Money) and USSD, not
cards. Today each app integrates each provider itself, handles the user's phone number and sometimes the
PIN, and the user cannot tell a real payment prompt from a fake one. docs/02 specifies **OS-level mobile
money**: apps request a payment through a capability; a **trusted system payment sheet** shows it; the
user authorises with the provider; the app never touches credentials.

## 2. Scope and status summary

| Part | Status |
|---|---|
| App ↔ Payments service protocol (`afri.pay.Payments`) | Designed (§5) |
| Payment request and receipt formats | Designed (§4) |
| System payment sheet behaviour | Designed (§6) |
| Rail adapters (M-Pesa, MTN MoMo, Airtel Money, USSD, bank) | Designed as an interface only (§7); no adapter specified per provider |
| Merchant onboarding, settlement, fees, disputes | **Not in scope** — the provider's own processes |
| Store payouts (AFR-0005 `offer`) | uses this protocol with the store or publisher as payee |

## 3. Terminology

- **Payer** — the device user. **Payee / merchant** — who receives money, identified at a provider (till
  number, paybill + account, MoMo merchant id, bank account).
- **Provider** — a licensed payment service (rail). **Adapter** — AfriSovera code that speaks one provider's
  API or USSD menu.
- **Request** — what an app asks to be paid. **Receipt** — the provider's confirmation, as relayed by the adapter.
- **Minor units** — integer amount in the currency's smallest unit (ISO 4217 exponent); floats are forbidden.

## 4. Formats

Canonical JSON (RFC 8785) as in AFR-0005 §4, signatures as 108-byte AFR-0003 blocks.

### 4.1 Payment request

```json
{"type":"afripay.request","v":1,"amount":15000,"currency":"KES","payee":"till:123456",
 "reference":"ORDER-42","merchant":"21fe31dfa154a261","expires":"2026-11-08T12:10:00Z",
 "nonce":"q8Z3t6Yx"}
```

| Field | Rule |
|---|---|
| `amount` | integer minor units, 1 … 10^12 |
| `currency` | ISO 4217 alpha code |
| `payee` | `<scheme>:<id>`; schemes `till`, `paybill:<n>/<account>`, `momo`, `msisdn` (E.164), `iban`, `store:<store>/<publisher>` |
| `reference` | merchant's reference, ≤ 32 printable ASCII |
| `merchant` | key id of the merchant's signing key, OPTIONAL; when present the request MUST carry a signature (context `AFRISOVERA-PAY-REQUEST-V1\0`) |
| `expires` | ≤ 30 minutes after creation |
| `nonce` | ≥ 8 characters, unique per merchant per request; the idempotency key |

### 4.2 Receipt

```json
{"type":"afripay.receipt","v":1,"request":"<hex SHA-256 of the canonical request>",
 "status":"settled","provider":"mpesa-ke","provider_ref":"<provider transaction id>",
 "amount":15000,"currency":"KES","time":"2026-11-08T12:03:11Z"}
```

`status` ∈ `settled`, `failed`, `expired`, `cancelled`, `pending`. The receipt is produced by the adapter from
the provider's callback or query; its authenticity is the provider's (the adapter verifies the provider's
callback per the provider's rules). The OS does **not** sign receipts as if it were the provider.

## 5. On-device protocol (AfriIDL, protocol ordinal 0x0201)

```
protocol Payments @0x0201 {
    request @1 (req: string:2048, sig: vector<u8>:108?) -> PayResult;   // blocks until a final state or timeout
    status  @2 (request_hash: array<u8,32>) -> PayResult;
}
struct PayResult { status: PayStatus; receipt: string:1024?; }
enum PayStatus : i32 { SETTLED = 0; FAILED = -1; EXPIRED = -2; CANCELLED = -3; PENDING = 1; DENIED = -4; }
```

An app reaches `Payments` only with the `pay` permission (AFR-0003 §6), which gives it a channel to the
Payments service and nothing else; it never talks to an adapter directly.

### 5.1 State machine (per request)

```
 Received ──validate (format, expiry, signature if any, amount limits)──► Presented (system sheet)
   │ invalid → DENIED                                 │ user cancels → CANCELLED
                                                      ▼
                                         ProviderChosen (user picks wallet/SIM)
                                                      ▼
                                         Authorising (provider prompt: STK push / USSD / app)
                                                      │ provider says success → SETTLED
                                                      │ provider says failure → FAILED
                                                      │ no answer within expiry → query provider:
                                                      │      definitive → SETTLED/FAILED, else EXPIRED
                                                      ▼
                                         PENDING (only if the provider reports pending; status() later)
```

- The same (`merchant` or app id, `nonce`) MUST NOT reach a provider twice: a retry returns the first result.
- A request is never resubmitted to a second provider after the first was contacted, unless the first
  reported a definitive failure.

## 6. The system payment sheet (normative behaviour)

- Drawn by Tonga in a layer apps cannot draw over or read; it shows the payee as the provider names it
  (looked up by the adapter, e.g. till name), the amount with currency, and the requesting app's name and
  publisher. It MUST NOT show text supplied by the app other than `reference`.
- The user's PIN is entered **only** in the provider's own channel (SIM toolkit push, USSD session, the
  provider's app). The sheet MUST NOT ask for a PIN, card number or password.
- A secure attention gesture (power button double-press) brings up a genuine-sheet indicator, so a fake
  sheet drawn by an app can be told apart.
- Plain-language and spoken (Asili voice) rendering for low-literacy users.

## 7. Rail adapter interface (AfriIDL, ordinal 0x0202, designed)

```
protocol RailAdapter @0x0202 {
    describe @1 () -> AdapterInfo;              // provider id, country, currencies, payee schemes
    payee_name @2 (payee: string:64) -> string:64?;
    start @3 (req_hash: array<u8,32>, amount: u64, currency: string:3, payee: string:64,
              reference: string:32, msisdn: string:16) -> PayStatus;
    query @4 (req_hash: array<u8,32>) -> PayResult;
}
```

Adapters are system-image components (not installable apps) and hold their provider credentials in the
platform keystore (designed); credentials MUST NOT be stored in the VFS, in git, or in an app package. A USSD
adapter drives the modem's USSD session and parses menus; it is inherently fragile and MUST time out to
EXPIRED rather than guess.

**Naming note.** Safaricom's M-Pesa developer API is itself called "Daraja". AfriSovera's Android
compatibility bridge is also named Daraja (docs/06). Adapter code and documents MUST say "M-Pesa API"
for the former to avoid confusion (review 2, R2-1).

## 8. Security considerations

- **No credential exposure**: apps never see the payer's phone number unless the user enters it for the
  app's own purposes; adapters receive the MSISDN from the SIM/user, not from the app.
- **Phishing**: only the system sheet can start a payment; an app drawing a look-alike can at most collect
  nothing (no PIN is ever typed into the sheet).
- **Amount tampering**: the sheet shows the request the Payments service validated; signed requests bind the
  amount to the merchant key.
- **Replay**: nonce + idempotency; receipts bound to the request hash.
- **Privacy**: payment history is stored on the device only; the OS sends no payment telemetry. Providers
  see what they must by law.
- **Fraud and disputes** are the provider's process; the OS records the request and receipt for the user.

## 9. Versioning

`"v":1` in every document; context strings versioned; protocol ordinals 0x0201/0x0202 with AfriIDL evolution
rules (AFR-0004 §6). A new payee scheme is a compatible change; unknown schemes are rejected with DENIED.

## 10. Test vectors

Request (canonical, 191 bytes):

```
{"amount":15000,"currency":"KES","expires":"2026-11-08T12:10:00Z","merchant":"21fe31dfa154a261","nonce":"q8Z3t6Yx","payee":"till:123456","reference":"ORDER-42","type":"afripay.request","v":1}
SHA-256    5a1ab3ba0f398fea6a25ef0f7d84eba70115a3fac86458aa21cb968e4ebd3355
signature  1f1eb2a19424b23fa7920fbeabcf2299d4a9e3070170e18104212547b9ed956e
           119da0bc31c02f5abfc9c5bde1c5c553cd53883ccc375aab5f6f12e876657608
           (RFC 8032 test key 1, context AFRISOVERA-PAY-REQUEST-V1\0)
```

Reject vectors: `"amount":150.00` (float); `"amount":0`; `"currency":"kes"` (lower case); `expires` 31
minutes after creation; the same nonce twice (second returns the first result, provider not contacted).

Conformance tests to write: a sandbox adapter (provider simulator in the cloud suite, never a live provider)
covering every state transition of §5.1, including a provider that answers late.

## 11. References

- docs/02 (AfriPay), docs/05 Phase 4, docs/12 (fees); AFR-0003, AFR-0004, AFR-0005.
- ISO 4217; RFC 8785; GSMA Mobile Money API specification (provider-neutral reference); provider API docs
  are read only under the owner's accounts and terms.
