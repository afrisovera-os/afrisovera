#!/usr/bin/env bash
# PC track step 5 (11 Nov 2026): bootplan on a Windows disk of 4096-byte blocks (an NVMe namespace, as many laptop SSDs
# have) whose recovery partition comes BEFORE the EFI partition, then the full and the not-closed-cleanly refusals, in
# three boots (read scripts/pc-bootplan-test.sh). The plan must name the EFI partition as GPT entry 2 (LBA 5376-71935)
# and the entry as nvme partition 2 (start 5376), counted in the disk's own 4 KiB blocks.
#   scripts/pc-bootplan-recfirst4k-test.sh [--selftest]
BOOTPLAN_KIND=recfirst4k exec bash "$(dirname "$0")/pc-bootplan-test.sh" "$@"
