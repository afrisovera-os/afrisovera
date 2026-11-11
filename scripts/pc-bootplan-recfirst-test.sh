#!/usr/bin/env bash
# PC track step 5 (11 Nov 2026): bootplan on a Windows disk whose recovery partition comes BEFORE the EFI partition, as
# some OEM disks have it, then the full and the not-closed-cleanly refusals, in two boots (read
# scripts/pc-bootplan-test.sh). The plan must name the EFI partition as GPT entry 2 and the entry as sata port 2
# partition 2 (start 43008).
#   scripts/pc-bootplan-recfirst-test.sh [--selftest]
BOOTPLAN_KIND=recfirst exec bash "$(dirname "$0")/pc-bootplan-test.sh" "$@"
