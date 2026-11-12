#!/usr/bin/env bash
# wasm-conv (claude/nia-conv, OS_REQUESTS item 4): i32.wrap_i64, i64.extend_i32_s/u, i32.load16_s/u and
# i32.store16. scripts/lib/conv-wasm.py writes conv.wasm and the exact lines it must print; the module goes on a
# disk image, the kernel boots with it and runs `nia conv.wasm` (an unverified app: print and print64 are
# allowed). Graded: every printed value, in order, ending with print(7); no unsupported opcode, no TRAP, no
# panic; the shell answers. Deterministic. Needs python3.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

T="${TMPDIR:-/tmp}/wasm-conv"
rm -rf "$T"; mkdir -p "$T"
python3 scripts/lib/conv-wasm.py "$T/conv.wasm" > "$T/want.txt" || { echo "WASM-CONV TEST FAILED (generator)"; exit 1; }
IMG="$T/disk.img"
python3 scripts/lib/mkvfs-big.py image "$IMG" conv.wasm="$T/conv.wasm" > /dev/null \
    || { echo "WASM-CONV TEST FAILED (disk image)"; exit 1; }

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -drive "file=$IMG,if=none,format=raw,id=d0" -device virtio-blk-device,drive=d0 \
    -serial tcp:127.0.0.1:4796,server,nowait \
    -qmp tcp:127.0.0.1:4797,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="$T/out.txt"
python3 scripts/shell-demo.py 4796 4797 "$T/shot.png" "nia conv.wasm" "mem" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

fail() { echo "WASM-CONV TEST FAILED ($1)"; exit 1; }
grep -aE "conv.wasm|\[wasm app\]|unsupported|TRAP" "$OUT" | sed 's/^/  /' | head -30
grep -aq "KERNEL PANIC\|panicked" "$OUT" && fail "kernel panic"
grep -aq "unsupported opcode" "$OUT" && fail "an opcode is still unsupported"
grep -aq "TRAP" "$OUT" && fail "the app trapped"
GOT="$(tr -d '\r' < "$OUT" | grep -aoE '\[wasm app\] print(64)?\(-?[0-9]+\)' | sed 's/^\[wasm app\] //')"
WANT="$(tr -d '\r' < "$T/want.txt")"
if [ "$GOT" != "$WANT" ]; then
    echo "  want:"; echo "$WANT" | sed 's/^/    /'
    echo "  got:"; echo "$GOT" | sed 's/^/    /'
    fail "the printed values differ"
fi
grep -aq "heap:" "$OUT" || fail "the shell stopped answering"
echo "WASM-CONV TEST PASSED  ($(echo "$WANT" | wc -l) values: wrap, extend_s, extend_u, load16_s, load16_u, store16, offset; the 64 KiB edge is graded in wasm-oob)"
