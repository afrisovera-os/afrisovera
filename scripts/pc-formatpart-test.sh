#!/usr/bin/env bash
# PC track step 5 (6 Nov 2026): `formatpart nvme [yes <code>]`: AfriSovera's partition (the one
# `installpart` adds to a GPT) formatted with the VFS on the owner's code, and the shell's files kept
# INSIDE that partition from then on, with Windows' partitions beside it never written. At boot the NVMe
# driver finds the partition again (healthy GPT, both headers + entries CRC, AfriSovera's type GUID, no
# overlap) and maps the VFS onto it.
#
# Boot 1: the Windows-like GPT on NVMe (9 MiB free), a blank USB stick: formatpart refuses (no partition
# yet); tablebackup; installpart nvme 8 shows its code. Boot 2: the install code typed (entry 5, LBA
# 100352-116735); formatpart nvme shows the partition and its code; a wrong code and `formatpart sata`
# are refused; the harness checks only the four table sectors changed. Boot 3: the driver says the
# partition is not formatted yet; the code typed -> AFRIVFS1 in it; lsnvme names it; a file written +
# synced; formatpart again says it is in use. The harness checks every changed sector lies inside the
# partition and its first sector is AFRIVFS1. Boot 4 (fresh firmware boot): the driver uses the partition
# by itself, the file comes back, and nothing outside the partition differs from the install.
# Runs the same in an ARM walk and a PC walk: it builds the PC kernel and puts the PC shim
# (scripts/lib/pc-arch) on PATH itself.
#   scripts/pc-formatpart-test.sh              # grader selftest, then the four QEMU boots
#   scripts/pc-formatpart-test.sh --selftest   # grader selftest only
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
    local IC FC
    IC="$(printf '%s\n' "$A" | grep -aoE 'type exactly: installpart nvme 8 yes [0-9a-f]{4}$' | head -1 | awk '{print $NF}')"
    [ -n "$IC" ] || _f "boot 1: installpart nvme 8 did not show its code"
    FC="$(printf '%s\n' "$B" | grep -aoE 'type exactly: formatpart nvme yes [0-9a-f]{4}$' | head -1 | awk '{print $NF}')"
    [ -n "$FC" ] || _f "boot 2: formatpart nvme did not show its code"
    has "$(blk "$A" 'formatpart nvme')" 'formatpart: nvme has no AfriSovera partition in a healthy GPT; make one with installpart first; nothing written' ||
        _f "boot 1: formatpart without a partition did not refuse"
    has "$A" 'harness: boot 1: disk images unchanged' || _f "boot 1: a disk image changed"
    has "$(blk "$B" "installpart nvme 8 yes $IC")" "installpart: AfriSovera's partition written as GPT entry 5, LBA 100352-116735 \(8 MiB\), .*" ||
        _f "boot 2: the install code did not write entry 5"
    local B2; B2="$(blk "$B" 'formatpart nvme')"
    has "$B2" "formatpart: nvme: AfriSovera's partition, sectors 100352-116735 \(8 MiB\); whatever is in it now is lost, no other partition is touched" ||
        _f "boot 2: formatpart did not name the partition"
    has "$(blk "$B" 'formatpart nvme yes zzzz')" 'formatpart: that code does not fit this partition now; nothing written' || _f "boot 2: a wrong code was not refused"
    has "$(blk "$B" 'formatpart sata')" 'formatpart: say which SATA port, e.g. formatpart sata2 \(see lsparts\); nothing written' || _f "boot 2: formatpart sata (no port) was not refused"
    has "$B" 'harness: boot 2: nvme image: only the four GPT table sectors changed' || _f "boot 2: formatpart (or the install) wrote more than the table"
    has "$C" "\[nvme\] AfriSovera's partition at sector 100352 \(8 MiB\) is not formatted yet \(formatpart nvme\)" ||
        _f "boot 3: the driver did not find the unformatted partition at boot"
    has "$(blk "$C" "formatpart nvme yes $FC")" "formatpart: nvme: AfriSovera's partition formatted \(AFRIVFS1 at sector 100352, 8 MiB\), read back OK; .*" ||
        _f "boot 3: the code did not format the partition"
    has "$(blk "$C" 'lsnvme')" "  storage: AfriSovera's partition, sectors 100352-116735 \(8 MiB\); nothing outside it is written" ||
        _f "boot 3: lsnvme does not show the partition as storage"
    has "$(printf '%s\n' "$C" | awk -v c="afrisovera> formatpart nvme" '$0==c{on=1;next} /^afrisovera>/{on=0} on')" \
        "formatpart: nvme already keeps AfriSovera's files in its partition at sector 100352 \(8 MiB\); nothing written" ||
        _f "boot 3: formatpart twice was not refused"
    has "$C" 'harness: boot 3: nvme image: every changed sector is inside the partition, AFRIVFS1 at sector 100352' ||
        _f "boot 3: something outside the partition was written (or no AFRIVFS1)"
    has "$D" "\[nvme\] AfriSovera's partition at sector 100352 \(8 MiB\) holds our files" || _f "boot 4: the driver did not find the formatted partition"
    has "$D" "\[nvme\] namespace 1: .*AfriSovera's partition \(AFRIVFS1\): used for storage, the rest left alone" || _f "boot 4: the partition is not used for storage"
    has "$(blk "$D" 'cat notes.txt')" 'HELLO-PART-6NOV' || _f "boot 4: notes.txt did not come back from the partition"
    has "$D" 'harness: boot 4: outside the partition the nvme image is as the install left it' || _f "boot 4: something outside the partition changed"
    if printf '%s\n' "$A" "$B" "$C" "$D" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[(nvme|ahci|xhci)\] .*(failed|timed out)|FAILED'; then
        _f "the kernel panicked or a disk command failed"
    fi
    return "$fail"
}

