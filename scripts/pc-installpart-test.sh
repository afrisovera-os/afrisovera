#!/usr/bin/env bash
# PC track step 5 (6 Nov 2026): `installpart <disk> <MiB> [yes <code>]` -- the one command that writes an
# internal disk. It adds AfriSovera's (empty, unformatted) partition to a healthy GPT disk as one new entry
# in the free space `installplan` names, in both GPT copies with both CRCs redone, and only when the USB
# stick holds a backup of that disk's table that matches the disk now AND the owner types the code the
# plan prints (tied to the table as it is).
#
# Boot 1: the Windows-like GPT on NVMe (9 MiB free between "Basic data" and "Recovery"), an MBR disk with
# room on SATA port 2, a blank stick. Refused, nothing written: no backup yet (installpart nvme 4); after
# tablebackup: 16 MiB does not fit, the MBR disk is not changed, a wrong code; `installpart nvme 8` shows
# the code. Every disk image byte-identical. Boot 2: the code typed -> entry 5, LBA 100352-116735;
# lsparts shows it with both headers and the entries CRC OK; tablecheck says CHANGED; the same command
# again finds no room. The harness checks the NVMe image itself: only sectors 1, 3, 131040, 131071 changed,
# both copies valid and equal, entry 5 = AfriSovera's type, a random version-4 GUID, the name. Boot 3:
# a fresh boot (the UEFI firmware reads the table first) still lists entry 5 and leaves the image alone.
# Runs the same in an ARM walk and a PC walk: it builds the PC kernel and puts the PC shim
# (scripts/lib/pc-arch) on PATH itself.
#   scripts/pc-installpart-test.sh              # grader selftest, then the three QEMU boots
#   scripts/pc-installpart-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

