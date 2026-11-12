#!/usr/bin/env bash
# PC track step 5 (6 Nov 2026): `tablerestore <disk> [yes <code>]`: the way back from an install. It writes
# the partition table saved on the USB stick (tablebackup) back onto the disk it came from, when that disk
# has CHANGED since: only the table's own sectors that differ (the backup GPT copy first), only on the code
# it prints (tied to the saved copy and to the disk as it is now), read back until tablecheck says
# unchanged. And `tablebackup` no longer overwrites a saved table whose disk has changed since (that copy is
# the way back) unless the owner types `tablebackup new`.
#
# Boot 1: the Windows-like GPT on NVMe, an MBR disk on SATA port 2, a blank stick. tablerestore with no
# backup refuses; tablebackup; tablerestore on an unchanged disk, an unknown disk and without a disk write
# nothing. Then the harness adds a fifth GPT entry to the NVMe image (both copies, every CRC: what an
# install does). Boot 2: tablecheck says CHANGED; tablebackup refuses to overwrite; tablerestore nvme says
# 4 of 67 sectors differ and shows the code; a wrong code and the unchanged SATA disk write nothing; every
# image (and the stick's backup area) byte-identical. Boot 3: the code typed -> 4 sectors written, tablecheck unchanged,
# lsparts back to 4 partitions with both headers OK; the harness finds the NVMe image byte-identical to the
# original. Boot 4: the change made again, `tablebackup new` replaces the copy, tablecheck unchanged.
# Runs the same in an ARM walk and a PC walk: it builds the PC kernel and puts the PC shim
# (scripts/lib/pc-arch) on PATH itself.
#   scripts/pc-tablerestore-test.sh              # grader selftest, then the four QEMU boots
#   scripts/pc-tablerestore-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

