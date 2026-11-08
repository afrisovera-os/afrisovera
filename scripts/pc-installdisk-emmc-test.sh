#!/usr/bin/env bash
# PC track step 3/5 (8 Nov 2026): installdisk on a cheap laptop's built-in eMMC (SDHCI) -- the same three boots as
# scripts/pc-installdisk-test.sh (read that file), the disk an eMMC instead of NVMe: most low-cost laptops have no other disk.
#   scripts/pc-installdisk-emmc-test.sh [--selftest]
INSTALLDISK_KIND=emmc exec bash "$(dirname "$0")/pc-installdisk-test.sh" "$@"
