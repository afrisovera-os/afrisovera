#!/usr/bin/env bash
# PC track step 3 (7 Nov 2026): SD cards in a PC's card reader (SD Host Controller, SDHCI).
#
# Boots the PC kernel (q35 + UEFI) four times with QEMU's `sdhci-pci` card reader. Boots 1-2:
# a blank 8 MiB standard-capacity (SDSC, byte-addressed) card; boots 3-4: a blank 4 GiB
# high-capacity (SDHC, block-addressed) card, so both ways of addressing a card are proved.
# In boot 1 and 3 the card must be found, named and sized, and NOT taken on its own (a card is
# used only once the owner types `claimdisk sd`, or when it already carries our marker); after
# `claimdisk sd` a file is written and synced; boots 2 and 4 must recognise the card as ours
# and read the file back from it. `lssd` must show the controller and the card.
# Runs the same in an ARM walk and a PC walk: it builds the PC kernel and puts the PC shim
# (scripts/lib/pc-arch) on PATH itself.
#   scripts/pc-sdhci-test.sh              # grader selftest, then the four QEMU boots
#   scripts/pc-sdhci-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

grade() { # grade <boot1> <boot2> <boot3> <boot4>
    local fail=0 A B C D
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] && [ -r "$2" ] && [ -r "$3" ] && [ -r "$4" ] || { echo "  FAIL: missing log"; return 1; }
    A="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    B="$(tr -d '\r' < "$2" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    C="$(tr -d '\r' < "$3" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    D="$(tr -d '\r' < "$4" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { printf '%s\n' "$1" | grep -aqE "$2"; }
    local CTL='^\[sdhci\] controller [0-9a-f]{2}:[0-9a-f]{2}\.[0-7] \(1b36:0007, SDHCI [1-4]\.00, base clock [0-9]+ MHz, 1 slot\(s\)\)$'
    has "$A" "$CTL" || _f "boot 1: the SDHCI controller line is missing or wrong"
    has "$C" "$CTL" || _f "boot 3: the SDHCI controller line is missing or wrong"
    # SDSC card, boots 1-2
    has "$A" '^\[sdhci\] card "[^"]+": SDSC, 16384 sectors \(8 MiB\), blank: not used unless claimed \(claimdisk sd\)$' ||
        _f "boot 1: the blank 8 MiB SDSC card was not found, sized, or kept unused until claimed"
    has "$A" '^sd: controller [0-9a-f]{2}:[0-9a-f]{2}\.[0-7] \(1b36:0007\) registers 0x[0-9a-f]+, SDHCI [1-4]\.00' ||
        _f "lssd does not show the controller"
    has "$A" '^  card "[^"]+" SDSC RCA 0x[0-9a-f]{4}: 16384 sectors \(8 MiB\), write-protect switch off  \(blank: claimdisk sd to use it\)$' ||
        _f "lssd does not show the SDSC card as blank and unclaimed"
    has "$A" '^claimdisk: the blank sd disk now holds AfriSovera.s files$' || _f "boot 1: claimdisk sd did not take the blank card"
    has "$A" '^\[vfs\] synced [0-9]+ files to disk$' || _f "boot 1: the VFS never synced to the SD card"
    has "$B" '^\[sdhci\] card "[^"]+": SDSC, 16384 sectors \(8 MiB\), ours \(AFRIVFS1\): used for storage$' ||
        _f "boot 2: the claimed SDSC card was not recognised as ours"
    has "$B" '^HELLO-SDSC-7NOV$' || _f "boot 2: notes.txt did not come back from the SDSC card"
    # SDHC card, boots 3-4
    has "$C" '^\[sdhci\] card "[^"]+": SDHC/SDXC, 8388608 sectors \(4096 MiB\), blank: not used unless claimed \(claimdisk sd\)$' ||
        _f "boot 3: the blank 4 GiB SDHC card was not found, sized, or kept unused until claimed"
    has "$C" '^claimdisk: the blank sd disk now holds AfriSovera.s files$' || _f "boot 3: claimdisk sd did not take the blank SDHC card"
    has "$C" '^\[vfs\] synced [0-9]+ files to disk$' || _f "boot 3: the VFS never synced to the SDHC card"
    has "$D" '^\[sdhci\] card "[^"]+": SDHC/SDXC, 8388608 sectors \(4096 MiB\), ours \(AFRIVFS1\): used for storage$' ||
        _f "boot 4: the claimed SDHC card was not recognised as ours"
    has "$D" '^HELLO-SDHC-7NOV$' || _f "boot 4: notes.txt did not come back from the SDHC card"
    local ALL; ALL="$(printf '%s\n%s\n%s\n%s\n' "$A" "$B" "$C" "$D")"
    if has "$ALL" '^\[sdhci\] .*(failed|not sent|never|did not|could not)'; then
        _f "an SD command failed: $(printf '%s\n' "$ALL" | grep -aE '^\[sdhci\] .*(failed|not sent|never|did not|could not)' | head -1)"
    fi
    if has "$ALL" 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)'; then
        _f "the kernel panicked or took an exception"
    fi
    return "$fail"
}

