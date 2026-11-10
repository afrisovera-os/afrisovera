#!/usr/bin/env bash
# CLOUD-ONLY. Download the NIST CAVP DRBG test vectors, hash them, extract the HMAC_DRBG .rsp files, and run the
# parser to produce the host-test fixtures and the boot-KAT Rust constant. Per the DRBG rules (ADR 0007):
#   (a) download the CAVP zip from csrc.nist.gov over HTTPS; log the URL, the file size, and the sha256 of the
#       zip and each extracted .rsp;
#   (b) the parser reads the .rsp bytes directly - nothing is transcribed or taken from memory;
#   (c) the boot-KAT Rust constant is generated from COUNT=0 of the no-reseed file, and the host test (run
#       separately) parses the .rsp, runs the DRBG and asserts its output equals the generated constant;
#   (d) the no-reseed .rsp sha256 is printed so it goes into the commit message.
# Needs network to csrc.nist.gov, so it runs in the cloud job, NEVER on the shared dev PC.
set -euo pipefail
cd "$(dirname "$0")/.."
# Default URL; the cloud operator confirms it resolves. The logged size + sha256 make a wrong/moved URL obvious.
URL="${1:-https://csrc.nist.gov/CSRC/media/Projects/cryptographic-algorithm-validation-program/documents/drbg/drbgtestvectors.zip}"
OUT="${2:-build/drbg-cavp}"
mkdir -p "$OUT/unz"
echo "CAVP URL: $URL"
curl -fsSL "$URL" -o "$OUT/drbgtestvectors.zip"
echo "zip bytes:  $(wc -c < "$OUT/drbgtestvectors.zip")"
echo "zip sha256: $(sha256sum "$OUT/drbgtestvectors.zip" | cut -d' ' -f1)"
unzip -o "$OUT/drbgtestvectors.zip" -d "$OUT/unz" >/dev/null
# The NIST zip holds one zip per test mode (pr_false, pr_true, no_reseed); unpack each into its own folder named
# after it, so the paths below say which mode a .rsp came from. Logged, so a changed layout is visible.
find "$OUT/unz" -type f -name '*.zip' | sort | while read -r z; do
  echo "inner zip: ${z#$OUT/unz/}  sha256 $(sha256sum "$z" | cut -d' ' -f1)"
  d="${z%.zip}"
  mkdir -p "$d"
  unzip -o "$z" -d "$d" >/dev/null
done
echo "HMAC_DRBG files found:"
find "$OUT/unz" -type f -iname '*HMAC_DRBG*.rsp' | sed "s#^$OUT/unz/#  #"
NO_RESEED=$(find "$OUT/unz" -ipath '*no_reseed*HMAC_DRBG.rsp' | head -1)
PR_FALSE=$(find "$OUT/unz" -ipath '*pr_false*HMAC_DRBG.rsp' | head -1)
[ -n "$NO_RESEED" ] || { echo "ERROR: no_reseed HMAC_DRBG.rsp not found in the zip" >&2; exit 2; }
[ -n "$PR_FALSE" ]  || { echo "ERROR: pr_false HMAC_DRBG.rsp not found in the zip" >&2; exit 2; }
NR_SHA=$(sha256sum "$NO_RESEED" | cut -d' ' -f1)
PF_SHA=$(sha256sum "$PR_FALSE"  | cut -d' ' -f1)
echo "no_reseed HMAC_DRBG.rsp: $NO_RESEED  sha256 $NR_SHA"
echo "pr_false  HMAC_DRBG.rsp: $PR_FALSE  sha256 $PF_SHA"
python3 -I scripts/drbg-cavp-parse.py "$NO_RESEED" --json "$OUT/no_reseed.json" \
        --rust-const kernel/src/drbg_kat.rs --rsp-sha256 "$NR_SHA"
python3 -I scripts/drbg-cavp-parse.py "$PR_FALSE"  --json "$OUT/pr_false.json"
echo
echo "drbg-cavp-fetch: wrote $OUT/no_reseed.json, $OUT/pr_false.json and kernel/src/drbg_kat.rs"
echo "RECORD IN THE COMMIT MESSAGE -> no_reseed HMAC_DRBG.rsp sha256: $NR_SHA"
echo "                               pr_false  HMAC_DRBG.rsp sha256: $PF_SHA"