grade() { # grade <boot1 log> <boot2 log> <boot3 log> <boot4 log>
    local fail=0 A B C D
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] && [ -r "$2" ] && [ -r "$3" ] && [ -r "$4" ] || { echo "  FAIL: missing log"; return 1; }
    cl() { tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g'; }
    A="$(cl "$1")"; B="$(cl "$2")"; C="$(cl "$3")"; D="$(cl "$4")"
    has() { grep -aqxE "$2" <<< "$1"; }
    blk() { printf '%s\n' "$1" | awk -v c="afrisovera> $2" '$0==c{on=1;next} /^afrisovera>/{on=0} on'; }
    local NW='nothing written' SAVED='tablebackup: nvme: GPT table, 67 sector\(s\) saved on the USB stick at sectors 2049-2116, CRC [0-9a-f]{8}, read back OK'
    local CODE
    CODE="$(printf '%s\n' "$B" | grep -aoE 'type exactly: tablerestore nvme yes [0-9a-f]{4}$' | head -1 | awk '{print $NF}')"
    [ -n "$CODE" ] || _f "boot 2: tablerestore nvme did not show the code to type"
    # boot 1: nothing to restore, in every way
    has "$(blk "$A" 'tablerestore nvme' | head -2)" "tablerestore: no partition-table backup on a USB stick, so nothing to restore from; $NW" ||
        _f "boot 1: tablerestore without a backup did not refuse"
    has "$(blk "$A" 'tablebackup')" "$SAVED" || _f "boot 1: tablebackup did not save the NVMe GPT"
    has "$A" "tablerestore: nvme: already the same as its backup, so nothing to restore; $NW" || _f "boot 1: an unchanged disk was not left alone"
    has "$(blk "$A" 'tablerestore sata9')" "tablerestore: no disk \"sata9\" here; say nvme or sataN \(see lsparts\); $NW" ||
        _f "boot 1: an unknown disk was not refused"
    has "$(blk "$A" 'tablerestore')" 'tablerestore: say the disk, .*' || _f "boot 1: no usage line without a disk"
    has "$A" 'harness: disk images unchanged' || _f "boot 1: a disk image changed"
    # boot 2: the disk changed; the old copy is kept; the code is shown, nothing written yet
    has "$(blk "$B" 'tablecheck')" 'tablecheck: nvme: CHANGED since the backup \(first difference in sector 1\); .*' ||
        _f "boot 2: tablecheck did not see the change"
    has "$(blk "$B" 'tablebackup')" "tablebackup: nvme: the USB stick holds this disk.s table from BEFORE it changed; that copy is the way back \(tablerestore\), so nothing was written\. To save the tables as they are now instead, type: tablebackup new" ||
        _f "boot 2: tablebackup did not refuse to overwrite the copy from before the change"
    printf '%s\n' "$(blk "$B" 'tablebackup')" | grep -aqE 'saved on the USB stick' && _f "boot 2: tablebackup overwrote the old copy"
    local B2; B2="$(blk "$B" 'tablerestore nvme')"
    has "$B2" 'tablerestore: nvme: CHANGED since the backup \(first difference in sector 1\)' || _f "boot 2: tablerestore did not report the change"
    has "$B2" "tablerestore: 4 of the table.s 67 sector\(s\) differ from the saved copy; .*" || _f "boot 2: not exactly 4 of 67 sectors differ"
    has "$(blk "$B" 'tablerestore nvme yes zzzz')" "tablerestore: that code does not fit this disk.s table now; $NW" || _f "boot 2: a wrong code was not refused"
    has "$(blk "$B" 'tablerestore sata2')" "tablerestore: sata port 2: already the same as its backup, so nothing to restore; $NW" ||
        _f "boot 2: the unchanged SATA disk was not left alone"
    has "$B" 'harness: disk images and the stick unchanged' || _f "boot 2: a disk image or the stick changed before the code was typed"
    printf '%s\n' "$A" "$B" | grep -aqE 'saved table is back' && _f "boot 1/2: something was restored without the code"
    # boot 3: the code typed -> the table as it was
    has "$(blk "$C" "tablerestore nvme yes $CODE")" "tablerestore: nvme: the saved table is back: 4 sector\(s\) written, read back OK, tablecheck now says unchanged; no partition.s own sectors were touched" ||
        _f "boot 3: the code did not put the saved table back (4 sectors)"
    has "$(blk "$C" 'tablecheck')" 'tablecheck: nvme: unchanged since the backup \(67 sector\(s\)\)' || _f "boot 3: tablecheck does not say unchanged"
    local C3; C3="$(blk "$C" 'lsparts')"
    has "$C3" '  gpt: 4 partition\(s\), usable LBA 34-131038, primary header OK, backup header OK, entries CRC OK' || _f "boot 3: the GPT is not whole with 4 partitions"
    printf '%s\n' "$C3" | grep -aqE '^  part 5:' && _f "boot 3: entry 5 is still listed"
    has "$C" 'harness: nvme image byte-identical to the original; other disk images unchanged' || _f "boot 3: the NVMe image is not exactly the original"
    # boot 4: tablebackup new replaces the copy
    has "$(blk "$D" 'tablebackup new')" "$SAVED" || _f "boot 4: tablebackup new did not save the changed table"
    has "$(blk "$D" 'tablecheck')" 'tablecheck: nvme: unchanged since the backup \(67 sector\(s\)\)' || _f "boot 4: tablecheck after tablebackup new"
    has "$D" "tablerestore: nvme: already the same as its backup, so nothing to restore; $NW" || _f "boot 4: tablerestore after tablebackup new"
    has "$D" 'harness: nvme image unchanged by the boot' || _f "boot 4: the NVMe image changed"
    if printf '%s\n' "$A" "$B" "$C" "$D" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[(nvme|ahci|xhci)\] .*(failed|timed out)|FAILED'; then
        _f "the kernel panicked or a disk command failed"
    fi
    return "$fail"
}

