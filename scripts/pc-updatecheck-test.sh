#!/usr/bin/env bash
# Safe updates step U1 (AFR-0010, 13 Nov 2026): `updatecheck` reads update.afu from a FAT32 USB stick and checks
# it whole (magic, header, key, Ed25519 signature over the context and header, machine, version, length, the boot
# file's SHA-256 and that it is an x86-64 UEFI program). It writes NOTHING.
#
# One QEMU boot per case, each with its own stick image (scripts/make-update.py stick), a blank NVMe disk and the
# command `updatecheck`:
#   nostick   no stick at all                   -> no USB stick
#   good      v2, demo key, MBR stick           -> OK, plus the demo-key line
#   goodgpt   the same on a GPT stick           -> OK
#   nofile    a FAT32 stick without update.afu  -> no update.afu
#   badsig    one signature byte changed        -> signature does not match
#   bootbyte  one boot-file byte changed        -> SHA-256 does not match
#   short     the file cut by one byte          -> length does not match
#   old       version 1 (this system's own)     -> not newer, naming both versions
#   magic     wrong magic                       -> not an AfriSovera update file
#   target    signed for "arm64-phone"          -> another kind of machine
#   unknown   signed by a key not in the table  -> key not trusted for updates
#   notpe     signed, but the boot file is no x86-64 UEFI program -> refused
#   reserved  signed, a reserved byte not zero  -> refused
#   looproot  the root folder's cluster chain loops -> refused after 65,536 entries, no hang
#   block4k   the good file on an MBR stick with 4096-byte blocks -> refused, not read in the wrong place
# Every boot: the boot line names release 1 and the demo key, the stick and the disk are byte-identical afterwards,
# and there is no panic.
#   scripts/pc-updatecheck-test.sh              # grader selftest, then the QEMU boots
#   scripts/pc-updatecheck-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

CASES=(nostick good goodgpt nofile badsig bootbyte short old magic target unknown notpe reserved looproot block4k)
declare -A WANT=(
    [nostick]="updatecheck: no USB stick; put update.afu on a FAT32 stick and plug it in; nothing written"
    [good]="updatecheck: OK: version 2 is newer than this system \(version 1\); nothing written"
    [goodgpt]="updatecheck: OK: version 2 is newer than this system \(version 1\); nothing written"
    [nofile]="updatecheck: no update.afu in the USB stick's top folder; nothing written"
    [badsig]="updatecheck: REFUSED: its signature does not match its header; nothing written"
    [bootbyte]="updatecheck: REFUSED: its boot file does not match the SHA-256 in its header; nothing written"
    [short]="updatecheck: REFUSED: its length does not match its header \(cut short or extra bytes\); nothing written"
    [old]="updatecheck: REFUSED: it is not newer than this system; nothing written"
    [magic]="updatecheck: REFUSED: it is not an AfriSovera update file \(wrong magic\); nothing written"
    [target]="updatecheck: REFUSED: it is made for another kind of machine, not pc-x86_64; nothing written"
    [unknown]="updatecheck: REFUSED: it is signed by a key this system does not trust for updates; nothing written"
    [notpe]="updatecheck: REFUSED: its boot file is not an x86-64 UEFI program; nothing written"
    [reserved]="updatecheck: REFUSED: its reserved bytes are not zero; nothing written"
    [block4k]="updatecheck: the USB stick has no FAT32 file system AfriSovera can read \(a partition table on a stick whose blocks are not 512 bytes, not read yet\)"
    [looproot]="updatecheck: REFUSED: the root folder's cluster chain is damaged; nothing written"
)
BOOTLINE='\[pc\] updates: release 1; trusts the DEMO release key: a test build, not for release'
GOODINFO='updatecheck: update.afu: version 2 for pc-x86_64, boot file 64 KiB, SHA-256 [0-9a-f]{16}'
DEMOLINE='updatecheck: signed by the DEMO release key: a test build, never a real update'

