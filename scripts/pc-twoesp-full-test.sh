#!/usr/bin/env bash
# PC track step 5 (11 Nov 2026): ONE EFI partition shared by Windows and Ubuntu (\EFI holds Microsoft, BOOT, ubuntu,
# as an Ubuntu installed after Windows leaves it) with too little room left for AfriSovera's boot file
# (make-disk-image.py gpt-esp-dual-full: 64 KiB free) on SATA port 4, a blank disk on port 2, a blank GPT disk on NVMe.
# One boot, as scripts/pc-twoesp-test.sh runs them (edk2 with a writable copy of its variable store):
#   bootplan            lists Microsoft, BOOT, ubuntu, counts the 64 KiB free, says the boot file does not fit, names
#                       no partition for it and plans no boot entry;
#   bootcopy            refuses (no EFI partition can take the file) and gives no code;
#   bootcopy yes 0000   refuses the same way, whatever code is typed;
#   bootentry           refuses (no partition holds the file) and gives no code;
#   bootplan            the same as the first time.
# Then the harness compares the image with a fresh copy: it must be byte-identical (nothing at all was written).
# TWOESPFULL_KIND=dual4k (the wrapper scripts/pc-twoesp-full4k-test.sh, 11 Nov 2026): the same shared partition with
# 64 KiB free on an NVMe namespace of 4096-byte blocks (make-disk-image.py gpt-esp-dual-full-4k: LBA 256-66815, 260 MiB,
# 4 KiB clusters), the blank disk on SATA port 2 and the blank GPT disk on port 4; the same boot and the same checks.
#   scripts/pc-twoesp-full-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-twoesp-full-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
KIND="${TWOESPFULL_KIND:-sata}"
case "$KIND" in sata|dual4k) ;; *) echo "PC TWOESP FULL TEST FAILED (harness: unknown TWOESPFULL_KIND $KIND)"; exit 1;; esac
PORT=6890; NAME="sata port 4"; ESPL="2048-206847, 100 MiB"; CLB=512; IMG=gpt-esp-dual-full; UP=""
[ "$KIND" = dual4k ] && { PORT=6930; NAME=nvme; ESPL="256-66815, 260 MiB"; CLB=4096; IMG=gpt-esp-dual-full-4k; UP=" NVME 4K"; }
SAME="harness: $NAME image byte-identical to a fresh copy"