M1="afrisovera> formatpart nvme
formatpart: nvme has no AfriSovera partition in a healthy GPT; make one with installpart first; nothing written
afrisovera> tablebackup
tablebackup: nvme: GPT table, 67 sector(s) saved on the USB stick at sectors 2049-2116, CRC 1a2b3c4d, read back OK
afrisovera> installpart nvme 8
installpart: nothing written yet. To write it, type exactly: installpart nvme 8 yes 3f9c
afrisovera>
harness: boot 1: disk images unchanged"
M2="afrisovera> installpart nvme 8 yes 3f9c
installpart: AfriSovera's partition written as GPT entry 5, LBA 100352-116735 (8 MiB), in both GPT copies, read back OK; no other partition was changed
afrisovera> formatpart nvme
formatpart: nvme: AfriSovera's partition, sectors 100352-116735 (8 MiB); whatever is in it now is lost, no other partition is touched
formatpart: nothing written yet. To format it, type exactly: formatpart nvme yes 77ab
afrisovera> formatpart nvme yes zzzz
formatpart: that code does not fit this partition now; nothing written
afrisovera> formatpart sata
formatpart: say which SATA port, e.g. formatpart sata2 (see lsparts); nothing written
afrisovera>
harness: boot 2: nvme image: only the four GPT table sectors changed"
M3="[nvme] AfriSovera's partition at sector 100352 (8 MiB) is not formatted yet (formatpart nvme)
[nvme] namespace 1: 131072 blocks of 512 B (64 MiB), holds other data: left alone
afrisovera> formatpart nvme yes 77ab
[vfs] synced 20 files to disk
formatpart: nvme: AfriSovera's partition formatted (AFRIVFS1 at sector 100352, 8 MiB), read back OK; the shell's files are kept there from now on and come back at the next boot
afrisovera> lsnvme
  namespace 1: 131072 blocks of 512 B (64 MiB)  <- storage
  storage: AfriSovera's partition, sectors 100352-116735 (8 MiB); nothing outside it is written
afrisovera> write notes.txt HELLO-PART-6NOV
afrisovera> sync
afrisovera> formatpart nvme
formatpart: nvme already keeps AfriSovera's files in its partition at sector 100352 (8 MiB); nothing written
afrisovera>
harness: boot 3: nvme image: every changed sector is inside the partition, AFRIVFS1 at sector 100352"
M4="[nvme] AfriSovera's partition at sector 100352 (8 MiB) holds our files
[nvme] namespace 1: 131072 blocks of 512 B (64 MiB), AfriSovera's partition (AFRIVFS1): used for storage, the rest left alone
[vfs] VFS restored from disk: 21 files
afrisovera> cat notes.txt
HELLO-PART-6NOV
afrisovera>
harness: boot 4: outside the partition the nvme image is as the install left it"