SV='tablebackup: nvme: GPT table, 67 sector(s) saved on the USB stick at sectors 2049-2116, CRC 1a2b3c4d, read back OK'
SAME='tablerestore: nvme: already the same as its backup, so nothing to restore; nothing written'
M1="afrisovera> tablerestore nvme
tablerestore: no partition-table backup on a USB stick, so nothing to restore from; nothing written
afrisovera> tablebackup
$SV
afrisovera> tablerestore nvme
$SAME
afrisovera> tablerestore sata9
tablerestore: no disk \"sata9\" here; say nvme or sataN (see lsparts); nothing written
afrisovera> tablerestore
tablerestore: say the disk, e.g. tablerestore nvme (it shows what would change and the code; nothing is written without the code)
afrisovera>
harness: disk images unchanged"
M2="afrisovera> tablecheck
tablecheck: nvme: CHANGED since the backup (first difference in sector 1); back it up again before any install
afrisovera> tablebackup
tablebackup: nvme: the USB stick holds this disk's table from BEFORE it changed; that copy is the way back (tablerestore), so nothing was written. To save the tables as they are now instead, type: tablebackup new
afrisovera> tablerestore nvme
tablerestore: nvme: CHANGED since the backup (first difference in sector 1)
tablerestore: 4 of the table's 67 sector(s) differ from the saved copy; a partition added since would leave the table (its sectors are not touched)
tablerestore: nothing written yet. To put the saved table back, type exactly: tablerestore nvme yes 5e7a
afrisovera> tablerestore nvme yes zzzz
tablerestore: nvme: CHANGED since the backup (first difference in sector 1)
tablerestore: that code does not fit this disk's table now; nothing written
afrisovera> tablerestore sata2
tablerestore: sata port 2: already the same as its backup, so nothing to restore; nothing written
afrisovera>
harness: disk images and the stick unchanged"
M3="afrisovera> tablerestore nvme yes 5e7a
tablerestore: nvme: CHANGED since the backup (first difference in sector 1)
tablerestore: nvme: the saved table is back: 4 sector(s) written, read back OK, tablecheck now says unchanged; no partition's own sectors were touched
afrisovera> tablecheck
tablecheck: nvme: unchanged since the backup (67 sector(s))
afrisovera> lsparts
  part 4: Windows recovery, LBA 118784-129023 (5 MiB), \"Recovery\"
  gpt: 4 partition(s), usable LBA 34-131038, primary header OK, backup header OK, entries CRC OK
afrisovera>
harness: nvme image byte-identical to the original; other disk images unchanged"
M4="afrisovera> tablebackup new
$SV
afrisovera> tablecheck
tablecheck: nvme: unchanged since the backup (67 sector(s))
afrisovera> tablerestore nvme
$SAME
afrisovera>
harness: nvme image unchanged by the boot"

selftest() {
    local W pass=0 bad=0
    W="$(mktemp -d "$T/pc-tablerestore-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <b1> <b2> <b3> <b4>
        local got
        printf '%s\n' "$3" > "$W/a"; printf '%s\n' "$4" > "$W/b"; printf '%s\n' "$5" > "$W/c"; printf '%s\n' "$6" > "$W/d"
        if grade "$W/a" "$W/b" "$W/c" "$W/d" > "$W/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$W/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm model-green green "$M1" "$M2" "$M3" "$M4"
    arm crlf green "$(printf '%s\n' "$M1" | sed 's/$/\r/')" "$(printf '%s\n' "$M2" | sed 's/$/\r/')" "$M3" "$M4"
    arm restore-without-backup red "$(printf '%s\n' "$M1" | sed '0,/^tablerestore: no partition-table backup.*/s//tablerestore: nvme: the saved table is back: 4 sector(s) written/')" "$M2" "$M3" "$M4"
    arm no-backup-saved red "$(printf '%s\n' "$M1" | grep -v '^tablebackup: nvme')" "$M2" "$M3" "$M4"
    arm unknown-disk-accepted red "$(printf '%s\n' "$M1" | grep -v 'no disk "sata9"')" "$M2" "$M3" "$M4"
    arm boot1-image-changed red "$(printf '%s\n' "$M1" | sed 's/disk images unchanged/disk images CHANGED/')" "$M2" "$M3" "$M4"
    arm change-unseen red "$M1" "$(printf '%s\n' "$M2" | sed 's/^tablecheck: nvme: CHANGED.*/tablecheck: nvme: unchanged since the backup (67 sector(s))/')" "$M3" "$M4"
    arm old-copy-overwritten red "$M1" "$(printf '%s\n' "$M2" | sed "s/^tablebackup: nvme: the USB stick holds.*/$SV/")" "$M3" "$M4"
    arm no-code-shown red "$M1" "$(printf '%s\n' "$M2" | grep -v 'type exactly')" "$M3" "$M4"
    arm wrong-count red "$M1" "$(printf '%s\n' "$M2" | sed 's/^tablerestore: 4 of/tablerestore: 9 of/')" "$M3" "$M4"
    arm wrong-code-accepted red "$M1" "$(printf '%s\n' "$M2" | sed 's/^tablerestore: that code does not fit.*/tablerestore: nvme: the saved table is back: 4 sector(s) written/')" "$M3" "$M4"
    arm sata-restored red "$M1" "$(printf '%s\n' "$M2" | grep -v '^tablerestore: sata port 2')" "$M3" "$M4"
    arm boot2-image-changed red "$M1" "$(printf '%s\n' "$M2" | sed 's/and the stick unchanged/or the stick CHANGED/')" "$M3" "$M4"
    arm code-mismatch red "$M1" "$M2" "$(printf '%s\n' "$M3" | sed 's/yes 5e7a/yes 0001/')" "$M4"
    arm not-restored red "$M1" "$M2" "$(printf '%s\n' "$M3" | grep -v 'saved table is back')" "$M4"
    arm still-changed red "$M1" "$M2" "$(printf '%s\n' "$M3" | sed 's/^tablecheck: nvme: unchanged.*/tablecheck: nvme: CHANGED since the backup (first difference in sector 3); x/')" "$M4"
    arm entry5-left red "$M1" "$M2" "$(printf '%s\n' "$M3" | sed 's/^  gpt: 4 partition/  part 5: Linux filesystem, LBA 100352-116735 (8 MiB), "Changed"\n  gpt: 4 partition/')" "$M4"
    arm gpt-broken red "$M1" "$M2" "$(printf '%s\n' "$M3" | sed 's/backup header OK/backup header MISSING\/DAMAGED/')" "$M4"
    arm image-not-original red "$M1" "$M2" "$(printf '%s\n' "$M3" | grep -v '^harness')" "$M4"
    arm new-not-saved red "$M1" "$M2" "$M3" "$(printf '%s\n' "$M4" | grep -v '^tablebackup: nvme')"
    arm boot4-image-changed red "$M1" "$M2" "$M3" "$(printf '%s\n' "$M4" | sed 's/unchanged by the boot/CHANGED by the boot/')"
    arm panic red "$M1" "$(printf '%s\nKERNEL PANIC at gpt.rs' "$M2")" "$M3" "$M4"
    arm empty red "" "" "" ""
    rm -rf "$W"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 23 ]
}

