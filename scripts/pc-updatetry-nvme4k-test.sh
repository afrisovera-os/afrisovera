#!/usr/bin/env bash
# Safe updates U4 (15 Nov 2026): updatetry, the trial, keep and updateback with the Windows-like disk on an NVMe namespace of 4096-byte blocks (the data disk on SATA).
# Read scripts/pc-updatetry-test.sh.
#   scripts/pc-updatetry-nvme4k-test.sh [--selftest]
UPTRY_KIND=nvme4k exec bash "$(dirname "$0")/pc-updatetry-test.sh" "$@"
