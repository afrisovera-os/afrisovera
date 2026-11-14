#!/usr/bin/env bash
# Device firmware files (13 Nov 2026; FIRMWARE_PLAN.md, the architect's verdict of 13 Nov): the loader reads a
# file from \EFI\afrisovera\firmware\ on an EFI system partition, checks its size and sha256 against fwdb.rs, and
# only then hands it to a device the row names. READ ONLY: the disk images must be byte-identical after the boot.
#
# 1. scripts/fw/fwtool.py check: every firmware/MANIFEST row has its licence text, the generated test file hashes
#    right, and fwdb.rs's table equals the MANIFEST. Then the same check on copies with a licence removed, a hash
#    changed in fwdb.rs and an unused licence file, which must each fail.
# 2. One QEMU boot: a blank GPT disk on NVMe, scripts/fw/make-fw-esp.py's disk on SATA port 2 (a real FAT32 EFI
#    partition with the test file and its damaged copies). `firmware` finds fwtest.bin there with its hash right;
#    `firmware test` sends the right file (to the test upload only), refuses a changed byte, a wrong size, a looping
#    chain, a chain that ends early, a cluster past the partition and a missing file, each with its line, refuses
#    the right file for a device its row does not name, and finds the buffer zero afterwards.
#   scripts/pc-firmware-test.sh              # grader selftest, the MANIFEST checks, then the QEMU boot
#   scripts/pc-firmware-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
PORT=7120
NCASES=9

grade() { # grade <boot log>
    local fail=0 A
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing log"; return 1; }
    A="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    blk() { printf '%s\n' "$1" | awk -v c="afrisovera> $2" '$0==c{on=1;next} /^afrisovera>/{on=0} on'; }
    in_blk() { grep -aqxF -e "$2" <<< "$1"; }
    local L F
    L="$(blk "$A" 'firmware')"; F="$(blk "$A" 'firmware test')"
    in_blk "$L" "firmware: 4 file(s) listed; each must match its size and sha256 before it may go to its device" ||
        _f "firmware did not say how many files are listed"
    in_blk "$L" "firmware: fwtest.bin (70000 bytes): present on sata port 2, sha256 as listed" ||
        _f "firmware did not find fwtest.bin on sata port 2 with its hash right"
    local c
    while IFS= read -r c; do
        in_blk "$F" "firmware test: $c (as expected)" || _f "firmware test: no line '$c (as expected)'"
    done << 'EOF'
fwtest.bin: sent, 70000 bytes
fwtest-byte.bin: refused: its sha256 is not the one listed
fwtest-size.bin: refused: it is 70001 bytes, not the size listed
fwtest-loop.bin: refused: its cluster chain loops back on itself
fwtest-short.bin: refused: its cluster chain ends before the file's size
fwtest-far.bin: refused: its cluster chain points outside the partition
fwtest-gone.bin: missing (it belongs in \EFI\afrisovera\firmware\)
fwtest.bin to a device its row does not name: refused: it is not for this device
the buffer is zero after every case
EOF
    in_blk "$F" "firmware test: $NCASES of $NCASES cases as expected" || _f "firmware test: not all $NCASES cases as expected"
    grep -aq 'NOT as expected' <<< "$F" && _f "firmware test: a case was NOT as expected"
    printf '%s\n' "$A" | grep -aqx 'harness: disk images unchanged' || _f "a disk image changed during the boot (or the check is missing)"
    printf '%s\n' "$A" | grep -aqx 'harness: fwtool check passed, and failed on each damaged copy' || _f "the MANIFEST checks did not pass (or are missing)"
    if printf '%s\n' "$A" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[(nvme|ahci)\] .*(failed|timed out)'; then
        _f "the kernel panicked or a disk command failed"
    fi
    return "$fail"
}