echo "== PC tablerestore: the saved partition table put back from the USB stick on the owner's code (PC track step 5) =="
if ! selftest; then echo "PC TABLERESTORE TEST FAILED (grader selftest); no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC TABLERESTORE SELFTEST ONLY; no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC TABLERESTORE TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
PY=python3; command -v python3 > /dev/null || PY=python

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC TABLERESTORE TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-tablerestore.XXXXXX")"
for k in gpt mbr-room; do
    "$PY" scripts/lib/make-disk-image.py "$k" "$W/$k.img" || { echo "PC TABLERESTORE TEST FAILED (harness: could not make $k.img)"; exit 1; }
done
cp "$W/gpt.img" "$W/gpt-orig.img"
head -c 8388608 /dev/zero > "$W/stick.img" # a fresh, blank 8 MiB USB stick
(cd "$W" && md5sum gpt.img mbr-room.img > before.md5 && md5sum mbr-room.img > other.md5)

# what an install does to the NVMe image: a fifth entry (Linux filesystem, LBA 100352-116735, "Changed") in
# both GPT copies with every CRC redone: a one-sided change would be repaired by the firmware itself
change() {
    "$PY" - "$W/gpt.img" << 'PYEOF'
import sys, struct, uuid, zlib
S = 512
p = sys.argv[1]
b = bytearray(open(p, "rb").read())
n = len(b) // S
e = bytearray(128)
e[:16] = uuid.UUID("0FC63DAF-8483-4772-8E79-3D69D8477DE4").bytes_le
e[16:32] = uuid.UUID("11111111-2222-4333-8444-555555555555").bytes_le
struct.pack_into("<QQ", e, 32, 100352, 116735)
e[56:56 + 14] = "Changed".encode("utf-16-le")
for a in (2, n - 33):
    b[a * S + 4 * 128:a * S + 5 * 128] = e
ents = bytes(b[2 * S:34 * S])
for h in (1, n - 1):
    o = h * S
    struct.pack_into("<I", b, o + 88, zlib.crc32(ents))
    struct.pack_into("<I", b, o + 16, 0)
    struct.pack_into("<I", b, o + 16, zlib.crc32(bytes(b[o:o + 92])))
open(p, "wb").write(b)
PYEOF
}

