#!/usr/bin/env bash
# hati (claude/nia-hati): Hati, the documents app (native apps engineer, nia-apps office step O2), bundled, runs in the
# desk's cell window by the engineer's own keys: it opens signed; "# Hello", Enter, "**bold** text" are typed; F2 shows
# the read view and F2 again the write view (each changes the screen); Ctrl+S saves d.md, F10 exports d.txt; Ctrl+Q
# returns. Graded:
# - the open line (signed) and a clean return;
# - the screenshots of the write view, the read view and the write view again: the read view differs from both;
# - after the run, `cat d.md` holds the Markdown as typed, and `cat d.txt` holds "Hello" and "bold text" with no marks;
# - Hati is granted draw and file only.
# Both kernels (on the PC through the pc-arch shim). Needs python3.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel 2>&1 | tail -1
T="${TMPDIR:-/tmp}/hati"
rm -rf "$T"; mkdir -p "$T/shots"
IMG="$T/disk.img"
printf 'x\n' > "$T/x.txt"
python3 scripts/lib/mkvfs-big.py image "$IMG" --sectors 32768 x.txt="$T/x.txt" > /dev/null || { echo "HATI TEST FAILED (disk image)"; exit 1; }
FAILS=0
fail() { echo "  FAIL: $1"; FAILS=$((FAILS + 1)); }

TYPE1="$(printf '# Hello' | od -An -tx1 | tr -d ' \n')"
TYPE2="$(printf '**bold** text' | od -An -tx1 | tr -d ' \n')"
qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -drive "file=$IMG,if=none,format=raw,id=d0" -device virtio-blk-device,drive=d0 \
    -serial tcp:127.0.0.1:5180,server,nowait \
    -qmp tcp:127.0.0.1:5181,server,nowait -kernel "$BIN" &
Q=$!
python3 scripts/lib/cell-keys.py 5180 5181 "$T/shots" \
    "gui" "desk mode pointer" "&nia hati.wasm" "@opened by hati.wasm" "~2" \
    ":$TYPE1" "~0.3" ":0a" "~0.3" ":$TYPE2" "~1" "!write" \
    ":1b4f51" "~1" "!read" ":1b4f51" "~1" "!write2" \
    ":13" "~0.5" ":642e6d64" ":0a" "~1.5" ":1b5b32317e" "~1.5" ":11" "=" \
    "cat d.md" "cat d.txt" "ls" > "$T/out.txt" 2>&1
pkill -f "tcp:127.0.0.1:5180," 2> /dev/null; kill "$Q" 2> /dev/null; wait "$Q" 2> /dev/null
tr -d '\r' < "$T/out.txt" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g; s/^[[:space:]]*//' > "$T/clean.txt"
grep -aE "^afrisovera> |^\[nia\]|^\[desk\] cell|\[wasm app\] file|Hello|bold|TRAP|DENIED" "$T/clean.txt" | sed 's/^/  /' | head -50
grep -aq "KERNEL PANIC\|panicked" "$T/clean.txt" && fail "kernel panic"
grep -aq "DENIED\|TRAP" "$T/clean.txt" && fail "$(grep -a 'DENIED\|TRAP' "$T/clean.txt" | head -1)"
seg() { awk -v c="afrisovera> $1" 'index($0, c) == 1 && length($0) == length(c) {on=1; next} on && index($0, "afrisovera>") {exit} on' "$T/clean.txt"; }
S="$(seg "nia hati.wasm")"
grep -aqE '^\[desk\] cell: opened by hati\.wasm \(signed\), [0-9]+x[0-9]+ grid$' <<< "$S" || fail "Hati's window did not open signed"
grep -aqxF "[nia] permissions granted: draw, file" <<< "$S" || fail "Hati is not granted exactly draw and file"
grep -aqxF "[nia] 'run' returned cleanly" <<< "$S" || fail "Hati did not return cleanly on Ctrl+Q"
for p in write read write2; do [ -s "$T/shots/$p.png" ] || fail "no screenshot $p.png"; done
for other in write write2; do
    d="$(python3 scripts/lib/png-same.py "$T/shots/read.png" "$T/shots/$other.png")"
    echo "  read against $other: $d"
    case "$d" in differ:*) ;; *) fail "F2's read view looks like the $other view ($d)";; esac
done
seg "cat d.md" | grep -aqxF "# Hello" || fail "d.md does not hold the heading as typed"
seg "cat d.md" | grep -aqF "**bold** text" || fail "d.md does not hold the marks as typed"
seg "cat d.txt" | grep -aqF "Hello" || fail "d.txt has no Hello"
seg "cat d.txt" | grep -aqF "bold text" || fail "d.txt has no plain \"bold text\""
seg "cat d.txt" | grep -aqF "**" && fail "d.txt still has the ** marks"
[ "$FAILS" -eq 0 ] || { echo "HATI TEST FAILED ($FAILS check(s))"; exit 1; }
echo "HATI TEST PASSED"
