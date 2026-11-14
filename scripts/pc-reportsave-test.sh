#!/usr/bin/env bash
# `report save` (report home, 14 Nov 2026; REPORT_HOME_PLAN.md, ARCHITECT_REVIEWS 13 Nov night): the field report
# written as a NEW file \AFREPORT\<n>.TXT on the USB stick AfriSovera started from, for an owner whose laptop
# reaches the internet only through a phone (tethering cannot reach the main PC).
#
# The stick is a FAT32 image made by scripts/lib/fat32img.py (MBR, 4 KiB clusters, the boot file in
# EFI/BOOT/BOOTX64.EFI), on QEMU's usb-storage behind an xHCI. Five boots of the PC kernel:
#   1. started from the stick: `report save` twice -> SAVED \AFREPORT\1.TXT and 2.TXT. Then, with QEMU stopped,
#      the image is read back: the two files are whole reports (title, "to: the USB stick", the end line counting
#      exactly), no full MAC and no device serial in them; every other file the same as before; both FAT copies
#      equal, the clean bit still set, FSInfo's free count right. Then tools/report-receiver.py --from-stick on the
#      folder copies both.
#   2. started from a READ-ONLY stick: refused (the first write fails, nothing points at it).
#   3. started from an internal SATA disk holding the same boot file, the stick plugged in: refused, nothing written.
#   4. a stick not closed cleanly: refused, nothing written.
#   5. a stick already holding 64 reports: the 65th refused, nothing written.
#   scripts/pc-reportsave-test.sh              # selftests (image tool, receiver), then the five boots
#   scripts/pc-reportsave-test.sh --selftest   # the selftests only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
SERIAL=AFRSER0002

echo "== PC report save (report save -> \\AFREPORT on the boot stick) =="
echo "  --- selftests (no QEMU) ---"
python3 scripts/lib/fat32img.py --selftest || { echo "PC REPORTSAVE TEST FAILED (fat32img selftest)"; exit 1; }
python3 scripts/lib/report-receiver-selftest.py || { echo "PC REPORTSAVE TEST FAILED (receiver selftest)"; exit 1; }
# one door (architect, 14 Nov): only disk.rs, gpt.rs and esp.rs call msc_write, and esp.rs makes its writing
# handle (BootStick) in exactly one place, report_save
callers="$(grep -rlE 'msc_write\(|xhci::msc_write\b' kernel/src --include='*.rs' | grep -v 'kernel/src/pc/xhci.rs' | sort | tr '\n' ' ')"
[ "$callers" = "kernel/src/pc/disk.rs kernel/src/pc/esp.rs kernel/src/pc/gpt.rs " ] ||
    { echo "PC REPORTSAVE TEST FAILED (one door: msc_write is called from: $callers)"; exit 1; }
[ "$(grep -v '^ *//' kernel/src/pc/esp.rs | grep -v 'struct BootStick' | grep -c 'BootStick(())')" = 1 ] ||
    { echo "PC REPORTSAVE TEST FAILED (one door: BootStick is made more than once in esp.rs)"; exit 1; }
echo "  one door: msc_write called from disk.rs, esp.rs, gpt.rs only; BootStick made once"
[ "${1:-}" = "--selftest" ] && { echo "PC REPORTSAVE SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC REPORTSAVE TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC REPORTSAVE TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-reportsave.XXXXXX")"
: > "$W/nv.img" && truncate -s 1M "$W/nv.img"
fail=0
_f() { echo "  FAIL: $*"; fail=1; }

