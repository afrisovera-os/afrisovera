#!/usr/bin/env bash
# wasm-file16 (claude/nia-a2, A2, NIA_A2_PLAN.md and the architect's three changes): Nia apps read and write files
# up to 16 KiB, a save reaches the disk, and a save cut part way is never passed off as the file. The app is
# filetest.wasm (scripts/lib/file-wasm.py), bundled only in test images (cargo feature test-apps, built into
# target/test-apps so no other suite sees it); a disk app never gets the file permission. Boots, one disk image:
#   1  w: 16384 written, 16384 read back, checksum A; the save's line says "on disk in N us";
#      x: 16,385 bytes refused (-3), the old file read back whole (16384, checksum A);
#      o: missing -1, cap 0 gives 0, cap 600 gives 600, afrisovera.conf -4, hello.wasm -4, then a write past the
#      app's memory is a TRAP (99 never printed); `ls -l` shows big.txt at 16384; the disk copy filetest2.wasm is
#      DENIED at file_write.
#   2  the same image after a power-off: r reads big.txt back whole (16384, checksum A).
#   3  v saves cut.txt over and over (pattern A, then B); QEMU is killed (SIGKILL) in the middle, three times.
#   4  after each cut, k reads cut.txt: it is damaged (-5, named at boot, marked in ls) or whole (checksum A or B),
#      never anything else. The counts of each are printed.
#   5  no disk: w writes 16384 and the line says kept in memory.
#   6  A5, file_list: a disk of 24 files with 24-byte names (the VFS full), one of them damaged (its checksum
#      wrong). l lists all 24 in slot order, 600 bytes, the damaged one among them (boot names it); with one byte
#      too few it gives -600 and writes nothing; a list past the app's memory is a TRAP.
# No panic. Runs on the PC kernel through the pc-arch shim as it is.
set -uo pipefail

python3 scripts/lib/embedded-same.py scripts/lib/file-wasm.py kernel/src/nia.rs || { echo "WASM-FILE16 TEST FAILED (the kernel's copy of the test app is stale)"; exit 1; }
export CARGO_TARGET_DIR="$PWD/target/test-apps"
BIN="$CARGO_TARGET_DIR/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel --features test-apps 2>&1 | tail -1
if [ -n "${PC_EFI:-}" ]; then # on the PC through the shim: the test-apps PC kernel
    cargo build -p mansa-kernel --bin mansa-pc --features pc,test-apps --target x86_64-unknown-uefi 2>&1 | tail -1
    export PC_EFI="$CARGO_TARGET_DIR/x86_64-unknown-uefi/debug/mansa-pc.efi"
fi

T="${TMPDIR:-/tmp}/wasm-file16"
rm -rf "$T"; mkdir -p "$T/shots"
read -r _ _ _ _ SUM_A _ SUM_B < <(python3 scripts/lib/file-wasm.py "$T/m") || { echo "WASM-FILE16 TEST FAILED (generator)"; exit 1; }
IMG="$T/disk.img"
python3 scripts/lib/mkvfs-big.py image "$IMG" --sectors 32768 filetest2.wasm="$T/m/filetest.wasm" > /dev/null ||
    { echo "WASM-FILE16 TEST FAILED (disk image)"; exit 1; }
FAILS=0
fail() { echo "  FAIL: $1"; FAILS=$((FAILS + 1)); }

