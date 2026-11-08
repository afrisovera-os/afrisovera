#!/usr/bin/env bash
# PC track step 5 (6 Nov 2026): `tablebackup` copies every internal disk's partition table to a USB stick
# (read back and CRC-checked; the internal disks are only READ) and `tablecheck` compares that copy, sector
# by sector, with the disk now -- the backup an install must have before it may ever change a table.
#
# Boot 1: the Windows-like GPT on NVMe, the full MBR disk on SATA port 2, the damaged GPT (backup header
# wiped) on SATA port 3 and a blank 8 MiB USB stick. `tablecheck` first finds no backup; `tablebackup`
# saves 67 sectors of the NVMe GPT (MBR, both headers, both entry arrays), 1 of each MBR disk and 34 of the
# damaged GPT (no backup header to copy). The harness then reads the stick image itself and checks the
# NVMe copy byte for byte against the NVMe image. Boot 2: the same disks, `tablecheck` says unchanged.
# Boot 3: the NVMe GPT with its first entry renamed, the stick's copy of SATA port 2 damaged, port 3 gone:
# CHANGED (first difference in sector 1, its header), damaged copy (data CRC), disk not here. Boot 4: no stick:
# nothing written. Every internal disk image is byte-identical after all four boots.
# Runs the same in an ARM walk and a PC walk: it builds the PC kernel and puts the PC shim
# (scripts/lib/pc-arch) on PATH itself.
#   scripts/pc-tablebackup-test.sh              # grader selftest, then the four QEMU boots
#   scripts/pc-tablebackup-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

grade() { # grade <boot1 log> <boot2 log> <boot3 log> <boot4 log>
    local fail=0 A B C D
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] && [ -r "$2" ] && [ -r "$3" ] && [ -r "$4" ] || { echo "  FAIL: missing log"; return 1; }
    cl() { tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g'; }
    A="$(cl "$1")"; B="$(cl "$2")"; C="$(cl "$3")"; D="$(cl "$4")"
    has() { printf '%s\n' "$1" | grep -aqxE "$2"; }
    blk() { printf '%s\n' "$1" | awk -v c="afrisovera> $2" '$0==c{on=1;next} /^afrisovera>/{on=0} on'; }
    local A0 A1 B1 C1 D0 D1 OK='read back OK'
    A0="$(blk "$A" 'tablecheck')"; A1="$(blk "$A" 'tablebackup')"; B1="$(blk "$B" 'tablecheck')"
    C1="$(blk "$C" 'tablecheck')"; D0="$(blk "$D" 'tablebackup')"; D1="$(blk "$D" 'tablecheck')"
    has "$A0" 'tablecheck: no partition-table backup on a USB stick \(run tablebackup\)' ||
        _f "boot 1: tablecheck on a blank stick did not say there is no backup"
    has "$A1" "tablebackup: nvme: GPT table, 67 sector\(s\) saved on the USB stick at sectors 2049-2116, CRC [0-9a-f]{8}, $OK" ||
        _f "boot 1: the NVMe GPT (both headers, both entry arrays: 67 sectors) was not saved in slot 1"
    has "$A1" "tablebackup: sata port 0: .*" || _f "boot 1: the boot disk (SATA port 0) was not looked at"
    has "$A1" "tablebackup: sata port 2: MBR table, 1 sector\(s\) saved on the USB stick at sectors 2569-2570, CRC [0-9a-f]{8}, $OK" ||
        _f "boot 1: the MBR disk on SATA port 2 was not saved in slot 3"
    has "$A1" "tablebackup: sata port 3: GPT \(backup header missing\) table, 34 sector\(s\) saved on the USB stick at sectors 2829-2863, CRC [0-9a-f]{8}, $OK" ||
        _f "boot 1: the damaged GPT on SATA port 3 was not saved as 34 sectors without its backup header"
    has "$A1" 'tablebackup: 4 table\(s\) saved on the USB stick; the internal disks were only read' ||
        _f "boot 1: tablebackup did not end with 4 tables saved"
    has "$A" 'harness: the stick.s NVMe copy matches the NVMe image byte for byte' ||
        _f "boot 1: the copy on the stick image does not match the NVMe image (or the check is missing)"
    has "$B1" 'tablecheck: nvme: unchanged since the backup \(67 sector\(s\)\)' || _f "boot 2: NVMe not unchanged"
    has "$B1" 'tablecheck: sata port 2: unchanged since the backup \(1 sector\(s\)\)' || _f "boot 2: SATA port 2 not unchanged"
    has "$B1" 'tablecheck: sata port 3: unchanged since the backup \(34 sector\(s\)\)' || _f "boot 2: SATA port 3 not unchanged"
    printf '%s\n' "$B1" | grep -aqE '(nvme|port [23]): (CHANGED|.*damaged|that disk)' && _f "boot 2: a disk that did not change was reported"
    has "$C1" 'tablecheck: nvme: CHANGED since the backup \(first difference in sector 1\) -- back it up again before any install' ||
        _f "boot 3: the renamed GPT entry (header CRC, sector 1) was not caught"
    has "$C1" 'tablecheck: sata port 2: the copy on the USB stick is damaged \(data CRC\)' ||
        _f "boot 3: the damaged copy of SATA port 2 was not caught"
    has "$C1" 'tablecheck: sata port 3: that disk is not here now' || _f "boot 3: the missing SATA port 3 was not noticed"
    has "$D0" "tablebackup: no USB stick to hold the backup \(plug in a blank stick or one of AfriSovera.s\) -- nothing written" ||
        _f "boot 4: tablebackup without a stick did not refuse"
    has "$D1" 'tablecheck: no partition-table backup on a USB stick \(run tablebackup\)' || _f "boot 4: tablecheck without a stick"
    for L in "$B1" "$C1"; do
        has "$L" 'tablecheck: read only -- nothing was written' || _f "a tablecheck did not finish"
    done
    for L in "$A" "$B" "$C" "$D"; do
        has "$L" 'harness: disk images unchanged' || _f "an internal disk image changed during a boot (or the check is missing)"
    done
    if printf '%s\n' "$A" "$B" "$C" "$D" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[(nvme|ahci|xhci)\] .*(failed|timed out)|FAILED'; then
        _f "the kernel panicked or a disk command failed"
    fi
    return "$fail"
}

