//! SD cards in a PC's card reader: roadmap PC step 3 (7 Nov 2026).
//!
//! Most laptops (and many desktops) have an SD card slot. When it sits on PCI as a standard
//! SD Host Controller (class 08:05, "SDHCI": Intel, O2 Micro/BayHub, Ricoh and many others),
//! this driver reads and writes the card in it. Realtek's own PCIe card readers (rtsx,
//! 10ec:52xx) are NOT SDHCI and are only named by `lssd`; USB card readers are already USB
//! storage (xhci.rs).
//!
//! The SD Host Controller Simplified Specification (v3.00): software reset, the card powered
//! at the highest voltage the slot offers, SD clock at <= 400 kHz for identification, then the
//! SD Physical Layer's start-up: CMD0, CMD8 (v2 cards), ACMD41 until powered up (CCS = high
//! capacity: SDHC/SDXC are addressed in 512-byte blocks, standard-capacity SDSC cards in bytes),
//! CMD2 (CID: the card's name), CMD3 (its address), CMD9 (CSD: its size), CMD7 (select), CMD16
//! (512-byte blocks), clock up to <= 25 MHz. One block per command (CMD17/CMD24), moved by
//! the CPU through the buffer data port (no DMA, so no address limits); every wait is polled.
//!
//! eMMC (8 Nov 2026): the built-in disk of most cheap laptops and tablets (Celeron/Atom/Pentium
//! Silver with 32-128 GB soldered on the board) sits on the same kind of controller. When the SD
//! start-up gets no answer to ACMD41, the MMC one is tried, as Linux does: CMD0, CMD1 until
//! powered up (sector or byte addressing from the OCR), CMD2 (CID: a 6-letter name), CMD3 (we
//! give it address 1), CMD9 (CSD), CMD7 (select), CMD8 (EXT_CSD: the real size SEC_COUNT of a
//! sector-addressed device, its eMMC version and boot partitions). The user area is used (never
//! the boot partitions); an eMMC has no write-protect switch, so the slot's pin is ignored, and
//! an embedded slot (capabilities 31:30 = 01) is not waited on for "card inserted". Every SDHCI
//! controller is tried in PCI order until one has a card or eMMC that answers (a laptop's empty
//! SD slot is often found before its eMMC controller).
//!
//! Installing onto it (8 Nov 2026): the eMMC is also a disk for `lsparts`, `installdisk emmc`, `installpart`,
//! `formatpart emmc`, `tablerestore` and `bootplan`/`bootcopy` (gpt.rs, esp.rs): `raw_read`/`table_write` see the
//! whole user area; at boot AfriSovera's partition in a healthy GPT, already formatted, holds the VFS, and an eMMC
//! in use comes before a USB stick (disk.rs), as NVMe and SATA do.
//!
//! Multi-block reads (8 Nov 2026): after the bus is set, the first 8 blocks are read with one CMD18
//! (block count + Auto CMD12) and compared with the same blocks read one at a time; when equal, a
//! read that misses fetches up to 16 blocks from it in one command and the next reads are served
//! from that copy (the VFS reads its sectors in order, so most reads cost no command at all).
//! Writes stay one CMD24 each and update the copy. `lssd` counts the reads.
//!
//! DMA (8 Nov 2026): when the slot offers ADMA2 (capabilities bit 19) and the driver's buffers lie
//! below 4 GiB, the multi-block commands (CMD18, CMD25) are moved by the controller itself: one
//! 32-bit descriptor per command. At start-up the first 8 blocks read by DMA must equal the CPU's
//! copy, and the first DMA write is read back like any first CMD25; a mismatch or an ADMA error
//! turns DMA off and the CPU moves the data as before. Single-block commands stay PIO.
//!
//! First slot of each controller only. The same safety rule as the SATA and NVMe disks: the
//! VFS stores files on the card only if it already carries the VFS marker, or if it is blank
//! and the owner types `claimdisk sd`. A card with a file system (a camera's or phone's card)
//! is listed but never written; nor is a card whose write-protect switch is on.

use crate::mmu;
use core::arch::asm;
use core::ptr::{addr_of, addr_of_mut, read_volatile, write_volatile};

// --- PCI configuration space (mechanism #1, ports 0xCF8/0xCFC) ---
unsafe fn outl(port: u16, v: u32) {
    asm!("out dx, eax", in("dx") port, in("eax") v, options(nomem, nostack, preserves_flags));
}
unsafe fn inl(port: u16) -> u32 {
    let v: u32;
    asm!("in eax, dx", out("eax") v, in("dx") port, options(nomem, nostack, preserves_flags));
    v
}
fn cfg32(b: u8, d: u8, f: u8, off: u8) -> u32 {
    let a = 0x8000_0000 | (b as u32) << 16 | (d as u32) << 11 | (f as u32) << 8 | (off as u32 & 0xFC);
    // SAFETY: the PCI configuration ports; single core with interrupts masked at init.
    unsafe {
        outl(0xCF8, a);
        inl(0xCFC)
    }
}
fn cfg_write32(b: u8, d: u8, f: u8, off: u8, v: u32) {
    let a = 0x8000_0000 | (b as u32) << 16 | (d as u32) << 11 | (f as u32) << 8 | (off as u32 & 0xFC);
    // SAFETY: as cfg32.
    unsafe {
        outl(0xCF8, a);
        outl(0xCFC, v);
    }
}

// --- SDHCI registers (Simplified Spec v3.00, section 2) ---
const BLKSIZE: u64 = 0x04; // u16
const BLKCNT: u64 = 0x06; // u16
const ARG: u64 = 0x08;
const XFERMODE: u64 = 0x0C; // u16
const CMD: u64 = 0x0E; // u16
const RESP: u64 = 0x10; // 4 x u32
const DATA: u64 = 0x20;
const PRESENT: u64 = 0x24;
const HOSTCTL: u64 = 0x28; // u8
const POWER: u64 = 0x29; // u8
const CLOCK: u64 = 0x2C; // u16
const TIMEOUT: u64 = 0x2E; // u8
const RESET: u64 = 0x2F; // u8
const NISTS: u64 = 0x30; // u16
const EISTS: u64 = 0x32; // u16
const NISTSEN: u64 = 0x34; // u16
const EISTSEN: u64 = 0x36; // u16
const NISIGEN: u64 = 0x38; // u16
const EISIGEN: u64 = 0x3A; // u16
const CAPS: u64 = 0x40;
const ADMA_ADDR: u64 = 0x58; // ADMA system address, 2 x u32
const VERSION: u64 = 0xFE; // u16

const PS_CMD_INHIBIT: u32 = 1 << 0;
const PS_DAT_INHIBIT: u32 = 1 << 1;
const PS_CARD_IN: u32 = 1 << 16;
const PS_WRITE_ENABLED: u32 = 1 << 19; // the card's write-protect switch is OFF
const NI_CMD_DONE: u16 = 1 << 0;
const NI_XFER_DONE: u16 = 1 << 1;
const NI_BUF_WRITE: u16 = 1 << 4;
const NI_BUF_READ: u16 = 1 << 5;
const NI_ERROR: u16 = 1 << 15;
const RESET_ALL: u8 = 1 << 0;
const RESET_CMD: u8 = 1 << 1;
const RESET_DAT: u8 = 1 << 2;

// command register: response type 1:0, CRC check bit 3, index check bit 4, data present bit 5
const R_NONE: u16 = 0x00;
const R_136: u16 = 0x01 | 1 << 3; // R2
const R_48: u16 = 0x02 | 1 << 3 | 1 << 4; // R1, R6, R7
const R_48B: u16 = 0x03 | 1 << 3 | 1 << 4; // R1b
const R_48_NOCHK: u16 = 0x02; // R3 (OCR): no CRC, no index
const DATA_PRESENT: u16 = 1 << 5;