grade() { # grade <boot log>
    local fail=0 A P1 P2 PB
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing log"; return 1; }
    A="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    # the Nth block of output after the command line "afrisovera> <cmd>" (exact), up to the next prompt
    blk() { printf '%s\n' "$A" | awk -v c="afrisovera> $1" -v n="$2" '$0==c{k++;on=(k==n);next} /^afrisovera>/{on=0} on'; }
    in_blk() { grep -aqxE "$2" <<< "$1"; }
    P1="$(blk bootplan 1)"; P2="$(blk bootplan 2)"
    for PB in "$P1" "$P2"; do
        local w; [ "$PB" = "$P1" ] && w="first" || w="second"
        in_blk "$PB" "bootplan: $NAME: EFI system partition \(GPT entry 1, LBA $ESPL\)" ||
            _f "$w bootplan: the shared EFI partition on $NAME was not listed"
        in_blk "$PB" "  FAT32, $CLB-byte clusters, 64 KiB free" || _f "$w bootplan: the 64 KiB free was not counted"
        in_blk "$PB" '  \\EFI holds: Microsoft, BOOT, ubuntu' || _f "$w bootplan: Microsoft, BOOT and ubuntu were not all seen"
        in_blk "$PB" '  plan: none; the boot file needs [1-9][0-9]* KiB here, only 64 KiB free' ||
            _f "$w bootplan: the full partition was not refused for room"
        if printf '%s\n' "$PB" | grep -aq '^  plan: AfriSovera'; then _f "$w bootplan: a copy was planned into the full partition"; fi
        if printf '%s\n' "$PB" | grep -aq '^bootplan: the boot file would go to'; then _f "$w bootplan: a partition was named for the boot file"; fi
        in_blk "$PB" '  boot entry: none planned; no EFI system partition with room' || _f "$w bootplan: a boot entry was planned"
        if printf '%s\n' "$PB" | grep -aq '^bootplan: sata port 2: EFI system partition'; then _f "$w bootplan: an EFI partition was found on the blank disk"; fi
    done
    in_blk "$(blk bootcopy 1)" 'bootcopy: no EFI system partition can take the boot file \(bootplan says why\); nothing written' ||
        _f "bootcopy did not refuse"
    in_blk "$(blk 'bootcopy yes 0000' 1)" 'bootcopy: no EFI system partition can take the boot file \(bootplan says why\); nothing written' ||
        _f "bootcopy yes 0000 did not refuse"
    in_blk "$(blk bootentry 1)" 'bootentry: no EFI system partition holds \\EFI\\afrisovera \(bootcopy puts it there first\); nothing written' ||
        _f "bootentry did not refuse"
    if printf '%s\n' "$A" | grep -aqE '^(bootcopy|bootentry): nothing written yet\. To do it'; then _f "a code was offered for a write"; fi
    if printf '%s\n' "$A" | grep -aqE '^bootcopy: copied to|added at the END of BootOrder and read back'; then _f "something was written"; fi
    printf '%s\n' "$A" | grep -aqxF "$SAME" || _f "the shared disk ($NAME) was changed"
    if printf '%s\n' "$A" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[(nvme|ahci)\] .*(failed|timed out)|FAILED|refused to write|could not be written|did not read back'; then
        _f "the kernel panicked, a disk command or a firmware write failed, or something FAILED"
    fi
    return "$fail"
}

PLAN="bootplan: AfriSovera's boot file is 2360 KiB (kept at power-on)
bootplan: sata port 4: EFI system partition (GPT entry 1, LBA 2048-206847, 100 MiB)
  FAT32, 512-byte clusters, 64 KiB free
  \\EFI holds: Microsoft, BOOT, ubuntu
  plan: none; the boot file needs 2361 KiB here, only 64 KiB free
  boot entry: none planned; no EFI system partition with room
bootplan: a plan only, so nothing was written; adding it will ask for the owner's word, with the way back"
M="afrisovera> bootplan
$PLAN
afrisovera> bootcopy
bootcopy: no EFI system partition can take the boot file (bootplan says why); nothing written
afrisovera> bootcopy yes 0000
bootcopy: no EFI system partition can take the boot file (bootplan says why); nothing written
afrisovera> bootentry
bootentry: no EFI system partition holds \\EFI\\afrisovera (bootcopy puts it there first); nothing written
afrisovera> bootplan
$PLAN
afrisovera>
$SAME"
# the dual4k kind's model: the partition counted in the NVMe disk's own 4 KiB blocks and clusters
[ "$KIND" = dual4k ] && M="$(printf '%s\n' "$M" | sed 's/^bootplan: sata port 4: EFI system partition (GPT entry 1, LBA 2048-206847, 100 MiB)$/bootplan: nvme: EFI system partition (GPT entry 1, LBA 256-66815, 260 MiB)/
s/^  FAT32, 512-byte clusters/  FAT32, 4096-byte clusters/; s/needs 2361 KiB here/needs 2364 KiB here/; s/^harness: sata port 4 image/harness: nvme image/')"

