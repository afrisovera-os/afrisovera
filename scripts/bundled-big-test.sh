#!/usr/bin/env bash
# bundled-big (claude/nia-a7, OS_REQUESTS A7): a bundled app runs from the kernel's own bytes, and one over 16 KiB
# (Andika, 29,238 bytes) is never a VFS file. The disk image holds stand-ins with bundled names: andika.wasm and
# hello.wasm are both wasm-textrun's f128 (it prints SUM_F128), so a run of either stand-in shows that number.
# Graded on the console:
#   boot: "bundled apps: 13 of 13 match the manifest", nothing refused; hello.wasm's bundled copy restored;
#   `ls`: the andika.wasm stand-in is KEPT (the person's file, never deleted at boot: the architect's A7 change 1)
#   and marked "(a file of yours; the built-in app is what runs)", and "29238  andika.wasm  (built in)" follows;
#   after everything ran, `ls -l` still shows the stand-in at its own size;
#   `verify andika.wasm` integrity OK, `verify andika.wasm tamper` TAMPERED;
#   `appget ... andika.wasm ...` refused as bundled, nothing written;
#   `nia hello.wasm` and `nia andika.wasm` at the shell: "built in", integrity OK, "from the kernel image" with the
#   built-in size, never the stand-in's number; Andika asks for and is granted draw and file only, with every
#   capability allowed by its kind (so net, AI and voice are withheld by its own list); with no desk its
#   cell_open gets -4 and it returns cleanly;
#   in the desk: Andika's cell window opens as signed, text is typed (andika-text.png differs from
#   andika-open.png), Ctrl+C does not stop it, Ctrl+\ asks, Ctrl+\ again after 3 s is "Close anyway", and its run
#   returns cleanly (it checks for kind 4 after every cell_event).
# No TRAP, no DENIED, no panic. Runs on the PC kernel through the pc-arch shim as it is. Needs python3.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

T="${TMPDIR:-/tmp}/bundled-big"
rm -rf "$T"; mkdir -p "$T/shots"
python3 scripts/lib/textrun-wasm.py "$T/m" > "$T/names.txt" || { echo "BUNDLED-BIG TEST FAILED (generator)"; exit 1; }
SUM="$(sed -nE 's/^f128 print\(([0-9]+)\).*/\1/p' "$T/names.txt")"
SIZE=29238
STAND="$(wc -c < "$T/m/f128.wasm" | tr -d ' ')"
IMG="$T/disk.img"
python3 scripts/lib/mkvfs-big.py image "$IMG" --sectors 32768 andika.wasm="$T/m/f128.wasm" hello.wasm="$T/m/f128.wasm" \
    > /dev/null || { echo "BUNDLED-BIG TEST FAILED (disk image)"; exit 1; }
HEX64="$(printf '0%.0s' $(seq 64))"
TYPE="$(printf 'hello andika' | od -An -tx1 | tr -d ' \n')"

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -drive "file=$IMG,if=none,format=raw,id=d0" -device virtio-blk-device,drive=d0 \
    -serial tcp:127.0.0.1:5170,server,nowait \
    -qmp tcp:127.0.0.1:5171,server,nowait -kernel "$BIN" &
QPID=$!
OUT="$T/out.txt"
python3 scripts/lib/cell-keys.py 5170 5171 "$T/shots" \
    "ls" "verify andika.wasm" "verify andika.wasm tamper" "appget 10.0.2.2 8000 andika.wasm $HEX64" \
    "nia hello.wasm" "nia andika.wasm" \
    "gui" "desk mode pointer" \
    "&nia andika.wasm" "@opened by andika.wasm" "~3" "!andika-open" \
    ":$TYPE" "~1.5" ":03" "~1" "!andika-text" ":1c" "~0.8" "!andika-ask" "~3.5" ":1c" "=" \
    "ls -l" "mem" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

tr -d '\r' < "$OUT" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g; s/^[[:space:]]*//' > "$T/clean.txt"
grep -aE "afrisovera> |^\[nia\]|^\[vfs\]|^\[desk\] cell|verify|=> |appget|built in|andika|TRAP|DENIED|\[wasm app\]|cell-keys:" \
    "$T/clean.txt" | grep -av "^\[desk\] plan" | sed 's/^/  /' | head -90