static mut BASE: u64 = 0;
static mut PCI_AT: (u8, u8, u8) = (0, 0, 0);
static mut PCI_ID: u32 = 0;
static mut SPEC: u16 = 0;
static mut BASE_MHZ: u32 = 0;
/// The card, once it answered: (sectors, high capacity, RCA, write enabled).
static mut CARD: Option<(u64, bool, u32, bool)> = None;
static mut NAME: [u8; 6] = [b' '; 6];
/// The device is an eMMC (MMC start-up): its EXT_CSD revision and boot partition size (KiB).
static mut MMC: Option<(u8, u32)> = None;
/// Probing for an SD card: an eMMC's silence to ACMD41 is expected, not reported.
static mut QUIET: bool = false;
/// SDHCI controllers found on PCI (each tried in turn).
static mut NCTL: u32 = 0;
/// What sector 0 says: 0 = unread, 1 = ours (AFRIVFS1), 2 = blank, 3 = other data.
static mut KIND: u8 = 0;
/// The card holds the VFS (ours at boot, or claimed).
static mut USED: bool = false;
/// AfriSovera's partition on an eMMC (installdisk / formatpart): (first sector, sectors), (0, 0) for none.
static mut PART: (u64, u64) = (0, 0);
/// A Realtek PCIe card reader (not SDHCI), named by `lssd` only.
static mut RTSX: Option<(u8, u8, u8, u16)> = None;
/// The data bus after start-up: (width 1/4/8, SD clock in kHz, high speed), and the start-up clock.
static mut BUS: (u8, u32, bool) = (1, 0, false);
static mut BUS0: u32 = 0;
/// Multi-block reads (CMD18) passed the start-up check.
static mut MULTI: bool = false;
/// Read-ahead: the blocks of the last CMD18, from block RA_AT, RA_N of them valid.
const RA_MAX: usize = 16;
static mut RA: [u8; RA_MAX * 512] = [0; RA_MAX * 512];
static mut RA_AT: u64 = 0;
static mut RA_N: u32 = 0;
/// Block reads asked for, CMD18 commands, CMD17 commands, reads served from the read-ahead.
static mut STATS: (u64, u64, u64, u64) = (0, 0, 0, 0);
/// Write-behind: the VFS's consecutive writes (its sync writes sectors 0..=2*MAX_FILES in order)
/// gathered here, from card block WB_AT, WB_N of them, and sent by one CMD25 when the run ends
/// (the last VFS sector, a gap, RA_MAX blocks) or before any other command touches the card.
static mut WB: [u8; RA_MAX * 512] = [0; RA_MAX * 512];
static mut WB_AT: u64 = 0;
static mut WB_N: u32 = 0;
/// Multi-block writes (CMD25): 0 = not tried yet, 1 = on (the first one read back equal), 2 = off.
static mut MULTIW: u8 = 0;
/// Blocks written, by CMD25, CMD25 commands, by single CMD24.
static mut WSTATS: (u64, u64, u64, u64) = (0, 0, 0, 0);
/// ADMA2 for the multi-block commands: 0 = not checked, 1 = on, 2 = off (the CPU moves the data).
static mut DMA: u8 = 0;
/// DMA commands, blocks moved by them.
static mut DSTATS: (u64, u64) = (0, 0);
// --- DMA structures: 1:1-mapped kernel memory, so their addresses are physical (as ahci.rs) ---
#[repr(C, align(4096))]
struct DmaBuf([u8; RA_MAX * 512]);
static mut DBUF: DmaBuf = DmaBuf([0; RA_MAX * 512]);
/// One 32-bit ADMA2 descriptor: attributes + length, then the data address.
#[repr(C, align(8))]
struct Desc([u32; 2]);
static mut DESC: Desc = Desc([0; 2]);

fn r32(off: u64) -> u32 {
    // SAFETY: BASE is mapped uncached by init before any caller runs.
    unsafe { read_volatile((BASE + off) as *const u32) }
}
fn w32(off: u64, v: u32) {
    // SAFETY: as r32.
    unsafe { write_volatile((BASE + off) as *mut u32, v) }
}
fn r16(off: u64) -> u16 {
    // SAFETY: as r32.
    unsafe { read_volatile((BASE + off) as *const u16) }
}
fn w16(off: u64, v: u16) {
    // SAFETY: as r32.
    unsafe { write_volatile((BASE + off) as *mut u16, v) }
}
fn r8(off: u64) -> u8 {
    // SAFETY: as r32.
    unsafe { read_volatile((BASE + off) as *const u8) }
}
fn w8(off: u64, v: u8) {
    // SAFETY: as r32.
    unsafe { write_volatile((BASE + off) as *mut u8, v) }
}

/// Poll `done` for up to `ms` milliseconds.
fn wait_ms(ms: u64, mut done: impl FnMut() -> bool) -> bool {
    let (t0, hz) = (crate::timer::cycles(), crate::timer::hz().max(1));
    loop {
        if done() {
            return true;
        }
        if crate::timer::cycles() - t0 > hz / 1000 * ms {
            return done();
        }
        core::hint::spin_loop();
    }
}

fn reset(what: u8) -> bool {
    w8(RESET, what);
    wait_ms(100, || r8(RESET) & what == 0)
}

/// One SD command. Returns the response registers (bits 127:8 of an R2 in RESP[119:0], or a
/// 48-bit response's 32-bit card status in word 0), or None on a timeout or error.
fn command(index: u8, arg: u32, flags: u16, xfer: u16) -> Option<[u32; 4]> {
    let busy = flags & DATA_PRESENT != 0 || flags & 0x3 == 0x3;
    let inhibit = PS_CMD_INHIBIT | if busy { PS_DAT_INHIBIT } else { 0 };
    if !wait_ms(500, || r32(PRESENT) & inhibit == 0) {
        crate::kprintln!("[sdhci] CMD{} not sent: the controller stayed busy (present state {:#x})", index, r32(PRESENT));
        reset(RESET_CMD | RESET_DAT);
        return None;
    }
    w16(NISTS, 0xFFFF);
    w16(EISTS, 0xFFFF);
    w32(ARG, arg);
    w16(XFERMODE, xfer);
    w16(CMD, (index as u16) << 8 | flags);
    let ok = wait_ms(500, || r16(NISTS) & (NI_CMD_DONE | NI_ERROR) != 0);
    let (ni, ei) = (r16(NISTS), r16(EISTS));
    if !ok || ni & NI_ERROR != 0 {
        w16(NISTS, 0xFFFF);
        w16(EISTS, 0xFFFF);
        reset(RESET_CMD | RESET_DAT);
        // CMD8 is allowed to go unanswered (a v1.x card); every other failure is reported
        if index != 8 && !unsafe { QUIET } {
            crate::kprintln!("[sdhci] CMD{} arg {:#x} failed (status {:#x}, error {:#x})", index, arg, ni, ei);
        }
        return None;
    }
    w16(NISTS, NI_CMD_DONE);
    Some([r32(RESP), r32(RESP + 4), r32(RESP + 8), r32(RESP + 12)])
}

fn app_command(index: u8, arg: u32, flags: u16, rca: u32) -> Option<[u32; 4]> {
    command(55, rca << 16, R_48, 0)?;
    command(index, arg, flags, 0)
}

/// The SD clock at no more than `khz`: the divisor from the base clock (capabilities 15:8 on
/// v3.00 controllers, 13:8 before), a 10-bit "divided clock" on v3.00, a power of two before.
/// Returns the clock the card now gets in kHz, 0 when the controller's clock never became stable.
fn set_clock(khz: u32) -> u32 {
    // SAFETY: scalar reads of boot-time state.
    let (spec, base) = unsafe { (SPEC, BASE_MHZ) };
    let base_khz = if base == 0 { 50_000 } else { base * 1000 }; // unknown: assume 50 MHz (no faster than asked)
    let (sel, got): (u16, u32) = if spec >= 2 {
        // SDCLK = base / (2N), N = 1..1023 (0 = the base clock itself)
        let n = if base_khz <= khz { 0 } else { base_khz.div_ceil(2 * khz).min(1023) } as u16;
        ((n & 0xFF) << 8 | (n >> 8 & 0x3) << 6, if n == 0 { base_khz } else { base_khz / (2 * n as u32) })
    } else {
        // SDCLK = base / div, div a power of two 1..256, written as div / 2
        let mut div = 1u32;
        while div < 256 && base_khz / div > khz {
            div *= 2;
        }
        (((div / 2) as u16) << 8, base_khz / div)
    };
    w16(CLOCK, 0);
    w16(CLOCK, sel | 1); // internal clock on
    if !wait_ms(150, || r16(CLOCK) & 2 != 0) {
        crate::kprintln!("[sdhci] the internal clock never became stable");
        return 0;
    }
    w16(CLOCK, sel | 1 | 4); // SD clock to the card
    crate::timer::busywait_ms(1);
    got
}

