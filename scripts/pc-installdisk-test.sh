#!/usr/bin/env bash
# PC track step 5 (6 Nov 2026): installdisk -- a whole internal disk given to AfriSovera (a
# spare PC), which then starts AfriSovera with NO USB stick.
#
# A 1 GiB NVMe disk that looks used (an MBR with an NTFS partition, as a Windows PC's disk
# would). Boot 1, from the USB-stick layout the shim gives every PC suite: lsparts, then
# `installdisk nvme` must show what is on the disk, say EVERYTHING is erased, give a code,
# and write nothing; a wrong code writes nothing either (the harness compares the image).
# Boot 2: the code typed -> a fresh GPT (an EFI system partition holding EFI\BOOT\BOOTX64.EFI
# + AfriSovera's partition, formatted), a file written. The harness then reads the image
# itself: protective MBR, both GPT headers, the two entries, and the FAT32 file system's
# EFI\BOOT\BOOTX64.EFI byte-identical to the kernel that was booted.
# Boot 3: qemu-system-x86_64 DIRECTLY, with only the NVMe disk -- no stick, no boot disk:
# the firmware must start AfriSovera from the NVMe disk, and the file must come back.
#   scripts/pc-installdisk-test.sh              # grader selftest, then the three QEMU boots
#   scripts/pc-installdisk-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
# INSTALLDISK_KIND=sata (scripts/pc-installdisk-sata-test.sh): the same on a SATA disk on AHCI port 2
# INSTALLDISK_KIND=emmc (scripts/pc-installdisk-emmc-test.sh): the same on a laptop's built-in eMMC (SDHCI);
#   QEMU's UEFI firmware has no SD/MMC driver, so boot 3 starts from the stick layout with the eMMC attached
#   and grades that the driver finds AfriSovera's partition and the file on it (real laptops' firmware boots eMMC)
KIND="${INSTALLDISK_KIND:-nvme}"
BOOT3_LINE='harness: boot 3: only the installed disk attached (no USB stick, no boot disk)'
BOOT3_RE='^harness: boot 3: only the installed disk attached \(no USB stick, no boot disk\)$'
if [ "$KIND" = sata ]; then
    CMD=sata2; NAME="sata port 2"; DRV='\[ahci\] port 2:'; DEV=(-device ide-hd,drive=sd,bus=ide.2); PORT=5170; UP=SATA
elif [ "$KIND" = emmc ]; then
    CMD=emmc; NAME=emmc; DRV='\[sdhci\] eMMC:'; DEV=(-device sdhci-pci -device emmc,drive=sd); PORT=5270; UP=eMMC
    BOOT3_LINE='harness: boot 3: the installed eMMC attached, started from the stick layout (QEMU firmware has no eMMC driver)'
    BOOT3_RE='^harness: boot 3: the installed eMMC attached, started from the stick layout \(QEMU firmware has no eMMC driver\)$'
else
    CMD=nvme; NAME=nvme; DRV='\[nvme\]'; DEV=(-device nvme,serial=afri01,drive=sd); PORT=5070; UP=NVMe
fi

