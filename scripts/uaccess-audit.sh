#!/usr/bin/env bash
# G2 parts 2 and 3 (architect, 15 Nov 2026): in the system-call files, user memory moves only through the door:
# pc/uaccess.rs on the PC, kernel/src/uaccess.rs on ARM. A raw copy there fails this check at merge time (on the PC
# it would halt the machine under SMAP). Each line in these files that uses read_volatile, write_volatile, copy_nonoverlapping or from_raw_parts must say why it is not
# ring-3 memory, on the same line: `// uaccess: <reason>` (comment-only lines are skipped).
set -u
cd "$(dirname "$0")/.."
FILES="kernel/src/pc/proc.rs kernel/src/pc/foreign.rs kernel/src/process.rs"
# every raw form: the volatile and plain reads and writes (functions and pointer methods), the unaligned ones,
# copies, fills and slices made from raw pointers (architect, 15 Nov 2026: ptr::write, ptr::read, write_unaligned and
# read_unaligned were missing)
PAT='read_volatile|write_volatile|read_unaligned|write_unaligned|ptr::read\(|ptr::write\(|ptr::copy|copy_nonoverlapping|write_bytes|from_raw_parts|\.(read|write)\('
bad="$(grep -nE "$PAT" $FILES \
    | grep -vE '^[^:]+:[0-9]+:[[:space:]]*//' | grep -v '// uaccess: ')"
if [ -n "$bad" ]; then
    echo "UACCESS AUDIT FAILED: a raw memory access in a system-call file without '// uaccess: <reason>':"
    echo "$bad" | sed 's/^/  /'
    echo "User memory moves only through kernel/src/pc/uaccess.rs (PC) or kernel/src/uaccess.rs (ARM): from_user, to_user."
    exit 1
fi
echo "UACCESS AUDIT PASSED ($FILES)"