portfree() { # wait (at most 15 s) until nothing holds the TCP ports $1 and $2, so a new QEMU can bind them
    local k
    for k in $(seq 60); do
        python3 -c 'import socket, sys
for p in sys.argv[1:]:
    s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); s.bind(("127.0.0.1", int(p))); s.close()' "$1" "$2" 2> /dev/null && return 0
        sleep 0.25
    done
    echo "  (ports $1 and $2 still busy after 15 s)"
}
boot() { # boot <name> <disk: 1 or 0> <kill: 0, or seconds after the last step to SIGKILL QEMU> STEP ...
    local n="$1" disk="$2" cut="$3"; shift 3
    local dev=() k # k: the stop loop below; the caller loops on its own k
    [ "$disk" = 1 ] && dev=(-drive "file=$IMG,if=none,format=raw,id=d0" -device virtio-blk-device,drive=d0)
    portfree 5176 5177
    qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
        -global virtio-mmio.force-legacy=false -device ramfb -display none "${dev[@]}" \
        -serial tcp:127.0.0.1:5176,server,nowait \
        -qmp tcp:127.0.0.1:5177,server,nowait -kernel "$BIN" &
    local q=$!
    python3 scripts/lib/cell-keys.py 5176 5177 "$T/shots" "$@" > "$T/$n.out" 2>&1
    # stop QEMU itself, by its serial port (through the PC shim $q is the shim, not QEMU), and wait until it is gone
    if [ "$cut" != 0 ]; then
        sleep "$cut"
        pkill -9 -f "tcp:127.0.0.1:5176," 2> /dev/null # a power cut: the sectors QEMU had written, nothing more
    else
        pkill -f "tcp:127.0.0.1:5176," 2> /dev/null
    fi
    for k in $(seq 20); do pgrep -f "tcp:127.0.0.1:5176," > /dev/null || break; sleep 0.25; done
    pkill -9 -f "tcp:127.0.0.1:5176," 2> /dev/null; kill "$q" 2> /dev/null; wait "$q" 2> /dev/null
    tr -d '\r' < "$T/$n.out" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g; s/^[[:space:]]*//' > "$T/$n.txt"
    grep -aq "KERNEL PANIC\|panicked" "$T/$n.txt" && fail "boot $n: kernel panic"
    grep -aE "^afrisovera> |\[wasm app\]|^\[vfs\]|^\[nia\] TRAP|DENIED" "$T/$n.txt" | sed "s/^/  $n | /" | head -40
    true
}
seg() { awk -v c="afrisovera> $2" -v n="${3:-1}" 'index($0, c) == 1 && length($0) == length(c) {k++; if (k == n) {on=1; next}}
        on && index($0, "afrisovera>") {exit} on' "$T/$1.txt"; }
prints() { grep -aoE '\[wasm app\] print\(-?[0-9]+\)' | grep -oE '[-]?[0-9]+' | tr '\n' ' ' | sed 's/ $//'; }

# --- 1: write, refuse, edges, permission
boot 1 1 0 "&nia filetest.wasm" ":77" "=" "&nia filetest.wasm" ":78" "=" "&nia filetest.wasm" ":6f" "=" "ls -l" \
    "&nia filetest2.wasm" ":77" "=" "mem"
S="$(seg 1 "nia filetest.wasm" 1)"
[ "$(printf '%s\n' "$S" | prints)" = "16384 16384 $SUM_A" ] || fail "w: printed [$(printf '%s\n' "$S" | prints)], not [16384 16384 $SUM_A]"
grep -aqE "file_write\('big\.txt', 16384 bytes\) -> ok, on disk in [0-9]+ us$" <<< "$S" || fail "w: no on-disk save line"
printf '%s\n' "$S" | grep -aoE "on disk in [0-9]+ us" | sed 's/^/  save time: /'
S="$(seg 1 "nia filetest.wasm" 2)"
[ "$(printf '%s\n' "$S" | prints)" = "-3 16384 $SUM_A" ] || fail "x: printed [$(printf '%s\n' "$S" | prints)], not [-3 16384 $SUM_A]"
S="$(seg 1 "nia filetest.wasm" 3)"
[ "$(printf '%s\n' "$S" | prints)" = "-1 0 600 -4 -4" ] || fail "o: printed [$(printf '%s\n' "$S" | prints)], not [-1 0 600 -4 -4]"
grep -aqxF "[nia] TRAP: out of bounds memory access" <<< "$S" || fail "o: no out-of-bounds TRAP"
seg 1 "ls -l" | grep -aqE "^16384  .*  big\.txt$" || fail "ls -l: big.txt is not 16384 bytes"
seg 1 "nia filetest2.wasm" | grep -aqF "DENIED: app called host 'file_write' without the capability" || fail "the disk copy was not DENIED"

# --- 2: after a power-off, the same image
boot 2 1 0 "&nia filetest.wasm" ":72" "=" "mem"
[ "$(seg 2 "nia filetest.wasm" | prints)" = "16384 $SUM_A" ] || fail "after a reboot: printed [$(seg 2 "nia filetest.wasm" | prints)], not [16384 $SUM_A]"

# --- 3 and 4: saves cut part way, three times
dam=0; old=0
for k in 1 2 3; do
    boot "cut$k" 1 0.$((2 + 3 * k)) "&nia filetest.wasm" ":76" "~1.5"
    boot "after$k" 1 0 "ls" "&nia filetest.wasm" ":6b" "=" "mem"
    got="$(seg "after$k" "nia filetest.wasm" | prints)"
    case "$got" in
        "-5 "*)
            dam=$((dam + 1))
            grep -aqxF "[vfs] cut.txt was damaged by a cut during a save; its bytes are kept as cut.txt and marked damaged" "$T/after$k.txt" ||
                fail "cut $k: damaged, but boot did not name it"
            seg "after$k" "ls" | grep -aqE "cut\.txt  \(damaged by a cut during a save\)$" || fail "cut $k: ls does not mark it"
            # every boot saves the whole VFS (the boot counter): the mark must outlive that save, never a fresh checksum
            boot "again$k" 1 0 "&nia filetest.wasm" ":6b" "=" "mem"
            grep -aqxF "[vfs] cut.txt is still marked damaged (a cut during an earlier save)" "$T/again$k.txt" ||
                fail "cut $k: the next boot lost the damaged mark"
            [ "$(seg "again$k" "nia filetest.wasm" | prints | cut -d' ' -f1)" = "-5" ] || fail "cut $k: read whole the boot after"
            ;;
        "16384 $SUM_A" | "16384 $SUM_B") old=$((old + 1)) ;;
        *) fail "cut $k: cut.txt read as [$got]: neither damaged nor a whole save" ;;
    esac
    # the next round's saves of cut.txt replace it (a fresh write clears the mark)
