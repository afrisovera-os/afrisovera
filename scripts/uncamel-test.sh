#!/usr/bin/env bash
# Verify the `uncamel <text>` command splits a camelCase/PascalCase/snake_case/kebab-case
# identifier into space-separated lowercase words: space before each uppercase that follows
# a lowercase, '_'/'-' become separators, all lowercased. helloWorld->hello world,
# AfriSoveraOS->afri sovera os (acronym OS glued). Self-checking: python mirrors the rule.
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
OUT="${TMPDIR:-/tmp}/uncamel.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/uncamel.png" \
    "echo warmup" "uncamel helloWorld" "uncamel AfriSoveraOS" "uncamel snake_case_here" "uncamel kebab-case-here" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "uncamel:|usage: uncamel" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def uncamel(t):  # mirror the kernel rule
    t = t.strip()
    out = []; prev = ""
    for c in t:
        if c in "_-":
            out.append(" "); prev = " "; continue
        if c.isupper() and prev.islower():
            out.append(" ")
        out.append(c.lower()); prev = c
    return "".join(out)

for t in ["helloWorld", "AfriSoveraOS", "snake_case_here", "kebab-case-here"]:
    want = f"uncamel: {uncamel(t)}"
    if want not in s:
        print(f"UNCAMEL TEST FAILED (missing '{want}')"); sys.exit(1)

# pin notable results explicitly (derived above)
assert "uncamel: hello world" in s              # camelCase
assert "uncamel: afri sovera os" in s           # PascalCase, acronym OS stays glued
assert "uncamel: snake case here" in s          # snake_case underscores
assert "uncamel: kebab case here" in s          # kebab-case hyphens

print("UNCAMEL TEST PASSED  (helloWorld->hello world, AfriSoveraOS->afri sovera os, "
      "snake_case_here/kebab-case-here->... here vs python)")
PY
