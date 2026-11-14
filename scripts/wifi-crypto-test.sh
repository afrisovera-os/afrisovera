#!/usr/bin/env bash
# wifi-crypto (W1 of the Wi-Fi plan, architect's verdict 14 Nov 2026): the Wi-Fi primitives, host side.
#   1. The vector parser's own selftest (scripts/wifi-kat-parse.py --selftest).
#   2. The committed vectors are what the published files give today: scripts/wifi-kat-fetch.sh --check downloads
#      RFC 2202, 6070, 3394, 4493, 3610 and hostapd's copy of IEEE 802.11 Annexes J.3/J.4, checks each against its
#      pinned sha256, parses them again and compares byte for byte with wlan-crypto/fixtures/kat.txt,
#      fixtures/SOURCE.txt and kernel/src/wlan/crypto_kat.rs.
#   3. Confinement: no file but kernel/src/wlan/crypto.rs names wlan_crypto or the crates under it, and no
#      manifest but kernel/Cargo.toml and wlan-crypto/Cargo.toml names them (a planted violation proves the check).
#   4. `cargo test` of wlan-crypto: every published vector, both directions, plus the refusals (bad lengths, a
#      tampered key wrap or CCM frame refused with its output zeroed, the passphrase rules).
# The PC image side (no SIMD, no cpuid in the crypto, `wifi selftest` in QEMU) is suite pc-wifikat.
#   scripts/wifi-crypto-test.sh              # everything
#   scripts/wifi-crypto-test.sh --selftest   # 1 and the confinement check's own selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
trap 'rc=$?; [ $rc -eq 0 ] || echo "WIFI-CRYPTO TEST FAILED (exit $rc)"' EXIT

CRATE_RE='\b(wlan_crypto|sha1|hmac|pbkdf2|aes|ccm|aes_kw|cmac|dbl)::|\bextern crate (wlan_crypto|sha1|hmac|pbkdf2|aes|ccm|aes_kw|cmac|dbl)\b|\buse (wlan_crypto|sha1|hmac|pbkdf2|aes|ccm|aes_kw|cmac|dbl)\b'
MANIFEST_RE='^\s*(wlan-crypto|sha1|pbkdf2|aes|ccm|aes-kw|cmac|dbl)\s*='

confined() { # confined <root> -> prints every violation, returns 1 if any
    local root="$1" bad=0 hits
    # Rust sources of the kernel and everything built from this tree, except the one file allowed
    hits="$(cd "$root" && grep -rHnE --include='*.rs' "$CRATE_RE" kernel userland tools 2> /dev/null \
            | grep -v '^kernel/src/wlan/crypto\.rs:')"
    [ -n "$hits" ] && { echo "  names the Wi-Fi crypto crates outside kernel/src/wlan/crypto.rs:"; echo "$hits" | sed 's/^/    /'; bad=1; }
    # manifests: only the kernel (the wlan-crypto line) and wlan-crypto itself
    hits="$(cd "$root" && find . -name Cargo.toml -not -path './vendor/*' -not -path './target/*' -not -path './.claude/*' \
            -not -path './wlan-crypto/Cargo.toml' -not -path './build/*' -print0 2> /dev/null \
            | xargs -0 grep -HnE "$MANIFEST_RE" 2> /dev/null | grep -v '^\./kernel/Cargo\.toml:[0-9]*:wlan-crypto = ')"
    [ -n "$hits" ] && { echo "  a manifest other than kernel/ and wlan-crypto/ depends on them:"; echo "$hits" | sed 's/^/    /'; bad=1; }
    return $bad
}

selftest() {
    local d bad=0
    python3 -I scripts/wifi-kat-parse.py --selftest || bad=1
    d="$(mktemp -d)"
    mkdir -p "$d/kernel/src/wlan" "$d/kernel/src/pc" "$d/userland" "$d/tools"
    echo 'pub fn f() { wlan_crypto::hmac_sha1(b"", &[]); }' > "$d/kernel/src/wlan/crypto.rs"
    printf '[dependencies]\nwlan-crypto = { path = "../wlan-crypto" }\n' > "$d/kernel/Cargo.toml"
    if confined "$d" > /dev/null; then echo "  ok   confinement: the allowed file and line pass"; else echo "  FAIL confinement: the allowed file flagged"; bad=1; fi
    echo 'use aes::Aes128;' > "$d/kernel/src/pc/sneak.rs"
    if confined "$d" > /dev/null; then echo "  FAIL confinement: aes used in pc/sneak.rs not flagged"; bad=1; else echo "  ok   confinement: a planted 'use aes' is flagged"; fi
    rm "$d/kernel/src/pc/sneak.rs"
    mkdir -p "$d/userland/app"; printf '[dependencies]\nsha1 = "0.10"\n' > "$d/userland/app/Cargo.toml"
    if confined "$d" > /dev/null; then echo "  FAIL confinement: a planted sha1 dependency not flagged"; bad=1; else echo "  ok   confinement: a planted sha1 dependency is flagged"; fi
    rm -rf "$d"
    return $bad
}

echo "  --- selftests ---"
selftest || exit 1
[ "${1:-}" = "--selftest" ] && { echo "WIFI-CRYPTO SELFTEST ONLY; no measurement taken"; trap - EXIT; exit 0; }

echo "  --- the committed vectors against a fresh download and parse ---"
scripts/wifi-kat-fetch.sh --check || exit 1

echo "  --- confinement (ADR 0007 row: only kernel/src/wlan/crypto.rs) ---"
confined . || exit 1
echo "  confined: only kernel/src/wlan/crypto.rs names the crates; only kernel/ and wlan-crypto/ manifests depend on them"

echo "  --- host tests (cargo test, wlan-crypto) ---"
HOST=$(rustc -vV | sed -n 's/host: //p')
cargo test --manifest-path wlan-crypto/Cargo.toml --target "$HOST" 2>&1 | tail -25
[ "${PIPESTATUS[0]}" -eq 0 ] || exit 1
trap - EXIT
echo "WIFI-CRYPTO TEST PASSED"
