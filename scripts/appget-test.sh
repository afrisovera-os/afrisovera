#!/usr/bin/env bash
# appget (claude/appget): install a Nia app over the network. A host HTTP server (scripts/lib/appget-server.py),
# reached as 10.0.2.2 through SLIRP, serves good and bad apps. Graded: an install with the right hash and a typed y
# runs and survives a reboot; upper-case hash accepted; n cancels; a wrong, malformed or swapped hash, a non-module,
# a too-big file, a 404, a bundled name, a bad name, no Content-Length and no server are each refused with one line
# and nothing written; bytes past Content-Length are ignored; a sideloaded app that calls net is denied; a reinstall
# replaces the bytes. No panic; the shell answers at the end. Needs python3.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

T="${TMPDIR:-/tmp}/appget"
rm -rf "$T"; mkdir -p "$T"
python3 scripts/lib/mkvfs-big.py bigwasm "$T/big.wasm"
declare -A H
while read -r n h; do H[$n]=$h; done < <(python3 scripts/lib/appget-server.py make "$T")
python3 scripts/lib/appget-server.py serve "$T" "$T/port" 2> "$T/server.log" &
SPID=$!
for _ in $(seq 50); do [ -s "$T/port" ] && break; sleep 0.1; done
P="$(cat "$T/port")"
echo "  server on port $P (guest: 10.0.2.2:$P)"
IMG="$T/disk.img"
head -c 4194304 /dev/zero > "$IMG"

boot() { # $1 = serial port (qmp = +1), rest = shell commands (":y" sends a raw key)
    local sp=$1; shift
    qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
        -global virtio-mmio.force-legacy=false -device ramfb -display none \
        -drive "file=$IMG,if=none,format=raw,id=d0" -device virtio-blk-device,drive=d0 \
        -netdev user,id=n0 -device virtio-net-device,netdev=n0 \
        -serial "tcp:127.0.0.1:$sp,server,nowait" \
        -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" -kernel "$BIN" &
    local qpid=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" AFRISH_CMD_QUIET="${AFRISH_CMD_QUIET:-8}" \
    python3 scripts/shell-demo.py "$sp" "$((sp+1))" "$T/shot.png" "$@" 2>&1
    kill "$qpid" 2>/dev/null
    wait "$qpid" 2>/dev/null
}
fail() { kill "$SPID" 2>/dev/null; echo "APPGET TEST FAILED ($1)"; exit 1; }
has() { grep -aqF -e "$1" <<< "$B1" || fail "$2"; }
UP="$(echo "${H[big.wasm]}" | tr a-f A-F)"
G="appget 10.0.2.2 $P"

echo "[boot 1] installs, refusals, a denied capability, a reinstall"
B1="$(boot 4776 \
    "$G big.wasm ${H[big.wasm]}" ":y" \
    "nia big.wasm" \
    "$G big.wasm $UP" ":n" \
    "$G big.wasm ${H[v2.wasm]}" \
    "$G big.wasm 1234" \
    "$G swap.wasm ${H[big.wasm]}" \
    "$G notwasm.wasm ${H[notwasm.wasm]}" \
    "$G huge.wasm ${H[huge.wasm]}" \
    "$G missing.wasm ${H[big.wasm]}" \
    "$G hello.wasm ${H[big.wasm]}" \
    "$G ../x.wasm ${H[big.wasm]}" \
    "$G nolen.wasm ${H[big.wasm]}" \
    "appget 10.0.2.2 1 big.wasm ${H[big.wasm]}" \
    "$G extra.wasm ${H[big.wasm]}" ":y" \
    "nia extra.wasm" \
    "$G netapp.wasm ${H[netapp.wasm]}" ":y" \
    "nia netapp.wasm" \
    "$G re.wasm ${H[big.wasm]}" ":y" \
    "$G re.wasm ${H[v2.wasm]}" ":y" \
    "nia re.wasm" \
    "ls" "mem")"