/// Find the controller and bring up the card in its first slot.
pub fn init() {
    let mut found = [(0u8, 0u8, 0u8, 0u32); 4];
    let mut n = 0;
    for b in 0..=255u8 {
        for d in 0..32u8 {
            for f in 0..8u8 {
                let id = cfg32(b, d, f, 0);
                if id & 0xFFFF == 0xFFFF {
                    if f == 0 {
                        break;
                    }
                    continue;
                }
                if crate::hwdb::pci_part(BINDS, b, d, f, id).is_some() && n < found.len() {
                    found[n] = (b, d, f, id);
                    n += 1;
                }
                // Realtek PCIe card readers (RTS5209/5227/5228/5229/522A/5249/524A/525A/5260/5261/5287): their own interface
                let dev = (id >> 16) as u16;
                if id & 0xFFFF == 0x10EC && matches!(dev, 0x5209 | 0x5227 | 0x5228 | 0x5229 | 0x522A | 0x5249 | 0x524A | 0x525A | 0x5260 | 0x5261 | 0x5287) {
                    // SAFETY: boot-time state, single core.
                    unsafe { RTSX = Some((b, d, f, dev)) };
                }
                if f == 0 && cfg32(b, d, f, 0x0C) >> 16 & 0x80 == 0 {
                    break; // not a multi-function device
                }
            }
        }
    }
    // SAFETY: boot-time state, single core.
    unsafe { NCTL = n as u32 };
    for &(b, d, f, id) in &found[..n] {
        if init_ctl(b, d, f, id) {
            return;
        }
    }
}

/// Bring up one controller and the card or eMMC in its first slot; true when it answered.
fn init_ctl(b: u8, d: u8, f: u8, id: u32) -> bool {
    // slot information (PCI config 0x40): the first slot's BAR
    let first = (cfg32(b, d, f, 0x40) & 7) as u8;
    let slots = (cfg32(b, d, f, 0x40) >> 4 & 7) + 1;
    let off = 0x10 + 4 * first.min(5);
    let lo = cfg32(b, d, f, off);
    if lo & 1 != 0 {
        crate::kprintln!("[sdhci] controller at {:02x}:{:02x}.{} has an I/O BAR, not memory: not used", b, d, f);
        return false;
    }
    let mut bar = (lo & !0xF) as u64;
    if lo >> 1 & 3 == 2 && off < 0x24 {
        bar |= (cfg32(b, d, f, off + 4) as u64) << 32;
    }
    if bar == 0 || !mmu::map_mmio(bar, 0x100) {
        crate::kprintln!("[sdhci] controller at {:02x}:{:02x}.{} has no usable register BAR", b, d, f);
        return false;
    }
    cfg_write32(b, d, f, 4, cfg32(b, d, f, 4) | 0x406); // memory decoding + bus mastering, INTx off (polled)
    // SAFETY: init runs once at boot, single core, before any reader.
    unsafe {
        BASE = bar;
        PCI_AT = (b, d, f);
        PCI_ID = id;
        SPEC = r16(VERSION) & 0xFF;
    }
    if !reset(RESET_ALL) {
        crate::kprintln!("[sdhci] controller {:02x}:{:02x}.{}: the reset never finished", b, d, f);
        return false;
    }
    let caps = r32(CAPS);
    // SAFETY: as above.
    unsafe {
        BASE_MHZ = if SPEC >= 2 { caps >> 8 & 0xFF } else { caps >> 8 & 0x3F };
    }
    w16(NISTSEN, 0x00FF & !(1 << 8)); // every status but the card interrupt, seen by polling
    w16(EISTSEN, 0x03FF);
    w16(NISIGEN, 0); // no interrupts
    w16(EISIGEN, 0);
    w8(TIMEOUT, 0x0E);
    crate::kprintln!(
        "[sdhci] controller {:02x}:{:02x}.{} ({:04x}:{:04x}, SDHCI {}.00, base clock {} MHz, {} slot(s))",
        b, d, f, id & 0xFFFF, id >> 16, unsafe { SPEC } + 1, unsafe { BASE_MHZ }, slots
    );
    // an embedded slot (an eMMC soldered on the board, SDHCI 3.00 slot type 01) has nothing to insert
    let embedded = unsafe { SPEC } >= 2 && caps >> 30 & 3 == 1;
    if !embedded && !wait_ms(100, || r32(PRESENT) & PS_CARD_IN != 0) {
        crate::kprintln!("[sdhci] no card in the slot");
        return false;
    }
    // power: the highest voltage the slot supports (3.3 V, 3.0 V, then 1.8 V)
    let volt: u8 = if caps & 1 << 24 != 0 {
        0b111
    } else if caps & 1 << 25 != 0 {
        0b110
    } else {
        0b101
    };
    w8(POWER, volt << 1);
    w8(POWER, volt << 1 | 1);
    crate::timer::busywait_ms(10);
    w8(HOSTCTL, 0); // 1-bit bus, normal speed, no DMA
    if set_clock(400) == 0 {
        return false;
    }
    crate::timer::busywait_ms(2); // >= 74 clocks before the first command
    if !card_start() {
        crate::kprintln!("[sdhci] the card in the slot did not answer: not used");
        // SAFETY: as above.
        unsafe { CARD = None };
        return false;
    }
    true
}

/// The SD start-up sequence; true when the card is selected and ready for block commands.
fn card_start() -> bool {
    if command(0, 0, R_NONE, 0).is_none() {
        return false;
    }
    // CMD8: voltage 2.7-3.6 V, check pattern 0xAA; a v1.x card stays silent
    let v2 = command(8, 0x1AA, R_48, 0).is_some_and(|r| r[0] & 0xFFF == 0x1AA);
    // ACMD41 until the card says it is powered up (busy bit 31 set), at most a second
    let want = 0x00FF_8000 | if v2 { 1 << 30 } else { 0 }; // 2.7-3.6 V window, HCS when v2
    let mut ocr = 0;
    let mut answered = false;
    let (t0, hz) = (crate::timer::cycles(), crate::timer::hz().max(1));
    // SAFETY: init, single core.
    unsafe { QUIET = true };
    while crate::timer::cycles() - t0 < hz {
        match app_command(41, want, R_48_NOCHK, 0) {
            Some(r) if r[0] & 1 << 31 != 0 => {
                ocr = r[0];
                break;
            }
            Some(_) => {
                answered = true;
                crate::timer::busywait_ms(10);
            }
            None => break,
        }
    }
    // SAFETY: as above.
    unsafe { QUIET = false };
    if ocr == 0 && !answered {
        // no SD answer at all: an eMMC (or an MMC card) answers CMD1 instead
        return mmc_start();
    }
    if ocr == 0 {
        crate::kprintln!("[sdhci] the card never finished powering up (ACMD41)");
        return false;
    }
    let high = ocr & 1 << 30 != 0;
    let Some(cid) = command(2, 0, R_136, 0) else { return false };
    // CID[103:64] (the product name, 5 ASCII bytes) sits at RESP bits 95:56
    let cid128 = cid[0] as u128 | (cid[1] as u128) << 32 | (cid[2] as u128) << 64 | (cid[3] as u128) << 96;
    let mut name = [b' '; 6];
    for (i, c) in name.iter_mut().take(5).enumerate() {
        let ch = (cid128 >> (88 - 8 * i)) as u8;
        *c = if ch.is_ascii_graphic() || ch == b' ' { ch } else { b'?' };
    }
    let Some(r6) = command(3, 0, R_48, 0) else { return false };
    let rca = r6[0] >> 16;
    let Some(csd) = command(9, rca << 16, R_136, 0) else { return false };
    let c = csd[0] as u128 | (csd[1] as u128) << 32 | (csd[2] as u128) << 64 | (csd[3] as u128) << 96;
    // CSD bit n is at RESP bit n - 8
    let field = |hi: u32, lo: u32| -> u64 { ((c >> (lo - 8)) & ((1u128 << (hi - lo + 1)) - 1)) as u64 };
    let sectors = match field(127, 126) {
        0 => {
            // CSD v1.0 (SDSC): (C_SIZE + 1) * 2^(C_SIZE_MULT + 2) * 2^READ_BL_LEN bytes
            let bytes = (field(73, 62) + 1) << (field(49, 47) + 2) << field(83, 80);
            bytes / 512
        }
        1 => (field(69, 48) + 1) * 1024, // CSD v2.0 (SDHC/SDXC): (C_SIZE + 1) * 512 KiB
        2 => (field(75, 48) + 1) * 1024, // CSD v3.0 (SDUC)
        _ => 0,
    };
    if sectors == 0 {
        crate::kprintln!("[sdhci] the card's size (CSD) could not be read");
        return false;
    }
    if command(7, rca << 16, R_48B, 0).is_none() {
        return false;
    }
    if !wait_ms(500, || r32(PRESENT) & PS_DAT_INHIBIT == 0) {
        return false;
    }
    if !high && command(16, 512, R_48, 0).is_none() {
        return false;
    }
    // data transfer: the 4-bit bus and high speed when the card and slot take them (speed_up)
    speed_up(rca, None);
    let wp_off = r32(PRESENT) & PS_WRITE_ENABLED != 0;
    finish(sectors, high, rca, wp_off, name);
    true
}

