#!/usr/bin/env bash
# PC track step 5 (11 Nov 2026): two EFI system partitions on SATA disks, Linux on port 2 (found first) and Windows on
# port 4. The same three boots as scripts/pc-twoesp-test.sh (read that file): the Windows one must be picked on purpose.
#   scripts/pc-twoesp-sata-test.sh [--selftest]
TWOESP_KIND=sata exec bash "$(dirname "$0")/pc-twoesp-test.sh" "$@"