done
echo "  saves cut part way: $dam found damaged, $old whole (old or new), 0 silent mixes allowed"

# --- 5: no disk
boot 5 0 0 "&nia filetest.wasm" ":77" "=" "mem"
seg 5 "nia filetest.wasm" | grep -aqE "file_write\('big\.txt', 16384 bytes\) -> ok \(no disk: kept in memory\)$" || fail "no disk: no kept-in-memory line"

# --- 6: A5, file_list on a full VFS with a damaged file
LIMG="$T/list.img"
NAMES=()
for k in $(seq -f %03g 1 24); do
    n="list-file-number-$k.txt" # 24 bytes: 17 + 3 + 4
    printf 'file %s\n' "$k" > "$T/m/$n"
    NAMES+=("$n=$T/m/$n")
done
python3 scripts/lib/mkvfs-big.py image "$LIMG" --sectors 32768 "${NAMES[@]}" > /dev/null || fail "list image"
# slot 3's meta: a checksum mark with a wrong checksum, so the kernel finds it damaged at load
python3 - "$LIMG" << 'PY'
import sys
f = open(sys.argv[1], "r+b")
f.seek((1 + 2 * 3) * 512 + 41); f.write(b"\0\0\0\0\xC1")
PY
IMG_SAVED="$IMG"; IMG="$LIMG"
boot 6 1 0 "&nia filetest.wasm" ":6c" "=" "mem"
IMG="$IMG_SAVED"
S="$(seg 6 "nia filetest.wasm")"
[ "$(printf '%s\n' "$S" | prints)" = "600 -600 85" ] || fail "l: printed [$(printf '%s\n' "$S" | prints)], not [600 -600 85]"
grep -aqxF "[nia] TRAP: out of bounds memory access" <<< "$S" || fail "l: no TRAP for a list past memory"
want="$(for k in $(seq -f %03g 1 24); do printf 'list-file-number-%s.txt\n' "$k"; done)"
# putc lines come as "[wasm app] <text>"
got="$(printf '%s\n' "$S" | sed -nE 's/^\[wasm app\] (list-file-number-[0-9]{3}\.txt)$/\1/p')"
[ "$got" = "$want" ] || fail "l: the listing is not the 24 names in slot order"
grep -aqxF "[vfs] list-file-number-004.txt was damaged by a cut during a save; its bytes are kept as list-file-number-004.txt and marked damaged" "$T/6.txt" ||
    fail "l: boot did not name the damaged file"

[ "$FAILS" -eq 0 ] || { echo "WASM-FILE16 TEST FAILED ($FAILS check(s))"; exit 1; }
echo "WASM-FILE16 TEST PASSED"