grade() { # grade <boot1 log> <boot2 log> <boot3 log>
    local fail=0 A B C
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] && [ -r "$2" ] && [ -r "$3" ] || { echo "  FAIL: missing log"; return 1; }
    A="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    B="$(tr -d '\r' < "$2" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    C="$(tr -d '\r' < "$3" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    hasA() { printf '%s\n' "$A" | grep -aqE "$1"; }
    hasB() { printf '%s\n' "$B" | grep -aqE "$1"; }
    hasC() { printf '%s\n' "$C" | grep -aqE "$1"; }
    hasA '^\[pc\] own boot file .*BOOTX64\.EFI \([0-9]+ KiB\) kept for installdisk$' ||
        _f "boot 1: the kernel did not keep its own boot file"
    hasA '^installdisk: EVERYTHING on this disk will be ERASED' || _f "boot 1: installdisk did not warn that everything is erased"
    hasA '^  mbr 1: type 0x07 ' || _f "boot 1: installdisk did not show what is on the disk now (the NTFS partition)"
    hasA "^installdisk: nothing written yet\\. Only on a PC that is AfriSovera.s alone, type exactly: installdisk $CMD yes [0-9a-f]{4}\$" ||
        _f "boot 1: installdisk did not give the code"
    hasA '^installdisk: that code does not fit this disk as it is now -- nothing written$' || _f "boot 1: a wrong code was not refused"
    hasA '^harness: boot 1: the disk image is unchanged$' || _f "boot 1: the disk was written before the code was typed"
    hasB "^installdisk: $NAME: GPT written \\(EFI system partition LBA 2048-616447, AfriSovera.s partition LBA 616448-[0-9]+\\), boot file [0-9]+ KiB written and read back OK" ||
        _f "boot 2: the table and boot file were not written and read back"
    hasB '^installdisk: AfriSovera.s partition formatted \([0-9]+ MiB\); the shell.s files are kept there from now on$' ||
        _f "boot 2: AfriSovera's partition was not formatted and used"
    hasB '^harness: boot 2: GPT ok \(protective MBR, both headers, 2 entries\), FAT32 EFI\\BOOT\\BOOTX64\.EFI is the booted kernel byte for byte$' ||
        _f "boot 2: the harness's own read of the disk image did not find a valid GPT + FAT32 boot file"
    hasC "$BOOT3_RE" || _f "boot 3: the harness did not boot from the installed disk alone"
    hasC '^\[pc\] MANSA-PC BOOT OK$' || _f "boot 3: AfriSovera did not start from the installed disk"
    hasC "^$DRV AfriSovera's partition at sector 616448 \\([0-9]+ MiB\\) holds our files\$" ||
        _f "boot 3: the driver did not find AfriSovera's partition with our files"
    hasC '^HELLO-INSTALL-6NOV$' || _f "boot 3: notes.txt did not come back from the installed disk"
    if printf '%s\n%s\n%s\n' "$A" "$B" "$C" | grep -aqE '^installdisk: .*FAILED'; then
        _f "installdisk failed: $(printf '%s\n%s\n' "$A" "$B" | grep -aE '^installdisk: .*FAILED' | head -1)"
    fi
    if printf '%s\n%s\n%s\n' "$A" "$B" "$C" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)'; then
        _f "the kernel panicked or took an exception"
    fi
    return "$fail"
}

B1="[pc] own boot file \\EFI\\BOOT\\BOOTX64.EFI (2205 KiB) kept for installdisk
afrisovera> installdisk nvme
installdisk: nvme, 1024 MiB, 512-byte blocks. What is on it now:
  mbr 1: type 0x07 (NTFS/exFAT), start 2048, 2095104 sectors (1023 MiB)
installdisk: EVERYTHING on this disk will be ERASED -- Windows or anything else on it is lost for good.
installdisk: nothing written yet. Only on a PC that is AfriSovera's alone, type exactly: installdisk nvme yes 3fa1
afrisovera> installdisk nvme yes zzzz
installdisk: that code does not fit this disk as it is now -- nothing written

