#!/usr/bin/env bash
# hesabu (claude/nia-hesabu): Hesabu, the spreadsheet (native apps engineer, nia-apps office step O4, H1), bundled,
# runs in the desk's cell window by the engineer's own keys. The disk holds big.csv (exactly 8,192 bytes) and
# over.csv (8,193 bytes), the files of Hesabu's own 8 KiB tests. In one run:
# - it opens signed; 12, Tab, 30, Tab, =A1+B1, Enter are typed; Ctrl+S, s.csv, Enter saves;
# - Ctrl+O over.csv is refused whole (the architect's 16 Nov rule: never a partial load);
# - Ctrl+O big.csv opens it; End, z, Enter makes the sheet 8,208 bytes; Ctrl+S is refused before anything is written;
# - Ctrl+Q, then n (do not save), returns;
# - a second run (H1b): 3, 1, 2 typed down column A, back to A1, Shift+Down twice, Ctrl+Up sorts; saved as o.csv;
# - a third run (H1b): A1 = 1, A2 = 2, B1 = =A1*10, Ctrl+C on B1, Down, Ctrl+V; saved as p.csv;
# - then s.csv, o.csv and p.csv are read, `ls -l` is taken, and those three and big.csv and over.csv are removed,
#   so the runs below have file slots (24 in all) for f.csv, f.csv.fmt and f.html;
# - a fourth run (H2): 2.5, Enter, Up, F4 three times (2 places); saved as f.csv; Ctrl+Q;
# - a fifth run (H2): Ctrl+O f.csv (the format must come back from f.csv.fmt), then F10 exports f.html.
# Graded:
# - the open line (signed), the grant of exactly draw and file, a clean return, no TRAP, DENIED or panic;
# - s.csv holds "12,30,=A1+B1" as saved;
# - over.csv was read but the screen after it changed (the refusal line), and big.csv was read at 8,192 bytes;
# - no file_write to big.csv at all, and `ls -l` still shows big.csv at 8192 and over.csv at 8193 bytes;
# - the screenshot after the refused save differs from the one after big.csv opened (the refusal line);
# - o.csv holds 1, 2, 3 (sorted) and p.csv holds "1,=A1*10" and "2,=A2*10" (the reference moved on paste);
# - f.csv still holds 2.5 (a format changes what is shown, not the file), f.csv.fmt sits beside it, and f.html's
#   table, exported only after a reopen, shows 2.50 under the title f.csv: the format survived the reopen;
# - all five runs return cleanly.
# Both kernels (on the PC through the pc-arch shim). Needs python3.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel 2>&1 | tail -1
T="${TMPDIR:-/tmp}/hesabu"
rm -rf "$T"; mkdir -p "$T/shots"
IMG="$T/disk.img"
python3 - "$T" <<'PY' || { echo "HESABU TEST FAILED (files)"; exit 1; }
import sys
t = sys.argv[1]
rows = ('x' * 100 + '\r\n') * 80
open(t + '/big.csv', 'wb').write((rows + 'y' * 30 + '\r\n').encode())
open(t + '/over.csv', 'wb').write((rows + 'y' * 31 + '\r\n').encode())
PY
[ "$(wc -c < "$T/big.csv" | tr -d ' ')" = 8192 ] && [ "$(wc -c < "$T/over.csv" | tr -d ' ')" = 8193 ] \
    || { echo "HESABU TEST FAILED (file sizes)"; exit 1; }
python3 scripts/lib/mkvfs-big.py image "$IMG" --sectors 32768 big.csv="$T/big.csv" over.csv="$T/over.csv" > /dev/null \
    || { echo "HESABU TEST FAILED (disk image)"; exit 1; }
FAILS=0
fail() { echo "  FAIL: $1"; FAILS=$((FAILS + 1)); }

