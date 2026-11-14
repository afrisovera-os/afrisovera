#!/usr/bin/env python3
"""Parse the published Wi-Fi crypto test vectors into the host fixture and the kernel's KAT constant (W1).

Per the architect's rule (Wi-Fi plan verdict, 14 Nov 2026, and the DRBG rule before it): NO vector byte is typed by
hand or taken from memory. scripts/wifi-kat-fetch.sh downloads each source file, logs its URL, size and sha256, and
runs THIS parser on the files as downloaded. The parser reads the bytes straight from them.

Sources and what is kept:
  rfc2202.txt   section 3, HMAC-SHA-1 test cases 1-7 (the HMAC-MD5 section is skipped)
  rfc6070.txt   PBKDF2-HMAC-SHA1; the 16,777,216-round case is skipped (too slow for a test), all others kept
  rfc3394.txt   AES key wrap with a 128-bit KEK (section 4.1); WPA2 only ever uses a 128-bit KEK
  rfc4493.txt   AES-CMAC examples 1-4
  rfc3610.txt   AES-CCM packet vectors with an 8-octet tag and a 13-octet nonce (CCMP's parameters: M=8, L=2)
  crypto_module_tests.c (hostapd, BSD licence): the IEEE 802.11 PRF-SHA1 cases (802.11 Annex J.3, label
                "prefix") and the passphrase-to-PSK cases (802.11 Annex J.4). Data only; no code is taken.

Modes:
  SRC_DIR --fixture OUT.txt --rust OUT.rs --sources SOURCE.txt   parse everything; write the outputs
  --selftest                                                     parse embedded samples; no source files needed
Run with `python3 -I` (stdlib only).
"""
import argparse
import hashlib
import os
import re
import sys

SOURCES = ["rfc2202.txt", "rfc6070.txt", "rfc3394.txt", "rfc4493.txt", "rfc3610.txt", "crypto_module_tests.c"]
SLOW_ROUNDS = 100000  # PBKDF2 cases above this are skipped (RFC 6070 has one with 16,777,216 rounds)


class ParseError(Exception):
    pass


def hexbytes(s):
    s = re.sub(r"[\s]", "", s)
    if s.lower().startswith("0x"):
        s = s[2:]
    if len(s) % 2 or not re.fullmatch(r"[0-9A-Fa-f]*", s):
        raise ParseError("not hex: %r" % s)
    return bytes.fromhex(s)


def c_string(lit):
    """The bytes of a C string literal body (no quotes); only the escapes these files use."""
    out = bytearray()
    i = 0
    while i < len(lit):
        c = lit[i]
        if c == "\\":
            n = lit[i + 1]
            if n == "0":
                out.append(0)
            elif n == "\\":
                out.append(0x5C)
            elif n == '"':
                out.append(0x22)
            else:
                raise ParseError("unknown escape \\%s" % n)
            i += 2
        else:
            out.append(ord(c))
            i += 1
    return bytes(out)


# ---- RFC 2202, section 3 (HMAC-SHA-1) -------------------------------------------------------------------------

def _rfc2202_value(v):
    v = v.strip()
    m = re.fullmatch(r"0x([0-9a-fA-F]{2}) repeated (\d+) times", v)
    if m:
        return bytes([int(m.group(1), 16)]) * int(m.group(2))
    m = re.fullmatch(r'"(.*)"', v)
    if m:
        return m.group(1).encode("ascii")
    if v.startswith("0x"):
        return hexbytes(v)
    raise ParseError("rfc2202 value %r" % v)


