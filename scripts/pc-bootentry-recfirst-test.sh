#!/usr/bin/env bash
# PC track step 5 (11 Nov 2026): AfriSovera's firmware boot entry on a Windows disk whose recovery partition comes
# BEFORE the EFI partition, as some OEM disks have it (GPT entry 1 recovery, entry 2 EFI at sector 43008). The same
# five boots as scripts/pc-bootentry-test.sh (read that file); bootcopy, bootentry and the entry's HD() node must say
# partition 2, start 43008 and the EFI partition's own GUID.
#   scripts/pc-bootentry-recfirst-test.sh [--selftest]
BOOTENTRY_KIND=recfirst exec bash "$(dirname "$0")/pc-bootentry-test.sh" "$@"