harness: boot 1: the disk image is unchanged"
B2="afrisovera> installdisk nvme yes 3fa1
installdisk: nvme: GPT written (EFI system partition LBA 2048-616447, AfriSovera's partition LBA 616448-2095103), boot file 2205 KiB written and read back OK (14500 sectors)
installdisk: AfriSovera's partition formatted (722 MiB); the shell's files are kept there from now on
installdisk: DONE -- take the USB stick out and restart (reboot): the PC starts AfriSovera from this disk

harness: boot 2: GPT ok (protective MBR, both headers, 2 entries), FAT32 EFI\\BOOT\\BOOTX64.EFI is the booted kernel byte for byte"
B3="$BOOT3_LINE
[pc] MANSA-PC BOOT OK
[nvme] AfriSovera's partition at sector 616448 (722 MiB) holds our files
afrisovera> cat notes.txt
HELLO-INSTALL-6NOV"
if [ "$KIND" = sata ]; then # the same samples as the SATA disk on port 2 says them
    SATA_SED='s/installdisk nvme/installdisk sata2/g; s/installdisk: nvme/installdisk: sata port 2/g; s/^\[nvme\] AfriSovera/[ahci] port 2: AfriSovera/'
    B1="$(printf '%s\n' "$B1" | sed "$SATA_SED")"
    B2="$(printf '%s\n' "$B2" | sed "$SATA_SED")"
    B3="$(printf '%s\n' "$B3" | sed "$SATA_SED")"
elif [ "$KIND" = emmc ]; then # the same samples as the eMMC says them
    EMMC_SED='s/installdisk nvme/installdisk emmc/g; s/installdisk: nvme/installdisk: emmc/g; s/^\[nvme\] AfriSovera/[sdhci] eMMC: AfriSovera/'
    B1="$(printf '%s\n' "$B1" | sed "$EMMC_SED")"
    B2="$(printf '%s\n' "$B2" | sed "$EMMC_SED")"
    B3="$(printf '%s\n' "$B3" | sed "$EMMC_SED")"
fi

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-installdisk-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <b1> <b2> <b3>
        local got
        printf '%s\n' "$3" > "$D/a"; printf '%s\n' "$4" > "$D/b"; printf '%s\n' "$5" > "$D/c"
        if grade "$D/a" "$D/b" "$D/c" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm real-shape green "$B1" "$B2" "$B3"
    arm crlf green "$(printf '%s\n' "$B1" | sed 's/$/\r/')" "$(printf '%s\n' "$B2" | sed 's/$/\r/')" "$(printf '%s\n' "$B3" | sed 's/$/\r/')"
    arm no-own-file red "$(printf '%s\n' "$B1" | grep -v 'own boot file')" "$B2" "$B3"
    arm no-warning red "$(printf '%s\n' "$B1" | grep -v ERASED)" "$B2" "$B3"
    arm written-early red "$(printf '%s\n' "$B1" | sed 's/image is unchanged/image CHANGED/')" "$B2" "$B3"
    arm wrong-code-taken red "$(printf '%s\n' "$B1" | grep -v 'does not fit')" "$B2" "$B3"
    arm not-written red "$B1" "$(printf '%s\n' "$B2" | grep -v 'GPT written')" "$B3"
    arm harness-bad-fat red "$B1" "$(printf '%s\n' "$B2" | sed 's/is the booted kernel byte for byte/differs from the booted kernel/')" "$B3"
    arm booted-with-stick red "$B1" "$B2" "$(printf '%s\n' "$B3" | grep -v '^harness: boot 3')"
    arm no-disk-boot red "$B1" "$B2" "$(printf '%s\n' "$B3" | grep -v 'BOOT OK')"
    arm file-lost red "$B1" "$B2" "$(printf '%s\n' "$B3" | grep -v '^HELLO')"
    arm failed red "$B1" "$(printf '%s\ninstalldisk: writing the partition table FAILED after 9 sectors' "$B2")" "$B3"
    arm panic red "$B1" "$B2" "$(printf '%s\nKERNEL PANIC' "$B3")"
    arm empty red "" "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 14 ]
}

echo "== PC install ($UP): a whole disk given to AfriSovera, booted with no USB stick (PC track step 5) =="
if ! selftest; then echo "PC INSTALLDISK TEST FAILED (grader selftest) -- no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC INSTALLDISK SELFTEST ONLY -- no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC INSTALLDISK TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
PY=python3; command -v python3 > /dev/null || PY=python

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC INSTALLDISK TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
REALQ="$(command -v qemu-system-x86_64)"
PATH="$PWD/scripts/lib/pc-arch:$PATH"
QDIR="$(dirname "$REALQ")"
OVMF="${PC_OVMF:-}"
for f in "$QDIR/../share/qemu/edk2-x86_64-code.fd" "$QDIR/share/edk2-x86_64-code.fd" /usr/share/qemu/edk2-x86_64-code.fd \
         /usr/share/OVMF/OVMF_CODE.fd /usr/share/OVMF/OVMF_CODE_4M.fd /usr/share/ovmf/OVMF.fd; do
    [ -z "$OVMF" ] && [ -r "$f" ] && OVMF="$f"
