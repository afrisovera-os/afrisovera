//! Minimal flattened device tree (FDT / DTB) parser.
//!
//! On real hardware — and on QEMU `virt` — the firmware hands the kernel a pointer
//! (in `x0` at entry) to a **device tree blob**: a big-endian, self-describing
//! description of the machine's memory and peripherals. Parsing it is how a kernel
//! discovers hardware instead of hard-coding it. This is a *minimal* reader: enough
//! to validate the blob, read the `model` string, pull RAM base/size from the
//! `/memory` node, and pick the UART / interrupt-controller / RTC nodes out of the
//! root's children — the first real step from a compile-time [`crate::board`] map
//! toward runtime hardware discovery. It does not build a general node/property API.
//!
//! For the UART the parser reports the *part* (`arm,pl011` vs `snps,dw-apb-uart` and
//! friends) alongside the base, because that is the fact a compile-time map is most
//! able to get silently wrong: a right address with a wrong driver is a dead console,
//! not a working one. `board check` (milestone 2.725) is the consumer — it diffs this
//! discovered view against the compiled-in map, peripheral by peripheral.
//!
//! The interrupt controller is read as **two** windows, not one: a GICv2 node's `reg`
//! lists the distributor and then the CPU interface, and a driver needs both. Reading
//! only the first (through milestone 2.726) is what kept `gic.rs` half on the
//! compile-time map; milestone 2.727 takes the second range too.
//!
//! FDT is spec'd by the Devicetree Specification; all header/struct words are
//! big-endian.

const FDT_MAGIC: u32 = 0xd00d_feed;
const FDT_BEGIN_NODE: u32 = 1;
const FDT_END_NODE: u32 = 2;
const FDT_PROP: u32 = 3;
const FDT_NOP: u32 = 4;
const FDT_END: u32 = 9;

/// Parsed facts from the device tree.
#[derive(Clone, Copy)]
pub struct Info {
    pub valid: bool,
    pub ram_base: u64,
    pub ram_size: u64,
    /// UART / GIC-distributor MMIO bases discovered from the tree (0 if not found).
    pub uart_base: u64,
    pub gic_base: u64,
    /// GICv2 **CPU-interface** base — the *second* `reg` range of the interrupt
    /// controller node (the first is the distributor), 0 if the node has only one.
    /// A GICv2 driver needs both windows, and they are two entries of one property:
    /// discovering the distributor alone would leave `gic.rs` half on the map.
    pub gic_cpu_base: u64,
    /// PL031-class RTC base discovered from the tree (0 if the tree has none).
    pub rtc_base: u64,
    /// Which UART *part* the tree says is at `uart_base`, in the same vocabulary as
    /// [`crate::board::Map::uart_kind`] ("PL011" / "8250dw"), or "" if no UART node
    /// was recognized. The base alone cannot catch a map that names the wrong part.
    pub uart_kind: &'static str,
    /// True when these facts came from the *bundled reference* DTB rather than a
    /// live pointer from firmware — used to keep the RAM size honest (a reference
    /// blob must never drive the real frame allocator).
    pub reference: bool,
}

// A real QEMU `virt` device tree (dumped with -M virt -m 384M), bundled so the
// parser can be exercised even though QEMU's ELF boot path passes x0=0 (no live
// DTB). On real hardware / a raw-image boot, firmware passes a live pointer.
static REFERENCE_DTB: &[u8] = include_bytes!("virt.dtb");

const MODEL_CAP: usize = 48;
static mut MODEL: [u8; MODEL_CAP] = [0; MODEL_CAP];
static mut MODEL_LEN: usize = 0;