grade() { # grade <dir with <case>.log files>
    local fail=0 c L n
    for c in "${CASES[@]}"; do
        [ -r "$1/$c.log" ] || { echo "  FAIL: $c: missing log"; fail=1; continue; }
        L="$(tr -d '\r' < "$1/$c.log" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
        n="$(grep -ac '^updatecheck: ' <<< "$L")"
        grep -aqxE "${WANT[$c]}" <<< "$L" || { echo "  FAIL: $c: no line: ${WANT[$c]}"; fail=1; }
        grep -aqE "$BOOTLINE" <<< "$L" || { echo "  FAIL: $c: no release/demo-key boot line"; fail=1; }
        grep -aqx 'harness: images unchanged' <<< "$L" || { echo "  FAIL: $c: the stick or the disk CHANGED"; fail=1; }
        grep -aqiE 'panic' <<< "$L" && { echo "  FAIL: $c: panic"; fail=1; }
        case "$c" in
            good|goodgpt)
                grep -aqxE "$GOODINFO" <<< "$L" || { echo "  FAIL: $c: no version/size/SHA line"; fail=1; }
                grep -aqx "$DEMOLINE" <<< "$L" || { echo "  FAIL: $c: no demo-key line"; fail=1; }
                [ "$n" -eq 3 ] || { echo "  FAIL: $c: $n updatecheck lines, expected 3"; fail=1; } ;;
            block4k)
                grep -aqx 'updatecheck: no update file read; nothing written' <<< "$L" || { echo "  FAIL: $c: no 'nothing written' line"; fail=1; }
                grep -aq 'updatecheck: OK' <<< "$L" && { echo "  FAIL: $c: said OK"; fail=1; }
                [ "$n" -eq 2 ] || { echo "  FAIL: $c: $n updatecheck lines, expected 2"; fail=1; } ;;
            old)
                grep -aqx 'updatecheck: it is version 1; this system is version 1' <<< "$L" || { echo "  FAIL: $c: no version line"; fail=1; }
                grep -aq 'updatecheck: OK' <<< "$L" && { echo "  FAIL: $c: said OK"; fail=1; }
                [ "$n" -eq 2 ] || { echo "  FAIL: $c: $n updatecheck lines, expected 2"; fail=1; } ;;
            *)
                grep -aq 'updatecheck: OK' <<< "$L" && { echo "  FAIL: $c: said OK"; fail=1; }
                [ "$n" -eq 1 ] || { echo "  FAIL: $c: $n updatecheck lines, expected 1"; fail=1; } ;;
        esac
    done
    return $fail
}

selftest() {
    local W pass=0 bad=0 c
    W="$(mktemp -d "$T/pc-updatecheck-st.XXXXXX")"
    good_logs() { # good_logs <dir>
        mkdir -p "$1"
        for c in "${CASES[@]}"; do
            {
                echo "[pc] updates: release 1; trusts the DEMO release key: a test build, not for release"
                echo "afrisovera> updatecheck"
                case "$c" in
                    good|goodgpt)
                        echo "updatecheck: update.afu: version 2 for pc-x86_64, boot file 64 KiB, SHA-256 0123456789abcdef"
                        echo "$DEMOLINE" ;;
                    old) echo "updatecheck: it is version 1; this system is version 1" ;;
                esac
                sed 's/\\//g' <<< "${WANT[$c]}"
                [ "$c" = block4k ] && echo "updatecheck: no update file read; nothing written"
                echo "harness: images unchanged"
            } > "$1/$c.log"
        done
    }
    arm() { # arm <name> <expect green|red> <sed on one case: case expr>
        local d="$W/$1"
        good_logs "$d"
        [ -n "$3" ] && sed -i "$4" "$d/$3.log"
        local got=red
        grade "$d" > "$d/out" 2>&1 && got=green
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$d/out"; fi
    }
    arm all-good green "" ""
    arm good-refused red good 's/^updatecheck: OK.*/updatecheck: REFUSED: its signature does not match its header; nothing written/'
    arm badsig-passed red badsig 's/^updatecheck: REFUSED.*/updatecheck: OK: version 2 is newer than this system (version 1); nothing written/'
    arm wrong-reason red bootbyte 's/does not match the SHA-256/does not match the hash/'
    arm no-demo-line red good '/DEMO release key: a test build, never/d'
    arm no-boot-line red old '/^\[pc\] updates/d'
    arm stick-changed red magic 's/images unchanged/images CHANGED/'
    arm panic red short '$a KERNEL PANIC at update.rs'
    arm extra-line red unknown '$a updatecheck: OK: version 2 is newer than this system (version 1); nothing written'
    arm downgrade-ok red old 's/^updatecheck: REFUSED.*/updatecheck: OK: version 1 is newer than this system (version 1); nothing written/'
    arm old-no-versions red old '/^updatecheck: it is version/d'
    arm block4k-read red block4k '/no update file read/d'
    arm missing-log red nostick 'd'
    rm -f "$W/missing-log/nostick.log"; grade "$W/missing-log" > /dev/null 2>&1 && { bad=$((bad+1)); echo "    arm missing-file: green  <-- GRADER BUG"; } || pass=$((pass+1))
    rm -rf "$W"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 14 ]
}

