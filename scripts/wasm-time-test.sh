#!/usr/bin/env bash
# wasm-time (claude/nia-time, OS_REQUESTS item 7): Nia's `time` reads the board's clock on both kernels (before,
# nia.rs read the ARM PL031 address on the PC too, where it is plain RAM). QEMU's clock starts at
# 2026-01-01 12:34:56 UTC. scripts/lib/time-wasm.py writes tm1 (time, sleep_ms(2000), time), the suite boots it
# from a disk image (an unverified app) between two `date` commands and asks Asili the time; scripts/lib/time-grade.py
# grades each value against the same boot's `date`. Needs python3.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

T="${TMPDIR:-/tmp}/wasm-time"
rm -rf "$T"; mkdir -p "$T"
python3 scripts/lib/time-wasm.py "$T/m" > /dev/null || { echo "WASM-TIME TEST FAILED (generator)"; exit 1; }
IMG="$T/disk.img"
python3 scripts/lib/mkvfs-big.py image "$IMG" "tm1.wasm=$T/m/tm1.wasm" > /dev/null || { echo "WASM-TIME TEST FAILED (disk image)"; exit 1; }

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M -rtc base=2026-01-01T12:34:56 \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -drive "file=$IMG,if=none,format=raw,id=d0" -device virtio-blk-device,drive=d0 \
    -serial tcp:127.0.0.1:4886,server,nowait \
    -qmp tcp:127.0.0.1:4887,server,nowait -kernel "$BIN" &
QPID=$!
OUT="$T/out.txt"
python3 scripts/lib/poll-keys.py 4886 "date" "nia tm1.wasm" "date" "ask the time" "mem" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "afrisovera> |UTC|\[wasm app\]|\[asili\]|TRAP|DENIED|poll-keys:" "$OUT" | tr -d '\r' | sed 's/^/  /' | head -30
python3 scripts/lib/time-grade.py "$OUT" || { echo "WASM-TIME TEST FAILED (see the checks above)"; exit 1; }
grep -aq "heap:" "$OUT" || { echo "WASM-TIME TEST FAILED (the shell stopped answering)"; exit 1; }
echo "WASM-TIME TEST PASSED  (time and the Asili clock answer agree with date; time moves 2 to 5 s over sleep_ms(2000))"
