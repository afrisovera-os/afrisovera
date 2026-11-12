#!/usr/bin/env bash
# PC track (7 Nov 2026): `fwinfo`: the firmware that started AfriSovera, READ ONLY.
#
# For the owner's real-PC try of the USB kit and before bootcopy/bootentry/bootnext: the firmware's vendor and
# revision, its UEFI version, whether its variable services answer, Secure Boot (SecureBoot) and Setup Mode
# (SetupMode), and one plain line for the owner (switch Secure Boot off first / the firmware can start
# AfriSovera from its own entry / start from the USB stick). Read once before boot services exit; nothing is
# written.
#
# Two power-ons, each with qemu-system-x86_64 DIRECTLY (edk2 code + a copy of its variable store):
#   Boot 1: the plain edk2 firmware (no Secure Boot built in): Secure Boot off or not reported, never on; the
#           owner's line matches it; vendor and revision agree with the kernel's own boot line.
#   Boot 2: edk2's Secure-Boot-capable firmware (SMM on) with an empty key store: Secure Boot reported OFF and
#           Setup Mode YES (no platform key): what a real PC with Secure Boot switched off and no keys says.
#   scripts/pc-fwinfo-test.sh              # grader selftest, then the two QEMU boots
#   scripts/pc-fwinfo-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

START='the firmware can start AfriSovera from its own boot entry \(bootplan, bootcopy, bootentry, bootnext\)'

common() { # common <cleaned serial text> <boot name>; prints FAIL lines
    local S="$1" b="$2" v1 v2
    has() { grep -aqE "$1" <<< "$S"; }
    has '^\[pc\] Secure Boot: (off|on|not reported by this firmware), kept for fwinfo$' ||
        echo "  FAIL: $b: at power-on the kernel did not keep the Secure Boot state"
    has '^fwinfo: the firmware that started AfriSovera, as read at power-on \(nothing is changed\)$' ||
        echo "  FAIL: $b: fwinfo did not answer"
    v1="$(printf '%s\n' "$S" | sed -nE 's/^\[pc\] firmware: (.+) \(revision (0x[0-9a-f]+)\)$/\1|\2/p' | head -1)"
    v2="$(printf '%s\n' "$S" | sed -nE 's/^  vendor: (.+) \(revision (0x[0-9a-f]+)\)$/\1|\2/p' | head -1)"
    if [ -z "$v2" ]; then echo "  FAIL: $b: fwinfo did not give the vendor and revision"
    elif [ "$v1" != "$v2" ]; then echo "  FAIL: $b: fwinfo's vendor ($v2) is not the one the kernel printed at boot ($v1)"; fi
    has '^  UEFI version: 2\.[0-9]+(\.[0-9])?$' || echo "  FAIL: $b: no UEFI 2.x version shown"
    has '^  variable services: working$' || echo "  FAIL: $b: the variable services were not found working"
    if has 'unexpected value|unclear Secure Boot|did not answer|not answering|^  Setup Mode: unknown'; then
        echo "  FAIL: $b: an unclear or failed variable read"; fi
    if has 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)'; then echo "  FAIL: $b: the kernel panicked"; fi
}