echo "== PC safe updates U1: updatecheck (AFR-0010) =="
if ! selftest; then echo "PC UPDATECHECK TEST FAILED (grader selftest); no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC UPDATECHECK SELFTEST ONLY; no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC UPDATECHECK TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
PY=python3; command -v python3 > /dev/null || PY=python

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC UPDATECHECK TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"
"$PY" scripts/make-update.py vectors > /dev/null || { echo "PC UPDATECHECK TEST FAILED (AFR-0010 vectors)"; exit 1; }

W="$(mktemp -d "$T/pc-updatecheck.XXXXXX")"
"$PY" scripts/lib/make-disk-image.py gpt "$W/disk.img" || { echo "PC UPDATECHECK TEST FAILED (harness: disk image)"; exit 1; }
# the update files and their sticks
"$PY" - "$W" << 'PYEOF' || { echo "PC UPDATECHECK TEST FAILED (harness: could not make the sticks)"; exit 1; }
import importlib.util, sys, os
W = sys.argv[1]
sp = importlib.util.spec_from_file_location("mu", "scripts/make-update.py")
mu = importlib.util.module_from_spec(sp); sp.loader.exec_module(mu)
boot = mu.stand_in_pe(64)
good = mu.update_file(boot, 2)
def flip(b, at):
    b = bytearray(b); b[at] ^= 0x01; return bytes(b)
notpe = bytearray(boot); notpe[0:2] = b"XZ"
files = {
    "good": (good, "mbr"),
    "goodgpt": (good, "gpt"),
    "nofile": (None, "mbr"),
    "badsig": (flip(good, 128 + 40), "mbr"),
    "bootbyte": (flip(good, 192 + 0x300), "mbr"),
    "short": (good[:-1], "mbr"),
    "old": (mu.update_file(boot, 1), "mbr"),
    "magic": (flip(good, 0), "mbr"),
    "target": (mu.update_file(boot, 2, target="arm64-phone"), "mbr"),
    "unknown": (mu.update_file(boot, 2, seed=bytes([0x77]) * 32), "mbr"),
    "notpe": (mu.update_file(bytes(notpe), 2), "mbr"),
    "reserved": (mu.update_file(boot, 2, reserved=b"\x01"), "mbr"),
}
for name, (data, table) in files.items():
    open(os.path.join(W, name + ".img"), "wb").write(mu.stick(data, table))
open(os.path.join(W, "block4k.img"), "wb").write(mu.stick(good, "mbr"))
open(os.path.join(W, "looproot.img"), "wb").write(mu.stick(None, "mbr", loop_root=True))
PYEOF

boot() { # boot <serial port> <case> <extra qemu args...>
    local sp=$1 c=$2; shift 2
    local log="$W/$c.log"
    (cd "$W" && md5sum disk.img *.img > "$c.md5")
    qemu-system-aarch64 -M virt -m 256M -display none \
        -drive "file=$W/disk.img,if=none,format=raw,id=sd" -device nvme,serial=afri01,drive=sd "$@" \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py "$sp" "$((sp+1))" "$W/shot.png" updatecheck > "$log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
    if (cd "$W" && md5sum -c --quiet "$c.md5" > /dev/null 2>&1); then printf '\nharness: images unchanged\n' >> "$log"
    else printf '\nharness: images CHANGED\n' >> "$log"; fi
}
port=5300
for c in "${CASES[@]}"; do
    echo "  --- boot: $c ---"
    if [ "$c" = nostick ]; then
        boot "$port" "$c"
    elif [ "$c" = block4k ]; then
        boot "$port" "$c" -drive "file=$W/$c.img,if=none,format=raw,id=st" -device qemu-xhci,id=xhci \
            -device usb-storage,drive=st,bus=xhci.0,logical_block_size=4096,physical_block_size=4096
    else
        boot "$port" "$c" -drive "file=$W/$c.img,if=none,format=raw,id=st" -device qemu-xhci,id=xhci -device usb-storage,drive=st,bus=xhci.0
    fi
    tr -d '\r' < "$W/$c.log" | grep -aE '^updatecheck|^\[pc\] updates|^harness' | sed 's/^/  | /'
    port=$((port + 4))
done

echo "  --- grading ---"
if grade "$W"; then
    rm -rf "$W"
    echo "PC UPDATECHECK TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC UPDATECHECK TEST FAILED"
exit 1