OK='read back OK'
H='harness: disk images unchanged'
M1="afrisovera> tablecheck
tablecheck: no partition-table backup on a USB stick (run tablebackup)
afrisovera> tablebackup
tablebackup: nvme: GPT table, 67 sector(s) saved on the USB stick at sectors 2049-2116, CRC 1a2b3c4d, $OK
tablebackup: sata port 0: MBR table, 1 sector(s) saved on the USB stick at sectors 2309-2310, CRC 0badf00d, $OK
tablebackup: sata port 2: MBR table, 1 sector(s) saved on the USB stick at sectors 2569-2570, CRC 01234567, $OK
tablebackup: sata port 3: GPT (backup header missing) table, 34 sector(s) saved on the USB stick at sectors 2829-2863, CRC 89abcdef, $OK
tablebackup: 4 table(s) saved on the USB stick; the internal disks were only read
afrisovera>
$H
harness: the stick's NVMe copy matches the NVMe image byte for byte"
M2="afrisovera> tablecheck
tablecheck: nvme: unchanged since the backup (67 sector(s))
tablecheck: sata port 0: unchanged since the backup (1 sector(s))
tablecheck: sata port 2: unchanged since the backup (1 sector(s))
tablecheck: sata port 3: unchanged since the backup (34 sector(s))
tablecheck: read only -- nothing was written
afrisovera>
$H"
M3="afrisovera> tablecheck
tablecheck: nvme: CHANGED since the backup (first difference in sector 1) -- back it up again before any install
tablecheck: sata port 0: unchanged since the backup (1 sector(s))
tablecheck: sata port 2: the copy on the USB stick is damaged (data CRC)
tablecheck: sata port 3: that disk is not here now
tablecheck: read only -- nothing was written
afrisovera>
$H"
M4="afrisovera> tablebackup
tablebackup: no USB stick to hold the backup (plug in a blank stick or one of AfriSovera's) -- nothing written
afrisovera> tablecheck
tablecheck: no partition-table backup on a USB stick (run tablebackup)
afrisovera>
$H"

selftest() {
    local W pass=0 bad=0
    W="$(mktemp -d "$T/pc-tablebackup-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <b1> <b2> <b3> <b4>
        local got
        printf '%s\n' "$3" > "$W/a"; printf '%s\n' "$4" > "$W/b"; printf '%s\n' "$5" > "$W/c"; printf '%s\n' "$6" > "$W/d"
        if grade "$W/a" "$W/b" "$W/c" "$W/d" > "$W/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$W/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm model-green green "$M1" "$M2" "$M3" "$M4"
    arm crlf green "$(printf '%s\n' "$M1" | sed 's/$/\r/')" "$M2" "$(printf '%s\n' "$M3" | sed 's/$/\r/')" "$M4"
    arm no-check-first red "$(printf '%s\n' "$M1" | grep -v '^tablecheck: no partition')" "$M2" "$M3" "$M4"
    arm nvme-short red "$(printf '%s\n' "$M1" | sed 's/67 sector(s) saved/35 sector(s) saved/')" "$M2" "$M3" "$M4"
    arm nvme-not-read-back red "$(printf '%s\n' "$M1" | sed '0,/read back OK/s//read back FAILED/')" "$M2" "$M3" "$M4"
    arm damaged-not-saved red "$(printf '%s\n' "$M1" | sed 's/^tablebackup: sata port 3: .*/tablebackup: sata port 3: the GPT is damaged -- skipped/')" "$M2" "$M3" "$M4"
    arm copy-mismatch red "$(printf '%s\n' "$M1" | grep -v '^harness: the stick')" "$M2" "$M3" "$M4"
    arm false-change red "$M1" "$(printf '%s\n' "$M2" | sed 's/^tablecheck: sata port 2: unchanged.*/tablecheck: sata port 2: CHANGED since the backup (first difference in sector 0) -- back it up again before any install/')" "$M3" "$M4"
    arm change-missed red "$M1" "$M2" "$(printf '%s\n' "$M3" | sed 's/^tablecheck: nvme: CHANGED.*/tablecheck: nvme: unchanged since the backup (67 sector(s))/')" "$M4"
    arm wrong-sector red "$M1" "$M2" "$(printf '%s\n' "$M3" | sed 's/difference in sector 1/difference in sector 2/')" "$M4"
    arm bad-copy-missed red "$M1" "$M2" "$(printf '%s\n' "$M3" | sed 's/^tablecheck: sata port 2: .*/tablecheck: sata port 2: unchanged since the backup (1 sector(s))/')" "$M4"
    arm gone-missed red "$M1" "$M2" "$(printf '%s\n' "$M3" | grep -v 'not here now')" "$M4"
    arm no-stick-wrote red "$M1" "$M2" "$M3" "$(printf '%s\n' "$M4" | sed 's/^tablebackup: no USB stick.*/tablebackup: 1 table(s) saved on the USB stick; the internal disks were only read/')"
    arm image-changed red "$M1" "$M2" "$(printf '%s\n' "$M3" | sed 's/disk images unchanged/disk images CHANGED/')" "$M4"
    arm panic red "$(printf '%s\nKERNEL PANIC at gpt.rs' "$M1")" "$M2" "$M3" "$M4"
    arm empty red "" "" "" ""
    rm -rf "$W"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 16 ]
}

