#!/usr/bin/env bash
# PC track step 5 (11 Nov 2026): the firmware starting AfriSovera once (BootNext) from a Windows-like SATA disk whose
# recovery partition comes BEFORE the EFI partition, as some OEM disks have. The same six boots as
# scripts/pc-bootnext-test.sh (read that file); bootcopy must copy to partition 2, and the entry the firmware starts
# must say HD(2,GPT,<entry 2's GUID>,start 43008).
#   scripts/pc-bootnext-recfirst-test.sh [--selftest]
BOOTNEXT_KIND=recfirst exec bash "$(dirname "$0")/pc-bootnext-test.sh" "$@"