echo "$B1" | grep -aE "appget|print\(|DENIED|TRAP|without file" | sed 's/^/  /' | head -80
grep -aq "KERNEL PANIC\|panicked" <<< "$B1" && fail "kernel panic"
has "appget: installed big.wasm (3000 bytes)" "big.wasm not installed"
has "print(90)" "big.wasm did not run whole"
has "'big.wasm' runs without file, net, AI and voice" "the sideloaded app ran with every capability"
has "appget: cancelled; nothing written" "n did not cancel (or the upper-case hash was refused)"
[ "$(echo "$B1" | grep -ac "SHA-256 matches the one you gave")" -ge 6 ] || fail "the upper-case hash did not reach consent"
[ "$(echo "$B1" | grep -ac "hash does not match: expected")" -ge 2 ] || fail "a wrong or swapped hash was not refused"
has "the SHA-256 must be 64 hex characters (got 4 characters)" "a malformed hash was not refused"
has "notwasm.wasm is not a Nia app" "a non-module was not refused"
has "bigger than 16,384 bytes" "a too-big app was not refused"
has "did not answer 200 OK" "a 404 was not refused"
has "hello.wasm is a bundled app and cannot be replaced" "a bundled name was not refused"
has "'../x.wasm' is not an app name" "a bad name was not refused"
has "sent no Content-Length" "a missing Content-Length was not refused"
has "no complete answer from the server" "no server, no clear line"
has "appget: installed extra.wasm (3000 bytes)" "bytes past Content-Length broke the install"
has "it will NOT get: net" "the consent screen did not list the withheld capability"
has "DENIED: app called host 'net'" "the withheld capability was not denied"
has "replaces the existing re.wasm (3000 bytes)" "the reinstall did not say it replaces"
has "print(77)" "the reinstalled app did not run its new code"
LS="$(echo "$B1" | tr -d '\r' | grep -aE '^ *[0-9]+  [A-Za-z0-9._-]+$')"
grep -aqE ' big\.wasm$' <<< "$LS" || fail "ls does not list big.wasm (cannot judge what was written)"
for n in swap notwasm huge missing nolen x; do
    grep -aqE "  $n\.wasm\$" <<< "$LS" && fail "$n.wasm was written after a refusal"
done
has "heap:" "the shell stopped answering"

echo "[boot 2] the same disk: the installed apps survived"
B2="$(boot 4786 "nia big.wasm" "nia re.wasm")"
echo "$B2" | grep -aE "print\(|restored" | sed 's/^/  /'
grep -aq "print(90)" <<< "$B2" || fail "boot 2: big.wasm did not survive"
grep -aq "print(77)" <<< "$B2" || fail "boot 2: re.wasm's new bytes did not survive"

echo "[boot 3] a disk whose hello.wasm differs from the bundled app: the bundled copy is restored and verified"
IMG="$T/tampered.img"
python3 scripts/lib/mkvfs-big.py image "$IMG" hello.wasm="$T/v2.wasm" note.txt="$T/notwasm.wasm"
B3="$(boot 4776 "nia hello.wasm" "verify hello.wasm" "mem")"
echo "$B3" | grep -aE "restored|integrity|=>|print\(|putc|output" | sed 's/^/  /' | head -12
grep -aq "KERNEL PANIC\|panicked" <<< "$B3" && fail "boot 3: kernel panic"
grep -aqF "[vfs] hello.wasm: the disk copy differed from the bundled app; bundled copy restored" <<< "$B3" \
    || fail "boot 3: no restored line for a differing hello.wasm"
grep -aqF "integrity OK for 'hello.wasm'" <<< "$B3" || fail "boot 3: the restored hello.wasm was not integrity OK"
grep -aqF "print(77)" <<< "$B3" && fail "boot 3: the disk's modified hello.wasm ran"
grep -aqF "=> integrity OK (bytes match the manifest)" <<< "$B3" || fail "boot 3: verify output changed"
grep -aq "heap:" <<< "$B3" || fail "boot 3: the shell stopped answering"

kill "$SPID" 2>/dev/null
echo "APPGET TEST PASSED  (hash-checked install with consent, every refusal one line, withheld capability denied, survives a reboot)"