echo "== PC partition-table backup to a USB stick (PC track step 5) =="
if ! selftest; then echo "PC TABLEBACKUP TEST FAILED (grader selftest) -- no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC TABLEBACKUP SELFTEST ONLY -- no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC TABLEBACKUP TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
PY=python3; command -v python3 > /dev/null || PY=python

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC TABLEBACKUP TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-tablebackup.XXXXXX")"
for k in gpt gpt-damaged mbr; do
    "$PY" scripts/lib/make-disk-image.py "$k" "$W/$k.img" || { echo "PC TABLEBACKUP TEST FAILED (harness: could not make $k.img)"; exit 1; }
done
# the same GPT with its first entry renamed (sector 2), for boot 3. Both copies and every CRC are updated:
# a GPT whose primary entries no longer match their CRC is repaired from the backup by the UEFI firmware
# itself (OVMF did exactly that), so a one-sided change would not reach the kernel.
"$PY" - "$W/gpt.img" "$W/gpt-changed.img" << 'PYEOF'
import sys, struct, zlib
S = 512
b = bytearray(open(sys.argv[1], "rb").read())
n = len(b) // S
for e in (2, n - 33):
    b[e * S + 60] ^= 0x01
ents = bytes(b[2 * S:34 * S])
for h in (1, n - 1):
    o = h * S
    struct.pack_into("<I", b, o + 88, zlib.crc32(ents))
    struct.pack_into("<I", b, o + 16, 0)
    struct.pack_into("<I", b, o + 16, zlib.crc32(bytes(b[o:o + 92])))
open(sys.argv[2], "wb").write(b)
PYEOF
head -c 8388608 /dev/zero > "$W/stick.img" # a fresh, blank 8 MiB USB stick
(cd "$W" && md5sum gpt.img gpt-damaged.img mbr.img gpt-changed.img > before.md5)

