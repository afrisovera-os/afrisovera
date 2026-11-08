# AFR-0003 test-vector generator. Uses the RFC 8032 TEST 1 key (public, NOT a real publisher key).
# Run: python -I afr0003_vectors.py  (writes hello.nia next to it)
import hashlib, struct
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives import serialization as s
mod = bytes.fromhex("0061736d01000000" "010401600000" "03020100" "0707010372756e0000" "0a040102000b")
print("module", len(mod), mod.hex())
d = hashlib.sha256(mod).hexdigest(); print("sha256", d)
# RFC 8032 test 1 key
sk = Ed25519PrivateKey.from_private_bytes(bytes.fromhex("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60"))
pk = sk.public_key().public_bytes(s.Encoding.Raw, s.PublicFormat.Raw); print("pk", pk.hex())
print("keyid", hashlib.sha256(pk).hexdigest()[:16])
print("v0 sig", sk.sign(hashlib.sha256(mod).digest()).hex())
# v1 package
manifest = b'id = "org.example.hello"\nname = "Hello"\nversion = "1.0.0"\nabi = "nia/1"\nentry = "run"\npermissions = ["print"]\n'
print("manifest len", len(manifest))
nsec = 3
hdr_len = 16 + 24*nsec
off_man = hdr_len
off_mod = off_man + len(manifest)
off_sig = off_mod + len(mod)
SIGLEN = 2+2+8+32+64
hdr = b"NIA\x00" + struct.pack("<HHI", 1, 1, nsec) + b"\x00"*4
tab = struct.pack("<IIQQ",1,0,off_man,len(manifest)) + struct.pack("<IIQQ",2,0,off_mod,len(mod)) + struct.pack("<IIQQ",15,0,off_sig,SIGLEN)
body = hdr + tab + manifest + mod
assert len(body)==off_sig
dig = hashlib.sha256(body).digest()
print("pkg digest", dig.hex())
msg = b"AFRISOVERA-NIA-SIG-V1\x00" + dig
sig = sk.sign(msg)
sigblk = struct.pack("<HH",1,0) + hashlib.sha256(pk).digest()[:8] + pk + sig
pkg = body + sigblk
print("pkg len", len(pkg), "sig", sig.hex())
print("header+table hex", (hdr+tab).hex())
open("hello.nia","wb").write(pkg)
