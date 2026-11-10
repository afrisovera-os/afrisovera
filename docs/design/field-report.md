# Field report: the spare PC reports home (8 Nov 2026)

The owner boots AfriSovera on a spare PC whose screen the team cannot see. `report <ip>[:port]`
sends one plain-text report from that PC to the owner's main PC over the home network, where
`tools/report-receiver.py` saves it in `build/field-reports/<time>-<sender>.txt` for the team.

## Rules it keeps

- **Owner-driven.** Nothing is sent unless the owner types `report`. Nothing runs at boot.
- **One-way.** One TCP connection, to the address typed, port 7700 by default. Whatever the far
  end sends is never read as data or as a command (only its TCP acknowledgements count).
- **No disk writes.** The report is built in memory (96 KiB) and sent.
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
lse1000, lsrtl, lssd, lshda, wifi. Then the console lines that look like trouble (PANIC,
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

## Tests

`scripts/pc-report-test.sh` (suite `pc-report`): two QEMU boots, Intel 82574L and Realtek
RTL8139C+, each against the real receiver. Usage text, a refused port, then a full report: the
guest's byte count, the receiver's and the file's must agree, the end line must count exactly,
every section in order, the card named in its probe and boot log. Grader selftest: 20 arms.

## Later

- Bluetooth: no Bluetooth driver yet.
- Wi-Fi: no Wi-Fi driver yet (the `wifi` probe names the chip and the driver it needs).
- A desktop button that asks for the address and sends the same report.
- Keeping a panic's text across a warm restart so the next boot can send it.
- `ping`, `net tcp ...` and the other network commands still use the QEMU addresses; only
  `report` speaks a real home network so far.
