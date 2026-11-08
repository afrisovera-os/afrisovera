//! Board peripheral map — the **one place** the kernel's MMIO base addresses live.
//!
//! Until now every driver hard-coded the QEMU `virt` memory map. That is fine for
//! emulation but a real SoC (Raspberry Pi 4, PINE64/Allwinner A64, etc.) puts its
//! UART, interrupt controller and timers at *different* physical addresses. This
//! module centralizes those bases so bringing up a new board is a matter of adding
//! a peripheral map, not hunting constants across the tree.
//!
//! Boards are described as [`Map`] values, so the kernel can *inspect* any known
//! board's map (`board <name>`) while exactly one of them is compiled in as the
//! [`ACTIVE`] board, selected by feature:
//!
//! | feature | board | status |
//! |---|---|---|
//! | `board-virt` (default) | QEMU `virt` | fully featured, boots |
//! | `board-raspi` | Raspberry Pi 3 / BCM2837 (`-M raspi3b`) | boots: MMU, heap, mailbox FB, timer IRQ |
//! | `board-rpi4` | Raspberry Pi 4 / BCM2711 | **map only** — no boot path yet |
//! | `board-pine64` | PINE64 / PinePhone, Allwinner A64 | **map only** — and its UART is not a PL011 |
//!
//! `board-rpi4` is deliberately map-only: the BCM2711 addresses are now recorded and
//! inspectable, but a bootable RPi4 build still needs a flat-binary/`armstub` entry
//! path, GIC-400 init and a VideoCore mailbox framebuffer. Those are Phase 3 items
//! written up in `docs/15-hardware-bringup.md`; this module is the address half of
//! that work, not a claim that the Pi 4 boots.
//!
//! `board-pine64` is map-only for a *stronger* reason, and the map says so out loud:
//! the A64's primary UART is a DesignWare 8250/16550-class part, not a PL011, so
//! rebasing our PL011 driver onto `0x01C2_8000` would drive the wrong registers.
//! Every board carries [`Map::uart_kind`] and [`Map::uart_driver`] precisely so a
//! board whose map is complete cannot be mistaken for a board that is ported.
//!
//! Milestone 2.724 closes the *driver* half of that gap: `uart_ns16550.rs` is a real
//! DesignWare 8250 driver, and [`UART_IS_8250`] — derived from `uart_kind`, so the
//! map stays the one source of truth — selects it as the console at compile time.
//! A `board-pine64` build now has an output *path*, not just an address. It still
//! has no boot path (ATF/U-Boot entry, DE2 framebuffer), so it remains map-only in
//! the table above; what changed is that its first character would now be formed
//! correctly if something ever branched to `kernel_main` on that SoC.

/// One mebibyte, so the DRAM figures below read as sizes rather than as hex nobody
/// checks.
const MIB: u64 = 1024 * 1024;

/// A board's peripheral (MMIO) map. A base of **0** means *this board has no such
/// peripheral* (or none we drive yet) — never "address zero".
#[derive(Clone, Copy)]
pub struct Map {
    /// Human-readable board name, as printed by `board`.
    pub name: &'static str,
    /// Primary UART base.
    pub uart: usize,
    /// Short name of the UART *part* at `uart`, as printed in the map line. The
    /// base alone is not enough: a different part means a different register
    /// layout, not just a different address.
    pub uart_kind: &'static str,
    /// `None` when the kernel's existing PL011 driver drives this UART, so the
    /// address is genuinely all that changes. `Some(why)` when the board needs a
    /// *different* UART driver — printed as a caveat so "the map is done" is never
    /// read as "the board is ported".
    pub uart_driver: Option<&'static str>,
    /// GICv2 distributor.
    pub gic_dist: usize,
    /// GICv2 CPU interface.
    pub gic_cpu: usize,
    /// PL031-class RTC.
    pub rtc: usize,
    /// GPIO controller (PL061 on virt, BCM GPIO on the Pis).
    pub gpio: usize,
    /// QEMU `fw_cfg` firmware-config device.
    pub fwcfg: usize,
    /// First virtio-mmio slot.
    pub virtio: u64,
    /// VideoCore mailbox property-interface base (the Pis' framebuffer path). `0`
    /// means this board has no such mailbox (virt's framebuffer comes from
    /// fw_cfg/ramfb instead, A64's from Display Engine 2 + HDMI) — see `pi_fb.rs`.
    pub mailbox: usize,
    /// Base of the board's main **DRAM**. Not an MMIO window like the rows above:
    /// it is where the machine's memory begins, and it is the one address range the
    /// kernel *runs out of* rather than pokes at. **0 is a real value here** (both Pis
    /// start DRAM at 0); a map with no DRAM to declare says so with `ram_size == 0`.
    pub ram_base: u64,
    /// The board's nominal DRAM size. Unlike a peripheral base this is genuinely
    /// variable at boot — QEMU sizes it with `-m`, and real SKUs of one board ship
    /// with different amounts — so a tree disagreeing here is **not** a map error the
    /// way a wrong UART base is. The map's figure is what the kernel falls back to
    /// when nothing better is discovered; see [`Resolved::ram_end`].
    pub ram_size: u64,
    /// Names this machine states in its **root** `compatible` — the device tree's
    /// own answer to "which board is this?". Not an address: it is what says the
    /// rest of the tree is even *about* this board (see [`machine_from`]).
    pub dt_compat: &'static [&'static str],
    /// One-line note on what the board still lacks, printed by `board <name>`.
    pub note: &'static str,
}

/// QEMU `-M virt`: the fully featured development board.
pub const VIRT: Map = Map {
    name: "qemu-virt (aarch64)",
    uart: 0x0900_0000, // PL011 primary UART
    uart_kind: "PL011",
    uart_driver: None,
    gic_dist: 0x0800_0000, // GICv2 distributor
    gic_cpu: 0x0801_0000,  // GICv2 CPU interface
    rtc: 0x0901_0000,      // PL031
    gpio: 0x0903_0000,     // PL061 GPIO
    fwcfg: 0x0902_0000,    // QEMU fw_cfg
    virtio: 0x0a00_0000,   // virtio-mmio slot 0
    mailbox: 0,             // no VideoCore mailbox: framebuffer comes from fw_cfg/ramfb
    ram_base: 0x4000_0000, // DRAM starts at 1 GiB on virt
    ram_size: 256 * MIB,   // what our run scripts pass as `-m 256M`
    dt_compat: &["linux,dummy-virt"],
    note: "full-featured default: boots, MMU, GICv2, ramfb, virtio",
};

/// Raspberry Pi 3 (BCM2837), as modeled by QEMU `-M raspi3b`.
///
/// The Pi 3 has **no GIC** — the ARM generic timer routes through the BCM2836/2837
/// per-core local interrupt controller at `0x4000_0000`, and the legacy peripheral
/// intc lives at `0x3F00_B200`; neither is a GICv2, so `gic.rs` does not apply and
/// the GIC bases stay 0. No PL031 RTC (the kernel uses the 1 MHz system timer at
/// `0x3F00_3004`), no `fw_cfg`, no virtio — the framebuffer comes from the VideoCore
/// mailbox at `0x3F00_B880` (see `pi_fb.rs`).
pub const BCM2837: Map = Map {
    name: "raspberry-pi (bcm2837)",
    uart: 0x3F20_1000, // BCM2837 PL011 (UART0)
    uart_kind: "PL011",
    uart_driver: None,
    gic_dist: 0,       // no GIC: local intc @ 0x4000_0000, legacy intc @ 0x3F00_B200
    gic_cpu: 0,
    rtc: 0,            // no PL031: system timer @ 0x3F00_3004
    gpio: 0x3F20_0000, // BCM2837 GPIO
    fwcfg: 0,          // no fw_cfg
    virtio: 0,         // no virtio-mmio
    mailbox: 0x3F00_B880, // VideoCore mailbox property interface (see `pi_fb.rs`)
    ram_base: 0,       // DRAM starts at 0; peripherals sit *above* it at 0x3F00_0000
    ram_size: 1024 * MIB, // 1 GiB, and the peripheral window bounds it in practice
    dt_compat: &["brcm,bcm2837", "raspberrypi,3-model-b"],
    note: "boots: MMU + heap + mailbox framebuffer + timer-IRQ scheduling",
};