def parse_rfc2202(text):
    # the section heading itself, not its line in the table of contents
    m = re.search(r"\n3\. Test Cases for HMAC-SHA-1\n", text)
    if not m:
        raise ParseError("rfc2202: no HMAC-SHA-1 section")
    start = m.start()
    end = text.find("\n4. Security Considerations", start)
    sec = text[start:end if end > 0 else len(text)]
    cases, cur = [], None
    lines = sec.splitlines()
    i = 0
    while i < len(lines):
        line = lines[i]
        i += 1
        m = re.match(r"^(test_case|key|key_len|data|data_len|digest)\s+=\s+(.*)$", line)
        if not m:
            continue
        k, v = m.group(1), m.group(2)
        # a quoted value may go on over the next lines; the RFC breaks it at a space (the length check proves it)
        while v.startswith('"') and not (len(v) > 1 and v.endswith('"')) and i < len(lines):
            v = v + " " + lines[i].strip()
            i += 1
        if k == "test_case":
            cur = {"n": int(v)}
            cases.append(cur)
        elif cur is None:
            # The published RFC 2202 text repeats a few lines after case 7 (stray data_len/digest lines, then cases
            # 6 and 7 again). Fields outside an open case are ignored; repeated cases are checked equal below.
            continue
        elif k in ("key_len", "data_len"):
            cur[k] = int(v)
        else:
            cur[k] = _rfc2202_value(v)
            if k == "digest":
                cur = None  # a case ends at its digest
    first = {}
    for c in cases:
        if c["n"] in first:
            if first[c["n"]] != c:
                raise ParseError("rfc2202: case %d printed twice with different values" % c["n"])
        else:
            first[c["n"]] = c
    cases = [first[n] for n in sorted(first)]
    if [c["n"] for c in cases] != list(range(1, 8)):
        raise ParseError("rfc2202: cases are not 1 to 7")
    out = []
    for c in cases:
        if len(c.get("key", b"")) != c["key_len"] or len(c.get("data", b"")) != c["data_len"]:
            raise ParseError("rfc2202: a length field does not match its value")
        if len(c["digest"]) != 20:
            raise ParseError("rfc2202: digest not 20 bytes")
        out.append((c["key"], c["data"], c["digest"]))
    if len(out) != 7:
        raise ParseError("rfc2202: expected 7 HMAC-SHA-1 cases, found %d" % len(out))
    return out


# ---- RFC 6070 (PBKDF2-HMAC-SHA1) ------------------------------------------------------------------------------

def parse_rfc6070(text):
    out, skipped = [], 0
    blocks = re.findall(r'P = "(.*?)" \((\d+) octets\)\s+S = "(.*?)" \((\d+) octets\)\s+c = (\d+)\s+dkLen = (\d+)'
                        r'\s+Output:\s+DK = ((?:[0-9a-f]{2}[ \n]+)+)\s*\((\d+) octets\)', text)
    for p, plen, s, slen, c, dklen, dk, dkoct in blocks:
        p, s = c_string(p), c_string(s)
        dk = hexbytes(dk)
        if len(p) != int(plen) or len(s) != int(slen) or len(dk) != int(dklen) or len(dk) != int(dkoct):
            raise ParseError("rfc6070: a stated length does not match")
        if int(c) > SLOW_ROUNDS:
            skipped += 1
            continue
        out.append((p, s, int(c), dk))
    if len(out) != 5 or skipped != 1:
        raise ParseError("rfc6070: expected 5 kept and 1 skipped, got %d and %d" % (len(out), skipped))
    return out


# ---- RFC 3394, section 4.1 (AES key wrap, 128-bit KEK) --------------------------------------------------------

def parse_rfc3394(text):
    s = text.find("\n4.1 Wrap 128 bits of Key Data with a 128-bit KEK")
    e = text.find("\n4.2 Wrap", s)
    if s < 0 or e < 0:
        raise ParseError("rfc3394: section 4.1 not found")
    sec = text[s:e]
    kek = re.search(r"KEK:\s+([0-9A-F]+)", sec)
    data = re.search(r"Key Data:\s+([0-9A-F]+)", sec)
    ct = re.search(r"Ciphertext:?\s+((?:[0-9A-F]{16} ?)+)", sec)
    if not (kek and data and ct):
        raise ParseError("rfc3394: a field of 4.1 is missing")
    kek, data, ct = hexbytes(kek.group(1)), hexbytes(data.group(1)), hexbytes(ct.group(1))
    if len(kek) != 16 or len(data) != 16 or len(ct) != 24:
        raise ParseError("rfc3394: unexpected sizes")
    return [(kek, data, ct)]


# ---- RFC 4493 (AES-CMAC) --------------------------------------------------------------------------------------