selftest() {
    local W pass=0 bad=0
    W="$(mktemp -d "$T/pc-formatpart-selftest.XXXXXX")" || return 1
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
    arm formatted-without-partition red "$(printf '%s\n' "$M1" | sed 's/^formatpart: nvme has no.*/formatpart: nothing written yet. To format it, type exactly: formatpart nvme yes 0001/')" "$M2" "$M3" "$M4"
    arm boot1-image-changed red "$(printf '%s\n' "$M1" | sed 's/disk images unchanged/disk images CHANGED/')" "$M2" "$M3" "$M4"
    arm no-install-code red "$(printf '%s\n' "$M1" | grep -v 'type exactly')" "$M2" "$M3" "$M4"
    arm no-format-code red "$M1" "$(printf '%s\n' "$M2" | grep -v 'type exactly')" "$M3" "$M4"
    arm wrong-partition red "$M1" "$(printf '%s\n' "$M2" | sed 's/sectors 100352-116735/sectors 100352-118783/')" "$M3" "$M4"
    arm wrong-code-accepted red "$M1" "$(printf '%s\n' "$M2" | sed 's/^formatpart: that code does not fit.*/formatpart: nvme: AfriSovera'"'"'s partition formatted/')" "$M3" "$M4"
    arm sata-accepted red "$M1" "$(printf '%s\n' "$M2" | grep -v 'say which SATA port')" "$M3" "$M4"
    arm boot2-wrote-more red "$M1" "$(printf '%s\n' "$M2" | grep -v '^harness')" "$M3" "$M4"
    arm not-found-at-boot red "$M1" "$M2" "$(printf '%s\n' "$M3" | grep -v 'is not formatted yet')" "$M4"
    arm not-formatted red "$M1" "$M2" "$(printf '%s\n' "$M3" | grep -v 'partition formatted')" "$M4"
    arm lsnvme-blind red "$M1" "$M2" "$(printf '%s\n' "$M3" | grep -v '  storage:')" "$M4"
    arm formatted-twice red "$M1" "$M2" "$(printf '%s\n' "$M3" | grep -v 'already keeps')" "$M4"
    arm wrote-outside red "$M1" "$M2" "$(printf '%s\n' "$M3" | grep -v '^harness')" "$M4"
    arm not-used-at-boot red "$M1" "$M2" "$M3" "$(printf '%s\n' "$M4" | grep -v 'holds our files')"
    arm not-storage red "$M1" "$M2" "$M3" "$(printf '%s\n' "$M4" | sed 's/AfriSovera.s partition (AFRIVFS1): used for storage, the rest left alone/holds other data: left alone/')"
    arm file-lost red "$M1" "$M2" "$M3" "$(printf '%s\n' "$M4" | grep -v '^HELLO')"
    arm outside-changed red "$M1" "$M2" "$M3" "$(printf '%s\n' "$M4" | grep -v '^harness')"
    arm panic red "$M1" "$M2" "$(printf '%s\nKERNEL PANIC at nvme.rs' "$M3")" "$M4"
    arm disk-failed red "$M1" "$M2" "$M3" "$(printf '%s\n[nvme] write of block 100352 failed (status 0x2)' "$M4")"
    arm empty red "" "" "" ""
    rm -rf "$W"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 22 ]
}

echo "== PC install: AfriSovera's partition formatted and used for the files, nothing outside it written (PC track step 5) =="
if ! selftest; then echo "PC FORMATPART TEST FAILED (grader selftest); no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC FORMATPART SELFTEST ONLY; no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC FORMATPART TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
PY=python3; command -v python3 > /dev/null || PY=python

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC FORMATPART TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-formatpart.XXXXXX")"
"$PY" scripts/lib/make-disk-image.py gpt "$W/gpt.img" || { echo "PC FORMATPART TEST FAILED (harness: could not make gpt.img)"; exit 1; }
cp "$W/gpt.img" "$W/gpt-orig.img"
head -c 8388608 /dev/zero > "$W/stick.img" # a fresh, blank 8 MiB USB stick
(cd "$W" && md5sum gpt.img > before.md5)