/// Raspberry Pi 4 (BCM2711) — **map only**, no boot path yet.
///
/// BCM2711 moves the peripheral window from the Pi 3's `0x3F00_0000` to
/// `0xFE00_0000` (low-peripheral mode), so the same PL011 driver works at a new
/// base. Unlike the Pi 3 this SoC *does* have a GICv2 — an ARM **GIC-400** whose
/// block starts at `0xFF84_0000` (distributor `+0x1000`, CPU interface `+0x2000`) —
/// so the GIC bases are real here rather than 0. Still absent: PL031 RTC, `fw_cfg`
/// and virtio-mmio; on-screen output would come from the VideoCore mailbox at
/// `0xFE00_B880`. Addresses per `docs/15-hardware-bringup.md`.
pub const BCM2711: Map = Map {
    name: "raspberry-pi-4 (bcm2711)",
    uart: 0xFE20_1000, // BCM2711 PL011 (UART0), peripheral base 0xFE00_0000
    uart_kind: "PL011",
    uart_driver: None,
    gic_dist: 0xFF84_1000, // GIC-400 distributor
    gic_cpu: 0xFF84_2000,  // GIC-400 CPU interface
    rtc: 0,                // no PL031 (I2C RTC or none)
    gpio: 0xFE20_0000,     // BCM2711 GPIO
    fwcfg: 0,              // no fw_cfg
    virtio: 0,             // no virtio-mmio
    // Same 0x00_B880 mailbox offset as BCM2837, rebased onto BCM2711's
    // 0xFE00_0000 low-peripheral window (already named in this map's doc
    // comment above; item 4 below is wiring a driver against it, not
    // discovering the address).
    mailbox: 0xFE00_B880,
    ram_base: 0,           // DRAM from 0; peripheral window moved up to 0xFE00_0000
    // The Pi 4 ships as 1/2/4/8 GiB. The map states the *smallest* SKU: under-claiming
    // costs frames a live tree can hand back, over-claiming would hand the allocator
    // addresses that are not memory on the board actually in front of us.
    ram_size: 1024 * MIB,
    dt_compat: &["brcm,bcm2711", "raspberrypi,4-model-b"],
    // 3.672: kernel/linker-rpi4.ld links the ELF at 0x8_0000 (where the VideoCore
    // firmware loads a flat `kernel8.img`), verified by objcopy + objdump only —
    // no QEMU rpi4 UART path or real board has run this image. GIC-400 init and
    // the mailbox framebuffer (items 3-4) are still open, so this stays "map
    // only" rather than becoming a claim this board boots.
    note: "map only: flat-binary link target added (0x8_0000, compile-checked only); \
           still needs GIC-400 init + mailbox FB",
};

/// PINE64 / PinePhone (Allwinner A64) — **map only**, and the map alone is *not* a
/// port of this board.
///
/// The A64 puts its peripherals in the Allwinner `0x01C0_0000` window: UART0 at
/// `0x01C2_8000`, PIO (pin controller / GPIO) at `0x01C2_0800`, and a real GICv2 at
/// `0x01C8_1000` (distributor) / `0x01C8_2000` (CPU interface). Addresses per the
/// blueprint table in `docs/15-hardware-bringup.md`.
///
/// The important entry is **not** an address: UART0 is a DesignWare 8250/16550-class
/// part, so unlike the two Pis this board cannot reuse our PL011 driver at a new
/// base — it needs a second UART driver behind the same interface (docs/15, "the
/// parts the base-address abstraction does not solve", item 1). It also boots
/// through ATF/U-Boot rather than `-kernel`, and its display path is Display
/// Engine 2 + TCON + HDMI, not a mailbox. No PL031 (the RTC lives in the AXP803
/// PMIC over I²C), no `fw_cfg`, no virtio-mmio.
pub const A64: Map = Map {
    name: "pine64 (allwinner a64)",
    uart: 0x01C2_8000, // UART0 — DesignWare 8250, *not* PL011
    uart_kind: "8250dw",
    uart_driver: Some(
        "DesignWare 8250/16550 — the PL011 driver does not apply (uart_ns16550 drives it)",
    ),
    gic_dist: 0x01C8_1000, // GICv2 distributor
    gic_cpu: 0x01C8_2000,  // GICv2 CPU interface
    rtc: 0,                // no PL031: RTC is in the AXP803 PMIC over I2C
    gpio: 0x01C2_0800,     // PIO pin controller
    fwcfg: 0,              // no fw_cfg
    virtio: 0,             // no virtio-mmio
    mailbox: 0,             // no VideoCore mailbox: display is DE2 + TCON + HDMI
    ram_base: 0x4000_0000, // A64 DRAM window starts at 1 GiB
    ram_size: 2048 * MIB,  // PINE64 / PinePhone ship 2 GiB (3 GiB on some SKUs)
    dt_compat: &["allwinner,sun50i-a64", "pine64"],
    note: "map only: needs ATF/U-Boot entry, an 8250 UART driver, DE2+HDMI framebuffer",
};

/// Every board the kernel knows an address map for, with the names `board <name>`
/// accepts (first name is the canonical one).
pub const KNOWN: [(&str, &[&str], &Map); 4] = [
    ("virt", &["virt", "qemu-virt", "qemu"], &VIRT),
    ("raspi", &["raspi", "pi3", "rpi3", "bcm2837"], &BCM2837),
    ("rpi4", &["rpi4", "pi4", "bcm2711"], &BCM2711),
    ("pine64", &["pine64", "pine", "pinephone", "a64", "sun50i"], &A64),
];

// --- The board compiled in for this build. Precedence when several board features
// are somehow enabled at once: raspi > rpi4 > pine64 > virt — i.e. the *bootable*
// Pi 3 path is never silently rebased onto another SoC's addresses, and the
// map-only boards never displace a board that actually boots.
#[cfg(all(
    not(feature = "board-raspi"),
    not(feature = "board-rpi4"),
    not(feature = "board-pine64")
))]
pub const ACTIVE: Map = VIRT;
#[cfg(feature = "board-raspi")]
pub const ACTIVE: Map = BCM2837;
#[cfg(all(feature = "board-rpi4", not(feature = "board-raspi")))]
pub const ACTIVE: Map = BCM2711;
#[cfg(all(
    feature = "board-pine64",
    not(feature = "board-raspi"),
    not(feature = "board-rpi4")
))]
pub const ACTIVE: Map = A64;

/// Human-readable name of the active board.
pub const NAME: &str = ACTIVE.name;

pub const UART_BASE: usize = ACTIVE.uart;
pub const GIC_DIST_BASE: usize = ACTIVE.gic_dist;
pub const GIC_CPU_BASE: usize = ACTIVE.gic_cpu;
pub const RTC_BASE: usize = ACTIVE.rtc;
pub const GPIO_BASE: usize = ACTIVE.gpio;
pub const FWCFG_BASE: usize = ACTIVE.fwcfg;
pub const VIRTIO_MMIO_BASE: u64 = ACTIVE.virtio;
pub const MAILBOX_BASE: usize = ACTIVE.mailbox;

/// Const string equality, so the console can pick its driver from [`Map::uart_kind`]
/// — the single source of truth for the UART *part* — at compile time.
pub const fn str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Which kernel driver drives a given UART part. Every part we map is either a
/// PL011 (`uart.rs`) or a DesignWare 8250/16550 (`uart_ns16550.rs`); a board whose
/// part is neither has no console and must add a driver before it can be booted.
pub const fn uart_driver_for(kind: &str) -> &'static str {
    if str_eq(kind, "PL011") {
        "pl011"
    } else if str_eq(kind, "8250dw") {
        "ns16550"
    } else {
        "none"
    }
}

/// True when the **active** board's console is a DesignWare 8250/16550 rather than
/// a PL011. `uart.rs` branches on this constant, so the unused driver folds away.
pub const UART_IS_8250: bool = str_eq(ACTIVE.uart_kind, "8250dw");

/// Look up a board by any of its accepted names (case-insensitive is not needed —
/// the shell already hands us the raw token; names are lowercase by convention).
pub fn lookup(name: &str) -> Option<&'static Map> {
    for (_, aliases, map) in KNOWN.iter() {
        for a in aliases.iter() {
            if *a == name {
                return Some(map);
            }
        }
    }
    None
}

/// Print one base line: an address, or `none` when the board lacks the peripheral.
fn line(label: &str, base: u64) {
    if base == 0 {
        crate::kprintln!("  {} none", label);
    } else {
        crate::kprintln!("  {} @ {:#010x}", label, base);
    }
}

/// Width of the label column shared by every map row, so the addresses line up.
const LABEL_W: usize = 16;

/// Print the UART row. Unlike the other rows the *part* is named, because it is not
/// the same everywhere: PL011 on virt and both Pis, DesignWare 8250 on the A64.
fn uart_line(m: &Map) {
    // "UART (" + kind + ")" padded out to the shared label column.
    let pad = LABEL_W.saturating_sub(7 + m.uart_kind.len());
    if m.uart == 0 {
        crate::kprintln!("  UART ({}){:w$} none", m.uart_kind, "", w = pad);
    } else {
        crate::kprintln!("  UART ({}){:w$} @ {:#010x}", m.uart_kind, "", m.uart, w = pad);
    }
    // A board whose UART is not a PL011 needs a *driver*, not just this address.
    if let Some(why) = m.uart_driver {
        crate::kprintln!("       ^ {}", why);
    }
}