# boot <name> <stick image> <usb|internal> <readonly on|off> <commands...>
boot() {
    local name="$1" img="$2" from="$3" ro="$4"
    shift 4
    local bi="" extra=()
    if [ "$from" = usb ]; then
        bi=",bootindex=0"
    else
        # an internal SATA disk holding the same boot file, started first; the stick last
        mkdir -p "$W/esp2/EFI/BOOT" && cp "$PC_EFI" "$W/esp2/EFI/BOOT/BOOTX64.EFI"
        extra=(-drive "if=none,id=esp2,format=raw,file=fat:rw:$W/esp2" -device "ide-hd,drive=esp2,bus=ide.2,bootindex=0")
        bi=",bootindex=9"
    fi
    qemu-system-aarch64 -M virt -m 256M -display none \
        -netdev user,id=n0 -device e1000e,netdev=n0 \
        -drive "if=none,id=nv,format=raw,file=$W/nv.img" -device "nvme,serial=$SERIAL,drive=nv" \
        -device qemu-xhci,id=xhci \
        -drive "if=none,id=stk,format=raw,readonly=$ro,file=$img" -device "usb-storage,bus=xhci.0,drive=stk$bi"         "${extra[@]}" \
        -serial "tcp:127.0.0.1:5246,server,nowait" -qmp "tcp:127.0.0.1:5247,server,nowait" 2> "$W/$name.qemu" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py 5246 5247 "$W/$name.png" "$@" > "$W/$name.log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
    tr -d '\r' < "$W/$name.log" | grep -aE '^report save:|^\[report\]|^\[pc\] own boot file|PANIC|EXCEPTION' | sed "s/^/  $name| /"
}
log() { tr -d '\r' < "$W/$1.log"; }
dump() { python3 scripts/lib/fat32img.py dump "$1"; }

# --- 1: the good stick, two saves
python3 scripts/lib/fat32img.py make "$W/good.img" "$PC_EFI"
dump "$W/good.img" > "$W/good.before.json"
boot good "$W/good.img" usb off "report save" "report save"
L="$(log good)"
grep -aqE '^\[pc\] own boot file \\EFI\\BOOT\\BOOTX64\.EFI \([0-9]+ KiB\) kept' <<< "$L" || _f "good: the boot file was not kept (did it start from the stick?)"
grep -aqE '^report save: SAVED \\AFREPORT\\1\.TXT \([0-9]+ bytes\), read back the same; [0-9]+ KiB free on the stick$' <<< "$L" || _f "good: the first save was not SAVED as 1.TXT"
grep -aqE '^report save: SAVED \\AFREPORT\\2\.TXT \([0-9]+ bytes\), read back the same; [0-9]+ KiB free on the stick$' <<< "$L" || _f "good: the second save was not SAVED as 2.TXT"
grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)' <<< "$L" && _f "good: the kernel faulted"
dump "$W/good.img" > "$W/good.after.json"
mkdir -p "$W/fromstick" "$W/copied"
python3 scripts/lib/fat32img.py extract "$W/good.img" "$W/fromstick" > /dev/null
python3 tools/report-receiver.py --from-stick "$W/fromstick" --out "$W/copied" > "$W/copy.log" 2>&1 || _f "good: the receiver's --from-stick failed: $(tail -1 "$W/copy.log")"
sed 's/^/  copy| /' "$W/copy.log" | head -4
python3 - "$W" "$SERIAL" <<'EOF' || fail=1
import json, os, re, sys
w, serial = sys.argv[1], sys.argv[2]
b = json.load(open(os.path.join(w, "good.before.json")))
a = json.load(open(os.path.join(w, "good.after.json")))
bad = []
new = sorted(set(a["files"]) - set(b["files"]))
if new != ["AFREPORT/1.TXT", "AFREPORT/2.TXT"]:
    bad.append("the new files are %r, not AFREPORT/1.TXT and 2.TXT" % new)
for p, h in b["files"].items():
    if a["files"].get(p) != h:
        bad.append("%s changed" % p)
if not a["fats_equal"]:
    bad.append("the two FAT copies differ after the saves")
if not a["clean"]:
    bad.append("the clean bit is not set after the saves")
if a["free"] != a["fsinfo_free"]:
    bad.append("FSInfo says %d free, the FAT %d" % (a["fsinfo_free"], a["free"]))
