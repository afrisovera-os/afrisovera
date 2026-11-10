#!/usr/bin/env bash
# PC track step 3 (10 Nov 2026): the partition tools on a SATA disk in IDE mode (pc/ide.rs). A Windows PC
# whose firmware keeps SATA in "IDE" mode can only take AfriSovera inside its own partition, so the IDE
# drives are disks for lsparts / tablebackup / installpart / formatpart / tablerestore by their number in
# lside (ide0 = primary master). The same four boots as pc-formatide, on QEMU's piix3-ide (primary
# master = the Windows-like 64 MiB GPT disk), a blank USB stick for the table backup:
# Boot 1: formatpart ide0 refuses (no partition yet), bare `formatpart ide` names no disk; lsparts lists
# ide0; tablebackup; installpart ide0 8 shows its code. Boot 2: the install code typed (entry 5, LBA
# 100352-116735); formatpart ide0 shows the partition and its code; a wrong code is refused; the harness
# checks only the four table sectors changed. Boot 3: the driver says the partition is not formatted yet;
# the code typed -> AFRIVFS1 in it; lside shows it as storage; a file written + synced; formatpart again
# says it is in use. The harness checks every changed sector lies inside the partition. Boot 4 (fresh
# boot): the driver uses the partition by itself, the file comes back, nothing outside it differs.
# Runs the same in an ARM walk and a PC walk: it builds the PC kernel and puts the PC shim
# (scripts/lib/pc-arch) on PATH itself.
#   scripts/pc-formatide-test.sh              # grader selftest, then the four QEMU boots
#   scripts/pc-formatide-test.sh --selftest   # grader selftest only
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
    local IC FC
    IC="$(printf '%s\n' "$A" | grep -aoE 'type exactly: installpart ide0 8 yes [0-9a-f]{4}$' | head -1 | awk '{print $NF}')"
    [ -n "$IC" ] || _f "boot 1: installpart ide0 8 did not show its code"
    FC="$(printf '%s\n' "$B" | grep -aoE 'type exactly: formatpart ide0 yes [0-9a-f]{4}$' | head -1 | awk '{print $NF}')"
    [ -n "$FC" ] || _f "boot 2: formatpart ide0 did not show its code"
    has "$(blk "$A" 'formatpart ide0')" 'formatpart: ide0 has no AfriSovera partition in a healthy GPT -- make one with installpart first; nothing written' ||
        _f "boot 1: formatpart without a partition did not refuse"
    has "$(blk "$A" 'formatpart ide')" 'formatpart: no disk "ide" here -- say nvme, sataN, ideN or emmc \(see lsparts\); nothing written' ||
        _f "boot 1: bare formatpart ide was not refused"
    has "$(blk "$A" 'lsparts')" 'lsparts: ide0 \(SATA in IDE mode\), 64 MiB, 512-byte sectors' || _f "boot 1: lsparts does not list ide0"
    has "$A" 'harness: boot 1: disk images unchanged' || _f "boot 1: a disk image changed"
    has "$(blk "$B" "installpart ide0 8 yes $IC")" "installpart: AfriSovera's partition written as GPT entry 5, LBA 100352-116735 \(8 MiB\), .*" ||
        _f "boot 2: the install code did not write entry 5"
    has "$(blk "$B" 'formatpart ide0')" "formatpart: ide0: AfriSovera's partition, sectors 100352-116735 \(8 MiB\); whatever is in it now is lost, no other partition is touched" ||
        _f "boot 2: formatpart did not name the partition"
    has "$(blk "$B" 'formatpart ide0 yes zzzz')" 'formatpart: that code does not fit this partition now -- nothing written' || _f "boot 2: a wrong code was not refused"
    has "$B" 'harness: boot 2: ide image: only the four GPT table sectors changed' || _f "boot 2: formatpart (or the install) wrote more than the table"
    has "$C" "\[ide\] ide0: AfriSovera's partition at sector 100352 \(8 MiB\) is not formatted yet \(formatpart ide0\)" ||
        _f "boot 3: the driver did not find the unformatted partition at boot"
    has "$(blk "$C" "formatpart ide0 yes $FC")" "formatpart: ide0: AfriSovera's partition formatted \(AFRIVFS1 at sector 100352, 8 MiB\), read back OK; .*" ||
        _f "boot 3: the code did not format the partition"
    has "$(blk "$C" 'lside')" "  ide0: primary master \(0x1f0\): QEMU HARDDISK, 131072 sectors \(64 MiB\)  <- storage \(AfriSovera's partition\)" ||
        _f "boot 3: lside does not mark ide0 as storage"
    has "$(blk "$C" 'lside')" "  storage partition: sectors 100352-116735 \(8 MiB\)" || _f "boot 3: lside does not show the partition"
    has "$(printf '%s\n' "$C" | awk -v c="afrisovera> formatpart ide0" '$0==c{on=1;next} /^afrisovera>/{on=0} on')" \
        "formatpart: ide0 already keeps AfriSovera's files in its partition at sector 100352 \(8 MiB\) -- nothing written" ||
        _f "boot 3: formatpart twice was not refused"
    has "$C" 'harness: boot 3: ide image: every changed sector is inside the partition, AFRIVFS1 at sector 100352' ||
        _f "boot 3: something outside the partition was written (or no AFRIVFS1)"
    has "$D" "\[ide\] ide0: AfriSovera's partition at sector 100352 \(8 MiB\) holds our files" || _f "boot 4: the driver did not find the formatted partition"
    has "$D" "\[ide\] primary master \(0x1f0\): ATA disk, 131072 sectors \(64 MiB\), LBA(28|48), AfriSovera's partition used for storage, the rest left alone" || _f "boot 4: the partition is not used for storage"
    has "$(blk "$D" 'cat notes.txt')" 'HELLO-IDE-10NOV' || _f "boot 4: notes.txt did not come back from the partition"
    has "$D" 'harness: boot 4: outside the partition the ide image is as the install left it' || _f "boot 4: something outside the partition changed"
    if printf '%s\n' "$A" "$B" "$C" "$D" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[(nvme|ahci|xhci|ide)\] .*(failed|timed out|busy)|FAILED'; then
        _f "the kernel panicked or a disk command failed"
    fi
    return "$fail"
}