/// Print a board's peripheral map. Shared by `board` (active) and `board <name>`.
fn print_map(m: &Map) {
    uart_line(m);
    // Which of the kernel's two console drivers this board's UART part needs. As
    // of milestone 2.724 both exist, so this row names a driver that is really in
    // the build rather than one that is planned.
    crate::kprintln!("  {:<16} {}", "console driver", uart_driver_for(m.uart_kind));
    line("GIC distributor ", m.gic_dist as u64);
    line("GIC cpu iface   ", m.gic_cpu as u64);
    line("RTC  (PL031)    ", m.rtc as u64);
    // The GPIO part differs too (PL061 on virt, BCM GPIO on the Pis, PIO on the
    // A64), so that label stays generic.
    line("GPIO            ", m.gpio as u64);
    line("fw_cfg          ", m.fwcfg as u64);
    line("virtio-mmio base", m.virtio);
    line("mailbox         ", m.mailbox as u64);
}

/// Print the active board's peripheral map (the `board` shell command).
pub fn describe() {
    crate::kprintln!("board: {}", NAME);
    print_map(&ACTIVE);
    crate::kprintln!("  (real boards define their own map; see docs/15)");
}

/// Print a *named* board's map — inspection, not the board we booted on
/// (`board <name>`).
pub fn describe_named(name: &str) {
    match lookup(name) {
        Some(m) => {
            let tag = if m.name == NAME { "active" } else { "not active — inspection only" };
            crate::kprintln!("board: {} ({})", m.name, tag);
            print_map(m);
            crate::kprintln!("  {}", m.note);
        }
        None => {
            crate::kprintln!("board: unknown board '{}'", name);
            list();
        }
    }
}

// --- `board check`: the compiled-in map vs. the device tree -------------------
//
// Every map in this file is a *claim*, typed in from a datasheet or a blueprint
// table. The device tree is the machine describing itself. Milestone 2.725 makes
// the kernel state the two side by side instead of trusting the claim: for each
// peripheral both values are printed with an explicit verdict, so a wrong constant
// shows up as a line of output rather than as silence on a serial port.

/// Format a 32-bit MMIO base as `0x........`, or `none` when the value is 0 (which
/// means "this side does not have this peripheral", never address zero).
fn addr_str(v: u64, buf: &mut [u8; 10]) -> &str {
    if v == 0 {
        return "none";
    }
    buf[0] = b'0';
    buf[1] = b'x';
    for i in 0..8 {
        let nib = ((v >> (28 - 4 * i)) & 0xf) as u8;
        buf[2 + i] = if nib < 10 { b'0' + nib } else { b'a' + nib - 10 };
    }
    core::str::from_utf8(&buf[..10]).unwrap_or("?")
}

/// Format a DRAM base. Unlike [`addr_str`] this never prints `none`: **0 is a real
/// base** for memory (both Pis start DRAM there), and the shorthand that keeps a
/// peripheral table honest would be a misreading on this row. Widens to 16 digits
/// above 4 GiB rather than silently truncating a range we could not otherwise show.
fn base_str(v: u64, buf: &mut [u8; 18]) -> &str {
    let n = if v > u32::MAX as u64 { 16 } else { 8 };
    buf[0] = b'0';
    buf[1] = b'x';
    for i in 0..n {
        let nib = ((v >> (4 * (n - 1 - i))) & 0xf) as u8;
        buf[2 + i] = if nib < 10 { b'0' + nib } else { b'a' + nib - 10 };
    }
    core::str::from_utf8(&buf[..2 + n]).unwrap_or("?")
}

/// Format a DRAM size in whole mebibytes — the unit the map, every `-m` flag and
/// every SKU sheet is written in. A size that is *not* a whole number of MiB rounds
/// down and gains a `+`, so an odd figure can never be read as a round one. 0 is
/// `none`: a map with no DRAM to declare, which is a statement, not a size.
fn mib_str(v: u64, buf: &mut [u8; 24]) -> &str {
    if v == 0 {
        return "none";
    }
    let mut n = v / MIB;
    let mut digits = [0u8; 20];
    let mut ndig = 0;
    if n == 0 {
        digits[0] = b'0';
        ndig = 1;
    }
    while n > 0 {
        digits[ndig] = b'0' + (n % 10) as u8;
        n /= 10;
        ndig += 1;
    }
    let mut i = 0;
    while i < ndig {
        buf[i] = digits[ndig - 1 - i];
        i += 1;
    }
    if v % MIB != 0 {
        buf[i] = b'+';
        i += 1;
    }
    buf[i..i + 4].copy_from_slice(b" MiB");
    core::str::from_utf8(&buf[..i + 4]).unwrap_or("?")
}

/// One comparison row. `comparable` is false when the tree we have describes a
/// *different machine* than the board we compiled for — then a difference is not a
/// finding, and saying "DISAGREE" would be a lie dressed as rigour.
///
/// Returns `(agree, disagree)` counters to add.
fn check_row(label: &str, map: &str, dtb: &str, comparable: bool) -> (u32, u32) {
    let (verdict, a, d) = if !comparable {
        ("n/a (other machine)", 0, 0)
    } else if dtb.is_empty() || dtb == "none" && map != "none" {
        // The tree says nothing about this peripheral: the map is unverified here,
        // which is neither agreement nor a contradiction.
        ("not in the tree", 0, 0)
    } else if map == dtb {
        ("agree", 1, 0)
    } else {
        ("DISAGREE", 0, 1)
    };
    let shown = if dtb.is_empty() { "-" } else { dtb };
    crate::kprintln!("  {:<16} {:<12} {:<12} {}", label, map, shown, verdict);
    (a, d)
}

/// Cross-check the compiled-in [`ACTIVE`] map against the device tree the kernel
/// actually parsed (`board check`).
///
/// This is docs/15 item 4 / suggested-order step 2 in its *verification* form: the
/// map is still what the drivers use, but the kernel can now say whether the machine
/// agrees with it. On QEMU's `-kernel` ELF path firmware passes no DTB pointer, so
/// what we diff against is the bundled reference `virt.dtb` — the check says so on
/// its first line, because a self-check against a fixture must never read as a
/// self-check against the live machine.
pub fn check() {
    let dt = crate::fdt::info();
    if !dt.valid {
        crate::kprintln!("board check: no device tree parsed — nothing to check the map against");
        return;
    }
    // The bundled fixture is a QEMU virt tree by construction. Comparing it to a
    // BCM2711 or A64 map would manufacture "disagreements" that only say the two
    // describe different machines.
    let comparable = !dt.reference || str_eq(NAME, VIRT.name);
    crate::kprintln!("board check: {} vs device tree", NAME);
    crate::kprintln!(
        "  tree source      {}",
        if dt.reference {
            "bundled reference virt.dtb (firmware passed no pointer)"
        } else {
            "live blob from firmware (x0)"
        }
    );
    crate::kprintln!("  tree model       '{}'", crate::fdt::model());
    if !comparable {
        crate::kprintln!("  ! the tree describes QEMU virt, not this board — rows below are");
        crate::kprintln!("    informational only; differences are expected, not map errors.");
    }
    crate::kprintln!("  {:<16} {:<12} {:<12} {}", "peripheral", "map", "device tree", "verdict");

    let (mut agree, mut disagree) = (0u32, 0u32);
    let (mut mb, mut db) = ([0u8; 10], [0u8; 10]);
    let (a, d) = check_row(
        "UART base",
        addr_str(ACTIVE.uart as u64, &mut mb),
        addr_str(dt.uart_base, &mut db),
        comparable,
    );
    agree += a;
    disagree += d;
    // The part, not just the address: a PL011 constant pointed at a 16550 window is
    // the failure mode this whole board/driver split exists to prevent.
    let (a, d) = check_row("UART part", ACTIVE.uart_kind, dt.uart_kind, comparable);
    agree += a;
    disagree += d;
    let (a, d) = check_row(
        "GIC distributor",
        addr_str(ACTIVE.gic_dist as u64, &mut mb),
        addr_str(dt.gic_base, &mut db),
        comparable,
    );
    agree += a;
    disagree += d;
    // The CPU interface is checked as its own row because it is its own `reg` range:
    // a tree that names the distributor and nothing else must show up here as an
    // unverified row, not be waved through on the distributor's agreement.
    let (a, d) = check_row(
        "GIC cpu iface",
        addr_str(ACTIVE.gic_cpu as u64, &mut mb),
        addr_str(dt.gic_cpu_base, &mut db),
        comparable,
    );
    agree += a;
    disagree += d;
    let (a, d) = check_row(
        "RTC (PL031)",
        addr_str(ACTIVE.rtc as u64, &mut mb),
        addr_str(dt.rtc_base, &mut db),
        comparable,
    );
    agree += a;
    disagree += d;
    // DRAM (2.730). The base is checked like any other row — memory starting somewhere
    // the map does not expect is a map error exactly as a wrong UART base is. A size of
    // 0 in the tree means the walker found no `/memory` node at all, which is silence,
    // not a claim of zero memory; `""` is how `check_row` is told that.
    let described = dt.ram_size != 0;
    let (mut mr, mut dr) = ([0u8; 18], [0u8; 18]);
    let (a, d) = check_row(
        "DRAM base",
        base_str(ACTIVE.ram_base, &mut mr),
        if described { base_str(dt.ram_base, &mut dr) } else { "" },
        comparable,
    );
    agree += a;
    disagree += d;
    // The size row is deliberately *not* counted. Every other row compares a constant
    // fixed by the silicon; this one compares a figure QEMU sets with `-m` and real
    // SKUs of one board ship differently. Calling that a disagreement would train the
    // operator to ignore the word on the rows where it means something.
    let (mut ms, mut ds) = ([0u8; 24], [0u8; 24]);
    let map_size = mib_str(ACTIVE.ram_size, &mut ms);
    let tree_size = if described { mib_str(dt.ram_size, &mut ds) } else { "-" };
    let verdict = if !comparable {
        "n/a (other machine)"
    } else if !described {
        "not in the tree"
    } else if ACTIVE.ram_size == dt.ram_size {
        "agree (not counted: size is variable)"
    } else {
        "differs — a size, not a map error"
    };
    crate::kprintln!("  {:<16} {:<12} {:<12} {}", "DRAM size", map_size, tree_size, verdict);

    if !comparable {
        crate::kprintln!("  => not comparable: cross-machine tree (0 checked)");
    } else {
        crate::kprintln!(
            "  => {} agree, {} disagree ({} rows the tree does not describe)",
            agree,
            disagree,
            6 - agree - disagree
        );
        if disagree > 0 {
            crate::kprintln!("  => the map is WRONG for this machine — trust the tree, fix board.rs");
        }
    }
    crate::kprintln!("  (only peripherals fdt.rs walks are checked; see docs/15 item 4)");
}

