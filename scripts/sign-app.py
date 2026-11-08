#!/usr/bin/env python3
"""Off-device app signer for AfriStore (publisher toolchain, v0).

Extracts a seeded app's exact bytes from kernel/src/nia.rs, computes its SHA-256,
and signs that digest with an Ed25519 key derived from a fixed demo seed (so the
result is reproducible). Emits the publisher public key (32B), the signature (64B),
and the digest as Rust byte arrays to paste into the kernel's signed manifest.

Real publishers would use their own secret key; the device only ever verifies.

Usage: sign-app.py WASM_SUM
"""
import hashlib
import re
import sys

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

# Fixed demo publisher seed (NOT a real key — reproducible for the built-in demo).
SEED = bytes(range(1, 33))  # 01 02 03 ... 20


def extract_bytes(src: str, name: str) -> bytes:
    m = re.search(r"static\s+" + re.escape(name) + r"\s*:\s*&\[u8\]\s*=\s*&\[(.*?)\];", src, re.S)
    if not m:
        raise SystemExit(f"array {name} not found in nia.rs")
    out = bytearray()
    for tok in re.split(r"[,\s]+", m.group(1)):
        tok = tok.strip()
        if not tok:
            continue
        if tok.startswith("0x") or tok.startswith("0X"):
            out.append(int(tok, 16))
        elif re.fullmatch(r"b'.'", tok):
            out.append(ord(tok[2]))
        elif re.fullmatch(r"b'\\.'", tok):
            out.append(ord(bytes(tok[2:4], "utf-8").decode("unicode_escape")))
        elif tok.isdigit():
            out.append(int(tok))
        else:
            raise SystemExit(f"unparseable token: {tok!r}")
    return bytes(out)


def rust_array(name: str, data: bytes) -> str:
    body = ", ".join(f"0x{b:02x}" for b in data)
    return f"pub const {name}: [u8; {len(data)}] = [{body}];"


def main():
    name = sys.argv[1] if len(sys.argv) > 1 else "WASM_SUM"
    src = open("kernel/src/nia.rs", encoding="utf-8").read()
    app = extract_bytes(src, name)
    digest = hashlib.sha256(app).digest()

    sk = Ed25519PrivateKey.from_private_bytes(SEED)
    pk = sk.public_key().public_bytes_raw()
    sig = sk.sign(digest)  # sign the app's SHA-256 digest

    print(f"// {name}: {len(app)} bytes, sha256 = {digest.hex()}")
    print(rust_array("PUBLISHER_PUBKEY", pk))
    print(rust_array("SUM_WASM_SIG", sig))


if __name__ == "__main__":
    main()
