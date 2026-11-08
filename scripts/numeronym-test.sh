#!/usr/bin/env bash
# Verify the `numeronym <text>` command abbreviates each word to a NUMERONYM:
# first letter + count of the middle letters + last letter. e.g.
# internationalization -> i18n, accessibility -> a11y, kubernetes -> k8s.
# Words shorter than 3 chars pass through unchanged. DISTINCT from acronym
# (initials only), slug/kebab/snake (case/separator), and per-char maps.
# Self-checking: python replicates the per-word rule and asserts each line.
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
OUT="${TMPDIR:-/tmp}/numeronym.out"
# Warmup `echo` settles the interactive capture (long-word sessions can otherwise
# truncate mid-echo under host load); the assertions ignore it.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/numeronym.png" \
    "echo warmup" "numeronym internationalization" "numeronym accessibility" \
    "numeronym kubernetes" "numeronym localization" "numeronym hello world" \
    "numeronym cat" "numeronym hi" "numeronym" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "numeronym:|usage: numeronym" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def num_word(w):
    return w if len(w) < 3 else f"{w[0]}{len(w) - 2}{w[-1]}"

def numeronym(t):
    return ' '.join(num_word(w) for w in t.split())

cases = ["internationalization", "accessibility", "kubernetes", "localization",
         "hello world", "cat", "hi"]
for text in cases:
    want = f"numeronym: {numeronym(text)}"
    if want not in s:
        print(f"NUMERONYM TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity-pin the classic numeronyms + edge cases (multi-digit counts, 2-word)
assert numeronym("internationalization") == "i18n"
assert numeronym("accessibility") == "a11y"
assert numeronym("kubernetes") == "k8s"
assert numeronym("localization") == "l10n"
assert numeronym("hello world") == "h3o w3d"
assert numeronym("cat") == "c1t"      # 3-letter word -> middle count 1
assert numeronym("hi") == "hi"        # <3 chars -> unchanged

if "usage: numeronym <text>" not in s:
    print("NUMERONYM TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("NUMERONYM TEST PASSED  (i18n/a11y/k8s/l10n incl multi-digit counts, 2-word "
      "case, short words unchanged, empty->usage)")
PY