def parse_rfc4493(text):
    s = text.find("4.  Test Vectors")
    e = text.find("5.  Acknowledgement", s)
    if s < 0 or e < 0:
        raise ParseError("rfc4493: section 4 not found")
    sec = text[s:e]
    k = re.search(r"\n\s+K\s+((?:[0-9a-f]{8} ?){4})", sec)
    if not k:
        raise ParseError("rfc4493: key not found")
    key = hexbytes(k.group(1))
    out = []
    for m in re.finditer(r"Example \d+: len = (\d+)\s+M\s+(.*?)AES-CMAC\s+((?:[0-9a-f]{8} ?){4})", sec, re.S):
        mlen, mtxt, tag = int(m.group(1)), m.group(2), hexbytes(m.group(3))
        # page breaks between examples are not in these ranges (checked by the length below)
        msg = b"" if "<empty string>" in mtxt else hexbytes(mtxt)
        if len(msg) != mlen:
            raise ParseError("rfc4493: example message length %d != %d" % (len(msg), mlen))
        out.append((key, msg, tag))
    if len(out) != 4:
        raise ParseError("rfc4493: expected 4 examples, found %d" % len(out))
    return out


# ---- RFC 3610 (AES-CCM packet vectors; keep M=8, 13-octet nonce) ----------------------------------------------

def _hex_lines_after(lines, i):
    """Hex octet rows that follow line i (rows of 'XX XX ...' with leading spaces), joined."""
    buf = []
    j = i + 1
    while j < len(lines) and re.fullmatch(r"\s+(?:[0-9A-F]{2}\s*)+", lines[j]):
        buf.append(lines[j])
        j += 1
    return hexbytes(" ".join(buf))


def parse_rfc3610(text):
    lines = text.splitlines()
    out, seen = [], 0
    i = 0
    while i < len(lines):
        if not re.search(r"=+ Packet Vector #\d+ =+", lines[i]):
            i += 1
            continue
        seen += 1
        key = nonce = inp = outp = None
        hdr = None
        j = i + 1
        while j < len(lines) and not re.search(r"Packet Vector #", lines[j]):
            L = lines[j]
            m = re.match(r"\s+AES Key =\s+(.*)$", L)
            if m:
                key = hexbytes(m.group(1))
            m = re.match(r"\s+Nonce =\s+(.*)$", L)
            if m:
                nonce = hexbytes(m.group(1))
            m = re.match(r"\s+Total packet length = (\d+)\. \[Input with (\d+) cleartext header octets\]", L)
            if m:
                inp = _hex_lines_after(lines, j)
                hdr = int(m.group(2))
                if len(inp) != int(m.group(1)):
                    raise ParseError("rfc3610: input length")
            m = re.match(r"\s+Total packet length = (\d+)\. \[Authenticated and Encrypted Output\]", L)
            if m:
                outp = _hex_lines_after(lines, j)
                if len(outp) != int(m.group(1)):
                    raise ParseError("rfc3610: output length")
            j += 1
        if None in (key, nonce, inp, outp, hdr):
            raise ParseError("rfc3610: packet vector %d incomplete" % seen)
        tag_len = len(outp) - len(inp)
        if tag_len == 8 and len(nonce) == 13:
            aad, plain = inp[:hdr], inp[hdr:]
            if outp[:hdr] != aad:
                raise ParseError("rfc3610: header not carried in the clear")
            out.append((key, nonce, aad, plain, outp[hdr:]))
        i = j
    if seen != 24 or len(out) != 12:
        raise ParseError("rfc3610: expected 24 vectors and 12 with M=8, got %d and %d" % (seen, len(out)))
    return out


# ---- hostapd crypto_module_tests.c: 802.11 PRF-SHA1 and passphrase-to-PSK -------------------------------------

def _c_arrays(text):
    """name -> bytes for `static const u8 name[] = { ... };` and `= "string";` (string bytes + NUL, as C)."""
    arr = {}
    for m in re.finditer(r"static const u8 (\w+)\[\]\s*=\s*\{([^}]*)\};", text):
        arr[m.group(1)] = bytes(int(x, 16) for x in re.findall(r"0x([0-9a-fA-F]{2})", m.group(2)))
    for m in re.finditer(r'static const u8 (\w+)\[\]\s*=\s*"((?:[^"\\]|\\.)*)";', text):
        arr[m.group(1)] = c_string(m.group(2)) + b"\x00"
    return arr


def _sizeof_arg(arr, name, minus):
    if name not in arr:
        raise ParseError("hostapd: no array %s" % name)
    v = arr[name]
    if minus:
        if not v.endswith(b"\x00"):
            raise ParseError("hostapd: sizeof(%s) - 1 on an array that is not a string" % name)
        return v[:-1]
    return v