M1="afrisovera> formatpart ide0
formatpart: ide0 has no AfriSovera partition in a healthy GPT -- make one with installpart first; nothing written
afrisovera> formatpart ide
formatpart: no disk \"ide\" here -- say nvme, sataN, ideN or emmc (see lsparts); nothing written
afrisovera> lsparts
lsparts: ide0 (SATA in IDE mode), 64 MiB, 512-byte sectors
lsparts: read only -- nothing was written
afrisovera> tablebackup
tablebackup: ide0: GPT table, 67 sector(s) saved on the USB stick at sectors 2049-2116, CRC 1a2b3c4d, read back OK
afrisovera> installpart ide0 8
installpart: nothing written yet. To write it, type exactly: installpart ide0 8 yes 3f9c
afrisovera>
harness: boot 1: disk images unchanged"
M2="afrisovera> installpart ide0 8 yes 3f9c
installpart: AfriSovera's partition written as GPT entry 5, LBA 100352-116735 (8 MiB), in both GPT copies, read back OK; no other partition was changed
afrisovera> formatpart ide0
formatpart: ide0: AfriSovera's partition, sectors 100352-116735 (8 MiB); whatever is in it now is lost, no other partition is touched
formatpart: nothing written yet. To format it, type exactly: formatpart ide0 yes 77ab
afrisovera> formatpart ide0 yes zzzz
formatpart: that code does not fit this partition now -- nothing written
afrisovera>
harness: boot 2: ide image: only the four GPT table sectors changed"
M3="[ide] ide0: AfriSovera's partition at sector 100352 (8 MiB) is not formatted yet (formatpart ide0)
[ide] primary master (0x1f0): ATA disk, 131072 sectors (64 MiB), LBA28, holds other data (Windows?): left alone
afrisovera> formatpart ide0 yes 77ab
[vfs] synced 20 files to disk
formatpart: ide0: AfriSovera's partition formatted (AFRIVFS1 at sector 100352, 8 MiB), read back OK; the shell's files are kept there from now on and come back at the next boot
afrisovera> lside
ide: controller 00:03.0, interface 0x80
  ide0: primary master (0x1f0): QEMU HARDDISK, 131072 sectors (64 MiB)  <- storage (AfriSovera's partition)
  storage partition: sectors 100352-116735 (8 MiB)
  storage: 120 sector(s) read, 49 written, 1 cache flush(es)
afrisovera> write notes.txt HELLO-IDE-10NOV
afrisovera> sync
afrisovera> formatpart ide0
formatpart: ide0 already keeps AfriSovera's files in its partition at sector 100352 (8 MiB) -- nothing written
afrisovera>
harness: boot 3: ide image: every changed sector is inside the partition, AFRIVFS1 at sector 100352"
M4="[ide] ide0: AfriSovera's partition at sector 100352 (8 MiB) holds our files
[ide] primary master (0x1f0): ATA disk, 131072 sectors (64 MiB), LBA28, AfriSovera's partition used for storage, the rest left alone
[vfs] VFS restored from disk: 21 files
afrisovera> cat notes.txt
HELLO-IDE-10NOV
afrisovera>
harness: boot 4: outside the partition the ide image is as the install left it"

selftest() {
    local W pass=0 bad=0
    W="$(mktemp -d "$T/pc-formatide-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <b1> <b2> <b3> <b4>
        local got
        printf '%s\n' "$3" > "$W/a"; printf '%s\n' "$4" > "$W/b"; printf '%s\n' "$5" > "$W/c"; printf '%s\n' "$6" > "$W/d"
        if grade "$W/a" "$W/b" "$W/c" "$W/d" > "$W/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$W/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm model-green green "$M1" "$M2" "$M3" "$M4"
    arm crlf green "$(printf '%s\n' "$M1" | sed 's/$/\r/')" "$(printf '%s\n' "$M2" | sed 's/$/\r/')" "$(printf '%s\n' "$M3" | sed 's/$/\r/')" "$M4"
    arm formatted-without-partition red "$(printf '%s\n' "$M1" | sed 's/^formatpart: ide0 has no.*/formatpart: nothing written yet. To format it, type exactly: formatpart ide0 yes 0001/')" "$M2" "$M3" "$M4"
    arm bare-ide-accepted red "$(printf '%s\n' "$M1" | grep -v 'no disk "ide" here')" "$M2" "$M3" "$M4"
    arm lsparts-blind red "$(printf '%s\n' "$M1" | grep -v '^lsparts: ide0')" "$M2" "$M3" "$M4"
    arm boot1-image-changed red "$(printf '%s\n' "$M1" | sed 's/disk images unchanged/disk images CHANGED/')" "$M2" "$M3" "$M4"
    arm no-install-code red "$(printf '%s\n' "$M1" | grep -v 'type exactly')" "$M2" "$M3" "$M4"
    arm no-format-code red "$M1" "$(printf '%s\n' "$M2" | grep -v 'type exactly')" "$M3" "$M4"
    arm wrong-partition red "$M1" "$(printf '%s\n' "$M2" | sed 's/sectors 100352-116735/sectors 100352-118783/')" "$M3" "$M4"
    arm wrong-code-accepted red "$M1" "$(printf '%s\n' "$M2" | sed 's/^formatpart: that code does not fit.*/formatpart: ide0: AfriSovera'"'"'s partition formatted/')" "$M3" "$M4"
    arm boot2-wrote-more red "$M1" "$(printf '%s\n' "$M2" | grep -v '^harness')" "$M3" "$M4"
    arm not-found-at-boot red "$M1" "$M2" "$(printf '%s\n' "$M3" | grep -v 'is not formatted yet')" "$M4"
    arm not-formatted red "$M1" "$M2" "$(printf '%s\n' "$M3" | grep -v 'partition formatted')" "$M4"
    arm lside-blind red "$M1" "$M2" "$(printf '%s\n' "$M3" | grep -v '<- storage')" "$M4"
    arm lside-no-partition red "$M1" "$M2" "$(printf '%s\n' "$M3" | grep -v 'storage partition:')" "$M4"
    arm formatted-twice red "$M1" "$M2" "$(printf '%s\n' "$M3" | grep -v 'already keeps')" "$M4"
    arm wrote-outside red "$M1" "$M2" "$(printf '%s\n' "$M3" | grep -v '^harness')" "$M4"
    arm not-used-at-boot red "$M1" "$M2" "$M3" "$(printf '%s\n' "$M4" | grep -v 'holds our files')"
    arm not-storage red "$M1" "$M2" "$M3" "$(printf '%s\n' "$M4" | sed 's/AfriSovera.s partition used for storage, the rest left alone/holds other data (Windows?): left alone/')"
    arm file-lost red "$M1" "$M2" "$M3" "$(printf '%s\n' "$M4" | grep -v '^HELLO')"
    arm outside-changed red "$M1" "$M2" "$M3" "$(printf '%s\n' "$M4" | grep -v '^harness')"
    arm panic red "$M1" "$M2" "$(printf '%s\nKERNEL PANIC at ide.rs' "$M3")" "$M4"
    arm disk-busy red "$M1" "$M2" "$M3" "$(printf '%s\n[ide] channel 0x1f0: still busy 5 s after reset: not used' "$M4")"
    arm empty red "" "" "" ""
    rm -rf "$W"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 24 ]
}