// --- `board machine`: is this tree even *about* this machine? -------------------
//
// `board check` (2.725) and `board resolved` (2.726) both rest on an unexamined
// assumption: that the tree in front of us describes the machine we compiled for.
// Everything they say is worthless if it does not — a BCM2711 map diffed against a
// virt tree produces five confident rows about nothing, and an override taken off
// such a tree would rebase real MMIO onto another SoC. `check` only ever dodged this
// by a special case for the *bundled* fixture; a LIVE tree from the wrong machine
// walked straight through. Milestone 2.728 turns the assumption into a checked
// claim: every [`Map`] states the root `compatible` names its machine answers to
// ([`Map::dt_compat`]), and the kernel reports them against what the tree calls
// itself.

/// What [`machine_from`] concluded about a tree's identity.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Machine {
    /// The tree names one of the board's `dt_compat` strings: it is this machine.
    Same,
    /// The tree names a machine, and it is not this one. Nothing derived from it —
    /// diff rows, resolved bases — describes the hardware we are running on.
    Other,
    /// The tree states no root identity at all. Not a contradiction: there is simply
    /// nothing to check, and saying "OTHER" would be an accusation we cannot support.
    Unstated,
}

impl Machine {
    pub fn as_str(self) -> &'static str {
        match self {
            Machine::Same => "same machine",
            Machine::Other => "OTHER MACHINE",
            Machine::Unstated => "tree names no machine",
        }
    }
}

/// True when `list` — a `compatible` list flattened to comma separators by
/// [`crate::fdt::root_compat`] — contains `name` as a whole element.
///
/// Whole-element, not substring: `brcm,bcm2711` must not be satisfied by a tree
/// saying `brcm,bcm2711x`, and `pine64` must not be satisfied by `pine64-plus` —
/// those are different machines whose names merely start alike. The flattening lost
/// the exact NUL boundaries, but every real name is itself `vendor,part`, so commas
/// are boundaries the names already contain; matching on them costs nothing and is
/// strictly tighter than a substring test.
fn list_has(list: &str, name: &str) -> bool {
    let (l, n) = (list.as_bytes(), name.as_bytes());
    if n.is_empty() || n.len() > l.len() {
        return false;
    }
    let mut i = 0;
    while i + n.len() <= l.len() {
        let starts = i == 0 || l[i - 1] == b',';
        let ends = i + n.len() == l.len() || l[i + n.len()] == b',';
        if starts && ends && &l[i..i + n.len()] == n {
            return true;
        }
        i += 1;
    }
    false
}

/// Does a tree calling itself `model` / `compat` describe the board `m` maps?
///
/// Pure, so the self-test can put trees in front of it that this machine will never
/// boot (`board machine test`) — the wrong-machine verdict is the whole point of the
/// milestone and must not be a branch nobody has run.
pub fn machine_from(m: &Map, model: &str, compat: &str) -> Machine {
    for want in m.dt_compat.iter() {
        if list_has(compat, want) {
            return Machine::Same;
        }
        // A tree with no root `compatible` may still name itself in `model` (QEMU's
        // virt tree uses the same string for both). Only consulted when `compatible`
        // is absent: where both exist, `compatible` is the machine-readable one.
        if compat.is_empty() && model == *want {
            return Machine::Same;
        }
    }
    if compat.is_empty() && model.is_empty() {
        Machine::Unstated
    } else {
        Machine::Other
    }
}

/// Which board the kernel *does* have a map for that this tree matches, if any —
/// so a wrong-machine verdict can name the machine instead of only denying ours.
pub fn board_from(model: &str, compat: &str) -> Option<&'static Map> {
    for (_, _, m) in KNOWN.iter() {
        if machine_from(m, model, compat) == Machine::Same {
            return Some(m);
        }
    }
    None
}

/// Report the tree's root identity against the compiled-in board (`board machine`).
pub fn describe_machine() {
    let dt = crate::fdt::info();
    let (model, compat) = (crate::fdt::model(), crate::fdt::root_compat());
    crate::kprintln!("board machine: {} vs device tree", NAME);
    if !dt.valid {
        crate::kprintln!("  no device tree parsed — the machine never named itself");
        crate::kprintln!("  => unverified: the map is a claim with nothing to check it against");
        return;
    }
    crate::kprintln!(
        "  tree source      {}",
        if dt.reference {
            "bundled reference virt.dtb (firmware passed no pointer)"
        } else {
            "live blob from firmware (x0)"
        }
    );
    crate::kprintln!("  tree model       '{}'", model);
    crate::kprintln!("  tree compatible  '{}'", compat);
    // The map's side of the claim, in the tree's own vocabulary.
    crate::kprint!("  map compatible   ");
    for (i, c) in ACTIVE.dt_compat.iter().enumerate() {
        crate::kprint!("{}'{}'", if i == 0 { "" } else { ", " }, c);
    }
    crate::kprintln!("");

    let verdict = machine_from(&ACTIVE, model, compat);
    crate::kprintln!("  verdict          {}", verdict.as_str());
    match verdict {
        Machine::Same => {
            crate::kprintln!("  => the tree describes THIS board — `board check` rows and any");
            crate::kprintln!("     resolved base really are about the hardware we are on");
        }
        Machine::Other => {
            match board_from(model, compat) {
                Some(m) => crate::kprintln!("  => WRONG MACHINE: this tree is a {} tree", m.name),
                None => crate::kprintln!("  => WRONG MACHINE: and no map here matches that tree"),
            }
            crate::kprintln!("     nothing derived from it describes this hardware: the `board check`");
            crate::kprintln!("     rows are cross-machine noise and no base may be taken from it.");
            // Not merely reported since 2.729: resolution refuses such a tree outright
            // (rule 6), so this verdict has a consequence the operator can check.
            crate::kprintln!("     resolution refuses it (rule 6) — `board resolved` shows map only");
        }
        Machine::Unstated => {
            crate::kprintln!("  => the tree names no machine — the map stays an unchecked claim");
        }
    }
}

