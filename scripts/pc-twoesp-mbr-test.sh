#!/usr/bin/env bash
# PC track step 5 (11 Nov 2026): an old Windows disk installed for a legacy BIOS (MBR, no EFI partition) on SATA port 2,
# found first, and the GPT Windows disk with its EFI partition on port 4. The same five boots as
# scripts/pc-twoesp-test.sh (read that file): bootplan calls port 2 an old MBR disk, and it stays byte for byte as it was.
#   scripts/pc-twoesp-mbr-test.sh [--selftest]
TWOESP_KIND=mbr exec bash "$(dirname "$0")/pc-twoesp-test.sh" "$@"
