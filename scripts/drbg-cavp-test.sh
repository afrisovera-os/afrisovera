#!/usr/bin/env bash
# drbg-cavp: the kernel's HMAC_DRBG (kernel/src/drbg.rs, compiled for the host by #[path]) against EVERY
# [SHA-256] [PredictionResistance = False] entry of the NIST CAVP HMAC_DRBG files, no-reseed and reseed
# (pr_false), plus the streaming SHA-256 and HMAC against RustCrypto, the SP 800-90B health tests (stuck and
# biased sources trip, a good one passes) and the fail-closed generator with mock sources (none / stuck / good /
# appears-late). The fixtures in drbg-host/fixtures/ were produced by scripts/drbg-cavp-fetch.sh from the NIST
# zip (URL, size and sha256 in drbg-host/fixtures/SOURCE.txt); nothing in them was typed by hand.
set -euo pipefail
# The runner reads "<NAME> TEST PASSED" / "<NAME> TEST FAILED" (scripts/lib/verdict.sh).
trap 'rc=$?; [ $rc -eq 0 ] || echo "DRBG-CAVP TEST FAILED (exit $rc)"' EXIT
cd "$(dirname "$0")/.."
python3 -I scripts/drbg-cavp-parse.py --selftest
HOST=$(rustc -vV | sed -n 's/host: //p')
DRBG_CAVP_DIR="$PWD/drbg-host/fixtures" cargo test --manifest-path drbg-host/Cargo.toml --target "$HOST" -- --test-threads=1
echo "DRBG-CAVP TEST PASSED"
