#!/usr/bin/env bash
# PC track step 5 (12 Nov 2026): the firmware starting AfriSovera once (BootNext) and the way back, on a disk set up as
# a real Windows and Ubuntu PC whose Windows Boot Manager entry the user HID from the firmware's boot menu: Boot0005
# written with attributes 9 (active, hidden), Ubuntu's Boot0006 first in BootOrder, one shared EFI partition on SATA.
# The same eight boots as the windows kind of scripts/pc-bootnext-test.sh (read that file); bootentries must show
# Windows' entry hidden, bootplan must say so, and it must stay exactly so.
#   scripts/pc-bootnext-winhidden-test.sh [--selftest]
BOOTNEXT_KIND=winhidden exec bash "$(dirname "$0")/pc-bootnext-test.sh" "$@"