used = (b["free"] - a["free"])
want = sum(-(-a["sizes"][p] // 4096) for p in new) + 1  # the files' clusters and AFREPORT's own
if used != want:
    bad.append("%d clusters were taken, %d expected" % (used, want))
full_mac = re.compile(r"(?<![0-9A-Fa-f:-])[0-9A-Fa-f]{2}([:-])[0-9A-Fa-f]{2}(\1[0-9A-Fa-f]{2}){4}(?![0-9A-Fa-f:-])")
for n in (1, 2):
    t = open(os.path.join(w, "fromstick", "AFREPORT", "%d.TXT" % n), "rb").read()
    s = t.decode("utf-8", "replace")
    if not s.startswith("AFRISOVERA FIELD REPORT 1\n"):
        bad.append("%d.TXT does not start with the title line" % n)
    if "\nto: the USB stick AfriSovera started from (report save)\n" not in s:
        bad.append("%d.TXT has no 'to: the USB stick' line" % n)
    if not re.search(r"^from: 10\.0\.2\.15 mac ([0-9a-f]{2}:){3}--:--:-- \(e1000\)$|^from: 0\.0\.0\.0 mac ([0-9a-f]{2}:){3}--:--:-- \(e1000\)$", s, re.M):
        bad.append("%d.TXT has no masked 'from' line" % n)
    for sec in ("hwscan", "usbaddr", "camera", "lsnvme"):
        if "\n== %s ==\n" % sec not in s:
            bad.append("%d.TXT has no %s section" % (n, sec))
    m = full_mac.search(s)
    if m:
        bad.append("%d.TXT holds a full MAC: %s" % (n, m.group(0)))
    if serial in s:
        bad.append("%d.TXT holds the NVMe serial" % n)
    em = re.search(r"\n== end of report \((\d+) bytes before this line\) ==\n\Z", s)
    if not em or int(em.group(1)) != len(t) - len(em.group(0).encode()):
        bad.append("%d.TXT's end line does not count the bytes before it" % n)
copied = sorted(os.listdir(os.path.join(w, "copied")))
if len(copied) != 2 or not all(re.fullmatch(r"\d{8}-\d{6}-stick-[12]\.txt", c) for c in copied):
    bad.append("the receiver copied %r" % copied)
for f in bad:
    print("  FAIL: good: " + f)
print("  good: %d new files, %d clusters taken, FATs equal %s, clean %s" % (len(new), used, a["fats_equal"], a["clean"]))
sys.exit(1 if bad else 0)
EOF

# --- 2: a read-only stick
python3 scripts/lib/fat32img.py make "$W/ro.img" "$PC_EFI"
dump "$W/ro.img" > "$W/ro.before.json"
boot ro "$W/ro.img" usb on "report save"
L="$(log ro)"
grep -aqE '^report save: a write FAILED\. No folder entry was written, so the stick shows no new file' <<< "$L" || _f "ro: a read-only stick was not refused at its first write"
grep -aqE '^report save: SAVED' <<< "$L" && _f "ro: SAVED on a read-only stick"
dump "$W/ro.img" | cmp -s - "$W/ro.before.json" || _f "ro: the read-only stick's image changed"

# --- 3: started from the internal disk (the rig's SATA ESP), the stick plugged in
python3 scripts/lib/fat32img.py make "$W/int.img" "$PC_EFI"
dump "$W/int.img" > "$W/int.before.json"
boot int "$W/int.img" internal off "report save"
L="$(log int)"
grep -aqE '^report save: AfriSovera started from [A-Za-z ]+ \(not a USB stick\), and report save writes only to the USB stick it started from; nothing written\.' <<< "$L" ||
    _f "int: a boot from the internal disk was not refused"
dump "$W/int.img" | cmp -s - "$W/int.before.json" || _f "int: the stick changed although AfriSovera started from the internal disk"

# --- 4: a stick not closed cleanly
python3 scripts/lib/fat32img.py make "$W/dirty.img" "$PC_EFI" --dirty
dump "$W/dirty.img" > "$W/dirty.before.json"
boot dirty "$W/dirty.img" usb off "report save"
L="$(log dirty)"
grep -aqE '^report save: the stick was not closed cleanly' <<< "$L" || _f "dirty: a stick not closed cleanly was not refused"
dump "$W/dirty.img" | cmp -s - "$W/dirty.before.json" || _f "dirty: the stick changed"

# --- 5: 64 reports already there
python3 scripts/lib/fat32img.py make "$W/full.img" "$PC_EFI" --reports 64
dump "$W/full.img" > "$W/full.before.json"
boot full "$W/full.img" usb off "report save"
L="$(log full)"
grep -aqE '^report save: \\AFREPORT already holds 64 reports\. Move them to the main PC' <<< "$L" || _f "full: the 65th report was not refused"
dump "$W/full.img" | cmp -s - "$W/full.before.json" || _f "full: the stick changed"

if [ "$fail" -eq 0 ]; then
    rm -rf "$W"
    echo "PC REPORTSAVE TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC REPORTSAVE TEST FAILED"
exit 1
