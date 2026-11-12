#!/usr/bin/env bash
# PC track step 5 (11 Nov 2026): the firmware starting AfriSovera once (BootNext) and the way back, on a disk set up as
# a real Windows and Ubuntu PC: ONE EFI partition both share, Windows Boot Manager's own entry (Boot0005) and Ubuntu's
# (Boot0006, first in BootOrder) written to the variable store before the first power-on, the disk on SATA.
# The same eight boots as the dual kinds of scripts/pc-bootnext-test.sh (read that file); neither entry may be started,
# renumbered or changed, and after the way back BootOrder must read exactly as at boot 1.
#   scripts/pc-bootnext-windows-test.sh [--selftest]
BOOTNEXT_KIND=windows exec bash "$(dirname "$0")/pc-bootnext-test.sh" "$@"