STICK=(-drive "file=$W/stick.img,if=none,format=raw,id=st" -device qemu-xhci,id=xhci -device usb-storage,drive=st,bus=xhci.0)
boot() { # boot <serial port> <log> <commands (| between)>
    local sp=$1 log=$2 cmds=$3
    local IFS='|'; set -f; local -a C=($cmds); set +f; unset IFS
    qemu-system-aarch64 -M virt -m 256M -display none \
        -drive "file=$W/gpt.img,if=none,format=raw,id=sd" -device nvme,serial=afri01,drive=sd "${STICK[@]}" \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py "$sp" "$((sp+1))" "$W/shot.png" "${C[@]}" > "$log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
}
# diff <old image> <new image> <mode>: the harness's own look at the NVMe image
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
    print("harness: boot 2: nvme image: " + ("only the four GPT table sectors changed" if diff == [1, 3, n - 32, n - 1] else "changed sectors %s" % diff[:10]))
elif m == "inside":
    ok = diff and all(P0 <= k <= P1 for k in diff) and b[P0 * S:P0 * S + 8] == b"AFRIVFS1"
    print("harness: boot 3: nvme image: " + ("every changed sector is inside the partition, AFRIVFS1 at sector %d" % P0 if ok else "changed sectors %s" % diff[:10]))
else:
    out = [k for k in diff if not P0 <= k <= P1]
    print("harness: boot 4: " + ("outside the partition the nvme image is as the install left it" if not out else "sectors outside the partition changed: %s" % out[:10]))
PYEOF
}
echo "  --- boot 1: no partition yet, tablebackup, the install plan + code ---"
boot 5050 "$W/boot1.log" 'formatpart nvme|tablebackup|installpart nvme 8'
if (cd "$W" && md5sum -c --quiet before.md5 > /dev/null 2>&1); then printf '\nharness: boot 1: disk images unchanged\n' >> "$W/boot1.log"
else printf '\nharness: boot 1: disk images CHANGED\n' >> "$W/boot1.log"; fi
IC="$(tr -d '\r' < "$W/boot1.log" | grep -aoE 'type exactly: installpart nvme 8 yes [0-9a-f]{4}$' | head -1 | awk '{print $NF}')"
echo "  install code: ${IC:-NONE}"
echo "  --- boot 2: install, formatpart's plan + code, a wrong code, sata ---"
boot 5054 "$W/boot2.log" "installpart nvme 8 yes ${IC:-none}|formatpart nvme|formatpart nvme yes zzzz|formatpart sata"
printf '\n' >> "$W/boot2.log"; check "$W/gpt-orig.img" "$W/gpt.img" table >> "$W/boot2.log"
cp "$W/gpt.img" "$W/gpt-installed.img"
FC="$(tr -d '\r' < "$W/boot2.log" | grep -aoE 'type exactly: formatpart nvme yes [0-9a-f]{4}$' | head -1 | awk '{print $NF}')"
echo "  format code: ${FC:-NONE}"
echo "  --- boot 3: the code typed, lsnvme, a file written, formatpart again ---"
boot 5058 "$W/boot3.log" "formatpart nvme yes ${FC:-none}|lsnvme|write notes.txt HELLO-PART-6NOV|sync|formatpart nvme"
printf '\n' >> "$W/boot3.log"; check "$W/gpt-installed.img" "$W/gpt.img" inside >> "$W/boot3.log"
echo "  --- boot 4: a fresh boot: the file back from the partition ---"
boot 5062 "$W/boot4.log" 'cat notes.txt|lsnvme|lsparts'
printf '\n' >> "$W/boot4.log"; check "$W/gpt-installed.img" "$W/gpt.img" outside >> "$W/boot4.log"
cat "$W/boot1.log" "$W/boot2.log" "$W/boot3.log" "$W/boot4.log" | tr -d '\r' |
    grep -aE '^afrisovera> (formatpart|installpart|cat|lsnvme)|^formatpart|^installpart: AfriSovera|^\[nvme\] (AfriSovera|namespace)|^  storage|^HELLO|^harness' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot1.log" "$W/boot2.log" "$W/boot3.log" "$W/boot4.log"; then
    rm -rf "$W"
    echo "PC FORMATPART TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC FORMATPART TEST FAILED"
exit 1
