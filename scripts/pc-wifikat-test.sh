#!/usr/bin/env bash
# pc-wifikat (W1 of the Wi-Fi plan, architect's verdict 14 Nov 2026): the Wi-Fi crypto inside the PC kernel.
#   1. The PC image has no SIMD and the Wi-Fi crypto has no cpuid (scripts/pc-nosimd-check.sh): the PC kernel saves
#      no SIMD state, so `aes` must be its software backend (aes_force_soft) and sha1 its soft one.
#   2. One QEMU boot (no Wi-Fi hardware is needed or used): `wifi selftest` runs every published vector in
#      kernel/src/wlan/crypto_kat.rs on the kernel's own build, and says how long one passphrase takes to become a
#      key (the architect asked for that number: a slow step must never look like a hang).
# Graded: the seven family lines with their exact counts (HMAC-SHA1 7, PBKDF2 5, passphrase 3, PRF 3, key wrap 1,
# CMAC 4, CCM 12), the "making keys" line before the two slow families, the summary "35 of 35 vectors PASS", no
# FAILED line, and the time for one passphrase. The grader's selftest feeds it a good log and five bad ones.
#   scripts/pc-wifikat-test.sh              # grader selftest, then the build, the disassembly and the QEMU boot
#   scripts/pc-wifikat-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

COUNTS='HMAC-SHA1 \(RFC 2202\): 7 of 7|PBKDF2-HMAC-SHA1 \(RFC 6070\): 5 of 5|passphrase to PMK \(802\.11 Annex J\.4\): 3 of 3|802\.11 PRF-SHA1 \(Annex J\.3\): 3 of 3|AES key wrap \(RFC 3394\): 1 of 1|AES-CMAC \(RFC 4493\): 4 of 4|AES-CCM, 8-byte tag \(RFC 3610\): 12 of 12'

grade() { # grade <serial log>
    local fail=0 S c
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { printf '%s\n' "$S" | grep -qE -e "$1"; }
    has '^wifi selftest: the Wi-Fi crypto against its published test vectors \(nothing is sent or changed\)' || _f "wifi selftest did not start"
    IFS='|' read -ra fam <<< "$COUNTS"
    for c in "${fam[@]}"; do
        has "^  $c PASS in [0-9]+ ms" || _f "no PASS line for: $c"
    done
    has '^  PBKDF2-HMAC-SHA1 \(RFC 6070\): making keys \(a few seconds\)' || _f "no 'making keys' line before PBKDF2"
    has '^  passphrase to PMK \(802\.11 Annex J\.4\): making keys \(a few seconds\)' || _f "no 'making keys' line before the passphrases"
    has '^  passphrase to PMK \(802\.11 Annex J\.4\): 3 of 3 PASS in [0-9]+ ms \([0-9]+ ms for one passphrase\)' || _f "no time for one passphrase"
    has '^wifi selftest: 35 of 35 vectors PASS; Wi-Fi may make keys$' || _f "no '35 of 35 vectors PASS' summary"
    has 'FAILED' && _f "a FAILED line was printed"
    has 'PANIC|panicked' && _f "the kernel panicked"
    if [ $fail -eq 0 ]; then
        echo "  graded: 7 families, 35 vectors PASS; one passphrase took $(printf '%s\n' "$S" | sed -n 's/.*(\([0-9]*\) ms for one passphrase).*/\1/p' | head -1) ms"
    fi
    return $fail
}

GOOD='afrisovera> wifi selftest
wifi selftest: the Wi-Fi crypto against its published test vectors (nothing is sent or changed)
  HMAC-SHA1 (RFC 2202): 7 of 7 PASS in 3 ms
  PBKDF2-HMAC-SHA1 (RFC 6070): making keys (a few seconds)
  PBKDF2-HMAC-SHA1 (RFC 6070): 5 of 5 PASS in 900 ms
  passphrase to PMK (802.11 Annex J.4): making keys (a few seconds)
  passphrase to PMK (802.11 Annex J.4): 3 of 3 PASS in 1800 ms (600 ms for one passphrase)
  802.11 PRF-SHA1 (Annex J.3): 3 of 3 PASS in 1 ms
  AES key wrap (RFC 3394): 1 of 1 PASS in 0 ms
  AES-CMAC (RFC 4493): 4 of 4 PASS in 1 ms
  AES-CCM, 8-byte tag (RFC 3610): 12 of 12 PASS in 4 ms
