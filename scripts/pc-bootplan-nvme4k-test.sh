#!/usr/bin/env bash
# PC track step 5 (11 Nov 2026): bootplan with the Windows-like disk on an NVMe namespace of 4096-byte blocks, as many
# laptop SSDs have: the plan, then the full and the not-closed-cleanly refusals, in three boots (read
# scripts/pc-bootplan-test.sh). The plan must name nvme partition 1 (start 256), counted in the disk's own 4 KiB blocks.
#   scripts/pc-bootplan-nvme4k-test.sh [--selftest]
BOOTPLAN_KIND=nvme4k exec bash "$(dirname "$0")/pc-bootplan-test.sh" "$@"
