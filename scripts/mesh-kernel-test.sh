#!/usr/bin/env bash
# mesh-kernel: the in-kernel generator and mesh checks, on the real ARM kernel build, two boots.
#
# Boot 1, WITH a virtio-rng device (a counted, health-tested strong source):
#   - `rng`         : the HMAC_DRBG boot KAT (NIST CAVP COUNT=0) says PASS and virtio-rng is healthy;
#   - `rng secret`  : a fail-closed secret draw succeeds;
#   - `mesh selftest`: SHA-256 kernel == mesh-crypto (review S3), DRBG KAT, fresh distinct ephemerals (S4), Noise
#                     XX + KK in memory, a flipped byte and a wrong KK static rejected, bundle seal/open with
#                     tamper and wrong-sender rejects, bundle size = bucket 0. Every line PASS, N/N.
# Boot 2, WITHOUT any strong source (QEMU cortex-a72 has no RNDR, no virtio-rng):
#   - `rng secret`  : REFUSED (NoStrongSource): the fail-closed half, which a test only of the happy path
#                     could never see;
#   - `mesh id`     : REFUSED, so the mesh cannot make a key from timer-only entropy;
#   - `rand 4`      : still works (non-secret callers keep working, as before).
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

T="${TMPDIR:-/tmp}"
OUT1="$T/mesh-kernel-1.out"
OUT2="$T/mesh-kernel-2.out"
. "$(dirname "$0")/lib/afrish-boot.sh"

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -device virtio-rng-device \
    -serial tcp:127.0.0.1:4404,server,nowait \
    -qmp tcp:127.0.0.1:4405,server,nowait -kernel "$BIN" &
QPID=$!
afrish_wait_boot
python3 scripts/shell-demo.py 4404 4405 "$T/mesh-kernel-1.png" "rng" "rng secret" "mesh selftest" "mesh id" > "$OUT1" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4406,server,nowait \
    -qmp tcp:127.0.0.1:4407,server,nowait -kernel "$BIN" &
QPID=$!
afrish_wait_boot
python3 scripts/shell-demo.py 4406 4407 "$T/mesh-kernel-2.png" "rng secret" "mesh id" "rand 4" > "$OUT2" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "\[rng\]|\[mesh\]|\[rand\]" "$OUT1" "$OUT2" | sed 's/^/  /'

python3 - "$OUT1" "$OUT2" <<'PY'
import re, sys
a = open(sys.argv[1], "rb").read().decode("utf-8", "replace")
b = open(sys.argv[2], "rb").read().decode("utf-8", "replace")
bad = []
def need(cond, what):
    if not cond:
        bad.append(what)
# boot 1
need(re.search(r"\[rng\] HMAC_DRBG-SHA256 kat=PASS", a), "boot 1: DRBG boot KAT did not say PASS")
need(re.search(r"\[rng\] source virtio-rng: healthy samples=(\d+)", a), "boot 1: virtio-rng not healthy")
m = re.search(r"\[rng\] source virtio-rng: healthy samples=(\d+)", a)
need(m and int(m.group(1)) >= 1024, "boot 1: fewer than 1024 start-up samples through the health tests")
need(re.search(r"\[rng\] secret OK [0-9a-f]{8}", a), "boot 1: rng secret was not OK")
rows = re.findall(r"\[mesh\] selftest ([a-z0-9-]+): (PASS|FAIL)", a)
need(len(rows) >= 12, "boot 1: expected >= 12 mesh selftest rows, got %d" % len(rows))
need(all(r[1] == "PASS" for r in rows), "boot 1: failing selftest rows: %s" % [r[0] for r in rows if r[1] != "PASS"])
t = re.search(r"\[mesh\] selftest (\d+)/(\d+) PASS", a)
need(t and t.group(1) == t.group(2), "boot 1: mesh selftest total not N/N")
need(re.search(r"\[mesh\] identity x25519 [0-9a-f]{64}", a), "boot 1: no mesh identity")
# boot 2
need(re.search(r"\[rng\] secret REFUSED: NoStrongSource", b), "boot 2: secret draw was NOT refused without a strong source")
need(re.search(r"\[mesh\] identity REFUSED: NoSecretRandom", b), "boot 2: mesh made a key without a strong source")
need(re.search(r"\[rand\]( [0-9a-f]{8}){4}", b), "boot 2: non-secret rand stopped working")
if bad:
    print("MESH-KERNEL TEST FAILED")
    for x in bad:
        print("  - " + x)
    sys.exit(1)
print("MESH-KERNEL TEST PASSED  (KAT PASS, virtio-rng healthy, secret OK; selftest %s/%s; fail-closed without a strong source)" % (t.group(1), t.group(2)))
PY