S1='[sdhci] controller 00:03.0 (1b36:0007, SDHCI 2.00, base clock 52 MHz, 1 slot(s))
[sdhci] card "QEMU!": SDSC, 16384 sectors (8 MiB), blank: not used unless claimed (claimdisk sd)
afrisovera> lssd
sd: controller 00:03.0 (1b36:0007) registers 0x81085000, SDHCI 2.00, base clock 52 MHz
  card "QEMU!" SDSC RCA 0x4567: 16384 sectors (8 MiB), write-protect switch off  (blank: claimdisk sd to use it)
afrisovera> claimdisk sd
claimdisk: the blank sd disk now holds AfriSovera'"'"'s files
[vfs] synced 17 files to disk
afrisovera> write notes.txt HELLO-SDSC-7NOV
[vfs] synced 18 files to disk'
S2='[sdhci] controller 00:03.0 (1b36:0007, SDHCI 2.00, base clock 52 MHz, 1 slot(s))
[sdhci] card "QEMU!": SDSC, 16384 sectors (8 MiB), ours (AFRIVFS1): used for storage
afrisovera> cat notes.txt
HELLO-SDSC-7NOV'
S3='[sdhci] controller 00:03.0 (1b36:0007, SDHCI 2.00, base clock 52 MHz, 1 slot(s))
[sdhci] card "QEMU!": SDHC/SDXC, 8388608 sectors (4096 MiB), blank: not used unless claimed (claimdisk sd)
afrisovera> claimdisk sd
claimdisk: the blank sd disk now holds AfriSovera'"'"'s files
[vfs] synced 17 files to disk
afrisovera> write notes.txt HELLO-SDHC-7NOV
[vfs] synced 18 files to disk'
S4='[sdhci] controller 00:03.0 (1b36:0007, SDHCI 2.00, base clock 52 MHz, 1 slot(s))
[sdhci] card "QEMU!": SDHC/SDXC, 8388608 sectors (4096 MiB), ours (AFRIVFS1): used for storage
afrisovera> cat notes.txt
HELLO-SDHC-7NOV'