/// Exercise [`machine_from`] against fabricated identities (`board machine test`).
///
/// The interesting verdict — a tree from another machine — cannot occur on a QEMU
/// virt boot, so without this it would ship unexecuted.
pub fn machine_selftest() {
    let (mut pass, mut total) = (0u32, 0u32);
    crate::kprintln!("board machine: identity self-test against synthetic root nodes");

    rule_ok(&mut pass, &mut total,
        machine_from(&VIRT, "linux,dummy-virt", "linux,dummy-virt") == Machine::Same,
        "a virt tree is the virt board's own machine");
    rule_ok(&mut pass, &mut total,
        machine_from(&BCM2711, "linux,dummy-virt", "linux,dummy-virt") == Machine::Other,
        "an rpi4 kernel handed a virt tree says OTHER MACHINE (the milestone's case)");
    rule_ok(&mut pass, &mut total,
        machine_from(&VIRT, "Raspberry Pi 4 Model B", "raspberrypi,4-model-b,brcm,bcm2711")
            == Machine::Other,
        "and the reverse: a virt kernel handed a Pi 4 tree is just as wrong");
    rule_ok(&mut pass, &mut total,
        machine_from(&BCM2711, "Raspberry Pi 4 Model B", "raspberrypi,4-model-b,brcm,bcm2711")
            == Machine::Same,
        "a name anywhere in the list counts, not only the first");
    rule_ok(&mut pass, &mut total,
        machine_from(&VIRT, "linux,dummy-virt", "") == Machine::Same,
        "a tree naming itself only in `model` is still identifiable");
    rule_ok(&mut pass, &mut total,
        machine_from(&VIRT, "", "") == Machine::Unstated,
        "a tree that names no machine is unstated, not wrong");
    rule_ok(&mut pass, &mut total,
        machine_from(&BCM2711, "", "brcm,bcm2712") == Machine::Other,
        "a near-miss SoC number is a different machine, not a match");
    rule_ok(&mut pass, &mut total,
        machine_from(&A64, "", "pine64-plus") == Machine::Other,
        "whole names only: `pine64` is not satisfied by `pine64-plus`");
    rule_ok(&mut pass, &mut total,
        board_from("", "brcm,bcm2837,raspberrypi,3-model-b").map(|m| m.name) == Some(BCM2837.name),
        "a wrong-machine tree can be named: the Pi 3 tree resolves to the Pi 3 map");

    crate::kprintln!("board machine: {}/{} identity rules held", pass, total);
}

// --- `board::resolved()`: letting the machine override the map -----------------
//
// Milestone 2.725 made the kernel *compare* the compiled-in map with the device
// tree. Comparing is not using: every driver still read a constant typed in from a
// datasheet. Milestone 2.726 closes that half — a **live** device tree now wins
// over the compiled-in constants for peripheral consumers, because a live blob is
// the machine describing itself and the map is only our claim about it.
//
// Four rules keep that override honest. Each is enforced by `resolve_from` and
// proved by `board resolved test`:
//
// 1. **Only a live blob may override.** `fdt.rs` falls back to a bundled reference
//    `virt.dtb` when firmware passed no pointer, so the parser is exercised on
//    QEMU's `-kernel` path. A fixture must never drive real MMIO — that would be
//    the same lie 2.725 refused to tell, one layer deeper.
// 2. **Silence is not absence.** Our walker only looks at root's children and takes
//    the first match, so a peripheral it did not find means "not discovered", not
//    "this board has none". A tree value of 0 therefore never replaces a real map
//    address; it leaves the row on the map.
// 3. **A base is not a part.** If the tree names a different UART *part* than the
//    map does, the base is NOT taken: driving 16550 registers with the PL011 driver
//    at a "more correct" address is a dead console, which is strictly worse than an
//    honestly wrong constant. The conflict is reported instead.
// 4. **The console stays compile-time.** `uart.rs` / `uart_ns16550.rs` keep reading
//    [`UART_BASE`], and they must: the console is brought up *before* the tree is
//    parsed, because otherwise a bad blob has nowhere to report itself. Relocating
//    the one device whose failure is silent is the last thing to switch over, not
//    the first — it waits for real silicon.
//
// Milestone 2.727 carries the resolved map into the *interrupt* driver and adds the
// fifth rule that move requires:
//
// 5. **The GIC's two windows move together or not at all.** A GICv2 driver needs a
//    distributor *and* a CPU interface, and the tree states both as two ranges of one
//    node's `reg`. Taking the distributor from the machine while the CPU interface
//    stays a compile-time constant would build a controller out of two different
//    machines' addresses — worse than either source alone, and precisely the kind of
//    unprovenanced address this whole line of milestones exists to stop. So if the
//    tree names only one of the two, *neither* is taken and the pair stays on the map
//    (reported as a partial discovery, not silently).
//
// Milestone 2.728 gave the kernel a verdict on *whose* machine the tree describes
// (`board machine`) but no consequence for it; 2.729 makes it the sixth rule, and the
// only one that bounds the tree as a whole rather than one peripheral:
//
// 6. **A tree from another machine overrides nothing.** Rules 2-5 each ask whether
//    one row may move; none of them asks whether the blob is about this board at all.
//    A `board-rpi4` kernel handed a live virt tree passes every other rule row by row
//    — the parts even match, both being PL011 — and rebases its whole map onto
//    another SoC's addresses. So resolution now consults [`machine_from`] first: on
//    [`Machine::Other`] the map is returned untouched, with the refusal recorded
//    ([`Resolved::wrong_machine`]) rather than left for the operator to infer from a
//    table that looks ordinary. [`Machine::Unstated`] is *not* a refusal — a tree
//    that names no machine contradicts nothing, and treating silence as an accusation
//    would disable resolution on every blob whose root omits `compatible`.
//
// Milestone 2.730 finally resolves the largest resource on the board — **DRAM** — and
// it needs a rule the peripheral rows never did. A wrong MMIO base costs you one
// device; a wrong RAM range is handed to the frame allocator, which then hands out
// addresses that are not memory:
//
// 7. **Impossible memory is refused, not believed.** RAM is the one row where a tree
//    can state something no machine could be: a range that wraps the address space, a
//    size smaller than the kernel already occupying it, one larger than our identity
//    map can address, or a base that is not even page-aligned. Rules 2-6 all ask *where
//    a value came from*; this one asks whether the value is possible at all, and a
//    claim that fails leaves DRAM on the map with the reason recorded
//    ([`Resolved::ram_refused`]). Note what it is *not*: a tree reporting a different
//    but plausible size is taken, because `-m 384M` and a 2 GiB SKU are the machine
//    being itself, not the machine lying.

/// Smallest DRAM a `/memory` node may plausibly claim. The kernel image, its stacks
/// and its page tables are already resident before the tree is read, so a machine
/// claiming less memory than that is describing something we are demonstrably not
/// running on.
pub const RAM_MIN: u64 = MIB;

/// Largest DRAM a `/memory` node may plausibly claim: `paging.rs` builds a 39-bit
/// identity map (512 GiB of VA), so memory beyond it is memory this kernel could not
/// address even if the board really had it. A bound from our own address space, not a
/// guess about what boards exist.
pub const RAM_MAX: u64 = 1 << 39;

/// Why a claimed DRAM range cannot be true, or `None` if it is possible (rule 7).
///
/// Pure and total, so `board resolved test` can put impossible trees in front of it —
/// on QEMU the live path never runs at all, and a rule that only ever sees good input
/// is a rule nobody has tested.
pub fn ram_impossible(base: u64, size: u64) -> Option<&'static str> {
    if base.checked_add(size).is_none() {
        // A range that wraps past the top of the address space describes no memory.
        Some("the range wraps past the end of the address space")
    } else if size < RAM_MIN {
        Some("less DRAM than the running kernel already occupies")
    } else if size > RAM_MAX {
        Some("more DRAM than the 39-bit identity map can address")
    } else if base % 4096 != 0 || size % 4096 != 0 {
        // The frame allocator hands out 4 KiB frames; a range that is not a whole
        // number of them cannot be what it says it is.
        Some("base or size is not 4 KiB page-aligned")
    } else {
        None
    }
}

/// Where a resolved value came from.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Src {
    /// The compiled-in map: no live tree, or the tree does not describe this row.
    Map,
    /// A live tree named this peripheral at the same address the map does.
    Confirmed,
    /// A live tree overrode the compiled-in value.
    Tree,
}

impl Src {
    pub fn as_str(self) -> &'static str {
        match self {
            Src::Map => "map (compile-time)",
            Src::Confirmed => "tree (confirms map)",
            Src::Tree => "tree (OVERRIDE)",
        }
    }
}

/// The peripheral addresses the kernel will actually use, after a live device tree
/// has had its say. Every field carries where it came from — an address without a
/// provenance is exactly what this milestone exists to stop shipping.
#[derive(Clone, Copy)]
pub struct Resolved {
    pub uart: usize,
    pub uart_kind: &'static str,
    pub gic_dist: usize,
    /// GICv2 CPU interface. Moves only in lockstep with `gic_dist` (rule 5) — a
    /// controller assembled from two machines' addresses is worse than either.
    pub gic_cpu: usize,
    pub rtc: usize,
    /// The DRAM range the kernel will size its frame allocator from. Base and size
    /// move as one (they are one `reg` entry, and half a range is not a range), so
    /// they share `ram_src`.
    pub ram_base: u64,
    pub ram_size: u64,
    /// Why a live tree's `/memory` claim was **not** taken (rule 7), or `None` when
    /// nothing was refused. A refusal is not the same as a tree that stayed silent:
    /// silence leaves `ram_src` on the map with nothing to report, a refusal leaves it
    /// on the map *and* owes the operator a reason.
    pub ram_refused: Option<&'static str>,
    pub uart_src: Src,
    pub gic_dist_src: Src,
    pub gic_cpu_src: Src,
    pub rtc_src: Src,
    pub ram_src: Src,
    /// True when a live blob (not the bundled fixture) was present. Presence is not
    /// permission: see `wrong_machine`, which is the one way a live tree can be here
    /// and still have overridden nothing.
    pub live: bool,
    /// True when a live tree named a machine that is not the compiled-in board: every
    /// row stayed on the map (rule 6). A tree naming *no* machine never sets this.
    pub wrong_machine: bool,
    /// True when a live tree named a different UART part than the map: the base was
    /// deliberately *not* taken (rule 3).
    pub uart_conflict: bool,
    /// True when a live tree named exactly *one* of the GIC's two windows: neither
    /// was taken (rule 5), and the pair stays on the map as a matched set.
    pub gic_partial: bool,
}

