#!/usr/bin/env bash
# dira-bin: the committed Dira program is the one its source builds (the architect's S4 rule: no hand-committed bytes).
# Rebuilds userland/dira with scripts/build-dira.sh into a temporary folder, with the pinned toolchain, and compares
# kernel/src/pc/dira-x86_64.bin and kernel/src/pc/dira_bin.rs (hash, source hash, toolchain) byte for byte.
set -u
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
echo "== dira-bin: the committed Dira program is the one its source builds =="
W="$(mktemp -d "$T/dira-bin.XXXXXX")"
if ! scripts/build-dira.sh "$W" > "$W/build.log" 2>&1; then
    tail -30 "$W/build.log"
    echo "DIRA BIN TEST FAILED (the program did not build)"
    exit 1
fi
cat "$W/build.log"
fail=0
cmp -s "$W/dira-x86_64.bin" kernel/src/pc/dira-x86_64.bin || { echo "  FAIL: kernel/src/pc/dira-x86_64.bin is not what the source builds"; fail=1; }
cmp -s "$W/dira_bin.rs" kernel/src/pc/dira_bin.rs || { echo "  FAIL: kernel/src/pc/dira_bin.rs differs:"; diff "$W/dira_bin.rs" kernel/src/pc/dira_bin.rs | head -12; fail=1; }
if [ "$fail" -eq 0 ]; then
    rm -rf "$W"
    echo "DIRA BIN TEST PASSED"
    exit 0
fi
echo "  rebuilt files kept in $W"
echo "DIRA BIN TEST FAILED"
exit 1
