#!/usr/bin/env bash
# Safe updates (15 Nov 2026): the data-format guard. updateback is refused when the disk holds a file system the
# older version cannot read (VFS V3's AFRIVFS2). Read scripts/pc-updatetry-test.sh.
#   scripts/pc-updatetry-fmt-test.sh [--selftest]
UPTRY_KIND=fmt exec bash "$(dirname "$0")/pc-updatetry-test.sh" "$@"