def parse_hostapd(text):
    arr = _c_arrays(text)
    prf = []
    for m in re.finditer(r'sha1_prf\((\w+), sizeof\(\w+\)( - 1)?, "(\w+)", (\w+), sizeof\(\w+\)( - 1)?,\s*'
                         r'res, sizeof\((\w+)\)\)', text):
        key = _sizeof_arg(arr, m.group(1), bool(m.group(2)))
        label = m.group(3).encode("ascii")
        data = _sizeof_arg(arr, m.group(4), bool(m.group(5)))
        want = arr[m.group(6)]
        prf.append((key, label, data, want))
    if len(prf) != 3:
        raise ParseError("hostapd: expected 3 sha1_prf cases, found %d" % len(prf))
    s = text.find("static const struct passphrase_test passphrase_tests[]")
    e = text.find("};", s)
    if s < 0 or e < 0:
        raise ParseError("hostapd: passphrase_tests not found")
    psk = []
    for m in re.finditer(r'\{\s*"([^"]*)",\s*"([^"]*)",\s*\{([^}]*)\}\s*\}', text[s:e]):
        want = bytes(int(x, 16) for x in re.findall(r"0x([0-9a-fA-F]{2})", m.group(3)))
        if len(want) != 32:
            raise ParseError("hostapd: a PSK is not 32 bytes")
        psk.append((m.group(1).encode("ascii"), m.group(2).encode("ascii"), want))
    if len(psk) != 3:
        raise ParseError("hostapd: expected 3 passphrase cases, found %d" % len(psk))
    return prf, psk


# ---- outputs --------------------------------------------------------------------------------------------------

def h(b):
    return b.hex() if b else "-"


def fixture_lines(v):
    out = []
    for k, d, t in v["hmac"]:
        out.append("hmac_sha1 %s %s %s" % (h(k), h(d), h(t)))
    for p, s, c, dk in v["pbkdf2"]:
        out.append("pbkdf2_sha1 %s %s %d %s" % (h(p), h(s), c, h(dk)))
    for p, s, want in v["psk"]:
        out.append("psk %s %s %s" % (h(p), h(s), h(want)))
    for k, lab, d, want in v["prf"]:
        out.append("prf_sha1 %s %s %s %s" % (h(k), h(lab), h(d), h(want)))
    for kek, data, ct in v["kw"]:
        out.append("aes_kw %s %s %s" % (h(kek), h(data), h(ct)))
    for k, m, t in v["cmac"]:
        out.append("cmac %s %s %s" % (h(k), h(m), h(t)))
    for k, n, a, p, c in v["ccm"]:
        out.append("ccm8 %s %s %s %s %s" % (h(k), h(n), h(a), h(p), h(c)))
    return out


def rs_bytes(b):
    return "&[" + ", ".join("0x%02x" % x for x in b) + "]"


