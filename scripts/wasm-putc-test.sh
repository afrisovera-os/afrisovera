#!/usr/bin/env bash
# wasm-putc (claude/nia-putc, OS_REQUESTS item 6): putc text is shown as it is written, safely.
# scripts/lib/putc-wasm.py writes the apps; the suite boots them from a disk image (unverified apps) and drives
# them with scripts/lib/poll-keys.py; scripts/lib/putc-grade.py grades the log. Graded: text shows before the app
# ends (pu1, checked before the key is typed); invalid and cut UTF-8 print U+FFFD, ESC/BEL/C1/bidi controls print
# ?, e-acute prints as it is, no raw ESC or BEL reaches the console (pu2); text survives a TRAP (pu3); 300
# characters all show (pu4); a character cut at the end is flushed and the line closed (pu5); a print between two
# putc calls gives three lines in order (pu6); time, random, pollkey and sleep_ms print nothing and keep the line
# open (pu7); an app that TRAPs with an invalid byte, then one that writes "hi"
# (the old kernel panicked on this pair). No panic; the shell answers. Needs python3.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

T="${TMPDIR:-/tmp}/wasm-putc"
rm -rf "$T"; mkdir -p "$T"
python3 scripts/lib/putc-wasm.py "$T/m" > "$T/names.txt" || { echo "WASM-PUTC TEST FAILED (generator)"; exit 1; }
ARGS=()
while read -r n; do ARGS+=("$n.wasm=$T/m/$n.wasm"); done < "$T/names.txt"
IMG="$T/disk.img"
python3 scripts/lib/mkvfs-big.py image "$IMG" "${ARGS[@]}" > /dev/null || { echo "WASM-PUTC TEST FAILED (disk image)"; exit 1; }

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -drive "file=$IMG,if=none,format=raw,id=d0" -device virtio-blk-device,drive=d0 \
    -serial tcp:127.0.0.1:4846,server,nowait \
    -qmp tcp:127.0.0.1:4847,server,nowait -kernel "$BIN" &
QPID=$!
OUT="$T/out.txt"
python3 scripts/lib/poll-keys.py 4846 \
    "&nia pu1.wasm" "~1.5" "?Hi" ":20" "=" \
    "nia pu2.wasm" "nia pu3.wasm" "nia pu4.wasm" "nia pu5.wasm" "nia pu6.wasm" "nia pu7.wasm" \
    "nia sa.wasm" "nia sb.wasm" \
    "mem" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "afrisovera> nia |\[wasm app\]|TRAP|poll-keys:" "$OUT" | tr -d '\r' | sed 's/^/  /' | head -40
python3 scripts/lib/putc-grade.py "$OUT" || { echo "WASM-PUTC TEST FAILED (see the checks above)"; exit 1; }
grep -aq "heap:" "$OUT" || { echo "WASM-PUTC TEST FAILED (the shell stopped answering)"; exit 1; }
echo "WASM-PUTC TEST PASSED  (shown at once; U+FFFD for bad UTF-8, ? for control and bidi bytes; survives a TRAP; no 256 limit; lines in order around print; quiet calls keep the line; the stale-byte pair is safe)"