selftest() {
    local D pass=0 bad=0 N=15
    D="$(mktemp -d "$T/pc-twoesp-full-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <log>
        local got
        printf '%s\n' "$3" > "$D/a"
        if grade "$D/a" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm model-green green "$M"
    arm crlf green "$(printf '%s\n' "$M" | sed 's/$/\r/')"
    arm copy-planned red "$(printf '%s\n' "$M" | sed '0,/^  plan: none.*$/s//  plan: AfriSovera'"'"'s boot file would be copied to \\EFI\\afrisovera\\afrisovera.efi (2361 KiB of the 64 KiB free); nothing else in this partition changes/')"
    arm partition-named red "$(printf '%s\n' "$M" | sed '0,/^  boot entry: none planned.*$/s//bootplan: the boot file would go to sata port 4 partition 1 (start 2048), the one Windows starts from; no other EFI partition is changed/')"
    arm entry-planned red "$(printf '%s\n' "$M" | sed 's/^  boot entry: none planned.*$/  boot entry: Boot0005 "AfriSovera" -> sata port 4 partition 1 (start 2048) would be added at the END of BootOrder/')"
    arm ubuntu-unseen red "$(printf '%s\n' "$M" | sed 's/holds: Microsoft, BOOT, ubuntu$/holds: Microsoft, BOOT/')"
    arm free-miscounted red "$(printf '%s\n' "$M" | sed 's/clusters, 64 KiB free$/clusters, 0 KiB free/')"
    arm second-plan-differs red "$(printf '%s\n' "$M" | awk '/^  \\EFI holds/{k++; if(k==2){print "  \\EFI holds: Microsoft, BOOT, ubuntu, afrisovera"; next}} {print}')"
    arm copy-gives-code red "$(printf '%s\n' "$M" | sed '0,/^bootcopy: no EFI.*$/s//bootcopy: nothing written yet. To do it, type exactly: bootcopy yes 3fa2/')"
    arm copy-yes-copied red "$(printf '%s\n' "$M" | awk '/^bootcopy: no EFI/{k++; if(k==2){print "bootcopy: copied to sata port 4 partition 1\\EFI\\afrisovera\\afrisovera.efi, read back the same"; next}} {print}')"
    arm entry-gives-code red "$(printf '%s\n' "$M" | sed 's/^bootentry: no EFI.*$/bootentry: nothing written yet. To do it, type exactly: bootentry yes 51c7/')"
    arm esp-on-blank red "$(printf '%s\n' "$M" | sed '0,/^  boot entry: none planned/s//bootplan: sata port 2: EFI system partition (GPT entry 1, LBA 2048-206847, 100 MiB)\n&/')"
    arm disk-changed red "$(printf '%s\n' "$M" | sed 's/byte-identical to a fresh copy/CHANGED (first difference at byte 1049088)/')"
    arm panic red "$(printf '%s\nKERNEL PANIC at esp.rs' "$M")"
    arm empty red ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq "$N" ]
}

WHERE=""; [ "$KIND" = dual4k ] && WHERE=", NVMe disk of 4096-byte blocks"
echo "== PC with one EFI partition shared by Windows and Ubuntu, too full for the boot file$WHERE: nothing planned, nothing written (PC track step 5) =="
if ! selftest; then echo "PC TWOESP FULL$UP TEST FAILED (grader selftest), no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC TWOESP FULL$UP SELFTEST ONLY, no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC TWOESP FULL$UP TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
PY=python3; command -v python3 > /dev/null || PY=python
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC TWOESP FULL$UP TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
export AFRISH_CMD_QUIET="${AFRISH_CMD_QUIET:-120}" AFRISH_CMD_CEILING="${AFRISH_CMD_CEILING:-180}"

REALQ="$(command -v qemu-system-x86_64)"
QDIR="$(dirname "$REALQ")"
OVMF="${PC_OVMF:-}"; VARS="${PC_OVMF_VARS:-}"
for f in "$QDIR/../share/qemu/edk2-x86_64-code.fd" "$QDIR/share/edk2-x86_64-code.fd" /usr/share/qemu/edk2-x86_64-code.fd; do
    if [ -z "$OVMF" ] && [ -r "$f" ] && [ -r "$(dirname "$f")/edk2-i386-vars.fd" ]; then OVMF="$f"; VARS="$(dirname "$f")/edk2-i386-vars.fd"; fi
