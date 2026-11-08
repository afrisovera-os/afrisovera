# Test vectors for AFR-0005 (store catalog), AFR-0006 (payment request), AFR-0007 (PKCE, pairwise sub).
# Key: RFC 8032 TEST 1 (public test key, NOT a real store/merchant key). Run: python -I afr0005_0007_vectors.py
import base64, hashlib, json
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives import serialization as s
sk = Ed25519PrivateKey.from_private_bytes(bytes.fromhex("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60"))
pk = sk.public_key().public_bytes(s.Encoding.Raw, s.PublicFormat.Raw)
kid = hashlib.sha256(pk).digest()[:8].hex()
def canon(o): return json.dumps(o, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()
def b64u(b): return base64.urlsafe_b64encode(b).rstrip(b"=").decode()
def sign(ctx, body):
    d = hashlib.sha256(body).digest()
    return d, sk.sign(ctx + d)
cat = {"type": "afristore.catalog", "v": 1, "store": "store.example.org", "seq": 7,
       "issued": "2026-11-08T00:00:00Z", "expires": "2026-11-15T00:00:00Z",
       "apps": [{"id": "org.example.hello", "version": "1.0.0", "publisher": kid,
                 "sha256": "ca84e810f56c7ea77d501220783aed924d0cda74f1a78427165dad4c1db07edf",
                 "size": 339, "permissions": ["print"], "urls": ["pkg/ca84e810f56c7ea77d501220783aed924d0cda74f1a78427165dad4c1db07edf.nia"]}]}
body = canon(cat)
d, sig = sign(b"AFRISOVERA-STORE-CATALOG-V1\0", body)
print("AFR-0005 catalog canonical (%d bytes):" % len(body)); print(body.decode())
print("digest", d.hex()); print("sig", sig.hex())
req = {"type": "afripay.request", "v": 1, "amount": 15000, "currency": "KES", "payee": "till:123456",
       "reference": "ORDER-42", "merchant": kid, "expires": "2026-11-08T12:10:00Z", "nonce": "q8Z3t6Yx"}
body = canon(req)
d, sig = sign(b"AFRISOVERA-PAY-REQUEST-V1\0", body)
print("\nAFR-0006 request canonical (%d bytes):" % len(body)); print(body.decode())
print("digest", d.hex()); print("sig", sig.hex())
v = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"
print("\nAFR-0007 PKCE S256 challenge", b64u(hashlib.sha256(v.encode()).digest()))
sub = b64u(hashlib.sha256(b"app.example.com" + b"\0" + b"user-0001" + b"\0" + bytes(16)).digest())
print("pairwise sub (sector app.example.com, account user-0001, salt 16x00)", sub)