done
[ -n "$OVMF" ] || { echo "PC INSTALLDISK TEST FAILED (harness: no x86-64 UEFI firmware)"; exit 1; }
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

W="$(mktemp -d "$T/pc-installdisk.XXXXXX")"
IMG="$W/disk.img"
# a 1 GiB disk that looks used: an MBR with one NTFS partition from sector 2048, an NTFS boot sector there
"$PY" - "$IMG" << 'PYEOF'
import sys, struct
f = open(sys.argv[1], "wb")
f.truncate(1 << 30)
m = bytearray(512)
m[446:462] = bytes([0x80, 0, 0, 0, 0x07, 0, 0, 0]) + struct.pack("<II", 2048, (1 << 21) - 2048)
m[510:512] = b"\x55\xaa"
f.seek(0); f.write(m)
n = bytearray(512); n[0:11] = b"\xebR\x90NTFS    "; n[510:512] = b"\x55\xaa"
f.seek(2048 * 512); f.write(n)
f.close()
PYEOF
md5() { "$PY" -c "import hashlib,sys; print(hashlib.md5(open(sys.argv[1],'rb').read()).hexdigest())" "$1"; }
M0="$(md5 "$IMG")"

boot_shim() { # boot_shim <serial port> <log> <commands (| between)>: from the stick layout, the disk attached
    local sp=$1 log=$2 cmds=$3
    local IFS='|'; set -f; local -a C=($cmds); set +f; unset IFS
    qemu-system-aarch64 -M virt -m 256M -display none \
        -drive "file=$IMG,if=none,format=raw,id=sd" "${DEV[@]}" \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py "$sp" "$((sp+1))" "$W/shot.png" "${C[@]}" > "$log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
}
echo "  --- boot 1: lsparts, installdisk $CMD (the warning + code), a wrong code ---"
boot_shim "$PORT" "$W/boot1.log" "lsparts|installdisk $CMD|installdisk $CMD yes zzzz"
if [ "$(md5 "$IMG")" = "$M0" ]; then printf '\nharness: boot 1: the disk image is unchanged\n' >> "$W/boot1.log"
else printf '\nharness: boot 1: the disk image CHANGED\n' >> "$W/boot1.log"; fi
IC="$(tr -d '\r' < "$W/boot1.log" | grep -aoE "type exactly: installdisk $CMD yes [0-9a-f]{4}\$" | head -1 | awk '{print $NF}')"
echo "  install code: ${IC:-NONE}"
echo "  --- boot 2: the code typed, a file written ---"
boot_shim "$((PORT + 4))" "$W/boot2.log" "installdisk $CMD yes ${IC:-none}|write notes.txt HELLO-INSTALL-6NOV|sync"
printf '\n' >> "$W/boot2.log"
"$PY" - "$IMG" "$PC_EFI" >> "$W/boot2.log" << 'PYEOF'
import sys, struct, zlib
d = open(sys.argv[1], "rb"); efi = open(sys.argv[2], "rb").read()
def sec(n, k=1):
    d.seek(n * 512); return d.read(512 * k)
def bad(why):
    print("harness: boot 2: the disk image is wrong: " + why); sys.exit(0)
