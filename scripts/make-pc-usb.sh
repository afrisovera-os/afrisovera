#!/usr/bin/env bash
# PC track step 4 (5 Nov 2026): the files for a bootable AfriSovera USB stick.
#
# A UEFI PC boots any FAT32 USB stick that holds EFI\BOOT\BOOTX64.EFI -- no disk image, no
# imaging tool. This builds the PC kernel and lays out exactly that, plus plain-English
# steps, in build/pc-usb/, and zips it to build/afrisovera-pc-usb.zip. Nothing is
# installed on the PC: the stick runs the OS live, and the PC's own system is not touched
# (the OS only ever writes a disk that is blank or already its own -- see pc/ahci.rs).
#   scripts/make-pc-usb.sh
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -1
EFI=target/x86_64-unknown-uefi/debug/mansa-pc.efi
OUT=build/pc-usb
rm -rf "$OUT"
mkdir -p "$OUT/EFI/BOOT"
cp "$EFI" "$OUT/EFI/BOOT/BOOTX64.EFI"
REV="$(git rev-parse --short HEAD 2> /dev/null || echo unknown)"
cat > "$OUT/README-USB.txt" << EOF
AfriSovera OS -- PC test stick (commit $REV, built $(date +%Y-%m-%d))

What this is: the AfriSovera kernel for ordinary PCs and laptops, run straight from a
USB stick. Nothing is installed. Your Windows (or anything else on the PC) is not changed.

Make the stick (any USB stick; everything on it is erased):
  1. In Windows, right-click the stick in File Explorer > Format... > File system: FAT32 > Start.
  2. Copy the EFI folder from here onto the stick, so the stick has EFI\\BOOT\\BOOTX64.EFI.

Start the PC from it:
  3. Switch the PC off. Switch it on and press the boot-menu key straight away
     (often F12, F11, F9, Esc or F8 -- the maker's logo screen usually says which).
  4. Pick the USB stick (it may say "UEFI: <stick name>").
  5. If the PC refuses to start it, Secure Boot is on: enter the firmware set-up
     (often F2 or Del), turn Secure Boot off, save, and try again. Turn it back on
     afterwards if you like; Windows does not need it changed.

What you should see: the AfriSovera boot screen, then the AfriSh shell. Type on the
built-in keyboard or a USB keyboard. Try, one at a time:
  help          every command
  lsusb         USB keyboard, mouse, network adapter it found
  lsps2         the built-in (PS/2) keyboard and touchpad
  lsnvme        an NVMe SSD (left alone unless it is blank)
  lsahci        SATA disks (left alone unless blank)
  lse1000       an Intel wired network card
  lsrtl         a Realtek wired network card (most home PCs)
  lssd          the SD card slot and the card in it (left alone unless blank)
  acpiinfo      how many processor cores it found
  ifconfig      the network address it got (plug in a network cable first)
  ping          talk to the router
  gui           the desktop screen -- with a USB mouse, click FILES, STORE, ASILI-AI or
                DEVICES at the bottom;   clear   back to text
  wm            three windows -- drag them by their title bars with a USB mouse
  write notes.txt hello   then  sync   saves a file -- only on a BLANK USB stick (a second,
                empty one) or a disk you claim; the boot stick is never written
Nothing here writes to your disks: a disk inside the PC is only used if it is completely
blank AND you type  claimdisk nvme  or  claimdisk sata , or if you type the code that
installdisk (below) shows you -- so your Windows disk is never touched by accident. A blank USB stick you plug in is used by itself.

INSTALLING ON A SPARE PC (one that is AfriSovera's alone -- EVERYTHING on its disk is ERASED):
  1. Start the spare PC from this stick as above.
  2. Type  lsparts  -- it lists the disks inside: "nvme" (an NVMe SSD) or "sata port N".
  3. Type  installdisk nvme   (or  installdisk sata0 , sata1 ... for a SATA disk).
     It shows what is on that disk now, warns that everything is erased, and gives a
     4-character code. Nothing is written yet.
  4. Only if that disk may be wiped, type exactly what it says, e.g.  installdisk nvme yes 3fa1
     Wait for "installdisk: DONE".
  5. Take the stick out and type  reboot . The PC now starts AfriSovera from its own disk;
     files you save (write notes.txt hello, then sync) stay on it.
  If it starts Windows or says "no bootable device", open the boot menu and pick the disk
  (it may be called "UEFI OS" or the disk's name). Secure Boot must be off.

If the laptop's own keyboard does nothing, plug in a USB keyboard (newer laptops wire
their keyboard in a way this test version cannot read yet); a USB mouse works too.
To leave, press the power button once: AfriSovera saves its files and switches off (if
nothing happens, hold it 4 seconds, and write down the "power button" boot line); take
the stick out and it starts Windows as normal.

SEND A REPORT TO YOUR MAIN PC (so the team can read how AfriSovera runs on this PC):
  1. On your main Windows PC, double-click  tools\\report-receiver.bat  in the AfriSovera
     folder (or run:  python tools\\report-receiver.py ). It shows a line such as
         report 192.168.1.20
     If Windows asks whether Python may use networks, tick "Private networks" and click
     Allow. That is your choice; without it the report cannot arrive. Nothing else changes.
  2. Plug this PC into the SAME router as the main PC with a network cable (Wi-Fi and
     Bluetooth do not work yet).
  3. Start AfriSovera here from the stick, wait for the  afrisovera>  prompt, and type the
     line from step 1, e.g.   report 192.168.1.20
  4. This PC says  [report] SENT ... bytes  and the main PC's window says  Saved ... .
     The report is in  build\\field-reports\\  for the team to read.
  It sends only when you type  report , only to the address you type, and nothing ever
  comes back in. If it says NOT SENT, the line after it says why in plain words.

What to send back (phone photos are perfect):
  1. The PC's make and model, and its processor (e.g. "Lenovo T14, Intel i5-1135G7").
  2. A photo of the WHOLE screen right after it starts (the boot lines matter).
  3. Did the laptop's own keyboard work, or only a USB one?
  4. Type these and photograph each answer:  acpiinfo   lsusb   lsnvme   lsahci   lse1000   lsrtl
  5. Anything that froze, went black or restarted, and at which step.
EOF
python3 - "$OUT" build/afrisovera-pc-usb.zip << 'PY'
import os, sys, zipfile
src, dst = sys.argv[1], sys.argv[2]
with zipfile.ZipFile(dst, "w", zipfile.ZIP_DEFLATED) as z:
    for root, _, files in os.walk(src):
        for f in files:
            p = os.path.join(root, f)
            z.write(p, os.path.relpath(p, src))
print("wrote", dst, os.path.getsize(dst), "bytes")
PY
echo "USB stick files in $OUT (copy the EFI folder to a FAT32 stick)"