fail() { echo "BUNDLED-BIG TEST FAILED ($1)"; exit 1; }
has() { grep -aqxF "$1" "$2" || fail "no line: $1"; }
hasre() { grep -aqE "$1" "$2" || fail "no line matching: $1"; }
[ -n "$SUM" ] || fail "harness: no f128 sum from the generator"
grep -aq "KERNEL PANIC\|panicked" "$T/clean.txt" && fail "kernel panic"
grep -aq "DENIED" "$T/clean.txt" && fail "a host call was denied: $(grep -a DENIED "$T/clean.txt" | head -1)"
grep -aq "TRAP" "$T/clean.txt" && fail "an app trapped: $(grep -a TRAP "$T/clean.txt" | head -1)"
grep -aq "print($SUM)" "$T/clean.txt" && fail "a stand-in ran (print($SUM))"
grep -aq "MISMATCH\|refused (not listed\|manifest full" "$T/clean.txt" && fail "$(grep -a "MISMATCH\|refused (not listed\|manifest full" "$T/clean.txt" | head -1)"
grep -aq "removed a file\|^\[vfs\] andika" "$T/clean.txt" && fail "boot touched the person's andika.wasm"

# the part of the log from the command typed ($1, a fixed string) to the next prompt line
seg() { awk -v c="afrisovera> $1" 'index($0, c) == 1 && length($0) == length(c) {on=1; next} on && index($0, "afrisovera>") {exit} on' "$T/clean.txt"; }

has "[nia] bundled apps: 13 of 13 match the manifest" "$T/clean.txt"
has "[vfs] hello.wasm: the disk copy differed from the bundled app; bundled copy restored" "$T/clean.txt"

seg "ls" > "$T/ls.txt"
[ "$(grep -ac 'andika' "$T/ls.txt")" -eq 2 ] || fail "ls: andika.wasm is not listed exactly twice (the file, the built-in app)"
grep -aqxF "$STAND  andika.wasm  (a file of yours; the built-in app is what runs)" "$T/ls.txt" ||
    fail "ls: no '$STAND  andika.wasm  (a file of yours; ...)' row"
grep -aqxE "$SIZE  andika\.wasm  \(built in\)" "$T/ls.txt" || fail "ls: no '$SIZE  andika.wasm  (built in)' row"
seg "ls -l" > "$T/lsl.txt"
grep -aqE "^$STAND  .*  andika\.wasm$" "$T/lsl.txt" || fail "ls -l: the person's andika.wasm ($STAND bytes) is gone after the runs"

seg "verify andika.wasm" > "$T/v1.txt"; has "=> integrity OK (bytes match the manifest)" "$T/v1.txt"
seg "verify andika.wasm tamper" > "$T/v2.txt"; has "=> TAMPERED - hash mismatch, app would be rejected" "$T/v2.txt"
seg "appget 10.0.2.2 8000 andika.wasm $HEX64" > "$T/ag.txt"
has "appget: andika.wasm is a bundled app and cannot be replaced; nothing written" "$T/ag.txt"

seg "nia hello.wasm" > "$T/hello.txt"
has "[nia] 'hello.wasm' is built in: running the kernel's own copy" "$T/hello.txt"
has "[nia] integrity OK for 'hello.wasm' (matches manifest)" "$T/hello.txt"
hasre "^\[nia\] loaded 'hello\.wasm' from the kernel image \([0-9]+ bytes\)$" "$T/hello.txt"
has "[nia] 'run' returned cleanly" "$T/hello.txt"

andika() { # andika <segment file> <where>
    has "[nia] 'andika.wasm' is built in: running the kernel's own copy" "$1"
    has "[nia] integrity OK for 'andika.wasm' (matches manifest)" "$1"
    has "[nia] loaded 'andika.wasm' from the kernel image ($SIZE bytes)" "$1"
    hasre "^\[nia\] loading module \($SIZE bytes\), granted caps = 0x3ff$" "$1"
    has "[nia] permissions asked: draw, file" "$1"
    has "[nia] permissions granted: draw, file" "$1"
    has "[nia] permissions withheld: nothing" "$1"
    has "[nia] 'run' returned cleanly" "$1"
}
seg "nia andika.wasm" > "$T/a-shell.txt"
andika "$T/a-shell.txt" shell
awk 'index($0, "afrisovera> nia andika.wasm") == 1 {k++} k == 2' "$T/clean.txt" > "$T/a-desk.txt"
andika "$T/a-desk.txt" desk
hasre '^\[desk\] cell: opened by andika\.wasm \(signed\), [0-9]+x[0-9]+ grid$' "$T/a-desk.txt"
[ "$(grep -acxF '[desk] cell: close request (Ctrl+\)' "$T/a-desk.txt")" -eq 1 ] || fail "desk: not one close request"
has "[desk] cell: close anyway offered" "$T/a-desk.txt"
has "[desk] cell: closed anyway (Ctrl+\\ again); the app stops at its next loop step" "$T/a-desk.txt"
hasre '^\[desk\] cell window closed: the app ended' "$T/a-desk.txt"

for p in andika-open andika-text andika-ask; do [ -s "$T/shots/$p.png" ] || fail "no screenshot $p.png"; done
d="$(python3 scripts/lib/png-same.py "$T/shots/andika-open.png" "$T/shots/andika-text.png")"
echo "  andika-open against andika-text: $d"
case "$d" in differ:*) ;; *) fail "typing changed nothing on the screen ($d)";; esac
d="$(python3 scripts/lib/png-same.py "$T/shots/andika-text.png" "$T/shots/andika-ask.png")"
echo "  andika-text against andika-ask: $d"
case "$d" in differ:*) ;; *) fail "the close question changed nothing on the screen ($d)";; esac
seg "mem" | grep -aq . || fail "the shell did not answer mem"
echo "BUNDLED-BIG TEST PASSED"