grade() { # grade <boot 1 log> <boot 2 log>
    local fail=0 S1 S2 out
    for f in "$1" "$2"; do [ -r "$f" ] || { echo "  FAIL: missing serial log $f"; return 1; }; done
    clean() { tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g'; }
    S1="$(clean "$1")"; S2="$(clean "$2")"
    out="$(common "$S1" "boot 1"; common "$S2" "boot 2")"
    [ -n "$out" ] && { printf '%s\n' "$out"; fail=1; }
    in1() { printf '%s\n' "$S1" | grep -aqE "$1"; }
    in2() { printf '%s\n' "$S2" | grep -aqE "$1"; }
    # boot 1: the plain firmware: off or not reported, the owner's line agreeing, both lines agreeing
    if in1 '^  Secure Boot: off$'; then
        in1 '^\[pc\] Secure Boot: off, kept for fwinfo$' || { echo "  FAIL: boot 1: fwinfo and the boot line disagree"; fail=1; }
        in1 "^  for the owner: Secure Boot is off( \\(keys are enrolled, so it can be switched back on later\\))?; $START\$" ||
            { echo "  FAIL: boot 1: Secure Boot off but the owner's line does not say AfriSovera can start"; fail=1; }
    elif in1 '^  Secure Boot: not reported by this firmware$'; then
        in1 '^\[pc\] Secure Boot: not reported by this firmware, kept for fwinfo$' || { echo "  FAIL: boot 1: fwinfo and the boot line disagree"; fail=1; }
        in1 "^  for the owner: this firmware has no Secure Boot to switch off; $START\$" ||
            { echo "  FAIL: boot 1: Secure Boot not reported but the owner's line does not say so"; fail=1; }
    else
        echo "  FAIL: boot 1: the plain firmware (no Secure Boot built in) was not read as off or not reported"; fail=1
    fi
    # boot 2: Secure-Boot-capable firmware, no keys: off, setup mode
    in2 '^\[pc\] Secure Boot: off, kept for fwinfo$' || { echo "  FAIL: boot 2: the boot line does not say Secure Boot off"; fail=1; }
    in2 '^  Secure Boot: off$' || { echo "  FAIL: boot 2: fwinfo does not say Secure Boot off"; fail=1; }
    in2 '^  Setup Mode: yes \(no platform key enrolled\)$' || { echo "  FAIL: boot 2: fwinfo does not say Setup Mode (no platform key)"; fail=1; }
    in2 "^  for the owner: Secure Boot is off; $START\$" || { echo "  FAIL: boot 2: the owner's line is not 'Secure Boot is off'"; fail=1; }
    return "$fail"
}

B1="[pc] firmware: EDK II (revision 0x10000)
[pc] Secure Boot: not reported by this firmware, kept for fwinfo
[pc] MANSA-PC BOOT OK
afrisovera> fwinfo
fwinfo: the firmware that started AfriSovera, as read at power-on (nothing is changed)
  vendor: EDK II (revision 0x10000)
  UEFI version: 2.7
  variable services: working
  Secure Boot: not reported by this firmware
  Setup Mode: not reported by this firmware
  for the owner: this firmware has no Secure Boot to switch off; the firmware can start AfriSovera from its own boot entry (bootplan, bootcopy, bootentry, bootnext)"
B2="[pc] firmware: EDK II (revision 0x10000)
[pc] Secure Boot: off, kept for fwinfo
[pc] MANSA-PC BOOT OK
afrisovera> fwinfo
fwinfo: the firmware that started AfriSovera, as read at power-on (nothing is changed)
  vendor: EDK II (revision 0x10000)
  UEFI version: 2.7
  variable services: working
  Secure Boot: off
  Setup Mode: yes (no platform key enrolled)
  for the owner: Secure Boot is off; the firmware can start AfriSovera from its own boot entry (bootplan, bootcopy, bootentry, bootnext)"
B1OFF="$(printf '%s\n' "$B2" | sed 's/^  Setup Mode: .*/  Setup Mode: no (a platform key is enrolled)/; s/Secure Boot is off;/Secure Boot is off (keys are enrolled, so it can be switched back on later);/')"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-fwinfo-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expected> <boot 1 text> <boot 2 text>
        local got
        printf '%s\n' "$3" > "$D/1"; printf '%s\n' "$4" > "$D/2"
        if grade "$D/1" "$D/2" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm shape-green green "$B1" "$B2"
    arm boot1-off green "$B2" "$B2"
    arm boot1-off-keys green "$B1OFF" "$B2"
    arm crlf green "$(printf '%s\n' "$B1" | sed 's/$/\r/')" "$(printf '%s\n' "$B2" | sed 's/$/\r/')"
    arm no-boot-line red "$(printf '%s\n' "$B1" | grep -v '^\[pc\] Secure Boot')" "$B2"
    arm no-fwinfo red "$B1" "$(printf '%s\n' "$B2" | sed -n '1,4p')"
    arm vendor-differs red "$(printf '%s\n' "$B1" | sed 's/^  vendor: EDK II/  vendor: EDK I/')" "$B2"
    arm revision-differs red "$B1" "$(printf '%s\n' "$B2" | sed 's/^  vendor: EDK II (revision 0x10000)/  vendor: EDK II (revision 0x10001)/')"
    arm no-version red "$(printf '%s\n' "$B1" | sed 's/^  UEFI version: .*/  UEFI version: 1.10/')" "$B2"
    arm vars-down red "$B1" "$(printf '%s\n' "$B2" | sed 's/^  variable services: working/  variable services: not answering/')"
    arm boot1-on red "$(printf '%s\n' "$B2" | sed 's/Secure Boot: off/Secure Boot: on/')" "$B2"
    arm boot1-lines-disagree red "$(printf '%s\n' "$B1" | sed 's/^\[pc\] Secure Boot: not reported by this firmware/[pc] Secure Boot: off/')" "$B2"
    arm boot1-wrong-verdict red "$(printf '%s\n' "$B1" | sed 's/^  for the owner: .*/  for the owner: Secure Boot is off; x/')" "$B2"
    arm boot2-not-reported red "$B1" "$B1"
    arm boot2-on red "$B1" "$(printf '%s\n' "$B2" | sed 's/Secure Boot: off/Secure Boot: on/')"
    arm boot2-user-mode red "$B1" "$B1OFF"
    arm boot2-unclear red "$B1" "$(printf '%s\n' "$B2"; echo '  for the owner: the firmware gave an unclear Secure Boot answer; look in its set-up screen before bootentry')"
    arm panic red "$B1" "$(printf '%s\nKERNEL PANIC\n' "$B2")"
    arm empty red "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 19 ]
}

