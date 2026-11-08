# AFR-0002 — Asili driver interface

| | |
|---|---|
| **Status** | **v0 Implemented** (in-kernel drivers, by convention); **v1 Designed** (user-space driver ABI) |
| **Version** | 0.1 (describes code at `485b863`, 2026-11-08) |
| **Editor** | Chief architect |
| **Code** | `kernel/src/asili.rs`, `kernel/src/virtio.rs`, `kernel/src/pc/{disk,nic,virtio_pci,nvme,ahci,sdhci,xhci,e1000,rtl8169,hda,main}.rs` |
| **Depends on** | AFR-0001 (capabilities, v1 syscalls 40–42), AFR-0004 (AfriIDL) |
| **Licence** | Apache-2.0 |

RFC 2119 key words apply.

---

## 1. Motivation

The driver gap is what has killed every new phone OS. docs/02 and docs/11 answer it with a *stable,
versioned driver ABI* under which drivers are sandboxed user processes holding only their own hardware.
Twenty-odd drivers now exist (virtio-mmio/pci, NVMe, AHCI, SDHCI/eMMC, xHCI with HID/mass-storage/CDC/
RNDIS/audio/FTDI, e1000/igb, RTL8169, HD Audio, PS/2, GOP), all inside the kernel and all following
unwritten conventions. This spec:

1. writes those conventions down (v0) so new drivers keep to them;
2. defines the user-space driver ABI (v1) they move to;
3. defines the device classes' protocols so a block or network driver can be replaced without touching
   its clients.

## 2. Status summary