impl Resolved {
    /// First address past the resolved DRAM. Saturating rather than wrapping, so no
    /// arithmetic here can ever produce an end *below* the base — rule 7 already
    /// refuses a wrapping range, and this makes the property hold even for the map's
    /// own figures, which rule 7 never sees.
    pub fn ram_end(&self) -> u64 {
        self.ram_base.saturating_add(self.ram_size)
    }
}

/// Resolution as a **pure function** of the map and the parsed tree, so the rules
/// above can be tested against synthetic trees on a machine that never boots with a
/// live DTB (`board resolved test`). `resolve()` is just this applied to the real
/// [`ACTIVE`] map and the real parse result.
///
/// `model` / `compat` are the tree's *root* identity ([`crate::fdt::model`],
/// [`crate::fdt::root_compat`]), taken as parameters rather than read from the parser
/// so rule 6 stays as testable as the rest: the wrong-machine case cannot be produced
/// by booting this kernel, only by handing the function a foreign name.
pub fn resolve_from(m: &Map, dt: &crate::fdt::Info, model: &str, compat: &str) -> Resolved {
    let mut r = Resolved {
        uart: m.uart,
        uart_kind: m.uart_kind,
        gic_dist: m.gic_dist,
        gic_cpu: m.gic_cpu,
        rtc: m.rtc,
        ram_base: m.ram_base,
        ram_size: m.ram_size,
        ram_refused: None,
        uart_src: Src::Map,
        gic_dist_src: Src::Map,
        gic_cpu_src: Src::Map,
        rtc_src: Src::Map,
        ram_src: Src::Map,
        live: false,
        wrong_machine: false,
        uart_conflict: false,
        gic_partial: false,
    };
    // Rule 1: a fixture (or no tree at all) leaves the map untouched.
    if !dt.valid || dt.reference {
        return r;
    }
    r.live = true;
    // Rule 6, before any row is considered: the per-peripheral rules below all assume
    // the blob is about this board. If it names a different machine, no row is worth
    // examining — a foreign address that passes rules 2-5 is still a foreign address.
    if machine_from(m, model, compat) == Machine::Other {
        r.wrong_machine = true;
        return r;
    }

    // Rule 2 applies to each row: 0 means "not discovered", so it never overrides.
    if dt.uart_base != 0 {
        // Rule 3: the part must match before the address may move.
        if !dt.uart_kind.is_empty() && !str_eq(dt.uart_kind, m.uart_kind) {
            r.uart_conflict = true;
        } else {
            r.uart_src = if dt.uart_base == m.uart as u64 { Src::Confirmed } else { Src::Tree };
            r.uart = dt.uart_base as usize;
        }
    }
    // Rule 5: the distributor and the CPU interface are one controller, so they move
    // as a pair or not at all. Exactly one of them found is a *partial* discovery —
    // reported, and refused, rather than half-applied.
    match (dt.gic_base != 0, dt.gic_cpu_base != 0) {
        (true, true) => {
            r.gic_dist_src =
                if dt.gic_base == m.gic_dist as u64 { Src::Confirmed } else { Src::Tree };
            r.gic_dist = dt.gic_base as usize;
            r.gic_cpu_src =
                if dt.gic_cpu_base == m.gic_cpu as u64 { Src::Confirmed } else { Src::Tree };
            r.gic_cpu = dt.gic_cpu_base as usize;
        }
        (false, false) => {}
        _ => r.gic_partial = true,
    }
    if dt.rtc_base != 0 {
        r.rtc_src = if dt.rtc_base == m.rtc as u64 { Src::Confirmed } else { Src::Tree };
        r.rtc = dt.rtc_base as usize;
    }
    // Rule 2 for DRAM, with the discriminator moved: a base of 0 is a real base here
    // (both Pis start memory at 0), so what says "the walker found no /memory node" is
    // a size of 0. A range with no size is not a range.
    if dt.ram_size != 0 {
        // Rule 7: the only rule that asks whether a value is *possible*, because this
        // is the only row whose consumer is the frame allocator.
        match ram_impossible(dt.ram_base, dt.ram_size) {
            Some(why) => r.ram_refused = Some(why),
            None => {
                r.ram_src = if dt.ram_base == m.ram_base && dt.ram_size == m.ram_size {
                    Src::Confirmed
                } else {
                    Src::Tree
                };
                r.ram_base = dt.ram_base;
                r.ram_size = dt.ram_size;
            }
        }
    }
    r
}

static mut RESOLVED: Resolved = Resolved {
    uart: ACTIVE.uart,
    uart_kind: ACTIVE.uart_kind,
    gic_dist: ACTIVE.gic_dist,
    gic_cpu: ACTIVE.gic_cpu,
    rtc: ACTIVE.rtc,
    ram_base: ACTIVE.ram_base,
    ram_size: ACTIVE.ram_size,
    ram_refused: None,
    uart_src: Src::Map,
    gic_dist_src: Src::Map,
    gic_cpu_src: Src::Map,
    rtc_src: Src::Map,
    ram_src: Src::Map,
    live: false,
    wrong_machine: false,
    uart_conflict: false,
    gic_partial: false,
};

/// Set once [`resolve`] has run. Consumers that touch MMIO — [`crate::gic`] above
/// all — read this to *prove* they were wired after resolution rather than asserting
/// it in a comment: an access made while this is false used an unresolved base, and
/// on a machine with a live tree that is the wrong address.
static RESOLVE_DONE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Fold the parsed device tree into the peripheral map. Called once at boot, after
/// `fdt::parse`. Until it runs — and on every machine that hands us no live blob —
/// [`resolved`] returns the compile-time map unchanged, so a consumer that reads it
/// early is never worse off than one reading the constants directly.
pub fn resolve() {
    let dt = crate::fdt::info();
    let r = resolve_from(&ACTIVE, &dt, crate::fdt::model(), crate::fdt::root_compat());
    // SAFETY: single-threaded boot path, written once before other cores/consumers.
    unsafe { RESOLVED = r };
    RESOLVE_DONE.store(true, core::sync::atomic::Ordering::Release);
}

/// Whether [`resolve`] has already folded the tree into the map. False means
/// [`resolved`] is still handing out the compile-time map — correct on a board with
/// no live tree, but a *bug* in any consumer that runs before boot got here.
pub fn is_resolved() -> bool {
    RESOLVE_DONE.load(core::sync::atomic::Ordering::Acquire)
}

/// The addresses peripheral drivers should use: the map, plus whatever a live tree
/// corrected. Cheap (a `Copy` of a small struct) and safe before [`resolve`] runs.
pub fn resolved() -> Resolved {
    // SAFETY: scalar read of Copy data set once at boot.
    unsafe { RESOLVED }
}

/// One row of the `board resolved` table.
fn resolved_row(label: &str, map: usize, val: usize, src: Src) {
    if val == 0 && map == 0 {
        crate::kprintln!("  {:<16} {:<12} {:<12} {}", label, "none", "none", src.as_str());
    } else {
        crate::kprintln!("  {:<16} {:#010x}   {:#010x}   {}", label, map, val, src.as_str());
    }
}

