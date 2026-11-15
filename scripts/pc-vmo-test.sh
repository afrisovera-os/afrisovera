#!/usr/bin/env bash
# pc-vmo: process memory for ring 3 (AFR-0001 7.6 "v0.5: the Dira subset", S1 of Dira M3), PC kernel.
#
# `vmotest` runs the ring-3 program `vmo` (pc/proc.rs), which prints every result with SYS_PRINT. Graded, in order:
#   a 1 MiB object: handle 0 at MAP_LO; its first and last words read 0 (zeroed); a written word reads back;
#   refusals: EXECUTE (E_PERM), size 100 (E_INVAL), past the 32 MiB cap (E_NOMEM), an empty handle (E_BADHANDLE);
#   a 4 KiB object after the 1 MiB one and its guard page; a buffer across that guard page refused (EPERM);
#   unmap (0), unmap again (E_INVAL); a 16 MiB object first fit after the 4 KiB one;
#   a 4 KiB object created and never mapped (handle 2; its reservation must be back after the process ends);
#   4 KiB objects until the handle table is full, each first fit in the freed 1 MiB, then E_NOMEM;
#   the jump into the 16 MiB object faults (vector 14, the RIP is the object): it is not executable.
# Then: every frame came back (the count before and after the run is the same), nothing stays reserved, `ps` shows no
# object memory, and pc-perf's two timing lines are there. The shell answers `mem` at the end.
set -u
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
PY="${PY:-python3}"
echo "== pc-vmo: ring-3 process memory (vmo_create, vmo_map, vmo_unmap), PC kernel =="
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC VMO TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"
W="$(mktemp -d "$T/pc-vmo.XXXXXX")"
qemu-system-aarch64 -M virt -m 256M -display none \
    -serial "tcp:127.0.0.1:5998,server,nowait" -qmp "tcp:127.0.0.1:5999,server,nowait" &
QPID=$!
$PY scripts/lib/cell-keys.py 5998 5999 "$W" "vmotest" "ps" "mem" > "$W/serial.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
S="$(tr -d '\r' < "$W/serial.log" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g; s/^[[:space:]]*//')"
grep -aE '^\[P[0-9]\]|^\[vmo\]|memory objects' <<< "$S" | sed 's/^/  | /' | head -40

fail=0
_f() { echo "  FAIL: $1"; fail=1; }
grep -aqE 'KERNEL PANIC|panicked' <<< "$S" && _f "the kernel panicked"
LO=0x10040000000
WANT="0x0 $LO 0x0 0x0 0x5eed 0xffffffffffffffff 0xfffffffffffffffb 0xfffffffffffffffa 0xfffffffffffffffe 0x10040101000 0xffffffffffffffff 0x0 0xfffffffffffffffb 0x10040103000 0x2 0x10040000000 0x10040002000 0x10040004000 0x10040006000 0x10040008000 0xfffffffffffffffa"
# only what the program printed after `vmotest` was typed (the boot's own ring-3 tests print [P0] lines too)
V="$(awk '/^afrisovera> vmotest/{k=1; next} k' <<< "$S")"
GOT="$(grep -aoE '^\[P0\] 0x[0-9a-f]+$' <<< "$V" | cut -d' ' -f2 | tr '\n' ' ' | sed 's/ $//')"
[ "$GOT" = "$WANT" ] || { _f "the program printed [$GOT]"; echo "        wanted [$WANT]"; }
grep -aqE '^\[vmo\] P0: 1024 KiB mapped at 0x10040000000, zeroed, not executable \([0-9]+ us\)$' <<< "$S" || _f "no map line for the 1 MiB object"
grep -aqE '^\[vmo\] P0: 16384 KiB mapped at 0x10040103000, zeroed, not executable \([0-9]+ us\)$' <<< "$S" || _f "no map line for the 16 MiB object"
grep -aqE '^\[P0\] fault: vector 14 at RIP 0x10040103000 ' <<< "$S" || _f "the jump into the object did not fault as not executable (vector 14 at its address)"
grep -aqE '^\[vmo\] test: 1 ended, 1 fault\(s\), frames in use ([0-9]+) before and \1 after, 0 pages still reserved \(ok\)$' <<< "$S" || _f "frames did not all come back, or something stayed reserved"
grep -aqE '^\[vmo\] bench: map 1024 KiB: [0-9]+ us$' <<< "$S" || _f "no 1 MiB timing line"
grep -aqE '^\[vmo\] bench: map 16384 KiB: [0-9]+ us$' <<< "$S" || _f "no 16 MiB timing line"
grep -aq "memory objects:" <<< "$S" && _f "ps shows object memory after the program ended"
grep -aq "heap:" <<< "$S" || _f "the shell stopped answering"

if [ "$fail" -eq 0 ]; then
    rm -rf "$W"
    echo "PC VMO TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC VMO TEST FAILED"
exit 1