d.seek(0, 2); blocks = d.tell() // 512
m = sec(0)
if m[510:512] != b"\x55\xaa" or m[450] != 0xEE: bad("no protective MBR")
hs = []
for lba in (1, blocks - 1):
    h = sec(lba)
    if h[:8] != b"EFI PART": bad("no GPT header at LBA %d" % lba)
    size = struct.unpack_from("<I", h, 12)[0]
    hh = bytearray(h[:size]); hh[16:20] = b"\0\0\0\0"
    if zlib.crc32(bytes(hh)) != struct.unpack_from("<I", h, 16)[0]: bad("header CRC at LBA %d" % lba)
    elba, n, esz, ecrc = struct.unpack_from("<QIII", h, 72)
    arr = sec(elba, n * esz // 512)
    if zlib.crc32(arr) != ecrc: bad("entry array CRC for the header at LBA %d" % lba)
    hs.append(arr)
if hs[0] != hs[1]: bad("the two entry arrays differ")
ents = [hs[0][i * 128:(i + 1) * 128] for i in range(128)]
used = [e for e in ents if any(e[:16])]
if len(used) != 2: bad("%d entries, not 2" % len(used))
esp_type = bytes.fromhex("28732AC11FF8D211BA4B00A0C93EC93B")
if used[0][:16] != esp_type: bad("entry 1 is not an EFI system partition")
start = struct.unpack_from("<Q", used[0], 32)[0]
b = sec(start)
bps, spc, rsvd, nf = struct.unpack_from("<HBHB", b, 11)
fsz, root = struct.unpack_from("<I", b, 36)[0], struct.unpack_from("<I", b, 44)[0]
if b[82:90] != b"FAT32   " or bps != 512: bad("not FAT32")
fat0 = start + rsvd; data0 = fat0 + nf * fsz
def fat(c):
    return struct.unpack_from("<I", sec(fat0 + c * 4 // 512), (c * 4) % 512)[0] & 0x0FFFFFFF
def chain(c):
    out = []
    while 2 <= c < 0x0FFFFFF8 and len(out) < 100000:
        out.append(c); c = fat(c)
    return out
def read(c, size=None):
    data = b"".join(sec(data0 + (k - 2) * spc, spc) for k in chain(c))
    return data if size is None else data[:size]
def find(c, name):
    dd = read(c)
    for i in range(0, len(dd), 32):
        e = dd[i:i + 32]
        if e[0] == 0: break
        if e[:11] == name:
            return (struct.unpack_from("<H", e, 20)[0] << 16) | struct.unpack_from("<H", e, 26)[0], struct.unpack_from("<I", e, 28)[0]
    bad("%s not found" % name.decode())
c, _ = find(root, b"EFI        ")
c, _ = find(c, b"BOOT       ")
c, size = find(c, b"BOOTX64 EFI")
if read(c, size) != efi: bad("EFI\\BOOT\\BOOTX64.EFI is not the booted kernel")
print("harness: boot 2: GPT ok (protective MBR, both headers, 2 entries), FAT32 EFI\\BOOT\\BOOTX64.EFI is the booted kernel byte for byte")
PYEOF
echo "  --- ${BOOT3_LINE#harness: }: cat notes.txt ---"
printf '%s\n' "$BOOT3_LINE" > "$W/boot3.log"
if [ "$KIND" = emmc ]; then
    boot_shim "$((PORT + 8))" "$W/boot3.tmp" "cat notes.txt|lsparts|lssd"
    cat "$W/boot3.tmp" >> "$W/boot3.log"
else
"$REALQ" -M q35 -m 256M -display none -drive "if=pflash,format=raw,readonly=on,file=$(winpath "$OVMF")" \
    -drive "file=$(winpath "$IMG"),if=none,format=raw,id=sd" "${DEV[@]}" \
    -serial "tcp:127.0.0.1:$((PORT + 8)),server,nowait" -qmp "tcp:127.0.0.1:$((PORT + 9)),server,nowait" &
q=$!
AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py "$((PORT + 8))" "$((PORT + 9))" "$W/shot.png" "cat notes.txt" "lsparts" >> "$W/boot3.log" 2>&1
kill "$q" 2> /dev/null
wait "$q" 2> /dev/null
fi
cat "$W/boot1.log" "$W/boot2.log" "$W/boot3.log" | tr -d '\r' |
    grep -aE '^afrisovera> (installdisk|cat)|^installdisk|^\[(nvme|ahci|sdhci)\].*AfriSovera|^\[pc\] (own boot|MANSA)|^HELLO|^harness' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot1.log" "$W/boot2.log" "$W/boot3.log"; then
    rm -rf "$W"
    echo "PC INSTALLDISK TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC INSTALLDISK TEST FAILED"
exit 1
