#!/usr/bin/env bash
# NVMe speed, slice 1 (12 Nov 2026, driver engineer, NVME_SPEED_PLAN.md; architect's conditions Q-a, Q-b, Q-c):
# 64 KiB read-ahead and write-gather runs, PRP lists, and a run longer than the controller takes in one command
# sent as several commands with ONE doorbell, completions taken in any order by command id.
#
# QEMU's NVMe is told to take only 16 KiB per command (mdts=2: 2^2 pages of 4 KiB), so every 64 KiB run really
# goes out as 4 commands in flight. Two disks, 512-byte and 4096-byte blocks, each 16 MiB and already holding
# AfriSovera's files (three files, two of them over 512 bytes: scripts/lib/mkvfs-big.py). Three boots each:
#   1. run big.wasm (its bytes past 512 come through the read-ahead), `nvmetest prp` (blocks from block 1 read
#      into a buffer at page offset 512: the first PRP entry starts inside its page, the rest in a PRP list,
#      compared with one block at a time: Q-c), write keep.txt, sync, lsnvme, then write lost.txt WITHOUT sync
#      and power off;
#   2. keep.txt is there, lost.txt is not (the VFS saves only at sync: a file written and not synced lives in
#      memory only and is lost at power-off; nothing the driver acknowledged is lost: the gather is drained at
#      the save's last sector and a Flush follows when the controller has a volatile write cache: Q-b). Then
#      `nvmetest drop` loses the next completion on purpose: the batch gets no answer within 5 s, the controller
#      is reset before any buffer is used again, the commands are sent again (Q-a); write drop.txt, sync, lsnvme;
#   3. drop.txt and keep.txt are there, big.wasm still runs; on the host, mkvfs-big.py check finds the three
#      original files' sectors exactly as written.
#   scripts/pc-nvmequeue-test.sh              # grader selftest, then the six QEMU boots
#   scripts/pc-nvmequeue-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

BAD='KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[nvme\] .*(failed|timed out|did not|refused|not come back)|multi-block (reads|writes) off|DIFFERENT|FAILED|blk self-test FAILED'
RE_Q='^  queue: most commands in flight at once ([0-9]+), PRP-list commands ([0-9]+), largest transfer ([0-9]+) KiB, controller takes 16 KiB per command$'