STICK=(-drive "file=$W/stick.img,if=none,format=raw,id=st" -device qemu-xhci,id=xhci -device usb-storage,drive=st,bus=xhci.0)
boot() { # boot <serial port> <log> <nvme image> <commands (| between)> <extra qemu args...>
    local sp=$1 log=$2 img=$3 cmds=$4; shift 4
    local IFS='|'; set -f; local -a C=($cmds); set +f; unset IFS
    qemu-system-aarch64 -M virt -m 256M -display none \
        -drive "file=$img,if=none,format=raw,id=sd" -device nvme,serial=afri01,drive=sd "$@" \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py "$sp" "$((sp+1))" "$W/shot.png" "${C[@]}" > "$log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
    if (cd "$W" && md5sum -c --quiet before.md5 > /dev/null 2>&1); then printf '\nharness: disk images unchanged\n' >> "$log"
    else printf '\nharness: disk images CHANGED\n' >> "$log"; fi
}
SATA2=(-drive "file=$W/mbr.img,if=none,format=raw,id=sb" -device ide-hd,drive=sb,bus=ide.2)
SATA3=(-drive "file=$W/gpt-damaged.img,if=none,format=raw,id=sc" -device ide-hd,drive=sc,bus=ide.3)
echo "  --- boot 1: GPT on NVMe, MBR on SATA 2, damaged GPT on SATA 3, blank stick: tablecheck, tablebackup ---"
boot 5006 "$W/boot1.log" "$W/gpt.img" 'tablecheck|tablebackup' "${SATA2[@]}" "${SATA3[@]}" "${STICK[@]}"
# the harness's own check: slot 1 on the stick is the NVMe image's table sectors, byte for byte
"$PY" - "$W/stick.img" "$W/gpt.img" >> "$W/boot1.log" << 'PYEOF'
import sys, zlib, struct
st = open(sys.argv[1], "rb").read()
dk = open(sys.argv[2], "rb").read()
S = 512
n = len(dk) // S
sec = lambda b, k: b[k * S:(k + 1) * S]
want = [0, 1] + list(range(2, 34)) + list(range(n - 33, n - 1)) + [n - 1]
rec = sec(st, 2049)
ok = sec(st, 2048)[:8] == b"AFRIPTBK" and rec[:8] == b"AFRIPTB1" and rec[8:12] == b"nvme"
data = b"".join(sec(st, 2050 + i) for i in range(len(want)))
ok = ok and data == b"".join(sec(dk, k) for k in want)
ok = ok and struct.unpack_from("<I", rec, 64)[0] == zlib.crc32(data)
ok = ok and struct.unpack_from("<I", rec, 504)[0] == zlib.crc32(rec[:504])
print("\nharness: the stick's NVMe copy " + ("matches the NVMe image byte for byte" if ok else "DOES NOT match the NVMe image"))
PYEOF
echo "  --- boot 2: the same disks: tablecheck ---"
boot 5010 "$W/boot2.log" "$W/gpt.img" 'tablecheck' "${SATA2[@]}" "${SATA3[@]}" "${STICK[@]}"
# boot 3: the stick's copy of SATA port 2 (slot 3, data sector 2570) damaged by one bit
"$PY" - "$W/stick.img" << 'PYEOF'
import sys
f = open(sys.argv[1], "r+b"); f.seek(2570 * 512 + 446 + 4); b = f.read(1); f.seek(2570 * 512 + 446 + 4)
f.write(bytes([b[0] ^ 0x01])); f.close()
PYEOF
echo "  --- boot 3: a GPT entry renamed, the stick's SATA 2 copy damaged, SATA 3 gone: tablecheck ---"
boot 5014 "$W/boot3.log" "$W/gpt-changed.img" 'tablecheck' "${SATA2[@]}" "${STICK[@]}"
echo "  --- boot 4: no USB stick: tablebackup, tablecheck ---"
boot 5018 "$W/boot4.log" "$W/gpt.img" 'tablebackup|tablecheck' "${SATA2[@]}"
cat "$W/boot1.log" "$W/boot2.log" "$W/boot3.log" "$W/boot4.log" | tr -d '\r' |
    grep -aE '^afrisovera> table|^table(backup|check)|^harness' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot1.log" "$W/boot2.log" "$W/boot3.log" "$W/boot4.log"; then
    rm -rf "$W"
    echo "PC TABLEBACKUP TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC TABLEBACKUP TEST FAILED"
exit 1