/// The MMC start-up (an eMMC); true when it is selected and ready for block commands.
fn mmc_start() -> bool {
    if command(0, 0, R_NONE, 0).is_none() {
        return false;
    }
    crate::timer::busywait_ms(1);
    // CMD1 until powered up (busy bit 31 set), at most a second: 2.7-3.6 V, sector mode offered
    let mut ocr = 0;
    let (t0, hz) = (crate::timer::cycles(), crate::timer::hz().max(1));
    while crate::timer::cycles() - t0 < hz {
        match command(1, 0x40FF_8000, R_48_NOCHK, 0) {
            Some(r) if r[0] & 1 << 31 != 0 => {
                ocr = r[0];
                break;
            }
            Some(_) => {
                crate::timer::busywait_ms(10);
            }
            None => return false,
        }
    }
    if ocr == 0 {
        crate::kprintln!("[sdhci] the eMMC never finished powering up (CMD1)");
        return false;
    }
    let high = ocr >> 29 & 3 == 2; // access mode: sector (10) or byte (00)
    let Some(cid) = command(2, 0, R_136, 0) else { return false };
    // CID[103:56] (the product name, 6 ASCII bytes) sits at RESP bits 95:48
    let cid128 = cid[0] as u128 | (cid[1] as u128) << 32 | (cid[2] as u128) << 64 | (cid[3] as u128) << 96;
    let mut name = [b' '; 6];
    for (i, c) in name.iter_mut().enumerate() {
        let ch = (cid128 >> (88 - 8 * i)) as u8;
        *c = if ch.is_ascii_graphic() || ch == b' ' { ch } else { b'?' };
    }
    let rca = 1; // on MMC the host gives the address
    if command(3, rca << 16, R_48, 0).is_none() {
        return false;
    }
    let Some(csd) = command(9, rca << 16, R_136, 0) else { return false };
    let c = csd[0] as u128 | (csd[1] as u128) << 32 | (csd[2] as u128) << 64 | (csd[3] as u128) << 96;
    let field = |hi: u32, lo: u32| -> u64 { ((c >> (lo - 8)) & ((1u128 << (hi - lo + 1)) - 1)) as u64 };
    if command(7, rca << 16, R_48B, 0).is_none() || !wait_ms(500, || r32(PRESENT) & PS_DAT_INHIBIT == 0) {
        return false;
    }
    set_clock(25_000); // legacy MMC speed: at most 26 MHz
    // EXT_CSD (CMD8, one 512-byte block): SEC_COUNT [215:212], EXT_CSD_REV [192], BOOT_SIZE_MULT [226]
    let mut ext = [0u8; 512];
    let have_ext = transfer(8, 0, &mut ext, false);
    let sectors = if high {
        if !have_ext {
            crate::kprintln!("[sdhci] the eMMC's EXT_CSD (its size) could not be read");
            return false;
        }
        u32::from_le_bytes([ext[212], ext[213], ext[214], ext[215]]) as u64
    } else {
        // byte-addressed (2 GiB or less): the CSD's size, as an SDSC card's
        ((field(73, 62) + 1) << (field(49, 47) + 2) << field(83, 80)) / 512
    };
    if sectors == 0 {
        crate::kprintln!("[sdhci] the eMMC's size could not be read");
        return false;
    }
    if !high && command(16, 512, R_48, 0).is_none() {
        return false;
    }
    if have_ext {
        speed_up(rca, Some(&ext)); // the 8- or 4-bit bus and 52 MHz high speed when offered
    } else {
        // SAFETY: init, single core.
        unsafe { BUS = (1, set_clock(25_000), false) };
    }
    let (rev, boot_kib) = if have_ext { (ext[192], ext[226] as u32 * 128) } else { (0xFF, 0) };
    // SAFETY: init, single core.
    unsafe { MMC = Some((rev, boot_kib)) };
    finish(sectors, high, rca, true, name); // no write-protect switch on a soldered chip
    true
}

/// The eMMC version from EXT_CSD_REV.
fn mmc_version(rev: u8) -> &'static str {
    match rev {
        0 => "4.0",
        1 => "4.1",
        2 => "4.2",
        3 => "4.3",
        5 => "4.41",
        6 => "4.5",
        7 => "5.0",
        8 => "5.1",
        0xFF => "?",
        _ => "5.1+",
    }
}

/// After start-up the card runs at 1 bit and <= 25 MHz: slow (about 1 MB/s at best), and an
/// eMMC-only laptop keeps its whole system there. As Linux does after start-up: SD cards (all
/// of which have 4 data lines) get the 4-bit bus (ACMD6) and, when the slot can do high speed
/// (capabilities bit 21), CMD6's high-speed function (50 MHz, its 64-byte status checked); an
/// eMMC gets the 8-bit bus when the slot is wired for it (SDHCI 3.00, capabilities bit 18), else
/// 4-bit, and HS_TIMING (52 MHz) when its EXT_CSD card type offers it, each a CMD6 SWITCH
/// checked by CMD13. A block read before the change is read again after it (sector 0 of an SD
/// card; an eMMC's read-only EXT_CSD fields): if it differs or cannot be read, card and slot go
/// back to the 1-bit default bus, which worked. One boot line says what the bus became.
fn speed_up(rca: u32, ext: Option<&[u8; 512]>) {
    let clk0 = set_clock(25_000);
    // SAFETY: init, single core.
    unsafe { (BUS, BUS0) = ((1, clk0, false), clk0) };
    let mut before = [0u8; 512];
    let reference = match ext {
        Some(e) => {
            before = *e;
            true
        }
        None => transfer(17, 0, &mut before, false),
    };
    if clk0 == 0 || !reference {
        return;
    }
    let caps = r32(CAPS);
    // SAFETY: scalar read.
    let spec = unsafe { SPEC };
    let hs_slot = caps & 1 << 21 != 0;
    // optional steps: a card that does not take one is not an error, so it is not reported
    // SAFETY: init, single core.
    unsafe { QUIET = true };
    let mut width = 1u8;
    let mut hs = false;
    match ext {
        None => {
            if app_command(6, 2, R_48, rca).is_some() {
                width = 4;
                w8(HOSTCTL, r8(HOSTCTL) | 0x02);
            }
            if hs_slot {
                // CMD6 mode 1 (switch), group 1 function 1 (high speed), the others unchanged
                let mut st = [0u8; 64];
                if transfer(6, 0x80FF_FFF1, &mut st, false) && st[16] & 0xF == 1 {
                    hs = true;
                }
            }
        }
        Some(e) => {
            let switch = |index: u32, value: u32| -> bool {
                command(6, 3 << 24 | index << 16 | value << 8, R_48B, 0).is_some()
                    && wait_ms(1000, || r32(PRESENT) & PS_DAT_INHIBIT == 0)
                    && command(13, rca << 16, R_48, 0).is_some_and(|r| r[0] & 1 << 7 == 0) // no SWITCH_ERROR
            };
            if e[192] >= 2 || e[196] != 0 {
                if spec >= 2 && caps & 1 << 18 != 0 && switch(183, 2) {
                    width = 8;
                    w8(HOSTCTL, r8(HOSTCTL) & !0x02 | 0x20);
                } else if switch(183, 1) {
                    width = 4;
                    w8(HOSTCTL, r8(HOSTCTL) | 0x02);
                }
                if hs_slot && e[196] & 0x02 != 0 && switch(185, 1) {
                    hs = true;
                }
            }
        }
    }
    let mut clk = clk0;
    if hs {
        w8(HOSTCTL, r8(HOSTCTL) | 0x04);
        clk = set_clock(if ext.is_some() { 52_000 } else { 50_000 });
    }
    // read the reference block again on the new bus
    let mut after = [0u8; 512];
    let same = clk != 0
        && match ext {
            None => transfer(17, 0, &mut after, false) && after == before,
            Some(e) => {
                transfer(8, 0, &mut after, false)
                    && after[212..216] == e[212..216]
                    && after[192] == e[192]
                    && after[196] == e[196]
                    && (!hs || after[185] == 1)
            }
        };
    // SAFETY: init, single core.
    unsafe { QUIET = false };
    if width == 1 && !hs {
        crate::kprintln!("[sdhci] bus: 1-bit, default speed, SD clock {} kHz (the card took no faster bus)", clk0);
        return;
    }
    if !same {
        // back to what worked: card first (commands use the CMD line only), then the slot
        reset(RESET_CMD | RESET_DAT);
        // SAFETY: init, single core.
        unsafe { QUIET = true };
        match ext {
            None => {
                let mut st = [0u8; 64];
                w8(HOSTCTL, 0);
                set_clock(25_000);
                if hs {
                    transfer(6, 0x80FF_FFF0, &mut st, false);
                }
                app_command(6, 0, R_48, rca);
            }
            Some(_) => {
                command(6, 3 << 24 | 185 << 16, R_48B, 0);
                command(6, 3 << 24 | 183 << 16, R_48B, 0);
                wait_ms(1000, || r32(PRESENT) & PS_DAT_INHIBIT == 0);
                w8(HOSTCTL, 0);
            }
        }
        // SAFETY: init, single core.
        unsafe { QUIET = false };
        let k = set_clock(25_000);
        // SAFETY: init, single core.
        unsafe { BUS = (1, k, false) };
        crate::kprintln!(
            "[sdhci] bus: {}-bit{} read back differently: back to 1-bit, default speed, SD clock {} kHz",
            width,
            if hs { " high speed" } else { "" },
            k
        );
        return;
    }
    // SAFETY: init, single core.
    unsafe { BUS = (width, clk, hs) };
    crate::kprintln!(
        "[sdhci] bus: {}-bit, {}, SD clock {} kHz (start-up: 1-bit, {} kHz){}",
        width,
        if hs { "high speed" } else { "default speed" },
        clk,
        clk0,
        match ext {
            Some(_) if hs => ", EXT_CSD HS_TIMING 1",
            _ => "",
        }
    );
}

