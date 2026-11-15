#!/usr/bin/env bash
# Safe updates U3 (14 Nov 2026): updatestage, the stage, again and remove boots with the Windows-like disk on an NVMe namespace of 4096-byte blocks.
# Read scripts/pc-updatestage-test.sh.
#   scripts/pc-updatestage-nvme4k-test.sh [--selftest]
UPSTAGE_KIND=nvme4k exec bash "$(dirname "$0")/pc-updatestage-test.sh" "$@"
