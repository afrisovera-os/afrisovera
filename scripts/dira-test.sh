#!/usr/bin/env bash
# dira: AfriSovera's web browser, milestone M1 (BROWSER_PLAN.md), on both kernels.
#
# First the host tests of the parsers (dira-host: url, http, html, a fuzz smoke, and the suite's own pages read on the
# host give exactly scripts/lib/dira-pages/*.expected). Then one boot of each kernel, with scripts/lib/dira-server.py
# on the host (10.0.2.2:$DIRA_PORT through QEMU's user network) and a store disk holding about.html:
#   console: `dira text` of a page (the text, title and links exactly), a chunked page, text/plain, a one-byte
#     (ISO-8859-1) page, a redirect, a redirect loop (5 and stop), a 404 (its page shown), a picture (not a page),
#     gzip (not yet), a 300 KB page (cut at 48,000 letters with a line saying so), https (M2), a port with nothing,
#     a server that never answers (stopped at 30 s), a store file and a missing one, bad addresses and the usage;
#   the desk window: the page opens, link 2 is followed, h goes back, Space scrolls, g types an address, a link
#     number that does not exist gives a status line, q closes; the window's grid hash (DIRA_HASH, the same on both kernels; empty = printed,
#     not graded) and desk verify 0 rows differ after; screenshots page.png, about.png and plain.png.
# The shell answers `mem` at the end and nothing panics.
# no pipefail: every check is `printf ... | grep -q`, and grep -q stops reading at its first match, so printf can get
# a broken pipe; under pipefail that turned a found line into a FAIL (os2 run 43615800184). A check's status is grep's.
set -u
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
PY="${PY:-python3}"
PORT="${DIRA_PORT:-18731}"
DIRA_HASH="${DIRA_HASH-0e9b879da68aba57}" # the window after the walk, the same on ARM and PC (the cloud runner 13 Nov); empty = printed only
PG=scripts/lib/dira-pages
G="http://10.0.2.2:$PORT"

echo "== dira: the web browser, M1 (HTTP/1.1 and HTML to text in a desk window), on both kernels =="
HOST="$(rustc -vV | sed -n 's/^host: //p')"
HT="$(cargo test --manifest-path dira-host/Cargo.toml --target "$HOST" 2>&1)"
echo "$HT" | grep -E "^test |test result" | sed 's/^/  host | /'
echo "$HT" | grep -q "test result: ok. 14 passed; 0 failed" || { echo "DIRA TEST FAILED (host tests)"; exit 1; }

W="$(mktemp -d "$T/dira.XXXXXX")"
$PY scripts/lib/dira-server.py "$PORT" "$PG" 2> "$W/server.log" &
SPID=$!
trap 'kill $SPID 2> /dev/null' EXIT
for _ in $(seq 50); do grep -q "on port" "$W/server.log" 2> /dev/null && break; sleep 0.1; done
grep -q "on port $PORT" "$W/server.log" || { echo "DIRA TEST FAILED (harness: the page server did not start on $PORT)"; cat "$W/server.log"; exit 1; }
$PY scripts/lib/mkvfs-big.py image "$W/disk.img" --sectors 32768 about.html="$PG/about.html" > /dev/null ||
    { echo "DIRA TEST FAILED (harness: disk image)"; exit 1; }

hex() { printf '%s' "$1" | od -An -tx1 | tr -d ' \n'; }
# an address typed as a person would: 8 letters at a time with a pause between (the desk queues 32 keys)
typed() { local h; h="$(hex "$1")"; while [ -n "$h" ]; do printf ':%s ~0.3 ' "${h:0:16}"; h="${h:16}"; done; }
STEPS=(
    "dira"
    "dira text ftp://x"
    "dira text http://user@x/"
    "dira text $G/index.html"
    "dira text $G/chunked.html"
    "dira text $G/plain.txt"
    "dira text $G/latin1.html"
    "dira text $G/redirect"
    "dira text $G/loop"
    "dira text $G/missing"
    "dira text $G/pic.png"
    "dira text $G/gzip"
    "dira text $G/big.html"
    "dira text https://10.0.2.2/"
    "dira text http://10.0.2.2:1/"
    "dira text $G/slow"
    "&dira text $G/slow?ctrlc" "~2" ":03" "="
    "dira text $G/tometa"
    "dira text $G/links.html"
    "dira text $G/badchunk"
    "dira text file:about.html"
    "dira text file nothere.html"
    "gui" "desk mode pointer"
    "&dira $G/index.html" "@[dira] $G/index.html: 200" "~1.5" "!page"
    ":32" ":0d" "@[dira] $G/about.html: 200" "~1.5" "!about"
    ":68" "~3"
    ":20" "~1" ":6b" "~1"
    ":67" "~0.5" $(typed "$G/plain.txt") ":0d" "@[dira] $G/plain.txt: 200" "~1.5" "!plain"
    ":393939" ":0d" "~1"
    ":71" "="
    "desk verify"
    "&dira $G/slow?close" "@[dira] window" "~2" ":1c" "="
    "mem"
)

