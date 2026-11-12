#!/usr/bin/env bash
# VFS files up to 16 KiB (claude/vfs-big): a 3,000-byte Nia app and a 5,000-byte text file live on the
# disk, survive a sync and a second boot, and every classic command still sees 512 bytes. Also: an image of
# small files only stays in the old format, and damaged images are clamped, never read past a slot.
# The disk layout is kernel/src/vfs.rs's; scripts/lib/mkvfs-big.py writes and checks it. Needs python3.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

T="${TMPDIR:-/tmp}/persist-big"
rm -rf "$T"; mkdir -p "$T"
M="python3 scripts/lib/mkvfs-big.py"
$M bigwasm "$T/big.wasm"
$M bigtext "$T/big.txt"
printf 'HELLO-SMALL' > "$T/note.txt"
FILES=(big.wasm="$T/big.wasm" big.txt="$T/big.txt" note.txt="$T/note.txt")

boot() { # $1 = image, $2 = serial port (qmp = +1), rest = shell commands
    local img=$1 sp=$2; shift 2
    qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
        -global virtio-mmio.force-legacy=false -device ramfb -display none \
        -drive "file=$img,if=none,format=raw,id=d0" -device virtio-blk-device,drive=d0 \
        -serial "tcp:127.0.0.1:$sp,server,nowait" \
        -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" -kernel "$BIN" &
    local qpid=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" \
    python3 scripts/shell-demo.py "$sp" "$((sp+1))" "$T/shot.png" "$@" 2>&1
    kill "$qpid" 2>/dev/null
    wait "$qpid" 2>/dev/null
}
fail() { echo "PERSIST-BIG TEST FAILED ($1)"; exit 1; }
nopanic() { grep -aq "KERNEL PANIC\|panicked" <<< "$1" && fail "$2: kernel panic"; return 0; }

echo "[boot 1] big files from the disk: run the app, cat the text, df, sync"
$M image "$T/a.img" "${FILES[@]}"
B1="$(boot "$T/a.img" 4720 "nia big.wasm" "cat big.txt" "df" "sync")"
echo "$B1" | grep -aE "restored|print\(|BIG-FILE|^df:|synced|vfs\]" | sed 's/^/  /'
nopanic "$B1" "boot 1"
grep -aq "VFS restored from disk: 3 files" <<< "$B1" || fail "boot 1: the disk was not loaded"
grep -aq "print(4242)" <<< "$B1" || fail "boot 1: big.wasm did not run"
grep -aq "print(90)" <<< "$B1" || fail "boot 1: big.wasm's last byte (past 512) did not arrive"
grep -aq "BIG-FILE-START" <<< "$B1" || fail "boot 1: cat big.txt shows nothing"
grep -aq "BIG-FILE-END" <<< "$B1" && fail "boot 1: cat showed past 512 bytes (classic view must stay 512)"
python3 - "$B1" <<'PY' || fail "boot 1: df does not add up"
import re, sys
m = re.search(r"df: (\d+)/24 files, (\d+) bytes used of (\d+) \((\d+) free\)", sys.argv[1])
assert m, "no df line"
files, used, cap, free = map(int, m.groups())
assert cap == 24 * 16384 and used + free == cap and used >= 8011, (files, used, cap, free)
PY
grep -aq "synced" <<< "$B1" || fail "boot 1: sync did not run"
$M check "$T/a.img" "${FILES[@]}" || fail "boot 1: the synced disk does not hold the files"

echo "[boot 2] the same disk again: the app still runs whole"
B2="$(boot "$T/a.img" 4726 "nia big.wasm")"
nopanic "$B2" "boot 2"
grep -aq "print(90)" <<< "$B2" || fail "boot 2: big.wasm did not survive the sync"

echo "[boot 3] small files only: the old format, no flag, no extension"
$M image "$T/s.img" note.txt="$T/note.txt"
B3="$(boot "$T/s.img" 4736 "write more.txt SMALL-TWO" "sync")"
nopanic "$B3" "boot 3"
grep -aq "synced" <<< "$B3" || fail "boot 3: sync did not run"
printf 'SMALL-TWO' > "$T/more.txt"
$M smallcheck "$T/s.img" || fail "boot 3: a small-files disk changed format"
$M check "$T/s.img" note.txt="$T/note.txt" || fail "boot 3: note.txt not saved as before"

echo "[boot 4] damaged lengths (dlen and true length 60,000): clamped, no panic"
$M image "$T/d.img" --damage dlen "${FILES[@]}"
B4="$(boot "$T/d.img" 4744 "ls" "cat big.txt" "df" "nia big.wasm" "mem")"
echo "$B4" | grep -aE "restored|print\(|^df:|heap:" | sed 's/^/  /'
nopanic "$B4" "boot 4"
grep -aq "heap:" <<< "$B4" || fail "boot 4: shell unresponsive after a damaged image"
grep -aq "BIG-FILE-END" <<< "$B4" && fail "boot 4: cat showed past 512 bytes"

echo "[boot 5] big flag on a disk too small for the extension: the 512-byte prefix and a line"
$M image "$T/t.img" --sectors 200 big.txt="$T/big.txt"
B5="$(boot "$T/t.img" 4764 "cat big.txt" "mem")"
nopanic "$B5" "boot 5"
grep -aq "could not be read; keeping the first 512 bytes" <<< "$B5" || fail "boot 5: no fallback line"
grep -aq "BIG-FILE-START" <<< "$B5" || fail "boot 5: the prefix is gone"
grep -aq "heap:" <<< "$B5" || fail "boot 5: shell unresponsive"

echo "[boot 6] an app opens a 1-pixel-wide window (Tonga's width floor, afriui.md section 19): no fault, shell answers"
$M thinwasm "$T/thin.wasm" 1
$M image "$T/w.img" thin.wasm="$T/thin.wasm"
B6="$(boot "$T/w.img" 4768 "nia thin.wasm" "mem")"
echo "$B6" | grep -aE "print\(|heap:|\[mansa\]" | sed 's/^/  /'
nopanic "$B6" "boot 6"
grep -aqE "\[mansa\] (SYNC EXCEPTION|unhandled exception|halting)" <<< "$B6" && fail "boot 6: the 1-pixel window faulted the kernel"
grep -aq "print(7777)" <<< "$B6" || fail "boot 6: the app did not run to its end"
grep -aq "heap:" <<< "$B6" || fail "boot 6: shell unresponsive after a 1-pixel window"

echo "PERSIST-BIG TEST PASSED  (16 KiB files persist and run; classic view 512; small format unchanged; damage clamped; 1-pixel window safe)"