/// The **root** node's `compatible` list — the machine naming itself, in the same
/// vocabulary `board.rs` uses (`linux,dummy-virt`, `brcm,bcm2711`, …). Kept apart
/// from the per-node `compatible` the peripheral walk reads at depth 2: that one
/// says *what a device is*, this one says *what machine we are on*.
const COMPAT_CAP: usize = 96;
static mut COMPAT: [u8; COMPAT_CAP] = [0; COMPAT_CAP];
static mut COMPAT_LEN: usize = 0;
static mut INFO: Info = Info {
    valid: false,
    ram_base: 0,
    ram_size: 0,
    uart_base: 0,
    gic_base: 0,
    gic_cpu_base: 0,
    rtc_base: 0,
    uart_kind: "",
    reference: false,
};

/// Read a big-endian u32 at `base + off`.
unsafe fn be32(base: usize, off: usize) -> u32 {
    let p = (base + off) as *const u8;
    ((*p as u32) << 24) | ((*p.add(1) as u32) << 16) | ((*p.add(2) as u32) << 8) | (*p.add(3) as u32)
}

/// Length of the NUL-terminated string at `base + off` (excluding the NUL).
unsafe fn cstr_len(base: usize, off: usize) -> usize {
    let mut n = 0;
    while *((base + off + n) as *const u8) != 0 {
        n += 1;
    }
    n
}

/// Compare the NUL-terminated string at `base + off` to `s`.
unsafe fn name_eq(base: usize, off: usize, s: &[u8]) -> bool {
    let l = cstr_len(base, off);
    if l != s.len() {
        return false;
    }
    for (i, &c) in s.iter().enumerate() {
        if *((base + off + i) as *const u8) != c {
            return false;
        }
    }
    true
}

/// Does the NUL-terminated string at `base + off` start with `s`?
unsafe fn name_starts(base: usize, off: usize, s: &[u8]) -> bool {
    for (i, &c) in s.iter().enumerate() {
        let ch = *((base + off + i) as *const u8);
        if ch == 0 || ch != c {
            return false;
        }
    }
    true
}

fn align4(n: usize) -> usize {
    (n + 3) & !3
}

/// Does the `len`-byte blob at `base + off` contain the byte sequence `needle`?
/// Used to match against a `compatible` property (a list of NUL-terminated names).
unsafe fn mem_contains(base: usize, off: usize, len: usize, needle: &[u8]) -> bool {
    if needle.is_empty() || needle.len() > len {
        return false;
    }
    let mut i = 0;
    while i + needle.len() <= len {
        let mut ok = true;
        for (j, &c) in needle.iter().enumerate() {
            if *((base + off + i + j) as *const u8) != c {
                ok = false;
                break;
            }
        }
        if ok {
            return true;
        }
        i += 1;
    }
    false
}

/// Parse the live DTB pointer `dtb` (from firmware in x0), caching the result.
pub fn parse(dtb: u64) {
    if dtb == 0 {
        return;
    }
    parse_ptr(dtb as usize, false);
}

/// Parse the bundled reference DTB — proves the parser works when firmware passed
/// no live pointer. Its RAM figure is a fixture and never drives the allocator.
pub fn parse_reference() {
    parse_ptr(REFERENCE_DTB.as_ptr() as usize, true);
}

