# Test vectors for AFR-0009 (Sovereign Mesh wire format). Run: python -I afr0009_vectors.py
# Keys are PUBLIC test keys only: Ed25519 from RFC 8032 TEST 1/2, X25519 from RFC 7748 section 6.1.
# HPKE (RFC 9180) mode_auth is implemented here by hand; the same code in base mode is cross-checked
# against the `cryptography` library's HPKE (which has base mode only) so the key schedule is independent.
import hashlib, hmac, struct
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PrivateKey, X25519PublicKey
from cryptography.hazmat.primitives.ciphers.aead import ChaCha20Poly1305
from cryptography.hazmat.primitives import serialization as s

RAW = (s.Encoding.Raw, s.PublicFormat.Raw)
def xpub(k): return k.public_key().public_bytes(*RAW)
def hm(k, m): return hmac.new(k, m, hashlib.sha256).digest()

# ---- RFC 5869 HKDF and RFC 9180 labelled HKDF -------------------------------------------------------
def extract(salt, ikm): return hm(salt or bytes(32), ikm)
def expand(prk, info, L):
    out, t, i = b"", b"", 1
    while len(out) < L:
        t = hm(prk, t + info + bytes([i])); out += t; i += 1
    return out[:L]
def hkdf(salt, ikm, info, L): return expand(extract(salt, ikm), info, L)

KEM_ID, KDF_ID, AEAD_ID = 0x0020, 0x0001, 0x0003
SUITE_KEM = b"KEM" + struct.pack(">H", KEM_ID)
SUITE = b"HPKE" + struct.pack(">HHH", KEM_ID, KDF_ID, AEAD_ID)
def lextract(suite, salt, label, ikm): return extract(salt, b"HPKE-v1" + suite + label + ikm)
def lexpand(suite, prk, label, info, L):
    return expand(prk, struct.pack(">H", L) + b"HPKE-v1" + suite + label + info, L)

def extract_and_expand(dh, kem_context):
    eae = lextract(SUITE_KEM, b"", b"eae_prk", dh)
    return lexpand(SUITE_KEM, eae, b"shared_secret", kem_context, 32)

def encap(pkR, skE, skS=None):
    enc = xpub(skE)
    dh = skE.exchange(X25519PublicKey.from_public_bytes(pkR))
    ctx = enc + pkR
    if skS is not None:  # AuthEncap
        dh += skS.exchange(X25519PublicKey.from_public_bytes(pkR)); ctx += xpub(skS)
    return extract_and_expand(dh, ctx), enc

def decap(enc, skR, pkS=None):
    dh = skR.exchange(X25519PublicKey.from_public_bytes(enc))
    ctx = enc + xpub(skR)
    if pkS is not None:  # AuthDecap
        dh += skR.exchange(X25519PublicKey.from_public_bytes(pkS)); ctx += pkS
    return extract_and_expand(dh, ctx)

def key_schedule(mode, ss, info):
    psk_id_hash = lextract(SUITE, b"", b"psk_id_hash", b"")
    info_hash = lextract(SUITE, b"", b"info_hash", info)
    ctx = bytes([mode]) + psk_id_hash + info_hash
    secret = lextract(SUITE, ss, b"secret", b"")
    return lexpand(SUITE, secret, b"key", ctx, 32), lexpand(SUITE, secret, b"base_nonce", ctx, 12)

def seal(mode, pkR, skE, skS, info, aad, pt):
    ss, enc = encap(pkR, skE, skS)
    key, nonce = key_schedule(mode, ss, info)
    return enc, ChaCha20Poly1305(key).encrypt(nonce, pt, aad)  # sequence number 0

def open_(mode, enc, skR, pkS, info, aad, ct):
    key, nonce = key_schedule(mode, decap(enc, skR, pkS), info)
    return ChaCha20Poly1305(key).decrypt(nonce, ct, aad)

# Cross-check the hand-written key schedule (base mode) against the library.
from cryptography.hazmat.primitives import hpke
_k = X25519PrivateKey.from_private_bytes(bytes(range(32)))
_lib = hpke.Suite(hpke.KEM.X25519, hpke.KDF.HKDF_SHA256, hpke.AEAD.CHACHA20_POLY1305)
_ct = _lib.encrypt(b"cross-check", _k.public_key(), info=b"i")
assert open_(0, _ct[:32], _k, None, b"i", b"", _ct[32:]) == b"cross-check"

# ---- Test identities --------------------------------------------------------------------------------
edA = Ed25519PrivateKey.from_private_bytes(bytes.fromhex("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60"))
edB = Ed25519PrivateKey.from_private_bytes(bytes.fromhex("4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb"))
xA = X25519PrivateKey.from_private_bytes(bytes.fromhex("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a"))
xB = X25519PrivateKey.from_private_bytes(bytes.fromhex("5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb"))
pA, pB = xpub(xA), xpub(xB)
assert xA.exchange(X25519PublicKey.from_public_bytes(pB)).hex() == \
    "4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742"  # RFC 7748 6.1

