#!/usr/bin/env bash
# PERFORMANCE suite (perf = the ARM phone/board kernel, pc-perf = the x86-64 PC kernel; 13 Nov 2026).
#
# The owner's mandate: keep AfriSovera fast on a 4 GB machine and good for servers. This suite boots the kernel
# under QEMU's instruction-count clock (-icount shift=0: 1 guest instruction = 1 ns), so the numbers the kernel
# measures itself repeat on any runner, and turns its own output into numbers (scripts/perf-bench.py, read it):
#   boot.kernel_ms        kernel clock at the shell prompt
#   mem.idle_*            heap and physical frames in use at the idle prompt; mem.idle_total_kib adds the image
#   mem.apps3_*           heap high-water rise while 3 apps draw windows (`wm`), and what they leave behind
#   image.*               the kernel's own RAM: code, read-only data, data, zeroed data (KiB)
#   cpu.sha256_*          `bench`: SHA-256 over 2 MiB
#   draw.*                `fbcon time|blit|wm`: the drawing timers (us)
#   timer.sleep50_over_ms `sleep 50` overshoot
#   net.sink_*            `net tcp sink`: bytes a peer pushes in 6 s (host-clock, graded loosely)
#
# GRADED against docs/perf/budgets.tsv (hard ceilings, owner-facing) and docs/perf/baseline.tsv (the last kept
# numbers; a metric worse than its tolerance is red, so a slowdown is caught the day it lands). To move the
# baseline after a deliberate change, see docs/perf/README.md. Every run's numbers go to
# build/perf/perf-<arch>.tsv and, on GitHub, to the run summary.
#
# Usage: scripts/perf-test.sh [--selftest]      (pc-perf-test.sh = the same with --pc)
set -uo pipefail
cd "$(dirname "$0")/.." || exit 2

ARCH=arm
PROF=debug          # what the owner's USB stick ships today (scripts/make-pc-usb.sh)
[ "${1:-}" = "--pc" ] && { ARCH=pc; shift; }
[ "${1:-}" = "--release" ] && { PROF=release; shift; }
LABEL="$ARCH"; [ "$PROF" = release ] && LABEL="$ARCH-rel"
CF=(); [ "$PROF" = release ] && CF=(--release)
python3 scripts/perf-bench.py --selftest | tail -1 | grep -q PASSED ||
    { python3 scripts/perf-bench.py --selftest; echo "PERF TEST FAILED (grader selftest)"; exit 1; }
[ "${1:-}" = "--selftest" ] && { echo "PERF SELFTEST PASSED"; exit 0; }

OUT="build/perf"
mkdir -p "$OUT"
if [ "$ARCH" = arm ]; then
    cargo build "${CF[@]}" -p mansa-kernel 2>&1 | tail -1
    K="target/aarch64-unknown-none/$PROF/mansa-kernel"
    OVMF=""
else
    cargo build "${CF[@]}" -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -1
    K="target/x86_64-unknown-uefi/$PROF/mansa-pc.efi"
    QDIR="$(dirname "$(command -v qemu-system-x86_64)")"
    OVMF=""
    for f in "$QDIR/../share/qemu/edk2-x86_64-code.fd" "$QDIR/share/edk2-x86_64-code.fd" \
             /usr/share/qemu/edk2-x86_64-code.fd /usr/share/OVMF/OVMF_CODE.fd /usr/share/ovmf/OVMF.fd; do
        [ -r "$f" ] && { OVMF="$f"; break; }
    done
    [ -n "$OVMF" ] || { echo "PERF TEST FAILED (no x86-64 UEFI firmware)"; exit 2; }
fi
[ -r "$K" ] || { echo "PERF TEST FAILED ($K was not built)"; exit 2; }

python3 scripts/perf-bench.py run --arch "$ARCH" --kernel "$K" --ovmf "${OVMF:-none}" --out "$OUT" --label "$LABEL" \
    > "$OUT/perf-$LABEL.run" 2>&1