grade() { # grade <boot1 log> <boot2 log> <boot3 log>
    local fail=0 A B C
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] && [ -r "$2" ] && [ -r "$3" ] || { echo "  FAIL: missing log"; return 1; }
    cl() { tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g'; }
    A="$(cl "$1")"; B="$(cl "$2")"; C="$(cl "$3")"
    has() { printf '%s\n' "$1" | grep -aqxE "$2"; }
    blk() { printf '%s\n' "$1" | awk -v c="afrisovera> $2" '$0==c{on=1;next} /^afrisovera>/{on=0} on'; }
    local NW='only a healthy GPT disk with room in a free entry can be changed \(see the plan above\)'
    local CODE
    CODE="$(printf '%s\n' "$A" | grep -aoE 'type exactly: installpart nvme 8 yes [0-9a-f]{4}$' | head -1 | awk '{print $NF}')"
    [ -n "$CODE" ] || _f "boot 1: installpart nvme 8 did not show the code to type"
    has "$(blk "$A" 'installpart nvme 4')" 'installpart: there is no partition-table backup on a USB stick -- run tablebackup first; nothing written' ||
        _f "boot 1: installpart without a backup did not refuse"
    has "$(blk "$A" 'tablebackup')" 'tablebackup: nvme: GPT table, 67 sector\(s\) saved on the USB stick at sectors 2049-2116, CRC [0-9a-f]{8}, read back OK' ||
        _f "boot 1: tablebackup did not save the NVMe GPT"
    local A16; A16="$(blk "$A" 'installpart nvme 16')"
    has "$A16" '  plan: the largest free space holds 9 MiB, less than the 16 MiB asked -- .*' && has "$A16" "installpart: nothing written -- $NW" ||
        _f "boot 1: 16 MiB in a 9 MiB gap was not refused"
    has "$(blk "$A" 'installpart sata2 8')" "installpart: nothing written -- $NW" || _f "boot 1: the MBR disk was not refused"
    local A8; A8="$(blk "$A" 'installpart nvme 8')"
    has "$A8" "  plan: AfriSovera's partition would be LBA 100352-116735 \(8 MiB\) in the free space LBA 100352-118783, as GPT entry 5; no other partition moves or shrinks" ||
        _f "boot 1: the plan for 8 MiB is not entry 5 at LBA 100352-116735"
    has "$A8" 'installpart: the table matches its backup on the USB stick' || _f "boot 1: the backup was not matched"
    has "$(blk "$A" 'installpart nvme 8 yes zzzz')" 'installpart: that code does not fit this disk.s table now -- nothing written' ||
        _f "boot 1: a wrong code was not refused"
    has "$(blk "$A" 'installpart nvme')" 'installpart: say the disk and the size in MiB, .*' || _f "boot 1: no usage line without a size"
    has "$A" 'harness: disk images unchanged' || _f "boot 1: a disk image changed before the code was typed"
    printf '%s\n' "$A" | grep -aqE 'partition written' && _f "boot 1: something was written"
    local B1 B2 B3 B4
    B1="$(blk "$B" "installpart nvme 8 yes $CODE")"
    has "$B1" "installpart: AfriSovera's partition written as GPT entry 5, LBA 100352-116735 \(8 MiB\), in both GPT copies, read back OK; no other partition was changed" ||
        _f "boot 2: the code did not write entry 5 (or it was not read back)"
    B2="$(blk "$B" 'lsparts')"
    has "$B2" '  part 5: AfriSovera, LBA 100352-116735 \(8 MiB\), "AfriSovera"' || _f "boot 2: lsparts does not list AfriSovera's partition"
    has "$B2" '  gpt: 5 partition\(s\), usable LBA 34-131038, primary header OK, backup header OK, entries CRC OK' ||
        _f "boot 2: the GPT is not whole after the write"
    B3="$(blk "$B" 'tablecheck')"
    has "$B3" 'tablecheck: nvme: CHANGED since the backup \(first difference in sector 1\) -- .*' || _f "boot 2: tablecheck did not see the change"
    B4="$(printf '%s\n' "$B" | awk -v c="afrisovera> installpart nvme 8 yes $CODE" '$0==c{n++} n==2&&$0!=c&&/^afrisovera>/{exit} n==2&&$0!=c')"
    has "$B4" "installpart: nothing written -- $NW" || _f "boot 2: the same install twice was not refused"
    has "$B" 'harness: nvme image: only sectors 1, 3, 131040, 131071 changed; both GPT copies valid and equal; entry 5 is AfriSovera.s, LBA 100352-116735, a random GUID' ||
        _f "boot 2: the NVMe image does not hold exactly the planned change"
    has "$B" 'harness: other disk images unchanged' || _f "boot 2: another disk image changed"
    has "$(blk "$C" 'lsparts')" '  part 5: AfriSovera, LBA 100352-116735 \(8 MiB\), "AfriSovera"' || _f "boot 3: entry 5 is gone after a fresh boot"
    has "$C" 'harness: nvme image unchanged by the boot' || _f "boot 3: the firmware or kernel changed the disk on a fresh boot"
    if printf '%s\n' "$A" "$B" "$C" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[(nvme|ahci|xhci)\] .*(failed|timed out)|FAILED'; then
        _f "the kernel panicked or a disk command failed"
    fi
    return "$fail"
}

NW='only a healthy GPT disk with room in a free entry can be changed (see the plan above)'
PL="  plan: AfriSovera's partition would be LBA 100352-116735 (8 MiB) in the free space LBA 100352-118783, as GPT entry 5; no other partition moves or shrinks"
M1="afrisovera> installpart nvme 4
installpart: nvme, 64 MiB, AfriSovera's partition of 4 MiB
installpart: there is no partition-table backup on a USB stick -- run tablebackup first; nothing written
afrisovera> tablebackup
tablebackup: nvme: GPT table, 67 sector(s) saved on the USB stick at sectors 2049-2116, CRC 1a2b3c4d, read back OK
afrisovera> installpart nvme 16
  plan: the largest free space holds 9 MiB, less than the 16 MiB asked -- make room by shrinking Windows from Windows itself (Disk Management, Shrink Volume)
