#!/usr/bin/env bash
# afri-elf (compatibility stage 0 step 3 groundwork, AFR-0001 7.7): the Daraja Linux bridge's ELF reader, on the host.
# `cargo test` in userland/afri-elf: a good static program's plan (segments, entry, AT_PHDR, the TLS template), every
# refusal by name (header, segments, W^X, executable stack, shared pages, the usual Linux address outside our area,
# interpreter and dynamic programs, TLS and program-header placement), and 200,000 random header mutations that must
# never panic, with every accepted file checked against the rules.
#   scripts/afri-elf-test.sh
set -uo pipefail
cd "$(dirname "$0")/../userland/afri-elf"
# the repository's .cargo/config.toml builds for the bare-metal ARM target by default: tests run on the host
HOST="$(rustc -vV | sed -n 's/^host: //p')"
out="$(cargo test --offline --target "$HOST" 2>&1)"; rc=$?
printf '%s\n' "$out" | grep -E '^test |test result:' | sed 's/^/  | /'
if [ $rc -eq 0 ] && grep -qE '^test result: ok\. 6 passed; 0 failed' <<< "$out"; then
    echo "AFRI-ELF TEST PASSED"; exit 0
fi
printf '%s\n' "$out" | tail -30
echo "AFRI-ELF TEST FAILED"; exit 1