/// Record the card, then look at sector 0: only a blank one or our own is ever written.
fn finish(sectors: u64, high: bool, rca: u32, wp_off: bool, name: [u8; 6]) {
    // SAFETY: init, single core.
    unsafe {
        CARD = Some((sectors, high, rca, wp_off));
        NAME = name;
    }
    multi_check(sectors);
    // Whose card is it? Only a blank one or our own is ever written.
    let mut s0 = [0u8; 512];
    let read = block(0, &mut s0, false);
    let kind = if !read {
        0
    } else if &s0[..8] == b"AFRIVFS1" {
        1
    } else if s0.iter().all(|&x| x == 0) {
        2
    } else {
        3
    };
    // SAFETY: as above.
    unsafe {
        KIND = kind;
        USED = kind == 1 && wp_off;
    }
    // SAFETY: scalar read.
    let mmc = unsafe { MMC };
    // an eMMC that installdisk gave a GPT: AfriSovera's partition in it, already formatted
    let mut part = None;
    if mmc.is_some() && kind == 3 {
        if let Some((a, l)) = crate::gpt::ours_emmc() {
            let mut p0 = [0u8; 512];
            let ok = block(a, &mut p0, false) && &p0[..8] == b"AFRIVFS1";
            if ok {
                // SAFETY: init, single core.
                unsafe {
                    PART = (a, l);
                    USED = true;
                }
            }
            part = Some((a, l, ok));
        }
    }
    let name = core::str::from_utf8(&name).unwrap_or("?").trim();
    let what = match kind {
        1 if wp_off => "ours (AFRIVFS1): used for storage",
        1 => "ours (AFRIVFS1) but write-protected: not used",
        2 if mmc.is_some() => "blank: not used unless claimed (claimdisk emmc)",
        2 => "blank: not used unless claimed (claimdisk sd)",
        3 if part.is_some_and(|p| p.2) => "holds a GPT with AfriSovera's partition: used for storage",
        3 if part.is_some() => "holds a GPT with AfriSovera's partition (not formatted yet): nothing else is touched",
        3 if mmc.is_some() => "holds other data (Windows or another system?): left alone",
        3 => "holds other data (a camera or phone card?): left alone",
        _ => "sector 0 could not be read: not used",
    };
    match mmc {
        Some((rev, boot)) => crate::kprintln!(
            "[sdhci] eMMC \"{}\" (built in): eMMC {}, {}-addressed, {} sectors ({} MiB), {}, {}",
            name,
            mmc_version(rev),
            if high { "sector" } else { "byte" },
            sectors,
            sectors / 2048,
            if boot == 0 { alloc::format!("no boot partitions") } else { alloc::format!("boot partitions 2 x {} KiB (untouched)", boot) },
            what
        ),
        None => crate::kprintln!(
            "[sdhci] card \"{}\": {}, {} sectors ({} MiB){}, {}",
            name,
            if high { "SDHC/SDXC" } else { "SDSC" },
            sectors,
            sectors / 2048,
            if wp_off { "" } else { ", write-protect switch ON" },
            what
        ),
    }
    match part {
        Some((a, l, true)) => crate::kprintln!("[sdhci] eMMC: AfriSovera's partition at sector {} ({} MiB) holds our files", a, l / 2048),
        Some((a, l, false)) => crate::kprintln!("[sdhci] eMMC: AfriSovera's partition at sector {} ({} MiB) is not formatted yet (formatpart emmc)", a, l / 2048),
        None => {}
    }
}

/// One 512-byte block: CMD24 (write), or a read served from the last multi-block read when it
/// holds the block, else a new multi-block read (CMD18) of up to RA_MAX blocks from it (when
/// the boot check passed), else CMD17. A write updates the read-ahead copy of its block.
fn block(n: u64, buf: &mut [u8; 512], write: bool) -> bool {
    // SAFETY: scalar read.
    let Some((sectors, high, _, _)) = (unsafe { CARD }) else { return false };
    if n >= sectors {
        return false;
    }
    // writes still gathered go to the card before anything else does
    flush();
    let arg = if high { n } else { n * 512 } as u32;
    // SAFETY: shell/boot context, single core: the read-ahead buffer and counters.
    unsafe {
        let hit = n >= RA_AT && n < RA_AT + RA_N as u64;
        let i = (n - if hit { RA_AT } else { 0 }) as usize * 512;
        if write {
            let ok = transfer(24, arg, buf, true);
            WSTATS.0 += 1;
            WSTATS.3 += 1;
            if hit {
                if ok {
                    (&mut *addr_of_mut!(RA))[i..i + 512].copy_from_slice(buf);
                } else {
                    RA_N = 0; // the card's copy is unknown now
                }
            }
            return ok;
        }
        STATS.0 += 1;
        if hit {
            buf.copy_from_slice(&(&*addr_of!(RA))[i..i + 512]);
            STATS.3 += 1;
            return true;
        }
        if MULTI {
            let k = RA_MAX.min((sectors - n) as usize);
            if k > 1 {
                RA_N = 0;
                if read_multi(arg, k) {
                    (RA_AT, RA_N) = (n, k as u32);
                    buf.copy_from_slice(&(&*addr_of!(RA))[..512]);
                    STATS.1 += 1;
                    return true;
                }
                // a card that took CMD18 at boot and now refuses it: single blocks from here on
                MULTI = false;
                crate::kprintln!("[sdhci] CMD18 at block {} failed: single-block reads from now on", n);
            }
        }
        STATS.2 += 1;
    }
    transfer(17, arg, buf, false)
}

