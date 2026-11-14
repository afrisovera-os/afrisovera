#!/usr/bin/env python3
"""AFR-0010 (safe updates) test vectors, 13 Nov 2026.

Vector 1: the 1 KiB stand-in boot file of `scripts/make-update.py pe --kib 1`, release version 2, target
pc-x86_64, signed with the DEMO release key (seed 21 22 .. 40). Ed25519 signatures are deterministic, so the
whole file is fixed. Any change to the header layout, the signing context or the key id rule changes these.

Run: python3 scripts/make-update.py vectors   (from the repo root)
"""
import hashlib
import importlib.util
import os

BOOT_SHA256 = "c0ff970d01d183a16a87be703586081d92f68ea5c3ed30c0533023dcb36fa16f"
HEADER = (
    "41465249555044318000000001000000"  # AFRIUPD1, header length 128, format 1
    "70632d7838365f363400000000000000"  # target "pc-x86_64"
    "0200000000000000c945cbf2a5602002"  # version 2, reserved 0, key id
    "0004000000000000"                  # boot file size 1024
    + BOOT_SHA256
    + "00" * 40                          # reserved
)
SIGNATURE = (
    "43ababa13a40174f46e08a0b69c827bdd2aa44d484a0c319ecdc8e6b790f0b5c"
    "42b25aef88bfd49acba13af1986d3297d387c2c61bb78547acaada0afa449607"
)
FILE_SHA256 = "2171b2ada5c9a469d82690b51e82e3e6d81042c9b69d41d952f922b97b748ab8"
FILE_LEN = 1216
DEMO_PUBLIC = "e7f162a10bec559afea195e4dce84b69568d5d2cb0963eb446c0685e2b17f2f0"


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    tool = os.path.join(here, "..", "..", "..", "scripts", "make-update.py")
    spec = importlib.util.spec_from_file_location("make_update", tool)
    mu = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mu)
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey

    bad = 0

    def check(name, ok):
        nonlocal bad
        print(("  ok   " if ok else "  FAIL ") + name)
        bad += 0 if ok else 1

    boot = mu.stand_in_pe(1)
    f = mu.update_file(boot, 2)
    check("demo public key", mu.public(mu.DEMO_SEED).hex() == DEMO_PUBLIC)
    check("boot file SHA-256", hashlib.sha256(boot).hexdigest() == BOOT_SHA256)
    check("header bytes", f[:128].hex() == HEADER)
    check("signature bytes", f[128:192].hex() == SIGNATURE)
    check("whole file", hashlib.sha256(f).hexdigest() == FILE_SHA256 and len(f) == FILE_LEN)
    pk = Ed25519PublicKey.from_public_bytes(bytes.fromhex(DEMO_PUBLIC))
    try:
        pk.verify(bytes.fromhex(SIGNATURE), b"AfriSovera update v1\0" + bytes.fromhex(HEADER))
        check("signature verifies over the context and header", True)
    except Exception:
        check("signature verifies over the context and header", False)
    try:
        pk.verify(bytes.fromhex(SIGNATURE), bytes.fromhex(HEADER))
        check("signature does NOT verify without the context", False)
    except Exception:
        check("signature does NOT verify without the context", True)
    print("AFR-0010 VECTORS " + ("PASSED" if bad == 0 else "FAILED"))
    return 1 if bad else 0


if __name__ == "__main__":
    raise SystemExit(main())