def p(label, v): print(f"{label:<28} {v.hex() if isinstance(v, bytes) else v}")

# ---- 5.2 identity record (128 bytes) ----------------------------------------------------------------
def identity_record(ed, x):
    body = ed.public_key().public_bytes(*RAW) + xpub(x)
    return body + ed.sign(b"AFRISOVERA-MESH-IDENTITY-V1\0" + hashlib.sha256(body).digest())
recA = identity_record(edA, xA)
print("== identity record (Alice)"); p("record", recA); assert len(recA) == 128

# ---- 5.3 contact root -------------------------------------------------------------------------------
def contact_root(sk, pk_other):
    lo, hi = sorted([xpub(sk), pk_other])
    return hkdf(lo + hi, sk.exchange(X25519PublicKey.from_public_bytes(pk_other)),
                b"AFRISOVERA-MESH-CONTACT-V1\0", 32)
root = contact_root(xA, pB); assert root == contact_root(xB, pA)
print("== contact root (Alice, Bob)"); p("root", root)

# ---- 5.4 recipient tag ------------------------------------------------------------------------------
def rtag(root, sender, recipient, period):
    d = b"\x00" if sender < recipient else b"\x01"
    return hm(root, b"AFRISOVERA-MESH-TAG-V1\0" + d + struct.pack(">Q", period))[:16]
T = 1794225600  # 2026-11-09T12:00:00Z
period = T // 3600
print("== recipient tags, period", period)
p("tag A->B", rtag(root, pA, pB, period)); p("tag B->A", rtag(root, pB, pA, period))
p("tag A->B next period", rtag(root, pA, pB, period + 1))

# ---- 5.5 beacon tag ---------------------------------------------------------------------------------
beacon_key = bytes([0x42]) * 32
def btag(k, t): return hm(k, b"AFRISOVERA-MESH-BEACON-V1\0" + struct.pack(">Q", t // 900))[:8]
print("== beacon tag (key 32 x 42)"); p("beacon tag at T", btag(beacon_key, T))

# ---- 4.3 short authentication string ----------------------------------------------------------------
h = hashlib.sha256(b"example Noise handshake hash").digest()
sas = int.from_bytes(hashlib.sha256(b"AFRISOVERA-MESH-SAS-V1\0" + h).digest()[:4], "big") % 1_000_000
print("== SAS"); p("h", h); p("SAS", f"{sas:06d}")

# ---- 6 bundle, Alice -> Bob, bucket 0 (256 bytes) ---------------------------------------------------
BUCKETS = [256, 1024, 4096, 16384]
def bundle(bucket, ttl, bid, tag, expires, created, mtype, body, skE, skS, pkR):
    size = BUCKETS[bucket]
    inner = struct.pack("<BBHQ", mtype, 0, len(body), created) + body
    inner += bytes(size - 92 - len(inner))
    hdr = struct.pack("<BBBB", 1, 0, ttl, bucket) + bid + tag + struct.pack("<Q", expires)
    aad = hdr[:2] + b"\x00" + hdr[3:]
    enc, ct = seal(2, pkR, skE, skS, b"AFRISOVERA-MESH-BUNDLE-V1\0", aad, inner)
    out = hdr + enc + ct; assert len(out) == size
    return out
skE = X25519PrivateKey.from_private_bytes(bytes([0x11]) * 32)
bid = bytes(range(16))
tag = rtag(root, pA, pB, period)
b = bundle(0, 8, bid, tag, T + 86400, T, 1, "Habari, Bob!".encode(), skE, xA, pB)
print("== bundle (256 bytes; text 'Habari, Bob!', ttl 8, expires T+24h)")
for i in range(0, len(b), 32): print("  " + b[i:i + 32].hex())
print("  sha256", hashlib.sha256(b).hexdigest())

# Recipient side: open with the sender's static key found via the tag; a relay's ttl change is harmless.
relayed = b[:2] + b"\x07" + b[3:]
aad = relayed[:2] + b"\x00" + relayed[3:44]
pt = open_(2, relayed[44:76], xB, pA, b"AFRISOVERA-MESH-BUNDLE-V1\0", aad, relayed[76:])
mtype, _, n, created = struct.unpack("<BBHQ", pt[:12])
assert (mtype, created, pt[12:12 + n]) == (1, T, "Habari, Bob!".encode())
# Reject: a different claimed sender (Bob's own key) must fail.
try:
    open_(2, relayed[44:76], xB, pB, b"AFRISOVERA-MESH-BUNDLE-V1\0", aad, relayed[76:]); raise SystemExit("FAIL")
except Exception as e:
    assert type(e).__name__ == "InvalidTag"
print("== checks: RFC 7748 DH ok, HPKE base cross-check ok, open after relay ok, wrong sender rejected")