/// CMD18: k blocks into RA in one command, the controller sending CMD12 when the count is reached
/// (block count enable + Auto CMD12, as Linux's sdhci does without SDMA/ADMA).
fn read_multi(arg: u32, k: usize) -> bool {
    // SAFETY: scalar read.
    if unsafe { DMA } == 1 {
        if adma(18, arg, k) {
            // SAFETY: shell/boot context, single core; the DMA has finished (transfer complete).
            unsafe { (&mut *addr_of_mut!(RA))[..k * 512].copy_from_slice(&(&(*addr_of!(DBUF)).0)[..k * 512]) };
            return true;
        }
        dma_off("an ADMA2 read failed");
    }
    w16(BLKSIZE, 512);
    w16(BLKCNT, k as u16);
    // block count enable (1), Auto CMD12 (2), read (4), multi-block (5); no DMA
    if command(18, arg, R_48 | DATA_PRESENT, 1 << 1 | 1 << 2 | 1 << 4 | 1 << 5).is_none() {
        return false;
    }
    // SAFETY: shell/boot context, single core.
    let ra = unsafe { &mut *addr_of_mut!(RA) };
    for b in 0..k {
        if !wait_ms(1000, || r16(NISTS) & (NI_BUF_READ | NI_ERROR) != 0) || r16(NISTS) & NI_ERROR != 0 {
            reset(RESET_CMD | RESET_DAT);
            return false;
        }
        w16(NISTS, NI_BUF_READ);
        for w in 0..128 {
            let o = b * 512 + w * 4;
            ra[o..o + 4].copy_from_slice(&r32(DATA).to_le_bytes());
        }
    }
    // transfer complete comes after the Auto CMD12's response
    if !wait_ms(1000, || r16(NISTS) & (NI_XFER_DONE | NI_ERROR) != 0) || r16(NISTS) & NI_ERROR != 0 {
        reset(RESET_CMD | RESET_DAT);
        return false;
    }
    w16(NISTS, NI_XFER_DONE | NI_CMD_DONE);
    true
}

/// CMD25: the k gathered blocks of WB in one command, the controller sending CMD12 when the count
/// is reached; done when the card has programmed them (transfer complete after the CMD12's busy).
fn write_multi(arg: u32, k: usize) -> bool {
    // SAFETY: scalar read.
    if unsafe { DMA } == 1 {
        // SAFETY: shell context, single core; no DMA is running.
        unsafe { (&mut (*addr_of_mut!(DBUF)).0)[..k * 512].copy_from_slice(&(&*addr_of!(WB))[..k * 512]) };
        if adma(25, arg, k) {
            return true;
        }
        dma_off("an ADMA2 write failed");
    }
    w16(BLKSIZE, 512);
    w16(BLKCNT, k as u16);
    // block count enable (1), Auto CMD12 (2), write (bit 4 clear), multi-block (5); no DMA
    if command(25, arg, R_48 | DATA_PRESENT, 1 << 1 | 1 << 2 | 1 << 5).is_none() {
        return false;
    }
    // SAFETY: shell context, single core.
    let wb = unsafe { &*addr_of!(WB) };
    for b in 0..k {
        if !wait_ms(1000, || r16(NISTS) & (NI_BUF_WRITE | NI_ERROR) != 0) || r16(NISTS) & NI_ERROR != 0 {
            reset(RESET_CMD | RESET_DAT);
            return false;
        }
        w16(NISTS, NI_BUF_WRITE);
        for w in 0..128 {
            let o = b * 512 + w * 4;
            w32(DATA, u32::from_le_bytes([wb[o], wb[o + 1], wb[o + 2], wb[o + 3]]));
        }
    }
    if !wait_ms(2000 + 250 * k as u64, || r16(NISTS) & (NI_XFER_DONE | NI_ERROR) != 0) || r16(NISTS) & NI_ERROR != 0 {
        reset(RESET_CMD | RESET_DAT);
        return false;
    }
    w16(NISTS, NI_XFER_DONE | NI_CMD_DONE);
    true
}

/// One multi-block command (CMD18 read or CMD25 write) of k blocks moved by the controller's
/// ADMA2 engine between DBUF and the card: one 32-bit descriptor (valid, end, "transfer data")
/// covers the whole run (k * 512 <= 8 KiB); block count + Auto CMD12 as without DMA; done at
/// transfer complete (after the CMD12, and for a write after the card's busy). An ADMA error
/// (error status bit 9) shows as an error like any other.
fn adma(index: u8, arg: u32, k: usize) -> bool {
    // SAFETY: shell/boot context, single core: the descriptor; no DMA is running.
    unsafe {
        let d = &mut (*addr_of_mut!(DESC)).0;
        d[0] = 0x0023 | ((k * 512) as u32) << 16; // valid (0), end (1), act = transfer (5:4 = 10)
        d[1] = addr_of!(DBUF) as u64 as u32;
    }
    // the descriptor and the data are in memory before the controller is told to fetch them
    core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    w32(ADMA_ADDR, addr_of!(DESC) as u64 as u32);
    w32(ADMA_ADDR + 4, 0);
    w16(BLKSIZE, 512);
    w16(BLKCNT, k as u16);
    let dir = if index == 18 { 1 << 4 } else { 0 };
    // DMA enable (0), block count enable (1), Auto CMD12 (2), direction (4), multi-block (5)
    if command(index, arg, R_48 | DATA_PRESENT, 1 | 1 << 1 | 1 << 2 | dir | 1 << 5).is_none() {
        return false;
    }
    if !wait_ms(2000 + 250 * k as u64, || r16(NISTS) & (NI_XFER_DONE | NI_ERROR) != 0) || r16(NISTS) & NI_ERROR != 0 {
        reset(RESET_CMD | RESET_DAT);
        return false;
    }
    w16(NISTS, NI_XFER_DONE | NI_CMD_DONE | 1 << 3);
    core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    // SAFETY: counters, single core.
    unsafe { DSTATS = (DSTATS.0 + 1, DSTATS.1 + k as u64) };
    true
}

/// DMA goes off for good (the CPU moves the data through the data port again); one line says why.
fn dma_off(why: &str) {
    // SAFETY: scalar state, single core.
    unsafe { DMA = 2 };
    w8(HOSTCTL, r8(HOSTCTL) & !0x18);
    crate::kprintln!("[sdhci] DMA: off ({}): the CPU moves the data from now on", why);
}

/// At start-up, once multi-block reads are on (RA holds the first k blocks, checked against single
/// reads): the same k blocks read again by ADMA2 must be equal, else DMA stays off. 32-bit
/// descriptors only, so DESC and DBUF must lie below 4 GiB. One boot line says which.
fn dma_check(k: usize) {
    let (caps, spec) = (r32(CAPS), unsafe { SPEC });
    let high = [addr_of!(DESC) as u64, addr_of!(DBUF) as u64 + (RA_MAX * 512) as u64 - 1].iter().any(|&a| a >> 32 != 0);
    let why = if spec < 1 || caps & 1 << 19 == 0 {
        "the slot has no ADMA2"
    } else if high {
        "its buffers lie above 4 GiB, out of reach of 32-bit descriptors"
    } else {
        ""
    };
    if !why.is_empty() {
        // SAFETY: init, single core.
        unsafe { DMA = 2 };
        crate::kprintln!("[sdhci] DMA: off ({}): the CPU moves the data", why);
        return;
    }
    w8(HOSTCTL, r8(HOSTCTL) & !0x18 | 0x10); // DMA select 10 = 32-bit ADMA2
    // SAFETY: init, single core.
    unsafe { QUIET = true };
    let read = adma(18, 0, k);
    // SAFETY: as above; the DMA has finished.
    let ok = read && unsafe { (&(*addr_of!(DBUF)).0)[..k * 512] == (&*addr_of!(RA))[..k * 512] };
    // SAFETY: as above.
    unsafe {
        QUIET = false;
        DSTATS = (0, 0);
        DMA = if ok { 1 } else { 2 };
    }
    if ok {
        crate::kprintln!("[sdhci] DMA: ADMA2 on (32-bit descriptors, one per multi-block command; {} blocks by DMA equal the CPU's copy)", k);
    } else {
        w8(HOSTCTL, r8(HOSTCTL) & !0x18);
        crate::kprintln!(
            "[sdhci] DMA: off ({}): the CPU moves the data",
            if read { "an ADMA2 read differed from the CPU's copy" } else { "the slot refused an ADMA2 read" }
        );
    }
}