/// Core walker over an FDT at absolute address `base`.
fn parse_ptr(base: usize, reference: bool) {
    // SAFETY: the DTB lives in identity-mapped memory; we only read it.
    unsafe {
        if be32(base, 0) != FDT_MAGIC {
            return;
        }
        let off_struct = be32(base, 8) as usize;
        let off_strings = be32(base, 12) as usize;

        let mut p = off_struct;
        let mut depth = 0i32;
        let mut addr_cells = 2u32;
        let mut size_cells = 2u32;
        let mut in_memory = false;
        let mut ram_base = 0u64;
        let mut ram_size = 0u64;
        let mut uart_base = 0u64;
        let mut gic_base = 0u64;
        let mut gic_cpu_base = 0u64;
        let mut rtc_base = 0u64;
        let mut uart_kind = "";
        // Per-node peripheral detection (children of root, depth 2). The UART is
        // tracked as a *part name* rather than a flag: two boards can both have "a
        // UART at some address" and still need different drivers.
        let mut cur_uart: &'static str = "";
        let mut cur_gic = false;
        let mut cur_rtc = false;
        let mut cur_reg = 0u64;
        let mut cur_has_reg = false;
        // The second `reg` range of the current node, when it has one. Only the GIC
        // uses it (distributor window, then CPU-interface window).
        let mut cur_reg2 = 0u64;
        let mut cur_has_reg2 = false;

        let mut guard = 0;
        loop {
            guard += 1;
            if guard > 100_000 {
                break; // malformed blob backstop
            }
            let tok = be32(base, p);
            p += 4;
            match tok {
                FDT_BEGIN_NODE => {
                    depth += 1;
                    let name_off = p;
                    let nl = cstr_len(base, name_off);
                    in_memory = depth == 2 && name_starts(base, name_off, b"memory");
                    if depth == 2 {
                        cur_uart = "";
                        cur_gic = false;
                        cur_rtc = false;
                        cur_reg = 0;
                        cur_has_reg = false;
                        cur_reg2 = 0;
                        cur_has_reg2 = false;
                    }
                    p += align4(nl + 1);
                }
                FDT_END_NODE => {
                    if depth == 2 && cur_has_reg {
                        if !cur_uart.is_empty() && uart_base == 0 {
                            uart_base = cur_reg;
                            uart_kind = cur_uart;
                        }
                        if cur_gic && gic_base == 0 {
                            gic_base = cur_reg;
                            // The CPU interface is the node's second range; a node
                            // that has only one leaves this 0 ("not discovered").
                            if cur_has_reg2 {
                                gic_cpu_base = cur_reg2;
                            }
                        }
                        if cur_rtc && rtc_base == 0 {
                            rtc_base = cur_reg;
                        }
                    }
                    depth -= 1;
                    if depth < 2 {
                        in_memory = false;
                    }
                }
                FDT_PROP => {
                    let len = be32(base, p) as usize;
                    p += 4;
                    let nameoff = be32(base, p) as usize;
                    p += 4;
                    let val = p;
                    let pn = off_strings + nameoff;
                    if depth == 1 && name_eq(base, pn, b"#address-cells") && len == 4 {
                        addr_cells = be32(base, val);
                    } else if depth == 1 && name_eq(base, pn, b"#size-cells") && len == 4 {
                        size_cells = be32(base, val);
                    } else if depth == 1 && name_eq(base, pn, b"model") {
                        let raw = if len > 0 { len - 1 } else { 0 };
                        let n = raw.min(MODEL_CAP);
                        let dst = core::ptr::addr_of_mut!(MODEL) as *mut u8;
                        for i in 0..n {
                            *dst.add(i) = *((base + val + i) as *const u8);
                        }
                        MODEL_LEN = n;
                    } else if depth == 1 && name_eq(base, pn, b"compatible") {
                        // A `compatible` is a list of NUL-terminated names. Store it
                        // with the separators turned into commas so it is one
                        // printable string, and so a substring match for one name
                        // still works — the boundaries carry no extra meaning here.
                        let raw = if len > 0 { len - 1 } else { 0 };
                        let n = raw.min(COMPAT_CAP);
                        let dst = core::ptr::addr_of_mut!(COMPAT) as *mut u8;
                        for i in 0..n {
                            let c = *((base + val + i) as *const u8);
                            *dst.add(i) = if c == 0 { b',' } else { c };
                        }
                        COMPAT_LEN = n;
                    } else if in_memory && name_eq(base, pn, b"reg") && len >= 8 {
                        let mut o = val;
                        for _ in 0..addr_cells {
                            ram_base = (ram_base << 32) | be32(base, o) as u64;
                            o += 4;
                        }
                        for _ in 0..size_cells {
                            ram_size = (ram_size << 32) | be32(base, o) as u64;
                            o += 4;
                        }
                    } else if depth == 2 && name_eq(base, pn, b"compatible") {
                        // Name the UART *part* in board.rs's vocabulary. `arm,pl011`
                        // is the ARM PrimeCell; `snps,dw-apb-uart` / `ns16550a` /
                        // `8250` are all the 16550-class part `uart_ns16550.rs`
                        // drives — the distinction the base address cannot carry.
                        if mem_contains(base, val, len, b"pl011") {
                            cur_uart = "PL011";
                        } else if mem_contains(base, val, len, b"dw-apb-uart")
                            || mem_contains(base, val, len, b"ns16550")
                            || mem_contains(base, val, len, b"8250")
                        {
                            cur_uart = "8250dw";
                        }
                        if mem_contains(base, val, len, b"gic") {
                            cur_gic = true;
                        }
                        if mem_contains(base, val, len, b"pl031") {
                            cur_rtc = true;
                        }
                    } else if depth == 2 && name_eq(base, pn, b"reg") && len >= 8 {
                        // First address in the node's reg, using root cell sizes.
                        let mut o = val;
                        let mut a = 0u64;
                        for _ in 0..addr_cells {
                            a = (a << 32) | be32(base, o) as u64;
                            o += 4;
                        }
                        cur_reg = a;
                        cur_has_reg = true;
                        // A `reg` is a list of (address, size) ranges in root cell
                        // sizes. Keep the second one too: the GICv2 node carries its
                        // distributor and CPU-interface windows in exactly that form,
                        // and taking only the first is what left `gic.rs` half on the
                        // compile-time map until milestone 2.727.
                        let range = (addr_cells + size_cells) as usize * 4;
                        if range > 0 && len >= range * 2 {
                            let mut o2 = val + range;
                            let mut b = 0u64;
                            for _ in 0..addr_cells {
                                b = (b << 32) | be32(base, o2) as u64;
                                o2 += 4;
                            }
                            cur_reg2 = b;
                            cur_has_reg2 = true;
                        }
                    }
                    p += align4(len);
                }
                FDT_NOP => {}
                FDT_END => break,
                _ => break,
            }
        }
        INFO = Info {
            valid: true,
            ram_base,
            ram_size,
            uart_base,
            gic_base,
            gic_cpu_base,
            rtc_base,
            uart_kind,
            reference,
        };
    }
}

