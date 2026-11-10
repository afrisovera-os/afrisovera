#!/usr/bin/env bash
# PC track step 3/5 (10 Nov 2026): installdisk on a SATA disk in IDE mode (ide0, QEMU's piix3-ide): the same three
# boots as scripts/pc-installdisk-test.sh (read that file): many 2006-2014 PCs keep SATA in IDE mode.
#   scripts/pc-installdisk-ide-test.sh [--selftest]
INSTALLDISK_KIND=ide exec bash "$(dirname "$0")/pc-installdisk-test.sh" "$@"