wifi selftest: 35 of 35 vectors PASS; Wi-Fi may make keys
afrisovera> '

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-wifikat-selftest.XXXXXX")" || return 1
    printf '%s' "$GOOD" > "$D/good"
    printf '%s' "$GOOD" | sed 's/12 of 12 PASS/11 of 12 PASS/' > "$D/count"
    printf '%s' "$GOOD" | sed 's/  AES-CMAC (RFC 4493): 4 of 4 PASS in 1 ms/  AES-CMAC (RFC 4493): vector 2 FAILED/' > "$D/failed"
    printf '%s' "$GOOD" | grep -v 'making keys' > "$D/nowarn"
    printf '%s' "$GOOD" | sed 's/35 of 35 vectors PASS; Wi-Fi may make keys/FAILED in 1 family; Wi-Fi will not make keys this boot/' > "$D/summary"
    printf '%s' "$GOOD" | sed 's/ (600 ms for one passphrase)//' > "$D/notime"
    echo "  --- grader selftest (no QEMU) ---"
    if grade "$D/good" > /dev/null; then pass=$((pass + 1)); else echo "  FAIL selftest: the good log was refused"; grade "$D/good"; bad=$((bad + 1)); fi
    for f in count failed nowarn summary notime; do
        if grade "$D/$f" > /dev/null; then echo "  FAIL selftest: the '$f' log was accepted"; bad=$((bad + 1)); else pass=$((pass + 1)); fi
    done
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ $bad -eq 0 ]
}

if ! selftest; then echo "PC WIFIKAT TEST FAILED (grader selftest); no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC WIFIKAT SELFTEST ONLY; no measurement taken"; exit 0; }

for d in "${QEMU_DIR:-}" "/c/Program Files/qemu" "$HOME/qemu/bin"; do
    [ -n "$d" ] || continue
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC WIFIKAT TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
PY=python3; command -v python3 > /dev/null || PY=python
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC WIFIKAT TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }

echo "  --- no SIMD in the image, no SIMD or cpuid in the Wi-Fi crypto ---"
NOSIMD_NEED_WLAN=1 scripts/pc-nosimd-check.sh "$PC_EFI" "$PWD/target/x86_64-unknown-uefi/debug/deps" || {
    echo "PC WIFIKAT TEST FAILED (SIMD or cpuid found)"; exit 1; }

export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
# PBKDF2 in a debug build under QEMU's software CPU: allow long silences while keys are made
export AFRISH_CMD_QUIET="${AFRISH_CMD_QUIET:-150}" AFRISH_CMD_CEILING="${AFRISH_CMD_CEILING:-200}"
REALQ="$(command -v qemu-system-x86_64)"
QDIR="$(dirname "$REALQ")"
OVMF=""; VARS=""
for d in "$QDIR/../share/qemu" "$QDIR/share" /usr/share/qemu; do
    if [ -z "$OVMF" ] && [ -r "$d/edk2-x86_64-code.fd" ] && [ -r "$d/edk2-i386-vars.fd" ]; then
        OVMF="$d/edk2-x86_64-code.fd"; VARS="$d/edk2-i386-vars.fd"; fi
done
for f in /usr/share/OVMF/OVMF_CODE_4M.fd /usr/share/OVMF/OVMF_CODE.fd; do
    if [ -z "$OVMF" ] && [ -r "$f" ] && [ -r "${f/CODE/VARS}" ]; then OVMF="$f"; VARS="${f/CODE/VARS}"; fi
done
[ -n "$OVMF" ] || { echo "PC WIFIKAT TEST FAILED (harness: no x86-64 UEFI firmware with a variable store file)"; exit 1; }

W="$(mktemp -d "$T/pc-wifikat.XXXXXX")"
cp "$VARS" "$W/vars.fd"
mkdir -p "$W/stick/EFI/BOOT"
cp "$PC_EFI" "$W/stick/EFI/BOOT/BOOTX64.EFI"

echo "  --- boot: no network, no Wi-Fi hardware; wifi selftest ---"
"$REALQ" -M q35 -m 256M -display none -nic none \
    -drive "if=pflash,format=raw,unit=0,readonly=on,file=$(winpath "$OVMF")" \
    -drive "if=pflash,format=raw,unit=1,file=$(winpath "$W/vars.fd")" \
    -drive "format=raw,file=fat:rw:$(winpath "$W/stick")" \
    -serial "tcp:127.0.0.1:5472,server,nowait" -qmp "tcp:127.0.0.1:5473,server,nowait" &
Q=$!
AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py 5472 5473 "$W/shot.png" \
    "wifi selftest" > "$W/boot.log" 2>&1
kill "$Q" 2> /dev/null
wait "$Q" 2> /dev/null
tr -d '\r' < "$W/boot.log" | grep -aE '^wifi selftest|^  [A-Z0-9a-z].*(PASS|FAILED|making keys)' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot.log"; then
    rm -rf "$W"
    echo "PC WIFIKAT TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC WIFIKAT TEST FAILED"
exit 1