hex() { printf '%s' "$1" | od -An -tx1 | tr -d ' \n'; }
qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -drive "file=$IMG,if=none,format=raw,id=d0" -device virtio-blk-device,drive=d0 \
    -serial tcp:127.0.0.1:5186,server,nowait \
    -qmp tcp:127.0.0.1:5187,server,nowait -kernel "$BIN" &
Q=$!
python3 scripts/lib/cell-keys.py 5186 5187 "$T/shots" \
    "gui" "desk mode pointer" "&nia hesabu.wasm" "@opened by hesabu.wasm" "~2" \
    ":$(hex 12)" ":09" ":$(hex 30)" ":09" ":$(hex =A1+B1)" ":0d" "~1" "!edit" \
    ":13" "~0.5" ":$(hex s.csv)" ":0d" "~1.5" "!saved" \
    ":0f" "~0.5" ":$(hex over.csv)" ":0d" "~1.5" "!over" \
    ":0f" "~0.5" ":$(hex big.csv)" ":0d" "~2" "!big" \
    ":1b5b46" "~0.3" ":7a" ":0d" "~1" ":13" "~1.5" "!oversave" \
    ":11" "~0.5" ":6e" "=" \
    "&nia hesabu.wasm" "@opened by hesabu.wasm" "~2" \
    ":33" ":0d" ":31" ":0d" ":32" ":0d" "~0.5" ":1b5b41" ":1b5b41" ":1b5b41" \
    ":1b5b313b3242" ":1b5b313b3242" "~0.3" ":1b5b313b3541" "~1" "!sorted" \
    ":13" "~0.5" ":$(hex o.csv)" ":0d" "~1.5" ":11" "=" \
    "&nia hesabu.wasm" "@opened by hesabu.wasm" "~2" \
    ":31" ":0d" ":32" ":0d" "~0.3" ":1b5b41" ":1b5b41" ":1b5b43" ":$(hex =A1*10)" ":0d" "~0.3" \
    ":1b5b41" ":03" "~0.3" ":1b5b42" ":16" "~1" "!pasted" \
    ":13" "~0.5" ":$(hex p.csv)" ":0d" "~1.5" ":11" "=" \
    "cat s.csv" "cat o.csv" "cat p.csv" "ls -l" \
    "rm s.csv" "rm o.csv" "rm p.csv" "rm big.csv" "rm over.csv" \
    "&nia hesabu.wasm" "@opened by hesabu.wasm" "~2" \
    ":$(hex 2.5)" ":0d" "~0.3" ":1b5b41" ":1b4f53" ":1b4f53" ":1b4f53" "~1" "!formatted" \
    ":13" "~0.5" ":$(hex f.csv)" ":0d" "~1.5" ":11" "=" \
    "&nia hesabu.wasm" "@opened by hesabu.wasm" "~2" \
    ":0f" "~0.5" ":$(hex f.csv)" ":0d" "~2" "!reopened" ":1b5b32317e" "~1.5" "!exported" ":11" "=" \
    "cat f.csv" "cat f.html" "ls" > "$T/out.txt" 2>&1