installpart: nothing written -- $NW
afrisovera> installpart sata2 8
  plan: AfriSovera's partition would be LBA 34816-51199 (8 MiB) in the free space LBA 34816-131071, as MBR slot 2; no other partition moves or shrinks
installpart: nothing written -- $NW
afrisovera> installpart nvme 8
$PL
installpart: the table matches its backup on the USB stick
installpart: nothing written yet. To write it, type exactly: installpart nvme 8 yes 3f9c
afrisovera> installpart nvme 8 yes zzzz
installpart: the table matches its backup on the USB stick
installpart: that code does not fit this disk's table now -- nothing written
afrisovera> installpart nvme
installpart: say the disk and the size in MiB, e.g. installpart nvme 8192 (it shows the plan and the code; nothing is written without the code)
afrisovera>
harness: disk images unchanged"
M2="afrisovera> installpart nvme 8 yes 3f9c
$PL
installpart: the table matches its backup on the USB stick
installpart: AfriSovera's partition written as GPT entry 5, LBA 100352-116735 (8 MiB), in both GPT copies, read back OK; no other partition was changed
installpart: the partition is empty (not formatted yet); the table as it was stays on the USB stick, so tablecheck now says CHANGED
afrisovera> lsparts
  part 5: AfriSovera, LBA 100352-116735 (8 MiB), \"AfriSovera\"
  gpt: 5 partition(s), usable LBA 34-131038, primary header OK, backup header OK, entries CRC OK
afrisovera> tablecheck
tablecheck: nvme: CHANGED since the backup (first difference in sector 1) -- back it up again before any install
afrisovera> installpart nvme 8 yes 3f9c
  plan: the largest free space holds 1 MiB, less than the 8 MiB asked -- make room
installpart: nothing written -- $NW
afrisovera>
harness: nvme image: only sectors 1, 3, 131040, 131071 changed; both GPT copies valid and equal; entry 5 is AfriSovera's, LBA 100352-116735, a random GUID
harness: other disk images unchanged"
M3="afrisovera> lsparts
  part 5: AfriSovera, LBA 100352-116735 (8 MiB), \"AfriSovera\"
afrisovera>
harness: nvme image unchanged by the boot"

