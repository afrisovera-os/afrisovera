#!/usr/bin/env bash
# daraja-linux (compatibility stage 0 step 3 groundwork, AFR-0001 7.7): the Linux bridge's kernel-free parts, on the
# host. `cargo test` in userland/daraja-linux: the bundle reader (cpio newc: files and folders found, links never
# served, paths that try to leave the bundle refused, broken archives refused, 100,000 mutations without a panic) and
# the start-up stack (the psABI layout: argc, argv, envp, the auxiliary vector with AT_PHDR, AT_ENTRY and AT_RANDOM,
# a 16-byte aligned stack pointer at every alignment of the top, and the refusals), the call table and the unknown-call limit.
#   scripts/daraja-linux-test.sh
set -uo pipefail
cd "$(dirname "$0")/../userland/daraja-linux"
# the repository's .cargo/config.toml builds for the bare-metal ARM target by default: tests run on the host
HOST="$(rustc -vV | sed -n 's/^host: //p')"
out="$(cargo test --offline --target "$HOST" 2>&1)"; rc=$?
printf '%s\n' "$out" | grep -E '^test |test result:' | sed 's/^/  | /'
if [ $rc -eq 0 ] && grep -qE '^test result: ok\. 16 passed; 0 failed' <<< "$out"; then
    echo "DARAJA-LINUX TEST PASSED"; exit 0
fi
printf '%s\n' "$out" | tail -30
echo "DARAJA-LINUX TEST FAILED"; exit 1