/// The parsed device-tree facts (valid=false if no/invalid DTB).
pub fn info() -> Info {
    // SAFETY: scalar read of Copy data set once at boot.
    unsafe { INFO }
}

/// The device tree `model` string, or "" if none.
pub fn model() -> &'static str {
    // SAFETY: MODEL/MODEL_LEN are written once at boot; read via raw pointer to
    // avoid forming a reference to a mutable static.
    unsafe {
        let s = core::slice::from_raw_parts(core::ptr::addr_of!(MODEL) as *const u8, MODEL_LEN);
        core::str::from_utf8(s).unwrap_or("")
    }
}

/// The **root** `compatible` list as one comma-separated string, or "" if the tree
/// states none. This is the machine's own name for itself — the only field in the
/// blob that says which board the *rest* of the tree describes, which is what
/// `board machine` (milestone 2.728) checks the compiled-in map against.
pub fn root_compat() -> &'static str {
    // SAFETY: COMPAT/COMPAT_LEN are written once at boot; read via raw pointer to
    // avoid forming a reference to a mutable static.
    unsafe {
        let s = core::slice::from_raw_parts(core::ptr::addr_of!(COMPAT) as *const u8, COMPAT_LEN);
        core::str::from_utf8(s).unwrap_or("")
    }
}

// `ram_end_or()` lived here until 2.731: a raw "tree's RAM end, else this fallback"
// read, and the last route by which a consumer could size itself from the tree
// without passing the rules of `board::resolve_from` (fixture trees, foreign
// machines, impossible claims). Its one caller — the frame allocator — now reads
// `board::resolved().ram_end()`, so the function is gone rather than left standing
// as a shortcut around them. The raw figures are still here in [`info`] for anyone
// reporting what the *tree* said, which is a different question from what we trust.