done
for f in /usr/share/OVMF/OVMF_CODE_4M.fd /usr/share/OVMF/OVMF_CODE.fd; do
    if [ -z "$OVMF" ] && [ -r "$f" ] && [ -r "${f/CODE/VARS}" ]; then OVMF="$f"; VARS="${f/CODE/VARS}"; fi
done
[ -n "$OVMF" ] && [ -r "$VARS" ] || { echo "PC TWOESP FULL$UP TEST FAILED (harness: no x86-64 UEFI firmware with a variable store file)"; exit 1; }

W="$(mktemp -d "$T/pc-twoesp-full-$KIND.XXXXXX")"
mk() { "$PY" scripts/lib/make-disk-image.py "$1" "$2" || { echo "PC TWOESP FULL$UP TEST FAILED (harness: could not make $2)"; exit 1; }; }
mk "$IMG" "$W/win.img"; mk blank "$W/other.img"; mk gpt "$W/blank.img"
cp "$W/win.img" "$W/win.fresh"
cp "$VARS" "$W/vars.fd"
mkdir -p "$W/stick/EFI/BOOT"
cp "$PC_EFI" "$W/stick/EFI/BOOT/BOOTX64.EFI"

if [ "$KIND" = dual4k ]; then # the kernel drives one NVMe controller: the shared partition on it, the blank GPT disk on port 4
    DEV=(-drive "file=$(winpath "$W/win.img"),if=none,format=raw,id=dw"
        -device nvme,serial=afri02,drive=dw,logical_block_size=4096,physical_block_size=4096
        -drive "file=$(winpath "$W/other.img"),if=none,format=raw,id=dl" -device ide-hd,drive=dl,bus=ide.2
        -drive "file=$(winpath "$W/blank.img"),if=none,format=raw,id=db" -device ide-hd,drive=db,bus=ide.4)
else
    DEV=(-drive "file=$(winpath "$W/blank.img"),if=none,format=raw,id=dn" -device nvme,serial=afri01,drive=dn
        -drive "file=$(winpath "$W/other.img"),if=none,format=raw,id=dl" -device ide-hd,drive=dl,bus=ide.2
        -drive "file=$(winpath "$W/win.img"),if=none,format=raw,id=dw" -device ide-hd,drive=dw,bus=ide.4)
fi
echo "  --- boot: bootplan, bootcopy, bootcopy yes 0000, bootentry, bootplan ---"
"$REALQ" -M q35 -m 256M -display none \
    -drive "if=pflash,format=raw,unit=0,readonly=on,file=$(winpath "$OVMF")" \
    -drive "if=pflash,format=raw,unit=1,file=$(winpath "$W/vars.fd")" \
    -drive "format=raw,file=fat:rw:$(winpath "$W/stick")" \
    "${DEV[@]}" \
    -serial "tcp:127.0.0.1:$PORT,server,nowait" -qmp "tcp:127.0.0.1:$((PORT+1)),server,nowait" &
Q=$!
AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py "$PORT" "$((PORT+1))" "$W/shot.png" \
    bootplan bootcopy 'bootcopy yes 0000' bootentry bootplan > "$W/boot.log" 2>&1
kill "$Q" 2> /dev/null
wait "$Q" 2> /dev/null
printf '\n' >> "$W/boot.log"   # the log ends at the shell's prompt, with no newline
if cmp -s "$W/win.img" "$W/win.fresh"; then echo "$SAME" >> "$W/boot.log"
else echo "harness: $NAME image CHANGED ($(cmp "$W/win.img" "$W/win.fresh" 2>&1 | head -1))" >> "$W/boot.log"; fi
tr -d '\r' < "$W/boot.log" |
    grep -aE '^afrisovera> boot|^bootplan: |^bootentr|^bootcopy: |^  (FAT32|\\EFI holds|plan:|boot entry)|^harness' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot.log"; then
    rm -rf "$W"
    echo "PC TWOESP FULL$UP TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC TWOESP FULL$UP TEST FAILED"
exit 1
