#!/usr/bin/env bash
# PC track step 5 (11 Nov 2026): the firmware starting AfriSovera once (BootNext) and the way back, on a disk whose ONE
# EFI partition Windows and Ubuntu share, with Ubuntu's OWN firmware entry (Boot0005 "ubuntu", first in BootOrder)
# written to the variable store before the first power-on. The same eight boots as the dual kinds
# of scripts/pc-bootnext-test.sh (read that file); Ubuntu's entry must never be started, must keep its number and read
# the same at every boot, and after the way back BootOrder must read exactly as at boot 1.
#   scripts/pc-bootnext-ubuntu-test.sh [--selftest]
BOOTNEXT_KIND=ubuntu exec bash "$(dirname "$0")/pc-bootnext-test.sh" "$@"