| Element | v0 (built) | v1 (designed) |
|---|---|---|
| Where drivers run | Kernel (EL1 / ring 0) | One user process per driver instance |
| Discovery | Each driver scans PCI itself (mechanism #1, ports `0xCF8/0xCFC`, buses 0–7); virtio-mmio slot walk on ARM | Device Manager enumerates PCI/ACPI/FDT once and binds |
| Registry | `asili::register(name, class, mmio, ok)`, 16 entries | Device Manager's device tree with stable paths |
| Interface to clients | Free functions with agreed signatures, selected by an if-chain | AfriIDL protocols over channels (§7) |
| Interrupts | Polled (INTx disabled) | `irq_wait` on an IRQ capability |
| DMA | Static buffers in 1:1-mapped kernel memory | `dma_alloc` → VMO + device address, IOMMU-confined |
| Restart on crash | None | Supervisor restarts with back-off (§6.4) |
| ABI version | None | Major.minor in the driver manifest (§9) |

## 3. Terminology

- **Driver** — code that operates one kind of hardware function.
- **Instance** — a driver bound to one device.
- **Class** — the kind of service an instance offers (`Storage`, `Net`, …).
- **Device Manager (DM)** — the user-space service that enumerates hardware, binds drivers and supervises
  instances (v1).
- **Resource capability** — a v1 handle to an MMIO range, an IRQ line, a PCI config space or a DMA
  window (AFR-0001 §7.3, calls 40–42).

## 4. v0: the conventions as built (normative for new in-kernel drivers)

### 4.1 Registry

```rust
pub enum Class { Firmware, Console, Interrupt, Timer, Display, Input, Storage, Net, Audio, Entropy, Gpio }
pub struct Device { pub name: &'static str, pub class: Class, pub mmio: u64, pub ok: bool }
const MAX_DEVICES: usize = 16;
pub fn register(name: &'static str, class: Class, mmio: u64, ok: bool);
pub fn count() -> usize;
pub fn get(i: usize) -> Option<Device>;
```

- A driver MUST register exactly once, after its `init` decided the device is present, with `ok`
  reporting whether bring-up succeeded. `mmio` is the register base when one exists, else 0.
- Names are lower-case, hyphenated, stable across releases (they appear in `devices` output and in test
  suites): e.g. `nvme`, `ahci-sata`, `intel-e1000`, `realtek-nic`, `sd-card`, `usb-storage`,
  `usb-ethernet`, `intel-hda`, `gop-display`.
- **Defect (v0):** a 17th registration is silently dropped. v0.1 SHOULD log it.

### 4.2 Lifecycle functions

Every driver module exposes:

| Function | Contract |
|---|---|
| `init()` | Find the device, reset it, bring it up. MUST NOT write to any storage medium. MUST tolerate absence (return quietly). |
| `present() -> bool` | True only after a successful `init`. |
| class functions (§4.3) | Valid only when `present()`. |
| `ls<x>` shell report | Optional read-only diagnostic (e.g. `lssd`, `lshda`, `lsrtl`). |

Probe order on the PC is fixed in `pc/main.rs` (`xhci`, `virtio`, `nvme`, `ahci`, `sdhci`, `ps2`, `hda`);
a driver MUST NOT depend on running before or after another unless that order is documented there.

### 4.3 Class interfaces (by signature)

**Block** (selected in `pc/disk.rs`: virtio-blk → NVMe → AHCI → eMMC → USB mass storage → SD):

```rust
pub fn blk_present() -> bool;
pub fn blk_capacity() -> u64;                               // in 512-byte sectors
pub fn read_sector(n: u64, out: &mut [u8; 512]) -> bool;    // false = I/O error, out undefined
pub fn write_sector(n: u64, data: &[u8; 512]) -> bool;      // true only after the device acknowledged
pub fn blk_selftest() -> bool;
```

Drivers with larger native blocks (4Kn) MUST present 512-byte sectors or refuse to bind. Raw access for
the partition tools goes through `gpt::Disk { Nvme, Sata(u32), Emmc }` and the drivers' `raw_read/raw_write`.

**Network** (`pc/nic.rs`, `WHICH ∈ {NONE=0, INTEL=1, USB=2, REALTEK=3}`):

```rust
pub fn mac() -> [u8; 6];
pub fn send(frame: &[u8]) -> bool;                  // one Ethernet frame without FCS
pub fn rx_ready() -> bool;
pub fn recv(dst: &mut [u8]) -> Option<usize>;       // one frame
```

The shared TCP/IP stack sees every NIC as a virtio-net device emulated over these four functions
(`virtio_pci.rs`, 12-byte virtio-net header, MAC in the config block). New NICs MUST implement the four
functions and nothing else.

**Audio** — `sound_*` functions multiplexed in `pc/disk.rs` (USB audio → HDA → virtio-sound), 44.1 kHz
stereo 16-bit PCM source; the driver resamples.

### 4.4 Hardware-access rules (v0, normative)

These keep in-kernel drivers movable to user space (§6):

1. PCI config access only through the driver's `cfg32`/`cfg_write32` helpers; command register set to
   memory decode + bus master + INTx disable (`|= 0x406`).
2. BARs mapped only with `mmu::map_mmio`.
3. DMA memory only in `#[repr(C, align(4096))]` statics or heap pages whose physical address is known;
   a driver MUST check the device's addressing limit (e.g. AHCI `S64A`, SDHCI 64-bit ADMA2 support)
   before handing it an address above 4 GiB.
4. Every wait on hardware MUST have a bounded timeout and report failure, never spin forever.
5. **Storage safety rule:** a driver MUST NOT write a medium unless it is blank and claimed by the owner
   (`claimdisk`), or already carries `AFRIVFS1`, or the write is an owner-confirmed install step
   (AFR-0008). The boot medium is never written by a block driver.
6. A new data path (DMA, multi-block, faster bus) MUST be verified against the slow path on first use
   and fall back to it on mismatch (the pattern of suites `pc-sdmulti`, `pc-sdwrite`, `pc-sddma`).

## 5. v1 architecture (Designed)

```
            client (VFS, netstack, Tonga, app via service)
                 │  AfriIDL protocol over a channel (§7)
                 ▼
   ┌───────────────────────────┐        ┌────────────────────┐
   │ driver process (instance) │◄──────►│  Device Manager    │ binds, supervises
   │  holds: MMIO, IRQ, DMA,   │ control│  (user space)      │
   │         PCI-config caps   │ channel└────────────────────┘
   └────────────┬──────────────┘
                │ capabilities granted by DM (from Mansa root resources)
                ▼
             hardware (behind the IOMMU where present)
```

### 5.1 Driver package and manifest

A driver ships as an AfriSovera package (AFR-0003 container, `kind = driver`) whose manifest carries a
`driver` section:

```
driver {
  abi      = "1.0"                         # Asili ABI version (§9)
  class    = "storage"                     # one of §7
  binds    = [ { bus = "pci", class = 0x010802 },                       # NVMe
               { bus = "pci", vendor = 0x8086, device = 0x10D3 } ]     # e1000e 82574L
  resources = [ "bar0", "irq", "dma32" ]   # requested; DM grants a subset or refuses
  restart  = "always"                      # always | never | on-failure
}
```

Binding rule (DM): candidates are drivers whose `binds` entry matches; an exact vendor+device match beats
a class match; ties go to the higher ABI minor, then to the system image over sideloaded drivers.

### 5.2 Resources (capabilities)

| Resource | Handle type | Syscall | Rights |
|---|---|---|---|
| MMIO range (one BAR) | `mmio` | `mmio_map` | READ, WRITE, MAP |
| Interrupt (INTx, MSI or MSI-X vector) | `irq` | `irq_wait`, `handle_close` | WAIT |
| PCI config space of one function | `pcicfg` | channel to DM (`pci.Config` protocol) | READ, WRITE (DM filters BAR and command writes) |
| DMA window | `bti` (bus transaction initiator) | `dma_alloc` | MAP |

A driver MUST NOT receive port-I/O ranges except on x86 legacy devices (PS/2, COM1, CMOS), where the
`ioport` resource is a range check in the kernel's I/O permission bitmap.

### 5.3 Interrupts

`irq_wait(h)` blocks until the interrupt fires; the kernel masks the line on delivery and unmasks it on
the next `irq_wait`. MSI/MSI-X is preferred; INTx sharing is resolved by the kernel delivering to every
holder of the line.

### 5.4 Lifecycle state machine (per instance)

```
            DM binds                 start() ok
 Unbound ──────────► Starting ──────────────────► Running
    ▲                   │ start() error / timeout 10 s   │ process exit / fault / watchdog
    │                   ▼                                ▼
    │               Failed ◄──────────────────────── Crashed
    │                   │ restart policy allows; back-off 0.1 s × 2^n, max 30 s
    │                   └──────────────► Starting
    │ device removed (hot-plug) / unbind
    └──────────────────── Stopping ◄── Running
```

- On `Crashed` the DM first revokes the instance's resource capabilities (closes them), then resets the
  device through its PCI function-level reset when available, then restarts.
- After 5 crashes within 60 s the instance stays `Failed` and the user is told in plain words.
- Clients see `E_PEERCLOSED` on their channel and MUST reconnect through the DM; block clients MUST
  re-issue unacknowledged writes (writes are idempotent at the sector level).

## 6. Moving a v0 driver to v1 (normative migration rules)

1. Replace `cfg32`/`cfg_write32` with the `pci.Config` protocol.
2. Replace `mmu::map_mmio(bar)` with `mmio_map(h)`.
3. Replace static DMA buffers with `dma_alloc`; program the device with the returned device address, never
   a CPU physical address.
4. Replace polling loops with `irq_wait` (polling MAY remain as a fallback with the same timeouts).
5. Replace the class free functions with a server for the class protocol (§7).
6. The driver's tests MUST pass unchanged at the class-protocol level.

## 7. Class protocols (Designed, AfriIDL)

All messages are AfriIDL v1 encoded (AFR-0004 §5). Only the shape is fixed here; the `.afriidl` files go
in `idl/asili/` when implemented.

### 7.1 `asili.block.Device` (protocol ordinal 0x0101)

```
protocol BlockDevice {
    method info() -> BlockInfo;                                   // ordinal 1
    method read(lba: u64, count: u32, buf: handle<vmo>, off: u64) -> Status;   // 2
    method write(lba: u64, count: u32, buf: handle<vmo>, off: u64, fua: bool) -> Status; // 3
    method flush() -> Status;                                     // 4
}
message BlockInfo { block_size: u32, block_count: u64, flags: u32 }   // flags: bit0 read-only, bit1 removable, bit2 boot medium
```

`BlockInfo` wire layout (AfriIDL v1 natural alignment, AFR-0004 §5.2; 24 bytes LE): `block_size` @0 (4),
4 bytes zero padding, `block_count` @8 (8), `flags` @16 (4), 4 bytes zero padding.
A driver MUST set bit 2 for the medium the system booted from; the VFS MUST NOT write a medium with bit 0
or bit 2 set except through the install path (AFR-0008).

### 7.2 `asili.net.Device` (0x0102)

`info() -> {mac: [u8;6], mtu: u16, link: u8}`; `send(frames: handle<vmo>, n: u32)`; `recv` delivered as
events on the channel, each carrying a VMO range of whole frames (no FCS, no virtio header).

### 7.3 `asili.audio.Output` (0x0103)

`formats() -> list`; `start(rate: u32, channels: u8, bits: u8, ring: handle<vmo>)`; position reported by
events every period. Resampling is the client's job in v1 (unlike v0, where each driver resamples).

### 7.4 `asili.input.Device` (0x0104)

Events: `key{code: u16, down: bool}`, `pointer_rel{dx: i16, dy: i16, buttons: u8}`,
`pointer_abs{x: u16, y: u16, max_x: u16, max_y: u16, buttons: u8}` — matching what xHCI HID, PS/2 and the
USB tablet already produce.

## 8. Security considerations

- **v0:** every driver is in the TCB. A bug in any of them is a kernel compromise; a malicious USB device
  attacks kernel code directly (HID report descriptors, RNDIS messages, mass-storage replies). Parsers of
  device-supplied data (HID descriptors, EXT_CSD, RNDIS, CDC descriptors, GPT) MUST bounds-check every
  length and offset taken from the device.
- **DMA:** without an IOMMU a bus-mastering device can read and write all physical memory; a user-space
  driver controlling such a device is therefore still in the TCB. v1 MUST program VT-d / AMD-Vi / SMMU when
  present so a driver's DMA is confined to its `bti`; where no IOMMU exists the DM MUST mark the instance
  `trusted` and only system-image drivers may bind.
- **Interrupt storms:** the kernel MUST rate-limit an IRQ whose holder does not `irq_wait`.
- **Config space:** the DM filters writes to BARs, the command register's bus-master bit and MSI address
  registers; a driver MUST NOT be able to retarget its own DMA or MSI writes.
- **Restart loops** are bounded (§5.4) so a faulty device cannot pin a CPU.

## 9. Versioning

- The Asili ABI is `major.minor`. v0 has no number; v1 starts at `1.0`.
- A minor version only adds protocol methods (new ordinals) or optional manifest fields; a driver built
  for `1.n` runs on any `1.m`, `m ≥ n`.
- A major version may remove methods; the DM MUST keep the previous major for at least two OS releases.
- Protocol ordinals (`0x01xx`) and method ordinals are never reused.

## 10. Test vectors

### 10.1 v0 (existing suites that grade these conventions)

| # | Property | Suite |
|---|---|---|
| A0-1 | `devices` lists each present driver once, with its stable name | `pc-desktop`, `pc-nvme`, `pc-ahci`, … |
| A0-2 | A blank disk is not written until claimed; a foreign disk is never written | `pc-nvme`, `pc-emmc`, `pc-sdhci` |
| A0-3 | A faster path is verified against the slow path and falls back on mismatch | `pc-sdmulti`, `pc-sdwrite`, `pc-sddma`, `pc-sdbus` |
| A0-4 | Every NIC carries DHCP, ping, TCP through the emulated virtio-net | `pc-e1000`, `pc-igb`, `pc-rtl`, `pc-usbnet`, `pc-usbtether` |
| A0-5 | Absent hardware: `init` returns quietly, nothing registered | every `pc-*` suite run without that device |

### 10.2 v1 (to be implemented with the DM)

| # | Input | Expected |
|---|---|---|
| A1-1 | `BlockInfo{512, 0x0000000000800000, 0x4}` | bytes `00 02 00 00 00 00 00 00 00 00 80 00 00 00 00 00 04 00 00 00 00 00 00 00` |
| A1-2 | driver process killed while Running | state Crashed → Starting within 0.1 s; client sees `E_PEERCLOSED`, reconnects, reads same data |
| A1-3 | 6 crashes in 60 s | instance Failed; no further restart |
| A1-4 | driver writes PCI command register to clear bus-master of another function | DM refuses (`E_PERM`) |
| A1-5 | driver programs DMA to an address outside its `bti` (IOMMU present) | device faults; DM restarts instance; other memory unchanged |

## 11. References

- docs/02-drivers-and-services-asili.md, docs/11-driver-model.md, ADR 0001.
- Fuchsia DFv2 driver framework and Zircon BTI/IOMMU objects; Linux `drivers/` for hardware behaviour only
  (ADR 0005: reference, never copied).
- Intel VT-d spec; PCI Express Base Spec §7 (config space), §6.6.2 (function-level reset).