G="afrisovera> firmware
firmware: 4 file(s) listed; each must match its size and sha256 before it may go to its device
firmware: fwtest.bin (70000 bytes): present on sata port 2, sha256 as listed
afrisovera> firmware test
firmware test: fwtest.bin (the right file, in a cluster chain with a gap)
firmware test: fwtest.bin: sent, 70000 bytes (as expected)
firmware test: fwtest-byte.bin (one byte changed)
firmware test: fwtest-byte.bin: refused: its sha256 is not the one listed (as expected)
firmware test: fwtest-size.bin (one byte longer)
firmware test: fwtest-size.bin: refused: it is 70001 bytes, not the size listed (as expected)
firmware test: fwtest-loop.bin (a cluster chain that loops)
firmware test: fwtest-loop.bin: refused: its cluster chain loops back on itself (as expected)
firmware test: fwtest-short.bin (a cluster chain that ends early)
firmware test: fwtest-short.bin: refused: its cluster chain ends before the file's size (as expected)
firmware test: fwtest-far.bin (a cluster number past the partition)
firmware test: fwtest-far.bin: refused: its cluster chain points outside the partition (as expected)
firmware test: fwtest-gone.bin (not on the disk)
firmware test: fwtest-gone.bin: missing (it belongs in \\EFI\\afrisovera\\firmware\\) (as expected)
firmware test: fwtest.bin to a device its row does not name: refused: it is not for this device (as expected)
firmware test: the buffer is zero after every case (as expected)
firmware test: 9 of 9 cases as expected
afrisovera>
harness: disk images unchanged
harness: fwtool check passed, and failed on each damaged copy"

selftest() {
    local D pass=0 bad=0 n=0
    D="$(mktemp -d "$T/pc-firmware-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <log text>
        local got
        n=$((n+1))
        printf '%s\n' "$3" > "$D/a"
        if grade "$D/a" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm model-green green "$G"
    arm crlf green "$(printf '%s\n' "$G" | sed 's/$/\r/')"
    arm not-found red "$(printf '%s\n' "$G" | sed 's/present on sata port 2, sha256 as listed/missing (it belongs in \\EFI\\afrisovera\\firmware\\)/')"
    arm wrong-disk red "$(printf '%s\n' "$G" | sed 's/present on sata port 2/present on nvme/')"
    arm byte-sent red "$(printf '%s\n' "$G" | sed 's/^firmware test: fwtest-byte.bin: refused: its sha256 is not the one listed/firmware test: fwtest-byte.bin: sent, 70000 bytes/')"
    arm size-read red "$(printf '%s\n' "$G" | sed 's/it is 70001 bytes, not the size listed/its sha256 is not the one listed/')"
    arm loop-hashed red "$(printf '%s\n' "$G" | sed 's/^\(firmware test: fwtest-loop.bin: refused: \)its cluster chain loops back on itself/\1its sha256 is not the one listed/')"
    arm short-other red "$(printf '%s\n' "$G" | grep -v 'fwtest-short.bin: refused')"
    arm far-other red "$(printf '%s\n' "$G" | sed 's/^\(firmware test: fwtest-far.bin: refused: \)its cluster chain points outside the partition/\1its sha256 is not the one listed/')"
    arm gone-found red "$(printf '%s\n' "$G" | grep -v 'fwtest-gone.bin: missing')"
    arm any-device red "$(printf '%s\n' "$G" | sed 's/a device its row does not name: refused: it is not for this device/a device its row does not name: sent, 70000 bytes/')"
    arm buffer-dirty red "$(printf '%s\n' "$G" | sed 's/the buffer is zero after every case (as expected)/the buffer is zero after every case (NOT as expected)/')"
    arm count-short red "$(printf '%s\n' "$G" | sed 's/9 of 9 cases/8 of 9 cases/')"
    arm image-changed red "$(printf '%s\n' "$G" | sed 's/disk images unchanged/disk images CHANGED/')"
    arm manifest-red red "$(printf '%s\n' "$G" | grep -v 'fwtool check passed')"
    arm panic red "$(printf '%s\nKERNEL PANIC at esp.rs' "$G")"
    arm empty red ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq "$n" ] && [ "$n" -eq 17 ]
}

