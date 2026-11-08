#!/usr/bin/env bash
# Verify `unpolybius <codes>` decodes space-separated 5x5 Polybius row-col pairs back to letters
# (I/J share 24 -> 'i'), the inverse of polybius. "11 12 13"->"abc", "23 15 31 31 34"->"hello",
# "24 11 55 55"->"iazz" (J folded), "34 43"->"os", malformed token->'?'. Round-trip
# unpolybius(polybius(x)) == x for j-free lowercase x. Self-checking: python Polybius mapping.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4998,server,nowait \
    -qmp tcp:127.0.0.1:4999,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/unpolybius.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/unpolybius.png" \
    "echo warmup" "unpolybius 11 12 13" "unpolybius 23 15 31 31 34" \
    "unpolybius 24 11 55 55" "unpolybius 34 43" "unpolybius 16 xx 21" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "unpolybius:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

ALPHA = "abcdefghiklmnopqrstuvwxyz"       # 25 letters, j removed

def unpolybius(codes):                    # DERIVED reference
    out = []
    for tok in codes.split():
        if len(tok) == 2 and tok[0] in "12345" and tok[1] in "12345":
            out.append(ALPHA[(int(tok[0]) - 1) * 5 + int(tok[1]) - 1])
        else:
            out.append("?")
    return "".join(out)

def polybius(t):                          # encoder reference for the round trip
    codes = []
    for ch in t:
        if ch.isascii() and ch.isalpha():
            c = ch.lower()
            if c == "j": c = "i"
            idx = ord(c) - ord("a")
            if c > "i": idx -= 1
            codes.append(f"{idx // 5 + 1}{idx % 5 + 1}")
    return " ".join(codes)

for codes in ["11 12 13", "23 15 31 31 34", "24 11 55 55", "34 43", "16 xx 21"]:
    want = "unpolybius: " + unpolybius(codes)
    if want not in s:
        print(f"UNPOLYBIUS TEST FAILED (missing '{want}')"); sys.exit(1)

assert unpolybius("11 12 13") == "abc"
assert unpolybius("23 15 31 31 34") == "hello"
assert unpolybius("24 11 55 55") == "iazz"    # J shares I's cell
assert unpolybius("34 43") == "os"
assert unpolybius("16 xx 21") == "??f"
for x in ["abc", "hello", "os", "quickbrownfox"]:   # j-free lowercase round trip
    assert unpolybius(polybius(x)) == x
print("UNPOLYBIUS TEST PASSED  (abc / hello / iazz / os / ??f + round-trip vs python)")
PY