grade() { # grade <boot1> <boot2> <boot3> <block bytes> <host check log>
    local fail=0 A B C bs=$4 nb k c p
    _f() { echo "  FAIL: $*"; fail=1; }
    for f in "$1" "$2" "$3"; do [ -r "$f" ] || { echo "  FAIL: missing log $f"; return 1; }; done
    A="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    B="$(tr -d '\r' < "$2" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    C="$(tr -d '\r' < "$3" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    if [ "$bs" = 512 ]; then nb=32768; k=128; c=32; p=127; else nb=4096; k=16; c=4; p=15; fi
    has() { printf '%s\n' "$2" | grep -aqE "$1"; }
    has "^\[nvme\] namespace 1: $nb blocks of $bs B \(16 MiB\), ours \(AFRIVFS1\): used for storage$" "$A" || _f "boot 1: the disk was not taken as ours"
    has "^\[nvme\] multi-block reads on \(up to $k blocks per read, $c per command; the first $k equal one at a time\)$" "$A" ||
        _f "boot 1: 64 KiB reads were not checked and switched on ($k blocks per read, $c per command)"
    has "^\[nvme\] the controller takes 16 KiB per command: a run of up to $k blocks goes out as up to 4 commands at once$" "$A" ||
        _f "boot 1: MDTS 2 (16 KiB) was not read, or runs are not split"
    has '^ *\[wasm app\] print\(4242\) = 0x1092$' "$A" && has '^ *\[wasm app\] print\(90\) = 0x5a$' "$A" || _f "boot 1: big.wasm did not run to its last byte"
    has "^nvmetest prp: $p blocks from block 1 read into a buffer at page offset 512 \(4 command\(s\), PRP list\): equal to one block at a time$" "$A" ||
        _f "boot 1: the odd-block read at a page offset was not equal (Q-c)"
    has "^\[nvme\] multi-block writes on \(up to $k blocks per write, $c per command; the first [0-9]+ read back equal\)$" "$A" ||
        _f "boot 1: gathered writes were not checked and switched on"
    has '^\[vfs\] synced [0-9]+ files to disk$' "$A" || _f "boot 1: no sync"
    local q; q="$(printf '%s\n' "$A" | grep -aE '^  queue: ' | tail -1)"
    if [[ "$q" =~ $RE_Q ]]; then
        [ "${BASH_REMATCH[1]}" -ge 2 ] || _f "boot 1: never more than ${BASH_REMATCH[1]} command in flight"
        [ "${BASH_REMATCH[2]}" -ge 1 ] || _f "boot 1: no command used a PRP list"
        [ "${BASH_REMATCH[3]}" -le 16 ] || _f "boot 1: a command moved ${BASH_REMATCH[3]} KiB, over the controller's 16"
    else
        _f "boot 1: lsnvme has no queue line"
    fi
    if has '^\[nvme\] the controller has a volatile write cache: every save ends with a Flush$' "$A"; then
        local fl sy
        fl="$(printf '%s\n' "$A" | sed -nE 's/^  volatile write cache: yes, ([0-9]+) Flush\(es\) sent.*/\1/p' | tail -1)"
        sy="$(printf '%s\n' "$A" | grep -acE '^\[vfs\] synced [0-9]+ files to disk$')"
        [ -n "$fl" ] && [ "$fl" -ge 1 ] || _f "boot 1: a volatile write cache but no Flush after the save (Q-b)"
        [ -n "$fl" ] && [ "$fl" -le "$sy" ] || _f "boot 1: $fl Flushes for $sy saves (one per save at most)"
    fi
    has "^KEEP-$bs$" "$B" || _f "boot 2: keep.txt (synced) did not survive the power-off"
    has "^LOST-$bs$" "$B" && _f "boot 2: lost.txt was never synced, yet it is there: the test is not testing what it says"
    has '^nvmetest: the next I/O completion will be lost on purpose' "$B" || _f "boot 2: nvmetest drop did not answer"
    has '^\[nvme\] nvmetest: the completion of command id [0-9]+ is lost on purpose$' "$B" || _f "boot 2: no completion was lost"
    has '^\[nvme\] [0-9]+ of [0-9]+ command\(s\) sent together got no answer$' "$B" || _f "boot 2: the lost completion was not noticed"
    has '^\[nvme\] a command got no answer: resetting the controller and making its queues again' "$B" || _f "boot 2: no reset (Q-a)"
    has '^\[nvme\] controller ready again after the reset' "$B" || _f "boot 2: the controller did not come back"
    has '^\[nvme\] sending the [0-9]+ command\(s\) again after the reset$' "$B" || _f "boot 2: the commands were not sent again"
    has '^  controller resets \(a command got no answer, or nvmereset\): 1$' "$B" || _f "boot 2: lsnvme does not count exactly one reset"
    has '^  completions: 0 late one\(s\) set aside, 1 lost on purpose \(nvmetest drop\)$' "$B" || _f "boot 2: lsnvme does not count the lost completion"
    has "^DROP-$bs$" "$C" || _f "boot 3: drop.txt (saved across the reset) is missing"
    has "^KEEP-$bs$" "$C" || _f "boot 3: keep.txt is missing"
    has '^ *\[wasm app\] print\(90\) = 0x5a$' "$C" || _f "boot 3: big.wasm no longer runs to its last byte"
    [ -r "$5" ] && grep -aqx 'mkvfs-big: check OK' "$5" || _f "host: the original files' sectors are not exactly as written ($(head -1 "$5" 2> /dev/null))"
    if printf '%s\n%s\n%s\n' "$A" "$B" "$C" | grep -aqE "$BAD"; then
        _f "$(printf '%s\n%s\n%s\n' "$A" "$B" "$C" | grep -aE "$BAD" | head -1)"
    fi
    return "$fail"
}

A1="[nvme] namespace 1: 32768 blocks of 512 B (16 MiB), ours (AFRIVFS1): used for storage
[nvme] controller 00:03.0 (NVMe 1.4), I/O queue pair ready
[nvme] multi-block reads on (up to 128 blocks per read, 32 per command; the first 128 equal one at a time)
[nvme] the controller takes 16 KiB per command: a run of up to 128 blocks goes out as up to 4 commands at once
[nvme] the controller has a volatile write cache: every save ends with a Flush
afrisovera> nia big.wasm
    [wasm app] print(4242) = 0x1092
    [wasm app] print(90) = 0x5a
afrisovera> nvmetest prp
nvmetest prp: 127 blocks from block 1 read into a buffer at page offset 512 (4 command(s), PRP list): equal to one block at a time
afrisovera> write keep.txt KEEP-512
wrote 8 bytes to keep.txt
afrisovera> sync
[nvme] multi-block writes on (up to 128 blocks per write, 32 per command; the first 128 read back equal)
[vfs] synced 4 files to disk
afrisovera> lsnvme
  queue: most commands in flight at once 4, PRP-list commands 40, largest transfer 16 KiB, controller takes 16 KiB per command
  volatile write cache: yes, 1 Flush(es) sent (one at the end of every save)
afrisovera> write lost.txt LOST-512
wrote 8 bytes to lost.txt"
A2="afrisovera> cat keep.txt
KEEP-512
afrisovera> cat lost.txt
cat: no such file
afrisovera> nvmetest drop
nvmetest: the next I/O completion will be lost on purpose (the reset path takes over)
afrisovera> write drop.txt DROP-512
afrisovera> sync
[nvme] nvmetest: the completion of command id 7 is lost on purpose
[nvme] 1 of 4 command(s) sent together got no answer
[nvme] a command got no answer: resetting the controller and making its queues again (CSTS 0x1)
[nvme] controller ready again after the reset; the read-ahead was dropped
[nvme] sending the 4 command(s) again after the reset
[vfs] synced 5 files to disk
afrisovera> lsnvme
  controller resets (a command got no answer, or nvmereset): 1
  completions: 0 late one(s) set aside, 1 lost on purpose (nvmetest drop)"
A3="afrisovera> cat drop.txt
DROP-512
afrisovera> cat keep.txt
KEEP-512
afrisovera> nia big.wasm
    [wasm app] print(4242) = 0x1092
    [wasm app] print(90) = 0x5a"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-nvmequeue-selftest.XXXXXX")" || return 1
    echo "mkvfs-big: check OK" > "$D/hostok"; printf 'mkvfs-big: slot 1: extension sector 2 differs