def rust_const(v, srcs):
    L = ["//! GENERATED by scripts/wifi-kat-parse.py from the published files below. Do not edit by hand.",
         "//! The bytes were read from the files as downloaded by scripts/wifi-kat-fetch.sh (never typed).",
         "//!"]
    for name, url, size, sha in srcs:
        L.append("//! %s  %d bytes  sha256 %s" % (name, size, sha))
        L.append("//!   %s" % url)
    L += ["#![allow(dead_code)]", "",
          "/// HMAC-SHA1 (RFC 2202 section 3): key, data, digest.",
          "pub const HMAC_SHA1: &[(&[u8], &[u8], &[u8])] = &["]
    L += ["    (%s, %s, %s)," % (rs_bytes(k), rs_bytes(d), rs_bytes(t)) for k, d, t in v["hmac"]]
    L += ["];", "", "/// PBKDF2-HMAC-SHA1 (RFC 6070): password, salt, rounds, derived key.",
          "pub const PBKDF2_SHA1: &[(&[u8], &[u8], u32, &[u8])] = &["]
    L += ["    (%s, %s, %d, %s)," % (rs_bytes(p), rs_bytes(s), c, rs_bytes(dk)) for p, s, c, dk in v["pbkdf2"]]
    L += ["];", "", "/// Passphrase to PSK (IEEE 802.11 Annex J.4, via hostapd's tests): passphrase, SSID, PSK.",
          "pub const PSK: &[(&[u8], &[u8], &[u8])] = &["]
    L += ["    (%s, %s, %s)," % (rs_bytes(p), rs_bytes(s), rs_bytes(w)) for p, s, w in v["psk"]]
    L += ["];", "", "/// 802.11 PRF-SHA1 (Annex J.3, via hostapd's tests): key, label, data, output.",
          "pub const PRF_SHA1: &[(&[u8], &[u8], &[u8], &[u8])] = &["]
    L += ["    (%s, %s, %s, %s)," % (rs_bytes(k), rs_bytes(a), rs_bytes(d), rs_bytes(w)) for k, a, d, w in v["prf"]]
    L += ["];", "", "/// AES key wrap, 128-bit KEK (RFC 3394 section 4.1): KEK, key data, ciphertext.",
          "pub const AES_KW: &[(&[u8], &[u8], &[u8])] = &["]
    L += ["    (%s, %s, %s)," % (rs_bytes(a), rs_bytes(b), rs_bytes(c)) for a, b, c in v["kw"]]
    L += ["];", "", "/// AES-128-CMAC (RFC 4493 section 4): key, message, tag.",
          "pub const CMAC: &[(&[u8], &[u8], &[u8])] = &["]
    L += ["    (%s, %s, %s)," % (rs_bytes(a), rs_bytes(b), rs_bytes(c)) for a, b, c in v["cmac"]]
    L += ["];", "", "/// AES-CCM, 8-octet tag, 13-octet nonce (RFC 3610): key, nonce, AAD, plaintext, ciphertext||tag.",
          "pub const CCM8: &[(&[u8], &[u8], &[u8], &[u8], &[u8])] = &["]
    L += ["    (%s, %s, %s, %s, %s)," % tuple(rs_bytes(x) for x in t) for t in v["ccm"]]
    L += ["];", ""]
    return "\n".join(L)


def parse_all(texts):
    prf, psk = parse_hostapd(texts["crypto_module_tests.c"])
    return {
        "hmac": parse_rfc2202(texts["rfc2202.txt"]),
        "pbkdf2": parse_rfc6070(texts["rfc6070.txt"]),
        "kw": parse_rfc3394(texts["rfc3394.txt"]),
        "cmac": parse_rfc4493(texts["rfc4493.txt"]),
        "ccm": parse_rfc3610(texts["rfc3610.txt"]),
        "prf": prf,
        "psk": psk,
    }


# ---- selftest: synthetic samples in each file's shape, so a parser change is caught with no download --------

SAMPLE_2202 = """
3. Test Cases for HMAC-SHA-1

test_case =     1
key =           0x0102
key_len =       2
data =          0xdd repeated 3 times
data_len =      3
digest =        0x00112233445566778899aabbccddeeff00112233
""" + "".join("""
test_case =     %d
key =           "k"
key_len =       1
data =          "d
                e"
data_len =      3
digest =        0x%s
""" % (n, ("%02x" % n) * 20) for n in range(2, 8)) + "\n4. Security Considerations\n"

DUP7 = """test_case =     7
key =           "k"
key_len =       1
data =          "d
                e"
data_len =      3
digest =        0x%s
""" % ("07" * 20)

SAMPLE_6070 = "".join("""
     Input:
       P = "pa\\0ss" (5 octets)
       S = "salt" (4 octets)
       c = %d
       dkLen = 4

     Output:
       DK = 01 02 03
            04             (4 octets)
""" % c for c in (1, 2, 3, 4, 5, 16777216))

SAMPLE_3394 = """
4.1 Wrap 128 bits of Key Data with a 128-bit KEK
   KEK:            000102030405060708090A0B0C0D0E0F
   Key Data:       00112233445566778899AABBCCDDEEFF
   Output:
   Ciphertext:  1FA68B0A8112B447 AEF34BD8FB5A7B82 9D3E862371D2CFE5

4.2 Wrap 128 bits of Key Data with a 192-bit KEK
"""

SAMPLE_4493 = """4.  Test Vectors
   K              2b7e1516 28aed2a6 abf71588 09cf4f3c
   Example 1: len = 0
   M              <empty string>
   AES-CMAC       bb1d6929 e9593728 7fa37d12 9b756746
   Example 2: len = 16
   M              6bc1bee2 2e409f96 e93d7e11 7393172a
   AES-CMAC       070a16b4 6b4d4144 f79bdd9d d04a287c
   Example 3: len = 4
   M              6bc1bee2
   AES-CMAC       00000000 00000000 00000000 00000001
   Example 4: len = 8
   M              6bc1bee2 2e409f96
   AES-CMAC       00000000 00000000 00000000 00000002
5.  Acknowledgement
"""