echo "== Device firmware files: read, size and sha256 checked, only to the device named, read only =="
if ! selftest; then echo "PC FIRMWARE TEST FAILED (grader selftest); no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC FIRMWARE SELFTEST ONLY; no measurement taken"; exit 0; }
PY=python3; command -v python3 > /dev/null || PY=python
W="$(mktemp -d "$T/pc-firmware.XXXXXX")"

echo "  --- the MANIFEST checks ---"
MAN=fail
if "$PY" scripts/fw/fwtool.py check; then
    MAN=ok
    cpy() { # a copy of the files fwtool reads, in the same layout, in $W/$1
        mkdir -p "$W/$1/scripts/fw" "$W/$1/kernel/src/pc" && cp scripts/fw/fwtool.py "$W/$1/scripts/fw/" &&
            cp -r firmware "$W/$1/" && cp kernel/src/pc/fwdb.rs "$W/$1/kernel/src/pc/"
    }
    cpy nolic && rm "$W/nolic/firmware/licences/LICENCE.fwtest"
    cpy hash && sed -i 's/hex("738d7966/hex("838d7966/' "$W/hash/kernel/src/pc/fwdb.rs"
    cpy spare && echo "a licence no row uses" > "$W/spare/firmware/licences/LICENCE.spare"
    for k in nolic hash spare; do
        if "$PY" "$W/$k/scripts/fw/fwtool.py" check > "$W/$k.log" 2>&1; then
            echo "  fwtool check passed on the damaged copy '$k'"; MAN=fail
        else
            echo "  fwtool check refused the damaged copy '$k': $(grep -m1 -v ' problem(s)$' "$W/$k.log")"
        fi
    done
fi

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC FIRMWARE TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC FIRMWARE TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

"$PY" scripts/lib/make-disk-image.py gpt "$W/gpt.img" || { echo "PC FIRMWARE TEST FAILED (harness: could not make gpt.img)"; exit 1; }
"$PY" scripts/fw/make-fw-esp.py "$W/fw.img" || { echo "PC FIRMWARE TEST FAILED (harness: could not make fw.img)"; exit 1; }
(cd "$W" && md5sum gpt.img fw.img > before.md5)

echo "  --- boot: a blank GPT disk on NVMe, the firmware test disk on SATA port 2: firmware, firmware test ---"
# the EFI partition sits on SATA, behind the boot stick, so the firmware's NvVars file is not written to it (as in
# pc-bootplan)
qemu-system-aarch64 -M virt -m 256M -display none \
    -drive "file=$W/gpt.img,if=none,format=raw,id=sd" -device "nvme,serial=afri01,drive=sd" \
    -drive "file=$W/fw.img,if=none,format=raw,id=sb" -device "ide-hd,drive=sb,bus=ide.2" \
    -serial "tcp:127.0.0.1:$PORT,server,nowait" -qmp "tcp:127.0.0.1:$((PORT+1)),server,nowait" &
q=$!
AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py "$PORT" "$((PORT+1))" "$W/shot.png" \
    "firmware" "firmware test" > "$W/boot.log" 2>&1
kill "$q" 2> /dev/null
wait "$q" 2> /dev/null
if (cd "$W" && md5sum -c --quiet before.md5 > /dev/null 2>&1); then printf '\nharness: disk images unchanged\n' >> "$W/boot.log"
else printf '\nharness: disk images CHANGED\n' >> "$W/boot.log"; fi
[ "$MAN" = ok ] && printf 'harness: fwtool check passed, and failed on each damaged copy\n' >> "$W/boot.log"
tr -d '\r' < "$W/boot.log" | grep -aE '^afrisovera> firmware|^firmware|^harness' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot.log"; then
    rm -rf "$W"
    echo "PC FIRMWARE TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC FIRMWARE TEST FAILED"
exit 1