mkvfs-big: check FAILED
' > "$D/hostbad"
    arm() { # arm <name> <expect> <boot1> <boot2> <boot3> [block bytes] [host log]
        local got
        printf '%s\n' "$3" > "$D/a"; printf '%s\n' "$4" > "$D/b"; printf '%s\n' "$5" > "$D/c"
        if grade "$D/a" "$D/b" "$D/c" "${6:-512}" "${7:-$D/hostok}" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    k4() { printf '%s\n' "$1" | sed 's/32768 blocks of 512 B/4096 blocks of 4096 B/; s/up to 128 blocks/up to 16 blocks/g; s/32 per command/4 per command/g;
        s/the first 128 /the first 16 /g; s/127 blocks from block 1/15 blocks from block 1/; s/-512$/-4096/'; }
    s() { printf '%s\n' "$1" | sed "$2"; }
    echo "  --- grader selftest (no QEMU) ---"
    arm model-512 green "$A1" "$A2" "$A3"
    arm model-4k green "$(k4 "$A1")" "$(k4 "$A2")" "$(k4 "$A3")" 4096
    arm crlf green "$(s "$A1" 's/$/\r/')" "$(s "$A2" 's/$/\r/')" "$(s "$A3" 's/$/\r/')"
    arm 4k-as-512 red "$(k4 "$A1")" "$(k4 "$A2")" "$(k4 "$A3")"
    arm not-split red "$(s "$A1" '/goes out as up to 4 commands/d')" "$A2" "$A3"
    arm reads-8k red "$(s "$A1" 's/up to 128 blocks per read, 32/up to 16 blocks per read, 16/')" "$A2" "$A3"
    arm one-in-flight red "$(s "$A1" 's/in flight at once 4/in flight at once 1/')" "$A2" "$A3"
    arm no-prp-list red "$(s "$A1" 's/PRP-list commands 40/PRP-list commands 0/')" "$A2" "$A3"
    arm over-mdts red "$(s "$A1" 's/largest transfer 16 KiB/largest transfer 64 KiB/')" "$A2" "$A3"
    arm prp-different red "$(s "$A1" 's/equal to one block at a time$/DIFFERENT from one block at a time/')" "$A2" "$A3"
    arm no-flush red "$(s "$A1" 's/yes, 1 Flush/yes, 0 Flush/')" "$A2" "$A3"
    arm flush-per-sector red "$(s "$A1" 's/yes, 1 Flush/yes, 30 Flush/')" "$A2" "$A3"
    arm no-vwc-ok green "$(s "$A1" '/volatile write cache/d')" "$A2" "$A3"
    arm keep-lost red "$A1" "$(s "$A2" '/^KEEP-512$/d')" "$A3"
    arm lost-kept red "$A1" "$(s "$A2" 's/^cat: no such file$/LOST-512/')" "$A3"
    arm no-reset red "$A1" "$(s "$A2" '/resetting the controller/d')" "$A3"
    arm not-resent red "$A1" "$(s "$A2" '/sending the 4 command/d')" "$A3"
    arm two-resets red "$A1" "$(s "$A2" 's/nvmereset): 1$/nvmereset): 2/')" "$A3"
    arm late-taken red "$A1" "$(s "$A2" 's/0 late one(s)/1 late one(s)/')" "$A3"
    arm no-comeback red "$A1" "$(printf '%s\n[nvme] the controller did not come back after the reset (CSTS 0x3): x' "$A2")" "$A3"
    arm drop-gone red "$A1" "$A2" "$(s "$A3" '/^DROP-512$/d')"
    arm wasm-broken red "$A1" "$A2" "$(s "$A3" '/print(90)/d')"
    arm host-differs red "$A1" "$A2" "$A3" 512 "$D/hostbad"
    arm panic red "$A1" "$A2" "$(printf '%s\nKERNEL PANIC\n' "$A3")"
    arm empty red "" "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 25 ]
}

echo "== PC NVMEQUEUE: 64 KiB runs, PRP lists, several commands in flight, reset and resend, save and Flush =="
if ! selftest; then echo "PC NVMEQUEUE TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC NVMEQUEUE SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC NVMEQUEUE TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
PY=python3; command -v python3 > /dev/null || PY=python

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC NVMEQUEUE TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-nvmequeue.XXXXXX")"
M="$PY scripts/lib/mkvfs-big.py"
$M bigwasm "$W/big.wasm" && $M bigtext "$W/big.txt" && printf 'HELLO-SMALL' > "$W/note.txt"
FILES=(big.wasm="$W/big.wasm" big.txt="$W/big.txt" note.txt="$W/note.txt")

boot() { # boot <img> <block bytes> <serial port> <log> <commands...>
    local img=$1 bs=$2 sp=$3 log=$4; shift 4
    qemu-system-aarch64 -M virt -m 256M -display none \
        -drive "file=$img,if=none,format=raw,id=nv" \
        -device "nvme,serial=afriq1,drive=nv,mdts=2,logical_block_size=$bs,physical_block_size=$bs" \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" $PY scripts/shell-demo.py "$sp" "$((sp+1))" "$W/shot.png" "$@" > "$log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
}

ok=1
for bs in 512 4096; do
    img="$W/nvme$bs.img"
    $M image "$img" --sectors 32768 "${FILES[@]}" || { echo "  harness: mkvfs-big.py image failed"; ok=0; continue; }
    if [ "$bs" = 512 ]; then sp=5480; else sp=5486; fi
    echo "  --- $bs-byte blocks, boot 1: big.wasm, nvmetest prp, write keep.txt, sync, lsnvme, write lost.txt (no sync) ---"
    boot "$img" "$bs" "$sp" "$W/b$bs-1.log" "nia big.wasm" "nvmetest prp" "write keep.txt KEEP-$bs" "sync" "lsnvme" "write lost.txt LOST-$bs"
    echo "  --- $bs-byte blocks, boot 2: cat keep.txt, cat lost.txt, nvmetest drop, write drop.txt, sync, lsnvme ---"
    boot "$img" "$bs" "$((sp+2))" "$W/b$bs-2.log" "cat keep.txt" "cat lost.txt" "nvmetest drop" "write drop.txt DROP-$bs" "sync" "lsnvme"
    echo "  --- $bs-byte blocks, boot 3: cat drop.txt, cat keep.txt, big.wasm ---"
    boot "$img" "$bs" "$((sp+4))" "$W/b$bs-3.log" "cat drop.txt" "cat keep.txt" "nia big.wasm"
    $M check "$img" "${FILES[@]}" > "$W/host$bs.log" 2>&1
    sed 's/^/  host: /' "$W/host$bs.log" | tail -3
    cat "$W/b$bs-1.log" "$W/b$bs-2.log" "$W/b$bs-3.log" | tr -d '\r' |
        grep -aE '^\[nvme\]|^  (namespace|reads|writes|queue|volatile|completions|controller resets)|^\[vfs\]|^nvmetest|^(KEEP|LOST|DROP)-|^print' | sed 's/^/  | /'
    echo "  --- grading ($bs-byte blocks) ---"
    grade "$W/b$bs-1.log" "$W/b$bs-2.log" "$W/b$bs-3.log" "$bs" "$W/host$bs.log" || ok=0
done

if [ "$ok" = 1 ]; then
    rm -rf "$W"
    echo "PC NVMEQUEUE TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC NVMEQUEUE TEST FAILED"
exit 1