echo "== PC install on a SATA disk in IDE mode: AfriSovera's partition formatted and used for the files, nothing outside it written (PC track step 3) =="
if ! selftest; then echo "PC FORMATIDE TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC FORMATIDE SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC FORMATIDE TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
PY=python3; command -v python3 > /dev/null || PY=python

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC FORMATIDE TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-formatide.XXXXXX")"
"$PY" scripts/lib/make-disk-image.py gpt "$W/gpt.img" || { echo "PC FORMATIDE TEST FAILED (harness: could not make gpt.img)"; exit 1; }
cp "$W/gpt.img" "$W/gpt-orig.img"
head -c 8388608 /dev/zero > "$W/stick.img" # a fresh, blank 8 MiB USB stick
(cd "$W" && md5sum gpt.img > before.md5)

STICK=(-drive "file=$W/stick.img,if=none,format=raw,id=st" -device qemu-xhci,id=xhci -device usb-storage,drive=st,bus=xhci.0)
boot() { # boot <serial port> <log> <commands (| between)>
    local sp=$1 log=$2 cmds=$3
    local IFS='|'; set -f; local -a C=($cmds); set +f; unset IFS
    qemu-system-aarch64 -M virt -m 256M -display none \
        -drive "file=$W/gpt.img,if=none,format=raw,id=sd" -device piix3-ide,id=pide -device ide-hd,drive=sd,bus=pide.0,unit=0 "${STICK[@]}" \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py "$sp" "$((sp+1))" "$W/shot.png" "${C[@]}" > "$log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
}
# check <old image> <new image> <mode>: the harness's own look at the IDE disk's image
check() {
    "$PY" - "$@" << 'PYEOF'
import sys
S, P0, P1 = 512, 100352, 116735
a = open(sys.argv[1], "rb").read()
b = open(sys.argv[2], "rb").read()
n = len(b) // S
diff = [k for k in range(n) if a[k * S:(k + 1) * S] != b[k * S:(k + 1) * S]]
m = sys.argv[3]
if m == "table":
    print("harness: boot 2: ide image: " + ("only the four GPT table sectors changed" if diff == [1, 3, n - 32, n - 1] else "changed sectors %s" % diff[:10]))
elif m == "inside":
    ok = diff and all(P0 <= k <= P1 for k in diff) and b[P0 * S:P0 * S + 8] == b"AFRIVFS1"
    print("harness: boot 3: ide image: " + ("every changed sector is inside the partition, AFRIVFS1 at sector %d" % P0 if ok else "changed sectors %s" % diff[:10]))
else:
    out = [k for k in diff if not P0 <= k <= P1]
    print("harness: boot 4: " + ("outside the partition the ide image is as the install left it" if not out else "sectors outside the partition changed: %s" % out[:10]))
PYEOF
}
echo "  --- boot 1: no partition yet, bare ide, lsparts, tablebackup, the install plan + code ---"
boot 5070 "$W/boot1.log" 'formatpart ide0|formatpart ide|lsparts|tablebackup|installpart ide0 8'
if (cd "$W" && md5sum -c --quiet before.md5 > /dev/null 2>&1); then printf '\nharness: boot 1: disk images unchanged\n' >> "$W/boot1.log"
else printf '\nharness: boot 1: disk images CHANGED\n' >> "$W/boot1.log"; fi
IC="$(tr -d '\r' < "$W/boot1.log" | grep -aoE 'type exactly: installpart ide0 8 yes [0-9a-f]{4}$' | head -1 | awk '{print $NF}')"
echo "  install code: ${IC:-NONE}"
echo "  --- boot 2: install, formatpart's plan + code, a wrong code ---"
boot 5074 "$W/boot2.log" "installpart ide0 8 yes ${IC:-none}|formatpart ide0|formatpart ide0 yes zzzz"
printf '\n' >> "$W/boot2.log"; check "$W/gpt-orig.img" "$W/gpt.img" table >> "$W/boot2.log"
cp "$W/gpt.img" "$W/gpt-installed.img"
FC="$(tr -d '\r' < "$W/boot2.log" | grep -aoE 'type exactly: formatpart ide0 yes [0-9a-f]{4}$' | head -1 | awk '{print $NF}')"
echo "  format code: ${FC:-NONE}"
echo "  --- boot 3: the code typed, lside, a file written, formatpart again ---"
boot 5078 "$W/boot3.log" "formatpart ide0 yes ${FC:-none}|lside|write notes.txt HELLO-IDE-10NOV|sync|formatpart ide0"
printf '\n' >> "$W/boot3.log"; check "$W/gpt-installed.img" "$W/gpt.img" inside >> "$W/boot3.log"
echo "  --- boot 4: a fresh boot: the file back from the partition ---"
boot 5082 "$W/boot4.log" 'cat notes.txt|lside|lsparts'
printf '\n' >> "$W/boot4.log"; check "$W/gpt-installed.img" "$W/gpt.img" outside >> "$W/boot4.log"
cat "$W/boot1.log" "$W/boot2.log" "$W/boot3.log" "$W/boot4.log" | tr -d '\r' |
    grep -aE '^afrisovera> (formatpart|installpart|cat|lside|lsparts)|^formatpart|^installpart: AfriSovera|^lsparts: ide|^\[ide\]|^  (ide0|storage)|^HELLO|^harness' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot1.log" "$W/boot2.log" "$W/boot3.log" "$W/boot4.log"; then
    rm -rf "$W"
    echo "PC FORMATIDE TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC FORMATIDE TEST FAILED"
exit 1
