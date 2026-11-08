#!/usr/bin/env bash
# PC track step 5 (6 Nov 2026): installdisk on a SATA disk (AHCI port 2) -- the same three boots as
# scripts/pc-installdisk-test.sh (read that file), the disk on SATA instead of NVMe: many spare PCs have a SATA SSD.
#   scripts/pc-installdisk-sata-test.sh [--selftest]
INSTALLDISK_KIND=sata exec bash "$(dirname "$0")/pc-installdisk-test.sh" "$@"
