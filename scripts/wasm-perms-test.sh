#!/usr/bin/env bash
# wasm-perms (claude/nia-perms, OS_REQUESTS item 8 step B): an app lists the permissions it needs in an
# `afri.permissions` custom section and gets only those, within what its kind allows (a disk app is unverified:
# never file, net, AI or voice). The grant is made once after every section, so the section works wherever it is,
# and other custom sections are skipped. An unknown name, two sections, a short or over-long section, 17 names and
# junk in the section are refused at load; an app with no section runs on the v0 grant, with a line saying so.
# scripts/lib/perms-wasm.py writes p1 to p11, the suite boots them from a disk image and scripts/lib/perms-grade.py
# grades each one's lines. Needs python3.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

T="${TMPDIR:-/tmp}/wasm-perms"
rm -rf "$T"; mkdir -p "$T"
python3 scripts/lib/perms-wasm.py "$T/m" > "$T/names.txt" || { echo "WASM-PERMS TEST FAILED (generator)"; exit 1; }
ARGS=(); CMDS=()
while read -r n; do ARGS+=("$n.wasm=$T/m/$n.wasm"); CMDS+=("nia $n.wasm"); done < "$T/names.txt"
IMG="$T/disk.img"
python3 scripts/lib/mkvfs-big.py image "$IMG" "${ARGS[@]}" > /dev/null || { echo "WASM-PERMS TEST FAILED (disk image)"; exit 1; }

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -drive "file=$IMG,if=none,format=raw,id=d0" -device virtio-blk-device,drive=d0 \
    -serial tcp:127.0.0.1:4906,server,nowait \
    -qmp tcp:127.0.0.1:4907,server,nowait -kernel "$BIN" &
QPID=$!
OUT="$T/out.txt"
python3 scripts/lib/poll-keys.py 4906 "${CMDS[@]}" "mem" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "afrisovera> nia |\[nia\] (permissions|parse error|unknown permission|DENIED|TRAP)|\[wasm app\]|PANIC|poll-keys:" \
    "$OUT" | tr -d '\r' | sed 's/^/  /' | head -70
python3 scripts/lib/perms-grade.py "$OUT" || { echo "WASM-PERMS TEST FAILED (see the checks above)"; exit 1; }
grep -aq "heap:" "$OUT" || { echo "WASM-PERMS TEST FAILED (the shell stopped answering)"; exit 1; }
echo "WASM-PERMS TEST PASSED  (only what is asked, within the kind; anywhere in the module; bad sections refused; v0 without one)"
