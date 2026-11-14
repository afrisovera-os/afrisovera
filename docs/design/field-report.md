# Field report: the spare PC reports home (8 Nov 2026)

The owner boots AfriSovera on a spare PC whose screen the team cannot see. `report <ip>[:port]`
sends one plain-text report from that PC to the owner's main PC over the home network, where
`tools/report-receiver.py` saves it in `build/field-reports/<time>-<sender>.txt` for the team.

## Rules it keeps

- **Owner-driven.** Nothing is sent unless the owner types `report`. Nothing runs at boot.
- **One-way.** One TCP connection, to the address typed, port 7700 by default. Whatever the far
  end sends is never read as data or as a command (only its TCP acknowledgements count).
- **No disk writes, except `report save`.** The report is built in memory (96 KiB) and sent. `report save`
  (below) is the one exception: a new file on the USB stick AfriSovera started from.
- **No full MAC and no serial numbers.** Every MAC address in the report (the `from:` line, ifconfig, the
  boot log) is cut to its maker half, `3c:52:82:--:--:--`, as hwscan does. No probe prints a serial number.
- **The console tail is the owner's typing.** The latest 16 KiB can hold anything typed there; `report`'s
  help says not to send or save it right after typing anything private.
- **Receive-only receiver.** Python standard library. It writes the bytes to a `.txt` file and
  nothing else: no reply, no parsing, nothing run. Capped at 1 MiB; private-network senders only
  (unless `--any`). The file is named from the sender's IP address, not from the report's text.
- **Firewall is the owner's click.** If Windows asks to let Python use private networks, the owner
  decides (README-USB says so). Nothing changes firewall settings.

## What the report holds

Title line, `commit:` (stamped by `kernel/build.rs` from git, `unknown` without it) and its date,
`build:` (debug/release), `from:` (leased address, MAC, card), `to:`, `cores:`/`memory:`. Then the
output of each read-only probe, each under `== <command> ==`: date, fwinfo, screeninfo, cpuinfo,
acpiinfo, mem, uptime, ifconfig, devices (the Asili list), lsvirtio, lsusb, lsps2, lsnvme, lsahci,
lside, lse1000, lsrtl, lssd, lshda, wifi, hwscan, usbaddr and camera (the list only; `camera on` is refused
while a report is being collected). Then the console lines that look like trouble (PANIC,
EXCEPTION, FAIL, stuck, timed out ...), the boot log (the first 16 KiB printed since power-on), the
latest 16 KiB of console, and an end line that counts the bytes before it, so a lost or doubled
byte is visible on arrival.

A kernel panic halts the PC, so it cannot be sent afterwards; the report says to photograph it.

## How it reaches a real home network (PC only)

The rest of the TCP/IP stack was written for QEMU's user-mode network: it sends from 10.0.2.15
and addresses every frame to the gateway's MAC. The report adds, in `virtio.rs` under
`cfg(feature = "pc")`, so the ARM kernel runs the same code (its md5 still changes once: lines
below the edit in `virtio.rs` move, and a debug build records line numbers):

- the **leased address** as the source (`set_src_ip` rewrites it and both checksums);
- the **next hop's own MAC**: ARP for the receiver when it is on the same subnet (the usual case),
  else for the router; ARP requests for our address are answered throughout;
- **reclaiming receive buffers** nobody read (a home network's broadcasts would otherwise fill
  the 8 buffers while the shell is idle and leave the card deaf);
- a **windowed stream** (8 segments of 512 bytes in flight, go-back-N after 1, 2, 4, 8 s of
  silence, 6 tries) and a FIN close.

It works on every card the PC kernel drives (Intel e1000/e1000e/igb, Realtek, USB Ethernet),
because they all sit behind the same emulated virtio-net device.

## `report save`: the same report onto the boot stick (14 Nov 2026)

A laptop online through a phone's tethering sits on the phone's own small network, so the main PC is out of
reach, and many laptops have no network socket. `report save` writes the report as a new file
`\AFREPORT\<n>.TXT` on the USB stick AfriSovera started from; `tools/report-from-stick.bat` (the receiver's
`--from-stick`) copies the reports to the main PC. The owner's page is docs/owner/REPORT_HOME.md. Rules
(ARCHITECT_REVIEWS 13 Nov night):

- **Only the USB stick it started from.** The firmware's device path for the boot device must hold a USB node
  (uefi.rs `boot_medium`), and the stick the xHCI driver holds must hold this very boot file (size and first
  64 KiB). An internal SATA, NVMe or eMMC disk is refused even when AfriSovera started from it.
- **bootcopy's checks and order** (esp.rs, now written for any `SectW`: a disk or the stick): the FAT32 closed
  cleanly and both FAT copies equal first; the file's clusters, then the first time `\AFREPORT`'s own cluster
  ("." "..", the file's entry), each before its FAT entries in both copies; FSInfo; the entry that names the
  new file or folder last. At least 1 MiB stays free.
- **Names:** 8.3 upper case, `<n>.TXT` with n one more than the highest there; at most 64 files.
- **Saved means on the stick:** flushed, the file read back and compared, the FATs equal and the clean bit
  still set, before `SAVED`.
- **The receiver's `--from-stick`** takes only `AFREPORT\<digits>.TXT` of at most 1 MiB, builds its own
  output names, writes new files only, never changes the stick, refuses the system drive. `--out` refuses a
  folder inside a git working tree unless git ignores it (`build/field-reports/` is ignored).
- A FAT32 stick is never the VFS stick, so the xHCI driver's `msc_attached` (it answers sector reads and
  writes) lets esp.rs reach it; the VFS still asks `msc_present`. This also lets the firmware files be read
  from a real FAT32 stick, which `msc_present` had stopped.

## Tests

`scripts/pc-report-test.sh` (suite `pc-report`): three QEMU boots, Intel 82574L, Realtek
RTL8139C+ and a USB Ethernet adapter (CDC-ECM), each against the real receiver, each with an NVMe disk whose
serial and the card's full MAC must not appear in the report. Usage text, a refused port, then a full report: the
guest's byte count, the receiver's and the file's must agree, the end line must count exactly,
every section in order, the card named in its probe and boot log. Grader selftest: 25 arms.

`scripts/pc-reportsave-test.sh` (suite `pc-reportsave`): five boots with a FAT32 stick image
(scripts/lib/fat32img.py): two saves read back from the image (every other file unchanged, FATs equal, clean
bit set, FSInfo right) and copied by `--from-stick`; refusals on a read-only stick, after starting from an
internal disk, on a stick not closed cleanly, and at the 65th report. Plus the receiver's own selftest.

## Later

- Bluetooth: no Bluetooth driver yet.
- Wi-Fi: no Wi-Fi driver yet (the `wifi` probe names the chip and the driver it needs).
- A desktop button that asks for the address and sends the same report.
- Keeping a panic's text across a warm restart so the next boot can send it.
- `ping`, `net tcp ...` and the other network commands still use the QEMU addresses; only
  `report` speaks a real home network so far.