selftest() {
    local Dd pass=0 bad=0
    Dd="$(mktemp -d "$T/pc-sdhci-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <b1> <b2> <b3> <b4>
        local got
        printf '%s\n' "$3" > "$Dd/1"; printf '%s\n' "$4" > "$Dd/2"; printf '%s\n' "$5" > "$Dd/3"; printf '%s\n' "$6" > "$Dd/4"
        if grade "$Dd/1" "$Dd/2" "$Dd/3" "$Dd/4" > "$Dd/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$Dd/log"; fi
    }
    crlf() { printf '%s\n' "$1" | sed 's/$/\r/'; }
    echo "  --- grader selftest (no QEMU) ---"
    arm sample green "$S1" "$S2" "$S3" "$S4"
    arm crlf green "$(crlf "$S1")" "$(crlf "$S2")" "$(crlf "$S3")" "$(crlf "$S4")"
    arm no-controller red "$(printf '%s\n' "$S1" | grep -v '^\[sdhci\] controller')" "$S2" "$S3" "$S4"
    arm no-card red "$(printf '%s\n' "$S1" | sed 's/^\[sdhci\] card.*$/[sdhci] no card in the slot/')" "$S2" "$S3" "$S4"
    arm wrong-size red "$(printf '%s\n' "$S1" | sed 's/^\(\[sdhci\] card.*\)16384 sectors (8 MiB)/\18192 sectors (4 MiB)/')" "$S2" "$S3" "$S4"
    arm blank-taken-unasked red "$(printf '%s\n' "$S1" | sed 's/blank: not used unless claimed (claimdisk sd)/ours (AFRIVFS1): used for storage/')" "$S2" "$S3" "$S4"
    arm not-claimed red "$(printf '%s\n' "$S1" | grep -v '^claimdisk:')" "$S2" "$S3" "$S4"
    arm no-sync red "$(printf '%s\n' "$S1" | grep -v synced)" "$S2" "$S3" "$S4"
    arm sdsc-not-ours red "$S1" "$(printf '%s\n' "$S2" | sed 's/ours (AFRIVFS1): used for storage/blank: not used unless claimed (claimdisk sd)/')" "$S3" "$S4"
    arm sdsc-not-persisted red "$S1" "$(printf '%s\n' "$S2" | grep -v '^HELLO')" "$S3" "$S4"
    arm echoed-only red "$S1" "$(printf '%s\n' "$S2" | sed 's/^HELLO-SDSC-7NOV$/afrisovera> cat notes.txt HELLO-SDSC-7NOV/')" "$S3" "$S4"
    arm lssd-missing red "$(printf '%s\n' "$S1" | grep -v '^  card\|^sd:')" "$S2" "$S3" "$S4"
    arm sdhc-as-sdsc red "$S1" "$S2" "$(printf '%s\n' "$S3" | sed 's#SDHC/SDXC#SDSC#')" "$S4"
    arm sdhc-wrong-size red "$S1" "$S2" "$(printf '%s\n' "$S3" | sed 's/8388608 sectors (4096 MiB)/2097152 sectors (1024 MiB)/')" "$S4"
    arm sdhc-not-claimed red "$S1" "$S2" "$(printf '%s\n' "$S3" | grep -v '^claimdisk:')" "$S4"
    arm sdhc-not-ours red "$S1" "$S2" "$S3" "$(printf '%s\n' "$S4" | sed 's/ours (AFRIVFS1): used for storage/holds other data (a camera or phone card?): left alone/')"
    arm sdhc-not-persisted red "$S1" "$S2" "$S3" "$(printf '%s\n' "$S4" | grep -v '^HELLO')"
    arm swapped-files red "$S1" "$(printf '%s\n' "$S2" | sed 's/HELLO-SDSC/HELLO-SDHC/')" "$S3" "$(printf '%s\n' "$S4" | sed 's/HELLO-SDHC/HELLO-SDSC/')"
    arm command-failed red "$S1" "$S2" "$S3" "$(printf '%s\n[sdhci] CMD24 arg 0x200 failed (status 0x8000, error 0x10)' "$S4")"
    arm panic red "$S1" "$S2" "$(printf '%s\nKERNEL PANIC at sdhci.rs' "$S3")" "$S4"
    arm empty red "" "" "" ""
    rm -rf "$Dd"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 21 ]
}

echo "== PC SD card reader / SDHCI (PC track step 3) =="
if ! selftest; then echo "PC SDHCI TEST FAILED (grader selftest) -- no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC SDHCI SELFTEST ONLY -- no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC SDHCI TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC SDHCI TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-sdhci.XXXXXX")"
head -c 8388608 /dev/zero > "$W/sdsc.img"            # a fresh, blank 8 MiB card (SDSC: QEMU makes cards up to 2 GiB standard capacity)
truncate -s 4G "$W/sdhc.img" 2> /dev/null ||
    { echo "PC SDHCI TEST FAILED (harness: could not make the 4 GiB sparse card image)"; rm -rf "$W"; exit 1; }

boot() { # boot <card image> <serial port> <log> <shell commands...>
    local img=$1 sp=$2 log=$3; shift 3
    qemu-system-aarch64 -M virt -m 256M -display none \
        -device sdhci-pci -drive "file=$img,if=none,format=raw,id=card" -device sd-card,drive=card \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py "$sp" "$((sp+1))" "$W/shot.png" "$@" > "$log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
}
echo "  --- boot 1: 8 MiB SDSC card: lssd, claimdisk sd, write notes.txt, sync ---"
boot "$W/sdsc.img" 5014 "$W/boot1.log" "lssd" "claimdisk sd" "write notes.txt HELLO-SDSC-7NOV" "sync"
echo "  --- boot 2: the same card, cat notes.txt ---"
boot "$W/sdsc.img" 5016 "$W/boot2.log" "cat notes.txt"
echo "  --- boot 3: 4 GiB SDHC card: lssd, claimdisk sd, write notes.txt, sync ---"
boot "$W/sdhc.img" 5018 "$W/boot3.log" "lssd" "claimdisk sd" "write notes.txt HELLO-SDHC-7NOV" "sync"
echo "  --- boot 4: the same card, cat notes.txt ---"
boot "$W/sdhc.img" 5020 "$W/boot4.log" "cat notes.txt"
cat "$W"/boot[1-4].log | tr -d '\r' | grep -aE '^\[sdhci\]|^sd:|^  card|^claimdisk|^\[vfs\]|HELLO' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot1.log" "$W/boot2.log" "$W/boot3.log" "$W/boot4.log"; then
    rm -rf "$W"
    echo "PC SDHCI TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC SDHCI TEST FAILED"
exit 1
