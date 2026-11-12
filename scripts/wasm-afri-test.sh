#!/usr/bin/env bash
# wasm-afri (claude/nia-afri, OS_REQUESTS item 8 step A): a Nia app's imports must come from module `afri` (or
# `env`, which the 23 built-in apps use, v0 only), matched as exact bytes; any other module is refused at load and
# its name is shown as escaped bytes. The parser's lengths (section size, module-name and field-name lengths) are
# checked adds: a 10-byte LEB near 2^64 used to overflow and panic the kernel. scripts/lib/afri-wasm.py writes
# af1 to af10, the suite boots them from a disk image and scripts/lib/afri-grade.py grades each one. Needs python3.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

T="${TMPDIR:-/tmp}/wasm-afri"
rm -rf "$T"; mkdir -p "$T"
python3 scripts/lib/afri-wasm.py "$T/m" > "$T/names.txt" || { echo "WASM-AFRI TEST FAILED (generator)"; exit 1; }
ARGS=(); CMDS=()
while read -r n; do ARGS+=("$n.wasm=$T/m/$n.wasm"); CMDS+=("nia $n.wasm"); done < "$T/names.txt"
IMG="$T/disk.img"
python3 scripts/lib/mkvfs-big.py image "$IMG" "${ARGS[@]}" > /dev/null || { echo "WASM-AFRI TEST FAILED (disk image)"; exit 1; }

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -drive "file=$IMG,if=none,format=raw,id=d0" -device virtio-blk-device,drive=d0 \
    -serial tcp:127.0.0.1:4896,server,nowait \
    -qmp tcp:127.0.0.1:4897,server,nowait -kernel "$BIN" &
QPID=$!
OUT="$T/out.txt"
python3 scripts/lib/poll-keys.py 4896 "${CMDS[@]}" "mem" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "afrisovera> nia |\[nia\] (parse error|import from)|\[wasm app\]|TRAP|PANIC|poll-keys:" "$OUT" | tr -d '\r' \
    | sed 's/^/  /' | head -50
python3 scripts/lib/afri-grade.py "$OUT" || { echo "WASM-AFRI TEST FAILED (see the checks above)"; exit 1; }
grep -aq "heap:" "$OUT" || { echo "WASM-AFRI TEST FAILED (the shell stopped answering)"; exit 1; }
echo "WASM-AFRI TEST PASSED  (afri and env load; any other module refused, its name escaped; 10-byte LEB lengths refused, no panic)"