/// Gather one VFS write (card block n); `last` = no write follows it in this run (the end of the
/// VFS area, or a lone write such as the self-test's), so the run goes to the card now.
fn queue(n: u64, data: &[u8; 512], last: bool) -> bool {
    // SAFETY: scalar read.
    let Some((sectors, ..)) = (unsafe { CARD }) else { return false };
    if n >= sectors {
        return false;
    }
    // SAFETY: shell context, single core: the write-behind buffer.
    unsafe {
        let mut ok = true;
        if WB_N > 0 && (n != WB_AT + WB_N as u64 || WB_N as usize == RA_MAX) {
            ok = flush();
        }
        if WB_N == 0 {
            WB_AT = n;
        }
        let i = WB_N as usize * 512;
        (&mut *addr_of_mut!(WB))[i..i + 512].copy_from_slice(data);
        WB_N += 1;
        if last || WB_N as usize == RA_MAX || !MULTI || MULTIW == 2 {
            ok = flush() && ok;
        }
        ok
    }
}

/// Send the gathered writes: one CMD25 when multi-block writes are on (the first one is read back
/// block by block and must be equal, else they go off and the run is rewritten), else CMD24 each.
/// The read-ahead copy of every block written is updated (dropped when a write did not take).
fn flush() -> bool {
    // SAFETY: shell context, single core: the write-behind buffer, read-ahead and counters.
    unsafe {
        let (at, k) = (WB_AT, WB_N as usize);
        if k == 0 {
            return true;
        }
        WB_N = 0;
        let high = CARD.is_some_and(|c| c.1);
        let arg = |b: u64| if high { b } else { b * 512 } as u32;
        let wb = &*addr_of!(WB);
        let mut done = false;
        if k > 1 && MULTI && MULTIW != 2 {
            let by_dma = DMA == 1;
            if write_multi(arg(at), k) {
                done = true;
                let by_dma = by_dma && DMA == 1;
                if MULTIW == 0 {
                    let mut one = [0u8; 512];
                    for b in 0..k {
                        done = done && transfer(17, arg(at + b as u64), &mut one, false) && wb[b * 512..b * 512 + 512] == one;
                    }
                }
                if !done && by_dma {
                    // the first DMA write read back differently: DMA off, the run rewritten below,
                    // and the next run tries CMD25 through the data port (checked again)
                    dma_off("the first ADMA2 write read back differently");
                } else if MULTIW == 0 {
                    MULTIW = if done { 1 } else { 2 };
                    if done {
                        crate::kprintln!("[sdhci] multi-block writes: on (CMD25 with Auto CMD12, up to {} blocks per command; the first {} read back equal)", RA_MAX, k);
                    } else {
                        crate::kprintln!("[sdhci] multi-block writes: off (CMD25 read back differently: rewritten one block per command)");
                    }
                }
                if done {
                    WSTATS = (WSTATS.0 + k as u64, WSTATS.1 + k as u64, WSTATS.2 + 1, WSTATS.3);
                }
            } else {
                MULTIW = 2;
                crate::kprintln!("[sdhci] CMD25 at block {} failed: single-block writes from now on", at);
            }
        }
        let mut ok = true;
        if !done {
            for b in 0..k {
                let mut one = [0u8; 512];
                one.copy_from_slice(&wb[b * 512..b * 512 + 512]);
                ok = transfer(24, arg(at + b as u64), &mut one, true) && ok;
                WSTATS.0 += 1;
                WSTATS.3 += 1;
            }
        }
        // the read-ahead copy follows the card
        let ra = &mut *addr_of_mut!(RA);
        for b in 0..k as u64 {
            let n = at + b;
            if n >= RA_AT && n < RA_AT + RA_N as u64 {
                if ok {
                    let i = (n - RA_AT) as usize * 512;
                    ra[i..i + 512].copy_from_slice(&wb[b as usize * 512..b as usize * 512 + 512]);
                } else {
                    RA_N = 0;
                }
            }
        }
        ok
    }
}

/// At start-up, after the bus is set: the first blocks read with one CMD18 must equal the same
/// blocks read one at a time (CMD17), else multi-block reads stay off. One boot line says which.
fn multi_check(sectors: u64) {
    let k = 8usize.min(sectors as usize);
    // SAFETY: init, single core.
    unsafe {
        (DMA, DSTATS) = (0, (0, 0));
        (MULTI, RA_N, STATS) = (false, 0, (0, 0, 0, 0));
        (MULTIW, WB_N, WSTATS) = (0, 0, (0, 0, 0, 0));
        QUIET = true;
    }
    let mut ok = k > 1 && read_multi(0, k);
    let multi_ok = ok;
    if ok {
        // SAFETY: as above.
        let high = unsafe { CARD.is_some_and(|c| c.1) };
        let mut one = [0u8; 512];
        for b in 0..k {
            let arg = if high { b } else { b * 512 } as u32;
            // SAFETY: as above.
            ok = ok && transfer(17, arg, &mut one, false) && unsafe { (&*addr_of!(RA))[b * 512..b * 512 + 512] == one };
        }
    }
    // SAFETY: as above.
    unsafe {
        QUIET = false;
        MULTI = ok;
    }
    if ok {
        crate::kprintln!("[sdhci] multi-block reads: on (CMD18 with Auto CMD12, read-ahead {} blocks; checked against {} single-block reads)", RA_MAX, k);
        dma_check(k);
    } else if multi_ok {
        crate::kprintln!("[sdhci] multi-block reads: off (CMD18 read back differently from single blocks)");
    } else {
        crate::kprintln!("[sdhci] multi-block reads: off (the card or slot refused CMD18): one block per command");
    }
}

/// One data command through the buffer data port: a 512-byte block, an eMMC's EXT_CSD, or an SD
/// card's 64-byte CMD6 status (the length is the buffer's, a multiple of 4).
fn transfer(index: u8, arg: u32, buf: &mut [u8], write: bool) -> bool {
    w16(BLKSIZE, buf.len() as u16);
    w16(BLKCNT, 1);
    let xfer = if write { 0 } else { 1 << 4 }; // one block, no DMA, direction
    let ready = if write { NI_BUF_WRITE } else { NI_BUF_READ };
    if command(index, arg, R_48 | DATA_PRESENT, xfer).is_none() {
        return false;
    }
    if !wait_ms(1000, || r16(NISTS) & (ready | NI_ERROR) != 0) || r16(NISTS) & NI_ERROR != 0 {
        // SAFETY: scalar read.
        if unsafe { QUIET } {
            reset(RESET_CMD | RESET_DAT);
            return false;
        }
        crate::kprintln!("[sdhci] CMD{} arg {:#x}: the buffer never became ready (status {:#x}, error {:#x})", index, arg, r16(NISTS), r16(EISTS));
        reset(RESET_CMD | RESET_DAT);
        return false;
    }
    w16(NISTS, ready);
    for i in 0..buf.len() / 4 {
        if write {
            w32(DATA, u32::from_le_bytes([buf[i * 4], buf[i * 4 + 1], buf[i * 4 + 2], buf[i * 4 + 3]]));
        } else {
            buf[i * 4..i * 4 + 4].copy_from_slice(&r32(DATA).to_le_bytes());
        }
    }
    // a write is done when the card has programmed it (transfer complete after DAT0 goes high)
    if !wait_ms(2000, || r16(NISTS) & (NI_XFER_DONE | NI_ERROR) != 0) || r16(NISTS) & NI_ERROR != 0 {
        // SAFETY: scalar read.
        if unsafe { QUIET } {
            reset(RESET_CMD | RESET_DAT);
            return false;
        }
        crate::kprintln!("[sdhci] CMD{} arg {:#x}: transfer did not complete (status {:#x}, error {:#x})", index, arg, r16(NISTS), r16(EISTS));
        reset(RESET_CMD | RESET_DAT);
        return false;
    }
    w16(NISTS, NI_XFER_DONE);
    true
}

/// `claimdisk sd`: take the blank card for the VFS (the owner asked for it).
pub fn claim() -> bool {
    // SAFETY: scalar state, shell context.
    unsafe {
        if !USED && KIND == 2 && CARD.is_some_and(|c| c.3) {
            USED = true;
            return true;
        }
    }
    false
}

pub fn present() -> bool {
    // SAFETY: scalar read.
    unsafe { USED }
}

