#!/usr/bin/env bash
# Safe updates U3 (14 Nov 2026): updatestage, every refusal (a bad signature, the 32 MiB cap, an EFI partition not closed cleanly, one nearly full, a hand-made "AfriSovera backup" entry, two entries named exactly "AfriSovera"), each leaving the disk byte for byte as it was.
# Read scripts/pc-updatestage-test.sh.
#   scripts/pc-updatestage-refuse-test.sh [--selftest]
UPSTAGE_KIND=refuse exec bash "$(dirname "$0")/pc-updatestage-test.sh" "$@"