pkill -f "tcp:127.0.0.1:5186," 2> /dev/null; kill "$Q" 2> /dev/null; wait "$Q" 2> /dev/null
tr -d '\r' < "$T/out.txt" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g; s/^[[:space:]]*//' > "$T/clean.txt"
grep -aE "^afrisovera> |^\[nia\]|^\[desk\] cell|\[wasm app\] file|csv|TRAP|DENIED" "$T/clean.txt" | sed 's/^/  /' | head -60
grep -aq "KERNEL PANIC\|panicked" "$T/clean.txt" && fail "kernel panic"
grep -aq "DENIED\|TRAP" "$T/clean.txt" && fail "$(grep -a 'DENIED\|TRAP' "$T/clean.txt" | head -1)"
seg() { awk -v c="afrisovera> $1" 'index($0, c) == 1 && length($0) == length(c) {on=1; next} on && index($0, "afrisovera>") {exit} on' "$T/clean.txt"; }
S="$(seg "nia hesabu.wasm")"
grep -aqE '^\[desk\] cell: opened by hesabu\.wasm \(signed\), [0-9]+x[0-9]+ grid$' <<< "$S" || fail "Hesabu's window did not open signed"
grep -aqxF "[nia] permissions granted: draw, file" <<< "$S" || fail "Hesabu is not granted exactly draw and file"
grep -aqxF "[nia] 'run' returned cleanly" <<< "$S" || fail "Hesabu did not return cleanly on Ctrl+Q, n"
grep -aqE "\[wasm app\] file_write\('s\.csv', [0-9]+ bytes\) -> ok" <<< "$S" || fail "s.csv was not saved"
grep -aqF "[wasm app] file_read('over.csv')" <<< "$S" || fail "over.csv was never read"
grep -aqF "[wasm app] file_read('big.csv') -> 8192 bytes" <<< "$S" || fail "big.csv was not read whole (8192 bytes)"
grep -aqF "file_write('big.csv'" <<< "$S" && fail "the over-8 KiB save reached file_write for big.csv"
seg "cat s.csv" | grep -aqxF "12,30,=A1+B1" || fail "s.csv does not hold 12,30,=A1+B1"
O="$(seg "cat o.csv" | grep -av '^[[:space:]]*$' | tr '\n' ' ')"
[ "$O" = "1 2 3 " ] || fail "Ctrl+Up did not sort 3, 1, 2 into 1, 2, 3 (o.csv: $O)"
P="$(seg "cat p.csv" | grep -av '^[[:space:]]*$' | tr '\n' ' ')"
[ "$P" = "1,=A1*10 2,=A2*10 " ] || fail "the pasted formula's reference did not move to A2 (p.csv: $P)"
[ "$(grep -acxF "[nia] 'run' returned cleanly" "$T/clean.txt")" -ge 5 ] || fail "not all five Hesabu runs returned cleanly"
seg "cat f.csv" | grep -aqxF "2.5" || fail "the format changed the file (f.csv must still hold 2.5)"
seg "cat f.html" | grep -aqF '<td class="n">2.50</td>' || fail "the export after the reopen does not show 2.50 (the format did not survive the reopen)"
seg "cat f.html" | grep -aqF "<title>f.csv</title>" || fail "the export is not f.csv's table"
F="$(seg "ls")"
grep -aqE "^32[[:space:]]+f\.csv\.fmt$" <<< "$F" || fail "no 32-byte f.csv.fmt beside the sheet (the column format was not kept)"
grep -aqE "[[:space:]]f\.html$" <<< "$F" || fail "F10 wrote no f.html"
[ -s "$T/shots/formatted.png" ] && [ -s "$T/shots/reopened.png" ] && [ -s "$T/shots/exported.png" ] || fail "no screenshot of the format, the reopen or the export"
L="$(seg "ls -l")"
grep -aqE "(^|[^0-9])8192[^0-9].*big\.csv|big\.csv.*[^0-9]8192([^0-9]|$)" <<< "$L" || fail "big.csv is not 8192 bytes after the refused save"
grep -aqE "(^|[^0-9])8193[^0-9].*over\.csv|over\.csv.*[^0-9]8193([^0-9]|$)" <<< "$L" || fail "over.csv is not 8193 bytes"
for p in edit saved over big oversave; do [ -s "$T/shots/$p.png" ] || fail "no screenshot $p.png"; done
for pair in "saved over" "big oversave"; do
    set -- $pair
    d="$(python3 scripts/lib/png-same.py "$T/shots/$1.png" "$T/shots/$2.png")"
    echo "  $1 against $2: $d"
    case "$d" in differ:*) ;; *) fail "no refusal line on the screen between $1 and $2 ($d)";; esac
done
[ "$FAILS" -eq 0 ] || { echo "HESABU TEST FAILED ($FAILS check(s))"; exit 1; }
echo "HESABU TEST PASSED"