R="$(tail -1 "$OUT/perf-$LABEL.run")"
if [ -z "$R" ] || [ ! -r "$R" ]; then
    sed 's/^/  /' "$OUT/perf-$LABEL.run"
    echo "  --- QEMU's own messages ---"; head -20 "$OUT/perf-$LABEL-qemu.err" 2>/dev/null | sed 's/^/  /'
    echo "  --- last serial lines ---"; tail -c 3000 "$OUT/perf-$LABEL.serial" 2>/dev/null | tr -d '\r' | sed 's/^/  /'
    echo "  --- where the CPU is (QEMU info registers, 3 samples) ---"
    grep -aE "^(RIP|RSP|RAX|RCX|RFL|CR2|EFER|PC=| *PC )|^----|^ ?pc" "$OUT/perf-$LABEL-regs.txt" 2>/dev/null | sed 's/^/  /'
    if [ "$ARCH" = pc ]; then
        # map RIP to a function: relink once with a map (same code), and note where the image was loaded
        timeout 200 cargo rustc "${CF[@]}" -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi \
            -- -C link-arg=/MAP:build/perf/mansa-pc-$LABEL.map > /dev/null 2>&1 &&
            python3 scripts/perf-bench.py riplookup "build/perf/mansa-pc-$LABEL.map" "$OUT/perf-$LABEL-regs.txt" \
                "$OUT/perf-$LABEL.serial"
    fi
    echo "PERF TEST FAILED (the $ARCH kernel did not reach the shell under the benchmark rig)"
    exit 1
fi

# Where the kernel's own RAM goes: its biggest buffers (information for the budgets, never graded).
echo "  --- the biggest symbols of the $ARCH image (KiB) ---"
echo "  --- the slowest boot steps (host seconds before each line; firmware included) ---"
sed 's/^/  /' "$OUT/perf-$LABEL-boot.txt" 2>/dev/null
echo "  --- the drawing and desktop timers as printed ---"
grep -aE '^\[(fbcon|fbblit|fbwm|desk)\]' "$OUT/perf-$LABEL.serial" | tr -d '\r' | cut -c1-240 | sed 's/^/  /'
if [ "$PROF" = release ]; then
    :   # the debug suite lists the symbols; the release image has the same buffers
elif [ "$ARCH" = arm ]; then
    NM="$(rustc --print sysroot)/lib/rustlib/x86_64-unknown-linux-gnu/bin/llvm-nm"
    [ -x "$NM" ] && "$NM" -S --size-sort -C "$K" 2>/dev/null | tail -20 |
        awk '{printf "  %8d KiB  %s  %s\n", strtonum("0x"$2)/1024, $3, $4}'
else
    # the .efi has no symbol table: relink the same code once with a map (time-boxed, never fatal)
    timeout 150 cargo rustc -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi \
        -- -C link-arg=/MAP:build/perf/mansa-pc.map > /dev/null 2>&1 &&
        python3 scripts/perf-bench.py mapsyms build/perf/mansa-pc.map &&
        python3 scripts/perf-bench.py mapmods build/perf/mansa-pc.map | tee build/perf/pc-modules-boot.txt ||
        echo "  (no map this run)"
fi

COMMIT="$(git rev-parse --short=8 HEAD 2>/dev/null || echo unknown)"
echo "== perf ($LABEL) at $COMMIT, $(date -u +%Y-%m-%d)"
python3 scripts/perf-bench.py grade --results "$R" --budgets docs/perf/budgets.tsv --baseline docs/perf/baseline.tsv
rc=$?
# a row per metric, ready to append to docs/perf/history.tsv: date commit arch metric value
awk -v d="$(date -u +%Y-%m-%d)" -v c="$COMMIT" -F'\t' '{print d"\t"c"\t"$1"\t"$2"\t"$3}' "$R" > "$OUT/history-rows-$LABEL.tsv"
echo "  --- history rows (docs/perf/history.tsv) ---"
sed 's/^/  HIST\t/' "$OUT/history-rows-$LABEL.tsv"
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
    { echo "### perf ($LABEL) at $COMMIT"; echo; echo '```'; cat "$OUT/history-rows-$LABEL.tsv"; echo '```'; } >> "$GITHUB_STEP_SUMMARY"
fi
if [ "$rc" -eq 0 ]; then
    echo "PERF TEST PASSED ($LABEL: every metric inside its budget and its baseline tolerance)"
else
    echo "PERF TEST FAILED ($LABEL: see the FAIL lines above)"
fi
exit "$rc"