selftest() {
    local W pass=0 bad=0
    W="$(mktemp -d "$T/pc-installpart-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <b1> <b2> <b3>
        local got
        printf '%s\n' "$3" > "$W/a"; printf '%s\n' "$4" > "$W/b"; printf '%s\n' "$5" > "$W/c"
        if grade "$W/a" "$W/b" "$W/c" > "$W/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$W/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm model-green green "$M1" "$M2" "$M3"
    arm crlf green "$(printf '%s\n' "$M1" | sed 's/$/\r/')" "$(printf '%s\n' "$M2" | sed 's/$/\r/')" "$M3"
    arm no-code-shown red "$(printf '%s\n' "$M1" | grep -v 'type exactly')" "$M2" "$M3"
    arm wrote-without-backup red "$(printf '%s\n' "$M1" | sed 's/^installpart: there is no partition-table backup.*/installpart: the table matches its backup on the USB stick/')" "$M2" "$M3"
    arm too-big-accepted red "$(printf '%s\n' "$M1" | sed '/^afrisovera> installpart nvme 16$/,/^afrisovera> installpart sata2/{/nothing written/d}')" "$M2" "$M3"
    arm mbr-accepted red "$(printf '%s\n' "$M1" | sed '/^afrisovera> installpart sata2 8$/,/^afrisovera> installpart nvme 8$/{/nothing written/d}')" "$M2" "$M3"
    arm wrong-code-accepted red "$(printf '%s\n' "$M1" | sed 's/^installpart: that code does not fit.*/installpart: AfriSovera'"'"'s partition written as GPT entry 5/')" "$M2" "$M3"
    arm boot1-image-changed red "$(printf '%s\n' "$M1" | sed 's/disk images unchanged/disk images CHANGED/')" "$M2" "$M3"
    arm code-mismatch red "$M1" "$(printf '%s\n' "$M2" | sed 's/yes 3f9c/yes 0001/')" "$M3"
    arm not-written red "$M1" "$(printf '%s\n' "$M2" | grep -v 'partition written as GPT entry 5')" "$M3"
    arm wrong-place red "$M1" "$(printf '%s\n' "$M2" | sed 's/written as GPT entry 5, LBA 100352/written as GPT entry 5, LBA 100353/')" "$M3"
    arm gpt-broken red "$M1" "$(printf '%s\n' "$M2" | sed 's/backup header OK, entries CRC OK/backup header MISSING\/DAMAGED, entries CRC OK/')" "$M3"
    arm change-unseen red "$M1" "$(printf '%s\n' "$M2" | sed 's/^tablecheck: nvme: CHANGED.*/tablecheck: nvme: unchanged since the backup (67 sector(s))/')" "$M3"
    arm twice-written red "$M1" "$(printf '%s\n' "$M2" | awk '/^afrisovera> installpart nvme 8 yes/{n++} n==2&&/nothing written/{print "installpart: AfriSovera'"'"'s partition written as GPT entry 6";next} {print}')" "$M3"
    arm image-wrong red "$M1" "$(printf '%s\n' "$M2" | grep -v '^harness: nvme image')" "$M3"
    arm other-disk-written red "$M1" "$(printf '%s\n' "$M2" | grep -v '^harness: other disk')" "$M3"
    arm lost-on-reboot red "$M1" "$M2" "$(printf '%s\n' "$M3" | grep -v 'part 5')"
    arm reboot-changed red "$M1" "$M2" "$(printf '%s\n' "$M3" | sed 's/unchanged by the boot/CHANGED by the boot/')"
    arm panic red "$(printf '%s\nKERNEL PANIC at gpt.rs' "$M1")" "$M2" "$M3"
    arm empty red "" "" ""
    rm -rf "$W"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 20 ]
}

echo "== PC install: AfriSovera's partition written into a GPT on the owner's code (PC track step 5) =="
if ! selftest; then echo "PC INSTALLPART TEST FAILED (grader selftest) -- no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC INSTALLPART SELFTEST ONLY -- no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC INSTALLPART TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
PY=python3; command -v python3 > /dev/null || PY=python

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC INSTALLPART TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-installpart.XXXXXX")"
for k in gpt mbr-room; do
    "$PY" scripts/lib/make-disk-image.py "$k" "$W/$k.img" || { echo "PC INSTALLPART TEST FAILED (harness: could not make $k.img)"; exit 1; }
