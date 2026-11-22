#!/usr/bin/env bash
# The release stick's gate (the performance plan, with the architect's conditions of 14 Nov 2026): every
# registered PC suite (pc-*) against the RELEASE build of the PC kernel, then the release build measured with and
# without integer overflow checks. A runner, not a suite: it takes hours, so run it on a CI runner or a cloud
# machine (bash scripts/pc-release-walk.sh [run-all-tests.sh options]), not on a desktop PC.
#
# How the suites test the release build: every PC suite takes the kernel from PC_EFI (pc-boot, pc-ipc, pc-paging and
# pc-ring3 since 14 Nov; the others already did). They still build the debug kernel first, which costs build time
# only. The perf suites are left out of the walk (they build their own profile) and run at the end.
# Every argument is passed on to run-all-tests.sh (e.g. --timeout 600, --results, --logdir).
set -uo pipefail
cd "$(dirname "$0")/.." || exit 2

echo "[release-walk] building the release PC kernel"
cargo build --release -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
REL="$PWD/target/x86_64-unknown-uefi/release/mansa-pc.efi"
[ -r "$REL" ] || { echo "[release-walk] $REL was not built" >&2; exit 2; }
md5sum "$REL" | sed 's/^/[release-walk] release PC md5: /'
# a copy the debug builds inside the suites cannot touch (they write target/.../debug only, but be sure)
mkdir -p build/release-walk
cp "$REL" build/release-walk/mansa-pc.efi
export PC_EFI="$PWD/build/release-walk/mansa-pc.efi"

ONLY="$(grep -oE '^SUITES=\(.*\)' scripts/run-all-tests.sh | tr ' ()' '\n\n\n' | grep -E '^pc-' | grep -vE '^pc-perf' | paste -sd, -)"
# RELEASE_SUITES="a,b": only those (a recheck of one failure); RELEASE_REPEAT=N: the list N times, to tell a steady
# failure from a timing one
ONLY="${RELEASE_SUITES:-$ONLY}"
rc=0
for r in $(seq 1 "${RELEASE_REPEAT:-1}"); do
    echo "[release-walk] round $r: $(echo "$ONLY" | tr ',' '\n' | wc -l) PC suites against $PC_EFI"
    scripts/run-all-tests.sh --only "$ONLY" "$@" || rc=1
done

echo "[release-walk] perf: release (overflow checks on), then the same with them off"
unset PC_EFI
scripts/perf-test.sh --pc --release; p1=$?
PERF_OVERFLOW=off scripts/perf-test.sh --pc --release; p2=$?
for l in pc-rel pc-rel-noovf; do
    grep -E "boot\.kernel_ms|cpu\.sha256_us|draw\.desk\.frame_us|image\.(text|total)" "build/perf/perf-$l.tsv" 2>/dev/null |
        sed "s/^/[release-walk] /"
done
echo "[release-walk] suites exit $rc, perf $p1, perf with overflow checks off $p2"
exit "$rc"