# The `| ` lines of the `dira text` run whose header names $1, up to its `dira: done` line.
body() { printf '%s\n' "$2" | awk -v h="dira: $1: " 'index($0, h) == 1 {k = 1; next} k && /^dira: done/ {exit} k && /^\| ?/ {sub(/^\| ?/, ""); print}'; }
# Everything from the line `afrisovera> $1` to the next prompt.
run() { printf '%s\n' "$2" | awk -v c="afrisovera> $1" 'index($0, c) == 1 && length($0) <= length(c) + 1 {k = 1; next} k && /^afrisovera> / {exit} k'; }

grade() { # grade <serial log> <kernel> <shot dir>
    local fail=0 S R
    _f() { echo "  FAIL ($2): $1"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL ($2): missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g; s/^[[:space:]]*//')"
    has() { printf '%s\n' "$1" | grep -aqF -- "$2" || _f "$3: no line: $2" "$K"; }
    hasre() { printf '%s\n' "$1" | grep -aqE -- "$2" || _f "$3: no line matching: $2" "$K"; }
    same() { # same <got> <expected file> <what>
        local want; want="$(cat "$2")"
        [ "$1" = "$want" ] || { _f "$3: the text differs from $2" "$K"; diff <(printf '%s\n' "$want") <(printf '%s\n' "$1") | head -20 | sed 's/^/      /'; }
    }
    K="$2"
    printf '%s\n' "$S" | grep -aqE 'KERNEL PANIC|panicked' && _f "the kernel panicked" "$K"

    has "$(run "dira" "$S")" "dira: usage: dira <address>" "usage"
    has "$(run "dira text ftp://x" "$S")" "dira: Dira opens http:// and https:// addresses only" "ftp refused"
    has "$(run "dira text http://user@x/" "$S")" "dira: addresses with a user name in them are refused" "user name refused"

    R="$(run "dira text $G/index.html" "$S")"
    hasre "$R" "^dira: $G/index.html: 200 OK, [0-9]+ bytes in [0-9]+ ms, at 10[.]0[.]2[.]2$" "index header (and the address it came from, M1b)"
    has "$R" "dira: title: Dira test page" "index title"
    same "$(body "$G/index.html" "$S")" "$PG/index.expected" "index text"
    for l in "[1] $G/chunked.html" "[2] $G/about.html" "[3] $G/plain.txt" "[4] $G/redirect"; do has "$R" "dira: link $l" "index links"; done
    has "$R" "dira: done: " "index done"
    printf '%s\n' "$R" | grep -aqE "this must not show|hidden drawing|not shown|color: red" && _f "index: script, style, comment or svg text shown" "$K"

    same "$(body "$G/chunked.html" "$S")" "$PG/chunked.expected" "chunked text"
    same "$(body "$G/plain.txt" "$S")" "$PG/plain.expected" "plain text"
    same "$(body "$G/latin1.html" "$S")" "$PG/latin1.expected" "latin1 text"
    R="$(run "dira text $G/redirect" "$S")"
    hasre "$R" "^dira: $G/about.html: 200 OK" "redirect followed"
    hasre "$R" "^dira: $G/about.html: 200 OK, [0-9]+ bytes in [0-9]+ ms, at 10[.]0[.]2[.]2 -> 10[.]0[.]2[.]2$" "every hop's address on the status line"
    same "$(body "$G/about.html" "$R")" "$PG/about.expected" "redirect text"
    has "$(run "dira text $G/loop" "$S")" "The site sent Dira round more than 5 redirects; Dira stopped." "redirect loop"
    R="$(run "dira text $G/missing" "$S")"
    hasre "$R" "^dira: $G/missing: 404 Not Found, " "404 header"
    same "$(body "$G/missing" "$S")" "$PG/missing.expected" "404 text"
    R="$(run "dira text $G/pic.png" "$S")"
    has "$R" "| This address is a file, not a page:" "picture refused"
    has "$R" "| image/png" "picture type"
    has "$(run "dira text $G/gzip" "$S")" "the server compressed the page, and Dira cannot read compressed pages yet" "gzip"
    R="$(run "dira text $G/big.html" "$S")"
    has "$R" "| [Dira: the page was cut here; M1 keeps the first 48,000 letters]" "big page cut line"
    hasre "$R" "^dira: done: 47[89][0-9][0-9] letters" "big page letters"
    # architect's Q3: no heap use above one segment while a 300 KB page is read
    local hb ha ub ua
    hb="$(printf '%s\n' "$R" | sed -n 's/^dira: heap before: [0-9]* bytes in use, high-water \([0-9]*\) bytes$/\1/p')"
    ha="$(printf '%s\n' "$R" | sed -n 's/^dira: heap after: [0-9]* bytes in use, high-water \([0-9]*\) bytes$/\1/p')"
    ub="$(printf '%s\n' "$R" | sed -n 's/^dira: heap before: \([0-9]*\) bytes in use.*/\1/p')"
    ua="$(printf '%s\n' "$R" | sed -n 's/^dira: heap after: \([0-9]*\) bytes in use.*/\1/p')"
    echo "  $K: heap around the 300 KB page: in use $ub -> $ua, high-water $hb -> $ha"
    if [ -z "$hb" ] || [ -z "$ha" ] || [ -z "$ub" ] || [ -z "$ua" ]; then
        _f "big page: no heap lines" "$K"
    else
        [ $((ha - hb)) -le 8192 ] || _f "big page: the heap high-water rose by $((ha - hb)) bytes (more than 8 KiB)" "$K"
        [ "$ua" -le $((ub + 1024)) ] || _f "big page: $((ua - ub)) bytes of heap left in use after it" "$K"
    fi
    has "$(run "dira text https://10.0.2.2/" "$S")" "| Secure pages (https://) are not supported yet." "https"
    has "$(run "dira text http://10.0.2.2:1/" "$S")" "| No answer from the server." "nothing listening"
    R="$(run "dira text $G/slow" "$S")"
    has "$R" "| No answer from the server." "a server that never answers"
    R="$(run "dira text $G/slow?ctrlc" "$S")"
    hasre "$R" "^\[dira\] stopped loading \(Ctrl\+C\): [0-9]{1,2} ms after it was asked, [0-9]{4} ms after the load began$" "Ctrl+C stops a hanging load at once"
    R="$(run "dira text $G/tometa" "$S")"
    has "$R" "| The site sent Dira to a private address, and Dira did not go there by itself:" "private redirect not followed"
    has "$R" "| http://169.254.169.254/latest/" "private redirect named"
    has "$R" "dira: link [1] http://169.254.169.254/latest/" "private redirect offered as a link"
    printf '%s\n' "$R" | grep -aq "dial 169.254" && _f "Dira connected to the link-local address" "$K"
    R="$(run "dira text $G/links.html" "$S")"
    hasre "$R" "^dira: done: [0-9]+ letters, [0-9]+ rows, 256 links$" "256 links numbered"
    has "$R" "| [Dira: this page has more links than M1 can number; the rest are plain text]" "link table full line"
    R="$(run "dira text $G/badchunk" "$S")"
    has "$R" "| The first part arrived." "a broken page keeps what arrived"
    has "$R" "| [Dira: the page stops here: the rest could not be read]" "a broken page says it stops"
    has "$R" "| a chunk-size line is longer than 64 bytes" "a broken page says why"
    R="$(run "dira text file:about.html" "$S")"
    hasre "$R" "^dira: file:about.html: read from the store, [0-9]+ bytes$" "store file header"
    same "$(body "file:about.html" "$S")" "$PG/about.expected" "store file text"
    has "$R" "dira: link [1] index.html" "store file link"
    has "$(run "dira text file nothere.html" "$S")" "| There is no file by that name in the store." "missing store file"

    R="$(run "dira $G/index.html" "$S")"
    has "$R" "[desk] cell: opened by Dira (built-in) (signed)" "window opened"
    hasre "$R" "^\[dira\] window [0-9]+x[0-9]+$" "window size"
    hasre "$R" "^\[dira\] $G/index.html: 200 OK, [0-9]+ bytes in [0-9]+ ms, at 10[.]0[.]2[.]2; [0-9]+ letters, [0-9]+ rows, 4 links$" "window page"
    has "$R" "[dira] follow link [2]" "link followed"
    hasre "$R" "^\[dira\] $G/about.html: 200 OK, .*; [0-9]+ letters, [0-9]+ rows, 1 links$" "linked page"
    has "$R" "[dira] back" "back"
    [ "$(printf '%s\n' "$R" | grep -acE "^\[dira\] $G/index.html: 200 OK")" -ge 2 ] || _f "back did not load the first page again" "$K"
    hasre "$R" "^\[dira\] $G/plain.txt: 200 OK" "typed address"
    has "$R" "[dira] closed (q)" "q closed"
    hasre "$R" "^\[desk\] cell window closed: the app ended \(0 event\(s\) dropped, grid hash [0-9a-f]{16}\)$" "window end"
    local h; h="$(printf '%s\n' "$R" | grep -aoE 'grid hash [0-9a-f]{16}' | tail -1 | cut -d' ' -f3)"
    echo "  $K: grid hash $h"
    if [ -n "$DIRA_HASH" ] && [ "$h" != "$DIRA_HASH" ]; then _f "grid hash $h, not $DIRA_HASH" "$K"; fi
    hasre "$(run "desk verify" "$S")" "^\[desk\] verify: 0 of [0-9]+ rows differ" "desk verify"
    R="$(run "dira $G/slow?close" "$S")"
    hasre "$R" "^\[dira\] stopped loading \(the close box\): [0-9]{1,2} ms after it was asked, [0-9]{4} ms after the load began$" "a close request stops a hanging load"
    has "$R" "[dira] closed (the close box)" "Dira ended on the close request"
    has "$R" "[desk] cell window closed: the app ended" "the window closed"
    has "$(run "mem" "$S")" "heap:" "the shell answers"
    for n in page about plain; do [ -s "$3/$n.png" ] || _f "screenshot $n.png missing" "$K"; done
    return $fail
}

winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }
show() { tr -d '\r' < "$1" | grep -aE '^\[dira\]|^\[desk\] (cell|verify)|^dira: [^l]|panic|PANIC' | sed "s/^/  $2 | /" | head -60; }
fails=0