/// Print the resolved peripheral map and where each value came from
/// (`board resolved`).
pub fn describe_resolved() {
    let r = resolved();
    let dt = crate::fdt::info();
    crate::kprintln!("board resolved: {}", NAME);
    crate::kprintln!(
        "  tree source      {}",
        if r.live && r.wrong_machine {
            "live blob from firmware (x0) — REFUSED: another machine's tree"
        } else if r.live {
            "live blob from firmware (x0) — may override the map"
        } else if dt.valid {
            "bundled reference virt.dtb (a fixture never drives MMIO)"
        } else {
            "none parsed — map only"
        }
    );
    crate::kprintln!("  {:<16} {:<12} {:<12} {}", "peripheral", "map", "in use", "source");
    resolved_row("UART base", ACTIVE.uart, r.uart, r.uart_src);
    // The part is never resolved *away* from the map: a base may only move within
    // one part (rule 3), so this row is what the resolved base is a base *of*.
    crate::kprintln!("  {:<16} {}", "UART part", r.uart_kind);
    resolved_row("GIC distributor", ACTIVE.gic_dist, r.gic_dist, r.gic_dist_src);
    // Printed right under the distributor because that is how it must be read: the
    // two are one controller, and their sources are always the same word (rule 5).
    resolved_row("GIC cpu iface", ACTIVE.gic_cpu, r.gic_cpu, r.gic_cpu_src);
    resolved_row("RTC (PL031)", ACTIVE.rtc, r.rtc, r.rtc_src);
    // DRAM, printed last because it is the row that is not like the others: not an
    // MMIO window the kernel pokes at, but the range it runs *out of*. Base and size
    // share one source — they are one `reg` entry, and half a range is not a range.
    let (mut mr, mut dr) = ([0u8; 18], [0u8; 18]);
    crate::kprintln!(
        "  {:<16} {:<12} {:<12} {}",
        "DRAM base",
        base_str(ACTIVE.ram_base, &mut mr),
        base_str(r.ram_base, &mut dr),
        r.ram_src.as_str()
    );
    let (mut ms, mut ds) = ([0u8; 24], [0u8; 24]);
    crate::kprintln!(
        "  {:<16} {:<12} {:<12} {}",
        "DRAM size",
        mib_str(ACTIVE.ram_size, &mut ms),
        mib_str(r.ram_size, &mut ds),
        r.ram_src.as_str()
    );
    if let Some(why) = r.ram_refused {
        crate::kprintln!("  ! the tree's /memory claim was REFUSED: {}", why);
        crate::kprintln!("    (rule 7: a wrong MMIO base costs you one device; a wrong RAM range");
        crate::kprintln!("     is handed to the allocator, which then hands out non-memory)");
    }
    if r.wrong_machine {
        crate::kprintln!("  ! this tree describes another machine — NOTHING was taken from it");
        crate::kprintln!("    (rule 6: every row above is the compile-time map. `board machine`");
        crate::kprintln!("     names the machine the tree really is.)");
    }
    if r.gic_partial {
        crate::kprintln!("  ! the tree names only one of the GIC's two windows — NEITHER taken");
        crate::kprintln!("    (a distributor and a CPU interface from different machines is");
        crate::kprintln!("     worse than an honestly wrong pair; see rule 5)");
    }
    if r.uart_conflict {
        crate::kprintln!("  ! the tree names a different UART part than the map — base NOT taken");
        crate::kprintln!("    (a right address with the wrong driver is a dead console)");
    }
    // The console is deliberately not a consumer of any of this; say so, every time,
    // so "the tree drives the kernel" is never read as more than it is.
    crate::kprintln!(
        "  console still uses the compile-time base {:#010x} ({} via {})",
        UART_BASE,
        ACTIVE.uart_kind,
        uart_driver_for(ACTIVE.uart_kind)
    );
    crate::kprintln!("  (console relocation waits for real silicon; see docs/15 item 4)");
    // The same admission for DRAM, and for the same reason: the heap is placed before
    // `fdt::parse` runs, so a resolved range it could have used did not exist yet.
    // Resolving a value the kernel does not yet consume is worth saying out loud.
    crate::kprintln!(
        "  heap still sized at compile time — placed before the tree is parsed; the",
    );
    crate::kprintln!("  resolved range is what the frame allocator will take (Phase 3)");
}

/// Build a synthetic [`crate::fdt::Info`] for the resolution self-test.
fn fake_tree(
    valid: bool,
    reference: bool,
    uart: u64,
    kind: &'static str,
    gic: u64,
    gic_cpu: u64,
    rtc: u64,
) -> crate::fdt::Info {
    crate::fdt::Info {
        valid,
        ram_base: 0,
        ram_size: 0,
        uart_base: uart,
        gic_base: gic,
        gic_cpu_base: gic_cpu,
        rtc_base: rtc,
        uart_kind: kind,
        reference,
    }
}

/// A tree whose only claim is a `/memory` node, for the rule-7 cases. Kept separate
/// from [`fake_tree`] so the dozen cases written before 2.730 keep reading as tests of
/// the rule they name instead of growing two arguments none of them use.
fn fake_ram(reference: bool, base: u64, size: u64) -> crate::fdt::Info {
    let mut t = fake_tree(true, reference, 0, "", 0, 0, 0);
    t.ram_base = base;
    t.ram_size = size;
    t
}

/// [`resolve_from`] against a tree whose root names no machine.
///
/// Every case written before milestone 2.729 fed the resolver a tree with no identity
/// at all, and rule 6 deliberately keeps that working: [`Machine::Unstated`] is not a
/// refusal. Naming the shape here keeps those cases about the rule they test instead
/// of repeating two empty strings ten times.
fn unnamed(m: &Map, dt: &crate::fdt::Info) -> Resolved {
    resolve_from(m, dt, "", "")
}

fn rule_ok(pass: &mut u32, total: &mut u32, ok: bool, what: &str) {
    *total += 1;
    if ok {
        *pass += 1;
        crate::kprintln!("  [ok]   {}", what);
    } else {
        crate::kprintln!("  [FAIL] {}", what);
    }
}

