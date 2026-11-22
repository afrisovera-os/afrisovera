#!/usr/bin/env bash
# wasm-stream (claude/nia-a6, A6, NIA_A6_PLAN.md and the architect's OK of 16 Nov): Nia apps read a file in pieces
# and write one through a stream that only the commit turns into the file. The app is streamtest.wasm
# (scripts/lib/stream-wasm.py), bundled only in test images (feature test-apps, target/test-apps). Two boots, one disk:
#   1  w: 16 KiB written in 1 KiB appends (16384), committed (16384, the save's line), file_size 16384, read back in
#      1 KiB pieces with file_read_at (16384 in all, checksum A), 0 at the end, -3 for a negative offset;
#      x: a second stream past 16 KiB is spoiled (-3), its commit is -3, and s.txt still reads 16384 and checksum A;
#      c: a stream is opened and 1 KiB appended, then Ctrl+C stops the app: exactly one "dropped: not committed"
#      line, and the TRAP;
#      r: s.txt is still 16384 bytes, checksum A (the Ctrl+C left the old file);
#      d: a second open drops the first stream with its line; b.txt is 5 bytes and a.txt was never written (-1);
#      s: afrisovera.conf and hello.wasm are refused at open (-4); append and commit with no stream give -1;
#      `ls -l` shows s.txt at 16384 and b.txt at 5.
#   2  the same image after a power-off: r reads s.txt back whole (16384 16384 checksum A).
# No panic. Runs on the PC kernel through the pc-arch shim as it is.
set -uo pipefail

python3 scripts/lib/embedded-same.py scripts/lib/stream-wasm.py kernel/src/nia.rs || { echo "WASM-STREAM TEST FAILED (the kernel's copy of the test app is stale)"; exit 1; }
export CARGO_TARGET_DIR="$PWD/target/test-apps"
BIN="$CARGO_TARGET_DIR/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel --features test-apps 2>&1 | tail -1
if [ -n "${PC_EFI:-}" ]; then # on the PC through the shim: the test-apps PC kernel
    cargo build -p mansa-kernel --bin mansa-pc --features pc,test-apps --target x86_64-unknown-uefi 2>&1 | tail -1
    export PC_EFI="$CARGO_TARGET_DIR/x86_64-unknown-uefi/debug/mansa-pc.efi"
fi

T="${TMPDIR:-/tmp}/wasm-stream"
rm -rf "$T"; mkdir -p "$T/shots"
read -r _ _ _ _ SUM_A _ _ < <(python3 scripts/lib/stream-wasm.py "$T/m") || { echo "WASM-STREAM TEST FAILED (generator)"; exit 1; }
IMG="$T/disk.img"
printf 'x\n' > "$T/x.txt"
python3 scripts/lib/mkvfs-big.py image "$IMG" --sectors 32768 x.txt="$T/x.txt" > /dev/null || { echo "WASM-STREAM TEST FAILED (disk image)"; exit 1; }
FAILS=0
fail() { echo "  FAIL: $1"; FAILS=$((FAILS + 1)); }