def _sample_3610():
    out = []
    for n in range(1, 25):
        tag = 8 if n <= 12 else 10
        inp = " ".join("%02X" % x for x in range(10))
        o = " ".join("%02X" % x for x in list(range(2)) + [0xEE] * (8 + tag))
        out.append("""   =============== Packet Vector #%d ==================
   AES Key =  C0 C1 C2 C3  C4 C5 C6 C7  C8 C9 CA CB  CC CD CE CF
   Nonce =    00 00 00 03  02 01 00 A0  A1 A2 A3 A4  A5
   Total packet length = 10. [Input with 2 cleartext header octets]
              %s
   CBC IV in: 59 00 00 00  03 02 01 00  A0 A1 A2 A3  A4 A5 00 17
   Total packet length = %d. [Authenticated and Encrypted Output]
              %s

""" % (n, inp, 10 + tag, o))
    return "".join(out)


SAMPLE_HOSTAPD = """
static const u8 key0[] =
{
	0x0b, 0x0b
};
static const u8 data0[] = "Hi";
static const u8 prf0[] =
{
	0x01, 0x02
};
static const u8 key1[] = "Jefe";
static const u8 data1[] = "w";
static const u8 prf1[] = { 0x03 };
static const u8 key2[] = { 0xaa };
static const u8 data2[] = { 0xdd, 0xdd };
static const u8 prf2[] = { 0x04 };
static const struct passphrase_test passphrase_tests[] =
{
	{ "password", "IEEE", { %s } },
	{ "a", "b", { %s } },
	{ "c", "d", { %s } },
};
	if (sha1_prf(key0, sizeof(key0), "prefix", data0, sizeof(data0) - 1,
		     res, sizeof(prf0)) == 0 &&
	if (sha1_prf(key1, sizeof(key1) - 1, "prefix", data1, sizeof(data1) - 1,
		     res, sizeof(prf1)) == 0 &&
	if (sha1_prf(key2, sizeof(key2), "prefix", data2, sizeof(data2),
		     res, sizeof(prf2)) == 0 &&
""" % tuple(", ".join(["0x%02x" % n] * 32) for n in (1, 2, 3))