/// Exercise the six override rules against synthetic trees (`board resolved test`).
///
/// QEMU's `-kernel` path never hands us a live DTB, so the override branch would
/// otherwise be code nobody has ever executed. Feeding [`resolve_from`] fabricated
/// `Info` values runs it on every machine, every build.
pub fn resolve_selftest() {
    // A stand-in board with all three peripherals, so "the map had a value here" is
    // true for every row regardless of which board is compiled in.
    let m = VIRT;
    let (mut pass, mut total) = (0u32, 0u32);
    crate::kprintln!("board resolved: rule self-test against synthetic device trees");

    let r = unnamed(&m, &fake_tree(false, false, 0, "", 0, 0, 0));
    rule_ok(&mut pass, &mut total, !r.live && r.uart == m.uart && r.uart_src == Src::Map,
        "no tree: every value stays on the compile-time map");

    // Rule 1: a reference blob whose addresses differ must change nothing.
    let r = unnamed(&m,
        &fake_tree(true, true, 0x1000_0000, "PL011", 0x2000_0000, 0x2001_0000, 0x3000_0000));
    rule_ok(&mut pass, &mut total,
        !r.live && r.uart == m.uart && r.gic_dist == m.gic_dist && r.gic_cpu == m.gic_cpu
            && r.rtc == m.rtc,
        "rule 1: bundled reference tree never overrides (fixture != machine)");

    // A live tree that agrees: same numbers, but now confirmed by the machine.
    let r = unnamed(&m, &fake_tree(true, false, m.uart as u64, "PL011", m.gic_dist as u64,
        m.gic_cpu as u64, m.rtc as u64));
    rule_ok(&mut pass, &mut total,
        r.live && r.uart == m.uart && r.uart_src == Src::Confirmed && r.gic_dist_src == Src::Confirmed
            && r.gic_cpu_src == Src::Confirmed,
        "live tree agreeing with the map reads as confirmed, not as an override");

    // A live tree that disagrees: the machine wins.
    let r = unnamed(&m,
        &fake_tree(true, false, 0x0900_0000, "PL011", 0x0810_0000, 0x0820_0000, 0x0905_0000));
    rule_ok(&mut pass, &mut total,
        r.gic_dist == 0x0810_0000 && r.gic_dist_src == Src::Tree
            && r.rtc == 0x0905_0000 && r.rtc_src == Src::Tree,
        "live tree overrides GIC + RTC bases the map got wrong");

    // Rule 2: a row the walker did not find must not zero a working address.
    let r = unnamed(&m, &fake_tree(true, false, 0, "", 0, 0, 0));
    rule_ok(&mut pass, &mut total,
        r.live && r.uart == m.uart && r.gic_dist == m.gic_dist && r.gic_cpu == m.gic_cpu
            && r.rtc == m.rtc && r.rtc_src == Src::Map,
        "rule 2: a tree silent on a peripheral leaves that row on the map");

    // Rule 3: right address, wrong part — refuse the base, report the conflict.
    let r = unnamed(&m, &fake_tree(true, false, 0x01C2_8000, "8250dw", 0, 0, 0));
    rule_ok(&mut pass, &mut total,
        r.uart_conflict && r.uart == m.uart && r.uart_src == Src::Map,
        "rule 3: UART part mismatch blocks the base override (no dead console)");

    // Rule 5, the whole point of milestone 2.727. Both windows named: both move, and
    // both carry the same provenance — one controller, one source.
    let r = unnamed(&m,
        &fake_tree(true, false, 0, "", 0x0810_0000, 0x0820_0000, 0));
    rule_ok(&mut pass, &mut total,
        r.gic_dist == 0x0810_0000 && r.gic_cpu == 0x0820_0000
            && r.gic_dist_src == Src::Tree && r.gic_cpu_src == Src::Tree && !r.gic_partial,
        "rule 5: both GIC windows named - the pair moves together");

    // Only the distributor named: taking it would pair a tree distributor with a map
    // CPU interface. Neither moves.
    let r = unnamed(&m, &fake_tree(true, false, 0, "", 0x0810_0000, 0, 0));
    rule_ok(&mut pass, &mut total,
        r.gic_partial && r.gic_dist == m.gic_dist && r.gic_cpu == m.gic_cpu
            && r.gic_dist_src == Src::Map && r.gic_cpu_src == Src::Map,
        "rule 5: distributor alone is a PARTIAL discovery - neither window moves");

    // ...and symmetrically, a CPU interface with no distributor.
    let r = unnamed(&m, &fake_tree(true, false, 0, "", 0, 0x0820_0000, 0));
    rule_ok(&mut pass, &mut total,
        r.gic_partial && r.gic_dist == m.gic_dist && r.gic_cpu == m.gic_cpu,
        "rule 5: CPU interface alone is refused the same way (symmetric)");

    // A wrong pair is still a *pair*: overriding one window must never leave the two
    // sources disagreeing, which is the state rule 5 exists to make unrepresentable.
    let r = unnamed(&m,
        &fake_tree(true, false, 0, "", m.gic_dist as u64, 0x0820_0000, 0));
    rule_ok(&mut pass, &mut total,
        r.gic_dist_src == Src::Confirmed && r.gic_cpu_src == Src::Tree
            && r.gic_cpu == 0x0820_0000 && !r.gic_partial,
        "rule 5: a pair may be half-confirmed half-overridden, but never half-taken");

    // Rule 6, milestone 2.729. This tree is the *best-formed* one in the whole test:
    // the UART part matches, both GIC windows are named, every address is plausible.
    // Rules 1-5 would take all of it. It belongs to another machine, so none of that
    // matters — the addresses are real, just not real *here*.
    let foreign = fake_tree(true, false, 0x0900_0000, "PL011",
        0x0810_0000, 0x0820_0000, 0x0905_0000);
    let r = resolve_from(&m, &foreign,
        "Raspberry Pi 4 Model B", "raspberrypi,4-model-b,brcm,bcm2711");
    rule_ok(&mut pass, &mut total,
        r.live && r.wrong_machine && r.uart == m.uart && r.gic_dist == m.gic_dist
            && r.gic_cpu == m.gic_cpu && r.rtc == m.rtc && r.rtc_src == Src::Map,
        "rule 6: another machine's tree overrides nothing, however well-formed");

    // ...and the gate is the *identity*, not the content: the same blob, now claiming
    // to be this board, overrides exactly as it did before 2.729. Without this the
    // rule could be satisfied by a resolver that simply stopped resolving.
    let r = resolve_from(&m, &foreign, "linux,dummy-virt", "linux,dummy-virt");
    rule_ok(&mut pass, &mut total,
        !r.wrong_machine && r.gic_dist == 0x0810_0000 && r.gic_dist_src == Src::Tree
            && r.rtc == 0x0905_0000 && r.rtc_src == Src::Tree,
        "rule 6: the same tree naming THIS machine still overrides (identity, not content)");

    // Silence is not an accusation. A root without `compatible` names no machine and
    // so contradicts none, or rule 6 would disable resolution on every terse blob.
    let r = unnamed(&m, &foreign);
    rule_ok(&mut pass, &mut total,
        !r.wrong_machine && r.rtc == 0x0905_0000 && r.rtc_src == Src::Tree,
        "rule 6: a tree naming NO machine is not a refusal - resolution proceeds");

    // Rule 1 still runs first, so a foreign fixture is refused for being a fixture.
    // The operator is owed the true reason, not whichever refusal fires last.
    let r = resolve_from(&m, &fake_tree(true, true, 0x1000_0000, "PL011", 0, 0, 0),
        "Raspberry Pi 4 Model B", "raspberrypi,4-model-b,brcm,bcm2711");
    rule_ok(&mut pass, &mut total,
        !r.live && !r.wrong_machine && r.uart == m.uart,
        "rule 6: a foreign FIXTURE is refused as a fixture (rule 1 first), not as a machine");

    // Rule 2 reaches DRAM too, but through a different discriminator: a base of 0 is a
    // real base for memory, so it is a *size* of 0 that means "no /memory node found".
    let r = unnamed(&m, &fake_tree(true, false, 0, "", 0, 0, 0));
    rule_ok(&mut pass, &mut total,
        r.ram_base == m.ram_base && r.ram_size == m.ram_size && r.ram_src == Src::Map
            && r.ram_refused.is_none(),
        "rule 2: a tree with no /memory node leaves DRAM on the map (size 0 = silence)");

    // A live tree sizing memory differently is the machine being itself — `-m 384M` is
    // not a lie — so a plausible range is taken, base and size together.
    let r = unnamed(&m, &fake_ram(false, m.ram_base, 384 * MIB));
    rule_ok(&mut pass, &mut total,
        r.ram_size == 384 * MIB && r.ram_base == m.ram_base && r.ram_src == Src::Tree
            && r.ram_end() == m.ram_base + 384 * MIB,
        "rule 7: a plausible /memory range overrides the map's nominal DRAM size");

    // Same numbers as the map: confirmed, not an override. The distinction matters more
    // here than anywhere else — DRAM size is the one row expected to vary per boot.
    let r = unnamed(&m, &fake_ram(false, m.ram_base, m.ram_size));
    rule_ok(&mut pass, &mut total, r.ram_src == Src::Confirmed && r.ram_refused.is_none(),
        "rule 7: a tree stating the map's own DRAM range reads as confirmed");

    // Impossible claim 1: less memory than the running kernel already occupies. The
    // machine printing this is not the machine we are executing on.
    let r = unnamed(&m, &fake_ram(false, m.ram_base, 4096));
    rule_ok(&mut pass, &mut total,
        r.ram_refused.is_some() && r.ram_size == m.ram_size && r.ram_src == Src::Map,
        "rule 7: DRAM smaller than the resident kernel is REFUSED, not believed");

    // Impossible claim 2: more memory than our own identity map can address. The bound
    // comes from paging.rs, not from a guess about which boards exist.
    let r = unnamed(&m, &fake_ram(false, m.ram_base, RAM_MAX + 4096));
    rule_ok(&mut pass, &mut total,
        r.ram_refused.is_some() && r.ram_size == m.ram_size && r.ram_src == Src::Map,
        "rule 7: DRAM beyond the 39-bit identity map is REFUSED");

    // Impossible claim 3: a range that wraps the address space. Checked first, because
    // every later test would have to do overflowing arithmetic to reach a verdict.
    let r = unnamed(&m, &fake_ram(false, 0xFFFF_FFFF_FFFF_F000, 0x2000));
    rule_ok(&mut pass, &mut total,
        r.ram_refused.is_some() && r.ram_base == m.ram_base && r.ram_src == Src::Map,
        "rule 7: a DRAM range that wraps the address space is REFUSED");

    // Impossible claim 4: a base that is not a whole number of 4 KiB frames. Plausible
    // to the eye, and unusable to the allocator that would be handed it.
    let r = unnamed(&m, &fake_ram(false, m.ram_base + 1, 256 * MIB));
    rule_ok(&mut pass, &mut total,
        r.ram_refused.is_some() && r.ram_base == m.ram_base,
        "rule 7: a DRAM base that is not page-aligned is REFUSED");

    // Rules 1 and 6 govern DRAM exactly as they govern the peripheral rows: a fixture
    // may not size our memory, and neither may another machine's tree — both with
    // ranges rule 7 would happily accept, or these would prove nothing.
    let r = unnamed(&m, &fake_ram(true, m.ram_base, 384 * MIB));
    rule_ok(&mut pass, &mut total,
        !r.live && r.ram_size == m.ram_size && r.ram_src == Src::Map && r.ram_refused.is_none(),
        "rule 1: a fixture tree may not size our DRAM (and that is not a rule-7 refusal)");

    let mut foreign_ram = fake_ram(false, m.ram_base, 384 * MIB);
    foreign_ram.uart_base = 0x0900_0000;
    foreign_ram.uart_kind = "PL011";
    let r = resolve_from(&m, &foreign_ram,
        "Raspberry Pi 4 Model B", "raspberrypi,4-model-b,brcm,bcm2711");
    rule_ok(&mut pass, &mut total,
        r.wrong_machine && r.ram_size == m.ram_size && r.ram_src == Src::Map,
        "rule 6: another machine's tree may not size our DRAM either");

    crate::kprintln!("board resolved: {}/{} rules held", pass, total);
}

/// List the boards the kernel has a map for (`board list`).
pub fn list() {
    crate::kprintln!("known boards:");
    for (canon, _, m) in KNOWN.iter() {
        let mark = if m.name == NAME { " (active)" } else { "" };
        crate::kprintln!("  {:<6} {}{}", canon, m.name, mark);
    }
}