boot() { # boot <name> STEP ...
    local n="$1" k; shift
    qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
        -global virtio-mmio.force-legacy=false -device ramfb -display none \
        -drive "file=$IMG,if=none,format=raw,id=d0" -device virtio-blk-device,drive=d0 \
        -serial tcp:127.0.0.1:5236,server,nowait \
        -qmp tcp:127.0.0.1:5237,server,nowait -kernel "$BIN" &
    local q=$!
    python3 scripts/lib/cell-keys.py 5236 5237 "$T/shots" "$@" > "$T/$n.out" 2>&1
    pkill -f "tcp:127.0.0.1:5236," 2> /dev/null
    for k in $(seq 20); do pgrep -f "tcp:127.0.0.1:5236," > /dev/null || break; sleep 0.25; done
    pkill -9 -f "tcp:127.0.0.1:5236," 2> /dev/null; kill "$q" 2> /dev/null; wait "$q" 2> /dev/null
    tr -d '\r' < "$T/$n.out" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g; s/^[[:space:]]*//' > "$T/$n.txt"
    grep -aq "KERNEL PANIC\|panicked" "$T/$n.txt" && fail "boot $n: kernel panic"
    grep -aE "^afrisovera> |\[wasm app\]|^\[nia\] (TRAP|file stream)|DENIED" "$T/$n.txt" | sed "s/^/  $n | /" | head -70
    true
}
seg() { awk -v c="afrisovera> $2" -v n="${3:-1}" 'index($0, c) == 1 && length($0) == length(c) {k++; if (k == n) {on=1; next}}
        on && index($0, "afrisovera>") {exit} on' "$T/$1.txt"; }
prints() { grep -aoE '\[wasm app\] print\(-?[0-9]+\)' | grep -oE '[-]?[0-9]+' | tr '\n' ' ' | sed 's/ $//'; }
run_is() { # run_is <boot> <occurrence> <label> <expected prints>
    local got
    got="$(seg "$1" "nia streamtest.wasm" "$2" | prints)"
    [ "$got" = "$4" ] || fail "$3: printed [$got], not [$4]"
}

# --- 1: every mode, in order, on one disk
boot 1 "&nia streamtest.wasm" ":77" "=" "&nia streamtest.wasm" ":78" "=" \
    "&nia streamtest.wasm" ":63" "~1" ":03" "=" "&nia streamtest.wasm" ":72" "=" \
    "&nia streamtest.wasm" ":64" "=" "&nia streamtest.wasm" ":73" "=" "ls -l" "mem"
run_is 1 1 "w (16 KiB in 1 KiB appends, read back in pieces)" "1 16384 16384 16384 16384 $SUM_A 0 -3"
seg 1 "nia streamtest.wasm" 1 | grep -aqE "file_commit\('s\.txt', 16384 bytes\) -> ok, on disk in [0-9]+ us$" || fail "w: no on-disk commit line"
run_is 1 2 "x (past 16 KiB spoils; the old file stays)" "1 16384 -3 -3 16384 $SUM_A"
S="$(seg 1 "nia streamtest.wasm" 3)"
[ "$(prints <<< "$S")" = "1 1024" ] || fail "c: printed [$(prints <<< "$S")], not [1 1024]"
[ "$(grep -acxF "[nia] file stream for 's.txt' dropped: not committed; the old file is kept" <<< "$S")" = 1 ] ||
    fail "c: not exactly one dropped line after Ctrl+C"
grep -aqE "^\[nia\] TRAP: " <<< "$S" || fail "c: Ctrl+C did not stop the app"
run_is 1 4 "r (after the Ctrl+C)" "16384 16384 $SUM_A"
S="$(seg 1 "nia streamtest.wasm" 5)"
[ "$(prints <<< "$S")" = "1 10 1 5 5 -1 5" ] || fail "d: printed [$(prints <<< "$S")], not [1 10 1 5 5 -1 5]"
grep -aqxF "[nia] file stream for 'a.txt' dropped: a second file_open_write; the old file is kept" <<< "$S" || fail "d: no line for the dropped first stream"
run_is 1 6 "s (system files at open; no stream)" "-4 -4 -1 -1"
L="$(seg 1 "ls -l")"
grep -aqE "^16384  .*  s\.txt$" <<< "$L" || fail "ls -l: s.txt is not 16384 bytes"
grep -aqE "^5  .*  b\.txt$" <<< "$L" || fail "ls -l: b.txt is not 5 bytes"
grep -aqE "  a\.txt$" <<< "$L" && fail "ls -l: a.txt exists (its stream was dropped, never committed)"

# --- 2: after a power-off, the same image
boot 2 "&nia streamtest.wasm" ":72" "=" "mem"
run_is 2 1 "after a reboot" "16384 16384 $SUM_A"

[ "$FAILS" -eq 0 ] || { echo "WASM-STREAM TEST FAILED ($FAILS check(s))"; exit 1; }
echo "WASM-STREAM TEST PASSED"