echo "  --- ARM kernel ---"
BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel 2>&1 | tail -1
[ -r "$BIN" ] || { echo "DIRA TEST FAILED (harness: $BIN was not built)"; exit 1; }
mkdir -p "$W/arm"
qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -drive "file=$W/disk.img,if=none,format=raw,id=d0" -device virtio-blk-device,drive=d0 \
    -netdev user,id=n0 -device virtio-net-device,netdev=n0 \
    -serial tcp:127.0.0.1:5992,server,nowait \
    -qmp tcp:127.0.0.1:5993,server,nowait -kernel "$BIN" &
QPID=$!
$PY scripts/lib/cell-keys.py 5992 5993 "$(winpath "$W/arm")" "${STEPS[@]}" > "$W/arm.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
show "$W/arm.log" arm
grade "$W/arm.log" arm "$W/arm" || fails=$((fails + 1))

echo "  --- PC kernel ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -1
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "DIRA TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
cp "$W/disk.img" "$W/nvme.img"
mkdir -p "$W/pc"
PATH="$PWD/scripts/lib/pc-arch:$PATH" qemu-system-aarch64 -M virt -m 256M -display none \
    -drive "file=$W/nvme.img,if=none,format=raw,id=nv" -device "nvme,serial=afridira,drive=nv" \
    -netdev user,id=n0 -device virtio-net-device,netdev=n0 \
    -serial "tcp:127.0.0.1:5994,server,nowait" -qmp "tcp:127.0.0.1:5995,server,nowait" &
QPID=$!
$PY scripts/lib/cell-keys.py 5994 5995 "$(winpath "$W/pc")" "${STEPS[@]}" > "$W/pc.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
show "$W/pc.log" "pc "
grade "$W/pc.log" pc "$W/pc" || fails=$((fails + 1))

echo "  server: $(grep -c 'dira-server: GET' "$W/server.log") requests"
# evidence for the review (the cloud job keeps build/dira-shots): the screenshots and both serial logs
mkdir -p build/dira-shots
for k in arm pc; do
    cp "$W/$k.log" "build/dira-shots/$k.log" 2> /dev/null
    for n in page about plain; do cp "$W/$k/$n.png" "build/dira-shots/$k-$n.png" 2> /dev/null; done
done
if [ "$fails" -eq 0 ]; then
    echo "  screenshots: $W/arm/page.png $W/arm/about.png $W/arm/plain.png $W/pc/page.png $W/pc/about.png $W/pc/plain.png"
    echo "DIRA TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "DIRA TEST FAILED"
exit 1