STICK=(-drive "file=$W/stick.img,if=none,format=raw,id=st" -device qemu-xhci,id=xhci -device usb-storage,drive=st,bus=xhci.0)
SATA2=(-drive "file=$W/mbr-room.img,if=none,format=raw,id=sb" -device ide-hd,drive=sb,bus=ide.2)
boot() { # boot <serial port> <log> <commands (| between)>
    local sp=$1 log=$2 cmds=$3
    local IFS='|'; set -f; local -a C=($cmds); set +f; unset IFS
    qemu-system-aarch64 -M virt -m 256M -display none \
        -drive "file=$W/gpt.img,if=none,format=raw,id=sd" -device nvme,serial=afri01,drive=sd "${SATA2[@]}" "${STICK[@]}" \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py "$sp" "$((sp+1))" "$W/shot.png" "${C[@]}" > "$log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
}
echo "  --- boot 1: no backup, tablebackup, an unchanged disk, an unknown disk, no disk ---"
boot 5034 "$W/boot1.log" 'tablerestore nvme|tablebackup|tablerestore nvme|tablerestore sata9|tablerestore'
if (cd "$W" && md5sum -c --quiet before.md5 > /dev/null 2>&1); then printf '\nharness: disk images unchanged\n' >> "$W/boot1.log"
else printf '\nharness: disk images CHANGED\n' >> "$W/boot1.log"; fi
change
(cd "$W" && md5sum gpt.img mbr-room.img > changed.md5); STK2="$(tail -c +1048577 "$W/stick.img" | md5sum)" # the stick from 1 MiB on: its files below are synced by every boot
echo "  --- boot 2: the disk changed: tablecheck, tablebackup kept back, the code, a wrong code, the SATA disk ---"
boot 5038 "$W/boot2.log" 'tablecheck|tablebackup|tablerestore nvme|tablerestore nvme yes zzzz|tablerestore sata2'
if (cd "$W" && md5sum -c --quiet changed.md5 > /dev/null 2>&1) && [ "$(tail -c +1048577 "$W/stick.img" | md5sum)" = "$STK2" ]; then printf '\nharness: disk images and the stick unchanged\n' >> "$W/boot2.log"
else printf '\nharness: disk images or the stick CHANGED\n' >> "$W/boot2.log"; fi
CODE="$(tr -d '\r' < "$W/boot2.log" | grep -aoE 'type exactly: tablerestore nvme yes [0-9a-f]{4}$' | head -1 | awk '{print $NF}')"
echo "  the code boot 2 showed: ${CODE:-NONE}"
echo "  --- boot 3: the code typed, tablecheck, lsparts ---"
boot 5042 "$W/boot3.log" "tablerestore nvme yes ${CODE:-none}|tablecheck|lsparts"
if cmp -s "$W/gpt.img" "$W/gpt-orig.img" && (cd "$W" && md5sum -c --quiet other.md5 > /dev/null 2>&1); then
    printf '\nharness: nvme image byte-identical to the original; other disk images unchanged\n' >> "$W/boot3.log"
else printf '\nharness: nvme image NOT the original (%s), or another disk image changed\n' "$(cmp "$W/gpt.img" "$W/gpt-orig.img" 2>&1 | head -1)" >> "$W/boot3.log"; fi
change
(cd "$W" && md5sum gpt.img > after.md5)
echo "  --- boot 4: the change made again: tablebackup new, tablecheck, tablerestore ---"
boot 5046 "$W/boot4.log" 'tablebackup new|tablecheck|tablerestore nvme'
if (cd "$W" && md5sum -c --quiet after.md5 > /dev/null 2>&1); then printf '\nharness: nvme image unchanged by the boot\n' >> "$W/boot4.log"
else printf '\nharness: nvme image CHANGED by the boot\n' >> "$W/boot4.log"; fi
cat "$W/boot1.log" "$W/boot2.log" "$W/boot3.log" "$W/boot4.log" | tr -d '\r' |
    grep -aE '^afrisovera> (table|lsparts)|^table(restore|backup|check)|^  part|^  gpt:|^harness' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot1.log" "$W/boot2.log" "$W/boot3.log" "$W/boot4.log"; then
    rm -rf "$W"
    echo "PC TABLERESTORE TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC TABLERESTORE TEST FAILED"
exit 1
