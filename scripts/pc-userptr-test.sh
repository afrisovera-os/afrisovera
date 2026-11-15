#!/usr/bin/env bash
# pc-userptr: gap G2 on the PC kernel (docs/security-model.md), the PC twin of el0-ptr.
#
# `uptrtest` runs the ring-3 program `uptr` (pc/proc.rs) with the address of a kernel canary. It hands every call
# that takes a user address four bad 16-byte ranges: the kernel canary, a non-canonical address, a range from the
# top of its own stack past the end of its ring-3 range, and one that wraps from the top of the address space into
# the kernel's low memory. Graded, in order, from its SYS_PRINT lines:
#   a 4 KiB object mapped at MAP_LO (so VMO_UNMAP has a live mapping to match); cell_open on its CELL handle (0);
#   for each range: SYS_AI with it as the prompt, SYS_AI with it as the reply buffer, SYS_SPEAK with it as the
#   text (each EPERM), SYS_VMO_UNMAP of it (E_INVAL), SYS_CELL_PUT of 16 bytes from it (E_FAULT), SYS_DNS_A with it
#   as the name (E_FAULT; uptr holds a NET handle, 1, so the call reaches its pointer);
#   SYS_AI with its own read-only code page as the reply buffer (EPERM: inside its range but not writable; a
#   kernel write there would fault in ring 0); the object's own unmap (0), then 0x5afe.
# Then: the canary is intact and was never spoken, the program ended without a fault, every frame came back, and
# the shell still answers. The other calls take no user address (SEND, RECV, the WIN calls, VMO_CREATE, CELL_OPEN,
# CELL_CURSOR, CELL_SIZE, CELL_EVENT, TCP_DIAL, TCP_CLOSE; VMO_MAP's offset must be 0). TCP_SEND and TCP_RECV take a
# pointer but check a live CONN handle first, which needs a network peer: they copy only through pc/uaccess.rs
# (scripts/uaccess-audit.sh), and their bad-pointer cases belong in the net suite's rig (open item). The test rig's fw_cfg flag and the desk are there
# because CELL_PUT checks its CELL handle before its pointer.
# Every copy goes through pc/uaccess.rs (G2 part 3), which walks the caller's own table, so the cases above are
# refused there even when a call's own range check is wrong. Both boots run on `-cpu max` (PC_CPU, read by the
# pc-arch shim) so SMAP is on: boot 1 must say "SMAP: on" and still pass every case (the copies run inside the
# stac..clac window); boot 2 runs `smapprobe`, a plain kernel read of a ring-3 page, which SMAP must stop and name;
# boot 3 runs `actest`: a program sets RFLAGS.AC itself (popfq), then asks for a raw kernel read (SYS_TEST_RAWREAD,
# test rig only). The kernel clears AC on every entry (cpu.rs isr_common), so SMAP must still stop it.
# RULE: a new system call that takes a user address adds its line here and to `uptr` (pc/proc.rs) in the same commit.
set -u
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
PY="${PY:-python3}"
echo "== pc-userptr: ring-3 calls handed kernel and bad addresses, PC kernel =="
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC USERPTR TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"
export PC_CPU="${PC_CPU:-max}"
W="$(mktemp -d "$T/pc-userptr.XXXXXX")"
qemu-system-aarch64 -M virt -m 256M -display none \
    -fw_cfg name=opt/afrisovera/test-grants,string=1 \
    -serial "tcp:127.0.0.1:6206,server,nowait" -qmp "tcp:127.0.0.1:6207,server,nowait" &
QPID=$!
timeout 200 $PY -u scripts/lib/cell-keys.py 6206 6207 "$W" "gui" "desk mode pointer" "uptrtest" "mem" > "$W/serial.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
qemu-system-aarch64 -M virt -m 256M -display none \
    -fw_cfg name=opt/afrisovera/test-grants,string=1 \
    -serial "tcp:127.0.0.1:6206,server,nowait" -qmp "tcp:127.0.0.1:6207,server,nowait" &
QPID=$!
timeout 120 $PY -u scripts/lib/cell-keys.py 6206 6207 "$W" "&smapprobe" "@SMAP stopped" "~1" > "$W/probe.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
P="$(tr -d '\r' < "$W/probe.log" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g; s/^[[:space:]]*//')"
grep -aE '^\[smap\]|SMAP' <<< "$P" | sed 's/^/  probe | /' | head -8
qemu-system-aarch64 -M virt -m 256M -display none \
    -fw_cfg name=opt/afrisovera/test-grants,string=1 \
    -serial "tcp:127.0.0.1:6206,server,nowait" -qmp "tcp:127.0.0.1:6207,server,nowait" &