done
cp "$W/gpt.img" "$W/gpt-orig.img"
head -c 8388608 /dev/zero > "$W/stick.img" # a fresh, blank 8 MiB USB stick
(cd "$W" && md5sum gpt.img mbr-room.img > before.md5 && md5sum mbr-room.img > other.md5)

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
echo "  --- boot 1: no backup, tablebackup, too big, MBR, the plan + code, a wrong code ---"
boot 5022 "$W/boot1.log" 'installpart nvme 4|tablebackup|installpart nvme 16|installpart sata2 8|installpart nvme 8|installpart nvme 8 yes zzzz|installpart nvme'
if (cd "$W" && md5sum -c --quiet before.md5 > /dev/null 2>&1); then printf '\nharness: disk images unchanged\n' >> "$W/boot1.log"
else printf '\nharness: disk images CHANGED\n' >> "$W/boot1.log"; fi
CODE="$(tr -d '\r' < "$W/boot1.log" | grep -aoE 'type exactly: installpart nvme 8 yes [0-9a-f]{4}$' | head -1 | awk '{print $NF}')"
echo "  the code boot 1 showed: ${CODE:-NONE}"
echo "  --- boot 2: the code typed, lsparts, tablecheck, the same again ---"
boot 5026 "$W/boot2.log" "installpart nvme 8 yes ${CODE:-none}|lsparts|tablecheck|installpart nvme 8 yes ${CODE:-none}"
# the harness's own check of the NVMe image: exactly the planned change, both copies whole
"$PY" - "$W/gpt-orig.img" "$W/gpt.img" >> "$W/boot2.log" << 'PYEOF'
import sys, struct, uuid, zlib
S = 512
a = open(sys.argv[1], "rb").read()
b = open(sys.argv[2], "rb").read()
n = len(b) // S
diff = [k for k in range(n) if a[k * S:(k + 1) * S] != b[k * S:(k + 1) * S]]
def hdr(lba):
    h = bytearray(b[lba * S:lba * S + 92])
    want = struct.unpack_from("<I", h, 16)[0]
    struct.pack_into("<I", h, 16, 0)
    # (entry array LBA at 72, entries CRC at 88)
    return h[:8] == b"EFI PART" and zlib.crc32(bytes(h)) == want, (struct.unpack_from("<Q", h, 72)[0], struct.unpack_from("<I", h, 88)[0])
ok1, (e1, c1) = hdr(1)
ok2, (e2, c2) = hdr(n - 1)
arr1, arr2 = b[e1 * S:(e1 + 32) * S], b[e2 * S:(e2 + 32) * S]
ok = diff == [1, 3, n - 32, n - 1] and ok1 and ok2 and arr1 == arr2 and zlib.crc32(arr1) == c1 == c2
e = arr1[4 * 128:5 * 128]
ok = ok and e[:16] == uuid.UUID("A5F1C0DE-0B5E-4A50-9AF1-50FE2A000001").bytes_le
g = uuid.UUID(bytes_le=bytes(e[16:32]))
ok = ok and g.version == 4 and g.variant == uuid.RFC_4122
ok = ok and struct.unpack_from("<QQQ", e, 32) == (100352, 116735, 0)
ok = ok and e[56:56 + 20].decode("utf-16-le") == "AfriSovera" and e[76:128] == bytes(52)
ok = ok and arr1[:4 * 128] == a[2 * S:2 * S + 4 * 128] and arr1[5 * 128:] == bytes(len(arr1) - 5 * 128)
if ok:
    print("\nharness: nvme image: only sectors 1, 3, %d, %d changed; both GPT copies valid and equal; entry 5 is AfriSovera's, LBA 100352-116735, a random GUID" % (n - 32, n - 1))
else:
    print("\nharness: nvme image NOT as planned: changed sectors %s, headers %s/%s" % (diff[:10], ok1, ok2))
PYEOF
if (cd "$W" && md5sum -c --quiet other.md5 > /dev/null 2>&1); then printf 'harness: other disk images unchanged\n' >> "$W/boot2.log"
else printf 'harness: other disk images CHANGED\n' >> "$W/boot2.log"; fi
(cd "$W" && md5sum gpt.img > after.md5)
echo "  --- boot 3: a fresh boot: lsparts ---"
boot 5030 "$W/boot3.log" 'lsparts'
if (cd "$W" && md5sum -c --quiet after.md5 > /dev/null 2>&1); then printf '\nharness: nvme image unchanged by the boot\n' >> "$W/boot3.log"
else printf '\nharness: nvme image CHANGED by the boot\n' >> "$W/boot3.log"; fi
cat "$W/boot1.log" "$W/boot2.log" "$W/boot3.log" | tr -d '\r' |
    grep -aE '^afrisovera> (installpart|table|lsparts)|^installpart|^tablecheck: nvme|^  part 5|^  gpt:|^  plan|^harness' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot1.log" "$W/boot2.log" "$W/boot3.log"; then
    rm -rf "$W"
    echo "PC INSTALLPART TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC INSTALLPART TEST FAILED"
exit 1
