#!/usr/bin/env bash
# wasm-poll (claude/nia-poll, OS_REQUESTS item 5): the pollkey and sleep_ms host functions. scripts/lib/poll-wasm.py
# writes seven small apps; the suite boots them from a disk image (unverified apps: input and time are allowed) and
# drives them with scripts/lib/poll-keys.py, which types raw keys at set times and measures each command on the
# host clock. Graded: pollkey never waits and returns keys in order; Ctrl+C read by pollkey or during a sleep
# TRAPs (never returned as 3); sleep_ms returns the time slept (1500 to 1600 for 1500; the clamp gives 10000 to
# 10100 for -1) and the host clock agrees within loose bands (1.2 to 4 s, 8 to 20 s); a key typed during a sleep
# is kept. No panic; the shell answers. Needs python3.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

T="${TMPDIR:-/tmp}/wasm-poll"
rm -rf "$T"; mkdir -p "$T"
python3 scripts/lib/poll-wasm.py "$T/m" > "$T/names.txt" || { echo "WASM-POLL TEST FAILED (generator)"; exit 1; }
ARGS=()
while read -r n; do ARGS+=("$n.wasm=$T/m/$n.wasm"); done < "$T/names.txt"
IMG="$T/disk.img"
python3 scripts/lib/mkvfs-big.py image "$IMG" "${ARGS[@]}" > /dev/null || { echo "WASM-POLL TEST FAILED (disk image)"; exit 1; }

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -drive "file=$IMG,if=none,format=raw,id=d0" -device virtio-blk-device,drive=d0 \
    -serial tcp:127.0.0.1:4826,server,nowait \
    -qmp tcp:127.0.0.1:4827,server,nowait -kernel "$BIN" &
QPID=$!
OUT="$T/out.txt"
python3 scripts/lib/poll-keys.py 4826 \
    "nia pk1.wasm" \
    "nia sl1.wasm" \
    "nia sl2.wasm" \
    "&nia pk2.wasm" "~2" ":61" "~1.5" ":62" "~1.5" ":71" "=" \
    "&nia pk3.wasm" "~2" ":41" "~1" ":03" "=" \
    "&nia sl3.wasm" "~2" ":03" "=" \
    "&nia sl4.wasm" "~1" ":78" "=" \
    "mem" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

tr -d '\r' < "$OUT" > "$T/clean.txt"
grep -aE "afrisovera> nia |\[wasm app\]|TRAP|poll-keys:" "$T/clean.txt" | sed 's/^/  /' | head -60
fail() { echo "WASM-POLL TEST FAILED ($1)"; exit 1; }
grep -aq "KERNEL PANIC\|panicked" "$T/clean.txt" && fail "kernel panic"
grep -aq "DENIED: app called host" "$T/clean.txt" && fail "a host function was denied (unknown name or capability)"

seg() {  # the log of one app: from its command line to the next prompt
    awk -v n="$1" 'index($0, "afrisovera>") && $0 ~ ("nia " n "[.]wasm[ \t]*$") {on=1; next}
        on && index($0, "afrisovera>") {exit} on' "$T/clean.txt"
}
prints() { seg "$1" | grep -aoE '\[wasm app\] print\(-?[0-9]+\)' | grep -oE -- '-?[0-9]+' | tr '\n' ' '; }
took() { grep -aF "poll-keys: '$1' took" "$T/clean.txt" | grep -oE "took [0-9.]+" | grep -oE "[0-9.]+"; }
after() { grep -aF "poll-keys: '$1' took" "$T/clean.txt" | grep -oE "[0-9.]+ s after" | grep -oE "^[0-9.]+"; }
between() { python3 -c "import sys; v=float(sys.argv[1]); sys.exit(0 if float(sys.argv[2]) <= v <= float(sys.argv[3]) else 1)" "$@"; }

# pk1: no key, -1 at once
[ "$(prints pk1)" = "-1 " ] || fail "pk1: pollkey with no key did not give -1 ($(prints pk1))"
between "$(took 'nia pk1.wasm')" 0 3 || fail "pk1: pollkey waited ($(took 'nia pk1.wasm') s)"

# sl1, sl2: the returned value (lower bound) and the host clock (a band that catches a wrong rate)
v="$(prints sl1)"; between "${v% }" 1500 1600 || fail "sl1: sleep_ms(1500) returned '$v'"
between "$(took 'nia sl1.wasm')" 1.2 4 || fail "sl1: the host clock says $(took 'nia sl1.wasm') s, not 1.2 to 4"
v="$(prints sl2)"; between "${v% }" 10000 10100 || fail "sl2: sleep_ms(-1) returned '$v', not the 10000 clamp"
between "$(took 'nia sl2.wasm')" 8 20 || fail "sl2: the host clock says $(took 'nia sl2.wasm') s, not 8 to 20"

# pk2: keys in order, the empty-poll count above 0, q ends it
set -- $(prints pk2)
[ "$#" -eq 3 ] && [ "$1" = 97 ] && [ "$2" = 98 ] && [ "$3" -gt 0 ] \
    || fail "pk2: expected 97 98 then a count above 0, got '$(prints pk2)'"
grep -aq "TRAP" <<< "$(seg pk2)" && fail "pk2: trapped"

# pk3: A is returned, Ctrl+C read by pollkey TRAPs and is never returned as 3
[ "$(prints pk3)" = "65 " ] || fail "pk3: expected only 65 printed, got '$(prints pk3)'"
grep -aqF "TRAP: stopped by Ctrl+C" <<< "$(seg pk3)" || fail "pk3: Ctrl+C read by pollkey did not TRAP"

# sl3: Ctrl+C during a sleep TRAPs, within about 1 s (3 s limit)
grep -aqF "TRAP: stopped by Ctrl+C" <<< "$(seg sl3)" || fail "sl3: Ctrl+C during sleep_ms did not TRAP"
between "$(after '&nia sl3.wasm')" 0 3 || fail "sl3: the TRAP came $(after '&nia sl3.wasm') s after the key, not within 3"

# sl4: a key typed during a sleep is kept for pollkey
[ "$(prints sl4)" = "120 " ] || fail "sl4: the key typed during the sleep was not kept ($(prints sl4))"

grep -aq "heap:" "$T/clean.txt" || fail "the shell stopped answering"
echo "WASM-POLL TEST PASSED  (pollkey -1 at once and keys in order; Ctrl+C TRAPs from pollkey and from a sleep in $(after '&nia sl3.wasm') s; sleep_ms(1500) = $(prints sl1| tr -d ' ') in $(took 'nia sl1.wasm') s; the clamp $(prints sl2 | tr -d ' ') in $(took 'nia sl2.wasm') s; a key kept across a sleep)"