QPID=$!
timeout 120 $PY -u scripts/lib/cell-keys.py 6206 6207 "$W" "&actest" "@SMAP stopped" "~1" > "$W/ac.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
A="$(tr -d '\r' < "$W/ac.log" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g; s/^[[:space:]]*//')"
grep -aE '^\[actest\]|SMAP|^\[P[0-9]\]' <<< "$A" | sed 's/^/  ac | /' | head -8
S="$(tr -d '\r' < "$W/serial.log" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g; s/^[[:space:]]*//')"
# only what came after `uptrtest` was typed (the boot's own ring-3 tests print [P0] lines too)
V="$(awk '/^afrisovera> uptrtest/{k=1; next} k' <<< "$S")"
grep -aE '^\[P[0-9]\]|^\[uptr\]|speaking' <<< "$V" | sed 's/^/  | /' | head -40

fail=0
_f() { echo "  FAIL: $1"; fail=1; }
grep -aqE 'KERNEL PANIC|panicked' <<< "$S" && _f "the kernel panicked"
EP=0xffffffffffffffff
INVAL=0xfffffffffffffffb
FAULT=0xfffffffffffffffc
NAMES=("the object's address" "cell_open")
WANT=(0x10040000000 0x0)
for r in "the kernel canary" "a non-canonical address" "a range past the top of its stack" "a range that wraps into the kernel"; do
    NAMES+=("AI prompt at $r" "AI reply buffer at $r" "SPEAK text at $r" "VMO_UNMAP of $r" "CELL_PUT from $r" "DNS_A name at $r")
    WANT+=($EP $EP $EP $INVAL $FAULT $FAULT)
done
NAMES+=("AI reply buffer at its own read-only code page" "the object's own unmap" "the end mark")
WANT+=($EP 0x0 0x5afe)
mapfile -t GOT < <(grep -aoE '^\[P[0-9]\] 0x[0-9a-f]+$' <<< "$V" | cut -d' ' -f2)
[ "${#GOT[@]}" -eq "${#WANT[@]}" ] || _f "the program printed ${#GOT[@]} results, wanted ${#WANT[@]}"
for i in "${!WANT[@]}"; do
    [ "${GOT[$i]:-none}" = "${WANT[$i]}" ] || _f "${NAMES[$i]}: got ${GOT[$i]:-none}, wanted ${WANT[$i]}"
done
grep -aqE '^\[uptr\] P[0-9] aims at the kernel canary at 0x[0-9a-f]+$' <<< "$V" || _f "no line naming the canary's address"
grep -aqx '\[uptr\] G2-CANARY intact' <<< "$V" || _f "the kernel canary was not reported intact"
grep -aq 'G2-CANARY OVERWRITTEN' <<< "$V" && _f "kernel memory was written through a ring-3 pointer"
grep -aq 'speaking: "G2-CANARY' <<< "$V" && _f "kernel memory was spoken through a ring-3 pointer"
grep -aqE '^\[uptr\] test: 1 ended, 0 fault\(s\), frames in use ([0-9]+) before and \1 after \(ok\)$' <<< "$V" \
    || _f "the program faulted, did not end, or frames did not all come back"
grep -aq "heap:" <<< "$S" || _f "the shell stopped answering"
grep -aq '^\[pc\] SMAP: on (CR4.SMAP)' <<< "$S" || _f "boot 1: SMAP was not on (-cpu max)"
grep -aq 'SMAP stopped a kernel access' <<< "$S" && _f "boot 1: SMAP stopped a kernel access (a copy outside pc/uaccess.rs)"
grep -aqE "^\[smap\] probe: a plain kernel read of P[0-9]'s code page at 0x10000000000, SMAP on$" <<< "$P" \
    || _f "boot 2: no probe line with SMAP on"
grep -aqE '^\[pc\] SMAP stopped a kernel access to ring-3 memory at 0x10000000000 \(RIP 0x[0-9a-f]+\): only pc/uaccess.rs may touch it; halted$' <<< "$P" \
    || _f "boot 2: SMAP did not stop the kernel's plain read of a ring-3 page"
grep -aq 'nothing stopped it' <<< "$P" && _f "boot 2: the kernel read ring-3 memory outside the door"
grep -aqE '^\[actest\] P[0-9] sets RFLAGS.AC, then asks for a raw kernel read of its code page, SMAP on$' <<< "$A" \
    || _f "boot 3: no actest line with SMAP on"
grep -aqE '^\[pc\] SMAP stopped a kernel access to ring-3 memory at 0x10000000000 \(RIP 0x[0-9a-f]+\): only pc/uaccess.rs may touch it; halted$' <<< "$A" \
    || _f "boot 3: with RFLAGS.AC set by the program, SMAP did not stop the kernel's raw read (AC not cleared at entry)"
grep -aq 'nothing stopped it' <<< "$A" && _f "boot 3: the raw read went through with the program's AC set"

if [ "$fail" -eq 0 ]; then
    rm -rf "$W"
    echo "PC USERPTR TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC USERPTR TEST FAILED"
exit 1