echo "== PC firmware facts: vendor, UEFI version, Secure Boot (fwinfo, read only) =="
if ! selftest; then echo "PC FWINFO TEST FAILED (grader selftest); no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC FWINFO SELFTEST ONLY; no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC FWINFO TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
PY=python3; command -v python3 > /dev/null || PY=python
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC FWINFO TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"

# the firmwares: plain edk2 and edk2 with Secure Boot built in, each with its variable store file
REALQ="$(command -v qemu-system-x86_64)"
QDIR="$(dirname "$REALQ")"
OVMF=""; VARS=""; SOVMF=""; SVARS=""
for d in "$QDIR/../share/qemu" "$QDIR/share" /usr/share/qemu; do
    if [ -z "$OVMF" ] && [ -r "$d/edk2-x86_64-code.fd" ] && [ -r "$d/edk2-i386-vars.fd" ]; then
        OVMF="$d/edk2-x86_64-code.fd"; VARS="$d/edk2-i386-vars.fd"; fi
    if [ -z "$SOVMF" ] && [ -r "$d/edk2-x86_64-secure-code.fd" ] && [ -r "$d/edk2-i386-vars.fd" ]; then
        SOVMF="$d/edk2-x86_64-secure-code.fd"; SVARS="$d/edk2-i386-vars.fd"; fi
done
for f in /usr/share/OVMF/OVMF_CODE_4M.fd /usr/share/OVMF/OVMF_CODE.fd; do
    if [ -z "$OVMF" ] && [ -r "$f" ] && [ -r "${f/CODE/VARS}" ]; then OVMF="$f"; VARS="${f/CODE/VARS}"; fi
done
for f in /usr/share/OVMF/OVMF_CODE_4M.secboot.fd /usr/share/OVMF/OVMF_CODE.secboot.fd; do
    v="${f/CODE/VARS}"; v="${v/.secboot/}"
    if [ -z "$SOVMF" ] && [ -r "$f" ] && [ -r "$v" ]; then SOVMF="$f"; SVARS="$v"; fi
done
[ -n "$OVMF" ] || { echo "PC FWINFO TEST FAILED (harness: no plain x86-64 UEFI firmware with a variable store file)"; exit 1; }
[ -n "$SOVMF" ] || { echo "PC FWINFO TEST FAILED (harness: no Secure-Boot-capable x86-64 UEFI firmware with a variable store file)"; exit 1; }

W="$(mktemp -d "$T/pc-fwinfo.XXXXXX")"
cp "$VARS" "$W/vars1.fd"; cp "$SVARS" "$W/vars2.fd"
mkdir -p "$W/stick/EFI/BOOT"
cp "$PC_EFI" "$W/stick/EFI/BOOT/BOOTX64.EFI"

boot() { # boot <serial port> <log> <code> <vars> <secure: 0|1>
    local sp=$1 log=$2 code=$3 vars=$4 sec=$5 m=q35
    local -a extra=()
    [ "$sec" = 1 ] && { m=q35,smm=on; extra=(-global driver=cfi.pflash01,property=secure,value=on); }
    "$REALQ" -M "$m" -m 256M -display none "${extra[@]}" \
        -drive "if=pflash,format=raw,unit=0,readonly=on,file=$(winpath "$code")" \
        -drive "if=pflash,format=raw,unit=1,file=$(winpath "$vars")" \
        -drive "format=raw,file=fat:rw:$(winpath "$W/stick")" \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py "$sp" "$((sp+1))" "$W/shot.png" fwinfo > "$log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
}

echo "  --- boot 1: the plain edk2 firmware, fwinfo ---"
boot 5460 "$W/boot1.log" "$OVMF" "$W/vars1.fd" 0
echo "  --- boot 2: edk2 with Secure Boot built in (SMM), no keys enrolled, fwinfo ---"
boot 5464 "$W/boot2.log" "$SOVMF" "$W/vars2.fd" 1
for b in 1 2; do
    tr -d '\r' < "$W/boot$b.log" | grep -aE '^\[pc\] (firmware|Secure Boot)|^fwinfo|^  (vendor|UEFI|variable|Secure|Setup|for the owner)' |
        sed "s/^/  | boot $b: /"
done

echo "  --- grading ---"
if grade "$W/boot1.log" "$W/boot2.log"; then
    rm -rf "$W"
    echo "PC FWINFO TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC FWINFO TEST FAILED"
exit 1
