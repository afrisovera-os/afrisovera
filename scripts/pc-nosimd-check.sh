#!/usr/bin/env bash
# The PC kernel saves no SIMD state, so no SIMD or CPU-feature code may be in it (architect's condition on the Wi-Fi
# crypto, 14 Nov 2026). Two checks:
#   1. The whole PC image (mansa-pc.efi), disassembled: no instruction that touches an xmm, ymm or zmm register,
#      and no AES-NI, SHA-NI or PCLMULQDQ instruction anywhere.
#   2. The Wi-Fi crypto's own code, from the build's object files of wlan-crypto and every crate under it
#      (aes, sha1, hmac, pbkdf2, ccm, aes-kw, cmac, ctr, cipher, digest, dbl, cpufeatures, ...): no `cpuid` either,
#      so no run-time CPU-feature detection is compiled in (the kernel's own cpuid uses elsewhere are not counted).
# Usage: scripts/pc-nosimd-check.sh [EFI] [DEPS_DIR]
#   defaults: target/x86_64-unknown-uefi/debug/mansa-pc.efi and target/x86_64-unknown-uefi/debug/deps
#   scripts/pc-nosimd-check.sh --selftest    checks the matcher on small made-up listings
# Prints "NOSIMD CLEAN" or "NOSIMD FOUND" with each offending line (at most 40).
set -uo pipefail
cd "$(dirname "$0")/.."

SIMD_RE='%[xyz]mm[0-9]|\b(aesenc|aesenclast|aesdec|aesdeclast|aeskeygenassist|aesimc|sha1rnds4|sha1nexte|sha1msg1|sha1msg2|sha256rnds2|sha256msg1|sha256msg2|pclmulqdq|vaesenc|vpclmulqdq)\b'
CPUID_RE='\bcpuid\b'
# crates whose object code is the Wi-Fi crypto (the lib name as rustc writes it: dashes become underscores)
CRATES='wlan_crypto aes sha1 hmac pbkdf2 ccm aes_kw cmac ctr cipher digest dbl cpufeatures crypto_common block_buffer inout aead subtle generic_array typenum'

scan() { # scan <listing file> <regex> -> prints matching instruction lines
    grep -E "$2" "$1" | grep -E '^\s*[0-9a-f]+:' | head -40
}

selftest() {
    local d bad=0
    d="$(mktemp -d)"
    printf '  1000: 48 89 c3  movq %%rax, %%rbx\n  1003: 0f a2  cpuid\n' > "$d/clean"
    printf '  2000: 66 0f 38 dc c1  aesenc %%xmm1, %%xmm0\n' > "$d/aes"
    printf '  2000: 0f 28 c1  movaps %%xmm1, %%xmm0\n' > "$d/sse"
    printf '  2000: c5 fd 6f 06  vmovdqa (%%rsi), %%ymm0\n' > "$d/avx"
    printf '  3000: 0f a2  cpuid\n' > "$d/cpuid"
    [ -z "$(scan "$d/clean" "$SIMD_RE")" ] || { echo "  FAIL selftest: clean listing flagged"; bad=1; }
    [ -n "$(scan "$d/aes" "$SIMD_RE")" ] || { echo "  FAIL selftest: aesenc not flagged"; bad=1; }
    [ -n "$(scan "$d/sse" "$SIMD_RE")" ] || { echo "  FAIL selftest: an xmm move not flagged"; bad=1; }
    [ -n "$(scan "$d/avx" "$SIMD_RE")" ] || { echo "  FAIL selftest: a ymm move not flagged"; bad=1; }
    [ -n "$(scan "$d/cpuid" "$CPUID_RE")" ] || { echo "  FAIL selftest: cpuid not flagged"; bad=1; }
    rm -rf "$d"
    [ $bad -eq 0 ] && echo "  nosimd selftest: 5 of 5 ok"
    return $bad
}

if [ "${1:-}" = "--selftest" ]; then selftest; exit $?; fi
selftest || { echo "NOSIMD FOUND (the checker's own selftest failed)"; exit 1; }

EFI="${1:-target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
DEPS="${2:-target/x86_64-unknown-uefi/debug/deps}"
LT="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/host: //p')/bin"
OBJDUMP="$LT/llvm-objdump"
[ -x "$OBJDUMP" ] || OBJDUMP="$(command -v llvm-objdump || true)"
[ -n "$OBJDUMP" ] || { echo "NOSIMD FOUND (harness: no llvm-objdump; rustup component add llvm-tools)"; exit 1; }
[ -r "$EFI" ] || { echo "NOSIMD FOUND (harness: $EFI not built)"; exit 1; }
W="$(mktemp -d)"
found=0

"$OBJDUMP" -d --no-show-raw-insn "$EFI" > "$W/image.s" 2> "$W/image.err" || { echo "  objdump failed on $EFI"; cat "$W/image.err"; found=1; }
n=$(grep -cE '^\s*[0-9a-f]+:' "$W/image.s" || true)
echo "  image: $EFI, $n instructions disassembled"
[ "$n" -gt 10000 ] || { echo "  FAIL: too few instructions to trust the listing"; found=1; }
hits="$(scan "$W/image.s" "$SIMD_RE")"
if [ -n "$hits" ]; then
    echo "  SIMD or crypto-extension instructions in the PC image:"
    echo "$hits" | sed 's/^/    /'
    found=1
else
    echo "  image: no xmm/ymm/zmm register, no AES-NI, SHA-NI or PCLMULQDQ instruction"
fi

if [ -d "$DEPS" ]; then
    m=0
    for c in $CRATES; do
        for lib in "$DEPS"/lib"$c"-*.rlib; do
            [ -e "$lib" ] || continue
            m=$((m + 1))
            "$OBJDUMP" -d --no-show-raw-insn "$lib" > "$W/lib.s" 2> /dev/null || true
            h="$(scan "$W/lib.s" "$SIMD_RE|$CPUID_RE")"
            if [ -n "$h" ]; then
                echo "  $(basename "$lib"): SIMD or cpuid:"
                echo "$h" | sed 's/^/    /'
                found=1
            fi
        done
    done
    echo "  Wi-Fi crypto objects: $m crate libraries disassembled (no SIMD, no cpuid allowed)"
    if [ "${NOSIMD_NEED_WLAN:-0}" = 1 ] && ! ls "$DEPS"/libwlan_crypto-*.rlib > /dev/null 2>&1; then
        echo "  FAIL: no wlan_crypto library in $DEPS (the PC kernel was built without it?)"
        found=1
    fi
else
    echo "  no $DEPS: object check skipped"
fi
rm -rf "$W"
if [ $found -eq 0 ]; then echo "NOSIMD CLEAN"; exit 0; fi
echo "NOSIMD FOUND"
exit 1