def selftest():
    bad = 0

    def ok(cond, what):
        nonlocal bad
        print("  %s %s" % ("ok  " if cond else "FAIL", what))
        if not cond:
            bad += 1

    v = parse_all({"rfc2202.txt": SAMPLE_2202, "rfc6070.txt": SAMPLE_6070, "rfc3394.txt": SAMPLE_3394,
                   "rfc4493.txt": SAMPLE_4493, "rfc3610.txt": _sample_3610(), "crypto_module_tests.c": SAMPLE_HOSTAPD})
    ok(v["hmac"][0] == (b"\x01\x02", b"\xdd\xdd\xdd", bytes.fromhex("00112233445566778899aabbccddeeff00112233")),
       "rfc2202: hex key, 'repeated N times' data, digest")
    ok(v["hmac"][1][0] == b"k" and v["hmac"][1][1] == b"d e" and len(v["hmac"]) == 7,
       "rfc2202: quoted strings over two lines, 7 cases")
    dup = SAMPLE_2202.replace("\n4. Security", "\ndata_len =      20\ndigest =        0x" + "00" * 20 +
                              "\n" + DUP7 + "\n4. Security")
    ok(len(parse_rfc2202(dup)) == 7, "rfc2202: stray lines ignored, an identical repeated case accepted")
    ok(len(v["pbkdf2"]) == 5 and v["pbkdf2"][0][0] == b"pa\x00ss", "rfc6070: \\0 escape, slow case skipped")
    ok(v["pbkdf2"][0][3] == b"\x01\x02\x03\x04", "rfc6070: DK across two lines")
    ok(v["kw"][0][2] == bytes.fromhex("1FA68B0A8112B447AEF34BD8FB5A7B829D3E862371D2CFE5"), "rfc3394: 4.1 ciphertext")
    ok(v["cmac"][0][1] == b"" and len(v["cmac"]) == 4, "rfc4493: empty message, 4 examples")
    ok(len(v["ccm"]) == 12 and v["ccm"][0][2] == b"\x00\x01" and len(v["ccm"][0][4]) == 16,
       "rfc3610: M=8 kept, M=10 dropped, header split off as AAD")
    ok(v["prf"][0] == (b"\x0b\x0b", b"prefix", b"Hi", b"\x01\x02"), "hostapd: sizeof(string) - 1 drops the NUL")
    ok(v["prf"][1][0] == b"Jefe" and v["prf"][2][2] == b"\xdd\xdd", "hostapd: string key, whole array data")
    ok(v["psk"][0] == (b"password", b"IEEE", b"\x01" * 32) and len(v["psk"]) == 3, "hostapd: passphrase table")
    # damaged inputs must be refused, not half-parsed
    for name, text, mod in (("rfc2202 length lie", SAMPLE_2202, lambda t: t.replace("key_len =       2", "key_len =       3")),
                            ("rfc2202 repeat differs", SAMPLE_2202,
                             lambda t: t.replace("\n4. Security", "\n" + DUP7.replace('"k"', '"x"') + "\n4. Security")),
                            ("rfc6070 length lie", SAMPLE_6070, lambda t: t.replace("(4 octets)\n", "(5 octets)\n", 1)),
                            ("rfc3610 vector lost", _sample_3610(), lambda t: t.replace("Packet Vector #24", "Packet Vektor #24"))):
        texts = {"rfc2202.txt": SAMPLE_2202, "rfc6070.txt": SAMPLE_6070, "rfc3394.txt": SAMPLE_3394,
                 "rfc4493.txt": SAMPLE_4493, "rfc3610.txt": _sample_3610(), "crypto_module_tests.c": SAMPLE_HOSTAPD}
        key = {"rfc2202 length lie": "rfc2202.txt", "rfc2202 repeat differs": "rfc2202.txt", "rfc6070 length lie": "rfc6070.txt",
               "rfc3610 vector lost": "rfc3610.txt"}[name]
        texts[key] = mod(text)
        try:
            parse_all(texts)
            ok(False, "refuses: " + name)
        except ParseError:
            ok(True, "refuses: " + name)
    rs = rust_const(v, [("x", "u", 1, "s")])
    ok("pub const CCM8" in rs and "Do not edit by hand" in rs, "rust constant written")
    print("wifi-kat-parse selftest: %s" % ("PASS" if bad == 0 else "%d FAIL" % bad))
    return bad == 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("src_dir", nargs="?")
    ap.add_argument("--fixture")
    ap.add_argument("--rust")
    ap.add_argument("--sources", help="SOURCE.txt: url and file name per line, as written by wifi-kat-fetch.sh")
    ap.add_argument("--selftest", action="store_true")
    a = ap.parse_args()
    if a.selftest:
        sys.exit(0 if selftest() else 1)
    if not (a.src_dir and a.fixture and a.rust and a.sources):
        ap.error("give SRC_DIR --fixture --rust --sources, or --selftest")
    urls = {}
    with open(a.sources, encoding="ascii") as f:
        for line in f:
            p = line.split()
            if len(p) >= 2 and not line.startswith("#"):
                urls[p[0]] = p[1]
    texts, srcs = {}, []
    for name in SOURCES:
        raw = open(os.path.join(a.src_dir, name), "rb").read()
        texts[name] = raw.decode("utf-8")
        srcs.append((name, urls.get(name, "?"), len(raw), hashlib.sha256(raw).hexdigest()))
    v = parse_all(texts)
    hdr = ["# GENERATED by scripts/wifi-kat-parse.py; one vector per line; '-' is an empty field. Sources:"]
    hdr += ["# %s %d bytes sha256 %s %s" % (n, s, sh, u) for n, u, s, sh in srcs]
    with open(a.fixture, "w", encoding="ascii", newline="\n") as f:
        f.write("\n".join(hdr + fixture_lines(v)) + "\n")
    with open(a.rust, "w", encoding="ascii", newline="\n") as f:
        f.write(rust_const(v, srcs))
    n = sum(len(x) for x in v.values())
    print("wifi-kat-parse: %d vectors (hmac %d, pbkdf2 %d, psk %d, prf %d, aes-kw %d, cmac %d, ccm8 %d)" % (
        n, len(v["hmac"]), len(v["pbkdf2"]), len(v["psk"]), len(v["prf"]), len(v["kw"]), len(v["cmac"]), len(v["ccm"])))


if __name__ == "__main__":
    main()