/// Asili v0 BINDS (pc/hwdb.rs): `init` decides from this table.
pub(crate) const BINDS: &[crate::hwdb::Part] = &[crate::hwdb::pci_class(0x08_05_00, 0xFF_FF_00, "SD host controller (SDHCI)")];

/// The PCI function this driver took, for `hwscan`.
pub fn at() -> Option<(u8, u8, u8)> {
    // SAFETY: scalar reads of boot-time state.
    unsafe { (USED).then_some(PCI_AT) }
}

/// The VFS is on the built-in eMMC (whole disk or AfriSovera's partition): an internal disk, so
/// disk.rs prefers it to a USB stick, as it does NVMe and SATA.
pub fn internal() -> bool {
    // SAFETY: scalar reads.
    unsafe { USED && matches!(MMC, Some(_)) }
}

/// Sectors the VFS sees: AfriSovera's partition when there is one, else the whole card.
pub fn capacity() -> u64 {
    // SAFETY: scalar reads.
    unsafe { if PART.1 > 0 { PART.1 } else { CARD.map_or(0, |c| c.0) } }
}

pub fn read_sector(n: u64, out: &mut [u8; 512]) -> bool {
    // SAFETY: scalar read.
    present() && n < capacity() && block(n + unsafe { PART.0 }, out, false)
}

/// The VFS's writes: gathered for one CMD25 while they run on in order (vfs.rs sync_to_disk writes
/// sectors 0..=2*MAX_FILES one by one); the run is on the card when the last of them returns.
pub fn write_sector(n: u64, data: &[u8; 512]) -> bool {
    let last = n >= 2 * crate::vfs::MAX_FILES as u64;
    // SAFETY: scalar reads.
    present() && n < capacity() && unsafe { CARD.is_some_and(|c| c.3) } && queue(n + unsafe { PART.0 }, data, last)
}

/// Read-only view for `lsparts` / `installdisk`: (512-byte sectors, block bytes) of a built-in eMMC,
/// whether or not it holds our files. SD cards are removable and are not offered.
pub fn raw_info() -> Option<(u64, u64)> {
    // SAFETY: scalar reads of boot-time state.
    let (mmc, card) = unsafe { (MMC, CARD) };
    if mmc.is_some() { card.map(|c| (c.0, 512)) } else { None }
}

/// Read one sector of the eMMC even when it is not ours (never writes).
pub fn raw_read(n: u64, out: &mut [u8; 512]) -> bool {
    raw_info().is_some_and(|(cap, _)| n < cap) && block(n, out, false)
}

/// Write one sector of the eMMC whoever owns it. ONLY installdisk/installpart/tablerestore/bootcopy
/// call this (gpt.rs, esp.rs), and only on the owner's typed code.
pub fn table_write(n: u64, buf: &[u8; 512]) -> bool {
    let mut b = *buf;
    raw_info().is_some_and(|(cap, _)| n < cap) && block(n, &mut b, true)
}

/// `formatpart emmc` / `installdisk emmc`: keep the VFS in AfriSovera's partition from now on.
pub fn use_part(a: u64, l: u64) -> bool {
    let Some((cap, _)) = raw_info() else { return false };
    // SAFETY: scalar state, shell context.
    unsafe {
        if USED || l == 0 || a.saturating_add(l) > cap {
            return false;
        }
        PART = (a, l);
        USED = true;
    }
    true
}

/// The partition in use (first sector, sectors), (0, 0) for none.
pub fn part() -> (u64, u64) {
    // SAFETY: scalar read.
    unsafe { PART }
}

/// `tablerestore emmc` took AfriSovera's partition out of the table: stop writing there.
pub fn release_part() -> bool {
    // SAFETY: scalar state, shell context.
    unsafe {
        if USED && PART.1 > 0 {
            USED = false;
            PART = (0, 0);
            return true;
        }
    }
    false
}

/// `lssd`: the controller, the card and what it is used for.
pub fn list() {
    // SAFETY: scalar reads of boot-time state.
    let (base, (b, d, f), id, spec, mhz, card, kind, used, rtsx) =
        unsafe { (BASE, PCI_AT, PCI_ID, SPEC, BASE_MHZ, CARD, KIND, USED, RTSX) };
    if base == 0 {
        match rtsx {
            Some((b, d, f, dev)) => crate::kprintln!(
                "sd: no SDHCI card reader; the Realtek card reader at {:02x}:{:02x}.{} (10ec:{:04x}, RTS{:04x}) uses its own interface: not driven (a USB card reader works)",
                b, d, f, dev, dev
            ),
            None => crate::kprintln!("sd: no SDHCI card reader on PCI (a USB card reader shows up as USB storage: lsusb)"),
        }
        return;
    }
    crate::kprintln!(
        "sd: controller {:02x}:{:02x}.{} ({:04x}:{:04x}) registers {:#x}, SDHCI {}.00, base clock {} MHz",
        b, d, f, id & 0xFFFF, id >> 16, base, spec + 1, mhz
    );
    let Some((sectors, high, rca, wp_off)) = card else {
        crate::kprintln!("  no card (or it did not answer)");
        return;
    };
    // SAFETY: scalar reads.
    let (name, mmc, nctl) = unsafe { (NAME, MMC, NCTL) };
    if nctl > 1 {
        crate::kprintln!("  ({} SDHCI controllers found; the first with a card or eMMC that answers is used)", nctl);
    }
    let kind_s = match mmc {
        Some(_) if high => "eMMC sector-addressed",
        Some(_) => "eMMC byte-addressed",
        None if high => "SDHC/SDXC",
        None => "SDSC",
    };
    crate::kprintln!(
        "  {} \"{}\" {} RCA {:#06x}: {} sectors ({} MiB), {}{}",
        if mmc.is_some() { "eMMC" } else { "card" },
        core::str::from_utf8(&name).unwrap_or("?").trim(),
        kind_s,
        rca,
        sectors,
        sectors / 2048,
        if mmc.is_some() {
            "built in (no write-protect switch)"
        } else if wp_off {
            "write-protect switch off"
        } else {
            "write-protect switch ON"
        },
        if used {
            "  <- storage"
        } else if kind == 3 && mmc.is_some() {
            "  (holds other data: never written)"
        } else if kind == 2 && mmc.is_some() {
            "  (blank: claimdisk emmc to use it)"
        } else if kind == 2 && wp_off {
            "  (blank: claimdisk sd to use it)"
        } else {
            "  (other data or write-protected: read-only to us)"
        }
    );
    // SAFETY: scalar reads.
    let (bus, bus0) = unsafe { (BUS, BUS0) };
    crate::kprintln!(
        "  bus: {}-bit, {}, SD clock {} kHz (start-up: 1-bit, {} kHz)",
        bus.0,
        if bus.2 { "high speed" } else { "default speed" },
        bus.1,
        bus0
    );
    // SAFETY: scalar reads.
    let (multi, st) = unsafe { (MULTI, STATS) };
    crate::kprintln!(
        "  reads: {} blocks, {} by multi-block CMD18 ({} commands), {} by single CMD17; multi-block {}",
        st.0,
        st.1 + st.3,
        st.1,
        st.2,
        if multi { "on" } else { "off" }
    );
    // SAFETY: scalar reads.
    let (mw, ws) = unsafe { (MULTIW, WSTATS) };
    crate::kprintln!(
        "  writes: {} blocks, {} by multi-block CMD25 ({} commands), {} by single CMD24; multi-block {}",
        ws.0,
        ws.1,
        ws.2,
        ws.3,
        match mw {
            1 => "on",
            2 => "off",
            _ if multi => "not tried yet",
            _ => "off",
        }
    );
    // SAFETY: scalar reads.
    let (dma, ds) = unsafe { (DMA, DSTATS) };
    if dma == 1 {
        crate::kprintln!("  dma: ADMA2 on (32-bit descriptors): {} multi-block commands moved {} blocks", ds.0, ds.1);
    } else {
        crate::kprintln!("  dma: off: the CPU moves every block through the data port");
    }
    // SAFETY: scalar read.
    let pt = unsafe { PART };
    if used && pt.1 > 0 {
        crate::kprintln!("  storage: AfriSovera's partition, sectors {}-{} ({} MiB); nothing outside it is written", pt.0, pt.0 + pt.1 - 1, pt.1 / 2048);
    }
}
