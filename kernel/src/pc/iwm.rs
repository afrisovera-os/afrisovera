//! The Intel Wireless 7265 / 3165 driver, power-up to "alive" (Wi-Fi W5, plan WIFI_W5_PLAN.md, the architect's
//! OK TO BUILD of 21 Nov 2026). `wifi up` finds the card, loads Intel's firmware through the firmware store (size and
//! sha256 checked against fwdb.rs's row for this exact PCI id), runs the INIT image to ALIVE, reads the NVM (MAC
//! address, channels, antennas), runs the calibration, then runs the REGULAR image to ALIVE. `wifi diag` prints each
//! step again with the registers; `wifi down` stops the card and gives every frame back.
//!
//! Facts from the published OpenBSD iwm driver (ISC), read for facts only (WIFI_W5_REFERENCE.md and
//! WIFI_W5_REGISTERS.md in the notes folder); every line here is written fresh.
//!
//! Rules kept here (the architect's):
//! - Polled. No interrupt is enabled at the PCI level and nothing runs in the timer interrupt; every wait is a bounded
//!   poll from the shell command, with interrupts on. Masked time added: none.
//! - DMA memory comes from the frame pool at `wifi up` (aligned contiguous runs), is checked against the chip's
//!   36-bit reach (`dma_check`), and is all given back at `wifi down` or on a failed bring-up; `wifi down` prints the
//!   frame count before `wifi up` and after.
//! - The firmware upload returns only after ALIVE. A refused file, a timeout or a bad status stops at that step with
//!   its registers.
//! - The MAC address is shown as its first three bytes only (the maker's prefix).
#![allow(dead_code)]

use core::fmt::Write;
use core::ptr::{addr_of, addr_of_mut, read_volatile, write_volatile};
use core::sync::atomic::{fence, Ordering};

use crate::iwm_fw::{self, Image};
use crate::iwm_reg::*;
use crate::iwm_rx::{self, Packet};

const PAGE: usize = 4096;
const RX_COUNT: usize = 1 << RX_RING_LOG;
/// The upload staging buffer: one DMA transfer at most.
const STAGE_FRAMES: usize = FH_TB_MAX / PAGE;
/// The chip's DMA reach.
const DMA_LIMIT: u64 = 1 << 36;
const IDS: [u16; 4] = [0x095A, 0x095B, 0x3165, 0x3166];

// ---- timing ---------------------------------------------------------------------------------------------------------

fn now_us() -> u64 {
    (crate::timer::cycles() as u128 * 1_000_000 / crate::timer::hz().max(1) as u128) as u64
}

fn delay_us(us: u64) {
    let end = now_us() + us;
    while now_us() < end {
        core::hint::spin_loop();
    }
}

// ---- the step log `wifi diag` prints again ---------------------------------------------------------------------------

const LINES: usize = 40;
const LINE: usize = 160;

struct Log {
    text: [[u8; LINE]; LINES],
    len: [usize; LINES],
    n: usize,
}

static mut LOG: Log = Log { text: [[0; LINE]; LINES], len: [0; LINES], n: 0 };

struct Line {
    b: [u8; LINE],
    n: usize,
}

impl Write for Line {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &c in s.as_bytes() {
            if self.n < LINE {
                self.b[self.n] = c;
                self.n += 1;
            }
        }
        Ok(())
    }
}

/// Print one step line and keep it for `wifi diag`.
fn say(args: core::fmt::Arguments) {
    let mut l = Line { b: [0; LINE], n: 0 };
    let _ = l.write_fmt(args);
    let s = core::str::from_utf8(&l.b[..l.n]).unwrap_or("?");
    crate::kprintln!("wifi up: {}", s);
    // SAFETY: shell context only (wifi up and wifi diag), one at a time.
    unsafe {
        let log = &mut *addr_of_mut!(LOG);
        if log.n < LINES {
            log.text[log.n][..l.n].copy_from_slice(&l.b[..l.n]);
            log.len[log.n] = l.n;
            log.n += 1;
        }
    }
}

macro_rules! step {
    ($($t:tt)*) => { say(format_args!($($t)*)) };
}

fn log_clear() {
    // SAFETY: shell context only.
    unsafe { (*addr_of_mut!(LOG)).n = 0 };
}

// ---- DMA memory -------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Run {
    pa: usize,
    frames: usize,
}

impl Run {
    fn bytes(&self) -> &'static mut [u8] {
        // SAFETY: a run of frames this driver holds from the pool (identity-mapped RAM) until `free_all`.
        unsafe { core::slice::from_raw_parts_mut(self.pa as *mut u8, self.frames * PAGE) }
    }
    fn end(&self) -> u64 {
        (self.pa + self.frames * PAGE) as u64
    }
}

struct Mem {
    /// the command queue's 256 TFDs (32 KiB)
    tfds: Run,
    /// the one command being sent
    cmd: Run,
    /// the scheduler's byte-count tables, 31 queues of 320 entries
    bc: Run,
    /// the keep-warm page
    kw: Run,
    /// 64 receive buffer descriptors
    rbd: Run,
    /// the receive status the chip writes (closed_rb_num)
    stat: Run,
    /// 64 receive buffers of 4 KiB
    rxb: Run,
    /// firmware upload staging, one DMA transfer
    stage: Run,
}

const PLAN: [(&str, usize, usize); 8] = [
    ("the command ring", TX_RING * TFD_SIZE / PAGE, PAGE),
    ("the command buffer", 1, PAGE),
    ("the byte-count tables", (BC_QUEUES * BC_ENTRIES * 2).div_ceil(PAGE), PAGE),
    ("the keep-warm page", 1, PAGE),
    ("the receive descriptors", 1, PAGE),
    ("the receive status", 1, PAGE),
    ("the receive buffers", RX_COUNT, PAGE),
    ("the upload buffer", STAGE_FRAMES, PAGE),
];

fn alloc_all() -> Result<Mem, &'static str> {
    let mut got = [Run { pa: 0, frames: 0 }; 8];
    for (i, &(what, n, align)) in PLAN.iter().enumerate() {
        match crate::frame::alloc_run(n, align) {
            Some(pa) => got[i] = Run { pa, frames: n },
            None => {
                for r in got.iter().take(i) {
                    crate::frame::free_run(r.pa, r.frames);
                }
                return Err(what);
            }
        }
    }
    Ok(Mem { tfds: got[0], cmd: got[1], bc: got[2], kw: got[3], rbd: got[4], stat: got[5], rxb: got[6], stage: got[7] })
}

impl Mem {
    fn runs(&self) -> [(&'static str, Run); 8] {
        [
            (PLAN[0].0, self.tfds),
            (PLAN[1].0, self.cmd),
            (PLAN[2].0, self.bc),
            (PLAN[3].0, self.kw),
            (PLAN[4].0, self.rbd),
            (PLAN[5].0, self.stat),
            (PLAN[6].0, self.rxb),
            (PLAN[7].0, self.stage),
        ]
    }
    fn free_all(&self) {
        for (_, r) in self.runs() {
            r.bytes().fill(0);
            crate::frame::free_run(r.pa, r.frames);
        }
    }
    /// Every region inside the chip's 36-bit reach and 4 KiB aligned: (outside, total, first outside).
    fn dma_check(&self) -> (usize, usize, &'static str) {
        let mut bad = (0, "");
        for (name, r) in self.runs() {
            if r.pa % PAGE != 0 || r.end() > DMA_LIMIT {
                if bad.0 == 0 {
                    bad.1 = name;
                }
                bad.0 += 1;
            }
        }
        (bad.0, 8, bad.1)
    }
    fn frames(&self) -> usize {
        self.runs().iter().map(|(_, r)| r.frames).sum()
    }
}

// ---- the device ---------------------------------------------------------------------------------------------------------

struct Dev {
    bar: u64,
    at: (u8, u8, u8),
    device: u16,
    hw_rev: u32,
    mem: Mem,
    cmd_cur: usize,
    rx_read: usize,
    rfkill: bool,
    phy_sku: u32,
    init_complete: bool,
    calib_results: u32,
    errors: u32,
    reply: [u8; 2056],
    reply_len: usize,
    nvm_hw: [u8; 2048],
    nvm_sw: [u8; 2048],
    nvm: Option<iwm_rx::NvmInfo>,
    up: bool,
}

static mut DEV: Option<Dev> = None;
/// The frame pool's count before the last `wifi up` (the architect's frames check).
static mut FRAMES_BEFORE: usize = 0;

fn mmio(bar: u64, off: u32) -> *mut u32 {
    (bar + off as u64) as *mut u32
}

impl Dev {
    fn r(&self, off: u32) -> u32 {
        // SAFETY: BAR0 is mapped uncached by `wifi up` before any register access; `off` is inside its 8 KiB.
        unsafe { read_volatile(mmio(self.bar, off)) }
    }
    fn w(&self, off: u32, v: u32) {
        // SAFETY: as `r`.
        unsafe { write_volatile(mmio(self.bar, off), v) }
    }
    fn set(&self, off: u32, bits: u32) {
        self.w(off, self.r(off) | bits);
    }
    fn clear(&self, off: u32, bits: u32) {
        self.w(off, self.r(off) & !bits);
    }
    /// Wait until `(reg & mask) == want`, in 10 us steps, for at most `us`.
    fn poll(&self, reg: u32, mask: u32, want: u32, us: u64) -> bool {
        let end = now_us() + us;
        loop {
            if self.r(reg) & mask == want {
                return true;
            }
            if now_us() >= end {
                return false;
            }
            delay_us(10);
        }
    }

    // -- MAC access, periphery and chip memory --
    fn lock(&self) -> bool {
        self.set(CSR_GP_CNTRL, GP_MAC_ACCESS_REQ);
        self.poll(CSR_GP_CNTRL, GP_MAC_CLOCK_READY | GP_GOING_TO_SLEEP, GP_MAC_CLOCK_READY, 150_000)
    }
    fn unlock(&self) {
        self.clear(CSR_GP_CNTRL, GP_MAC_ACCESS_REQ);
    }
    fn prph_r(&self, a: u32) -> u32 {
        self.w(HBUS_TARG_PRPH_RADDR, prph_addr(a));
        self.r(HBUS_TARG_PRPH_RDAT)
    }
    fn prph_w(&self, a: u32, v: u32) {
        self.w(HBUS_TARG_PRPH_WADDR, prph_addr(a));
        self.w(HBUS_TARG_PRPH_WDAT, v);
    }
    fn prph_set(&self, a: u32, bits: u32) {
        self.prph_w(a, self.prph_r(a) | bits);
    }
    fn prph_clear(&self, a: u32, bits: u32) {
        self.prph_w(a, self.prph_r(a) & !bits);
    }
    fn mem_w(&self, a: u32, v: u32) {
        self.w(HBUS_TARG_MEM_WADDR, a);
        self.w(HBUS_TARG_MEM_WDAT, v);
    }
    fn regs(&self) -> (u32, u32, u32, u32) {
        (self.r(CSR_GP_CNTRL), self.r(CSR_HW_IF_CONFIG), self.r(CSR_INT), self.r(CSR_FH_INT_STATUS))
    }
    fn say_regs(&self) {
        let (gp, hw, int, fh) = self.regs();
        step!("  registers: GP_CNTRL {:#010x} HW_IF_CONFIG {:#010x} INT {:#010x} FH_INT_STATUS {:#010x}", gp, hw, int, fh);
    }

    // -- power --
    fn set_hw_ready(&self) -> bool {
        self.set(CSR_HW_IF_CONFIG, HW_IF_NIC_READY);
        let ok = self.poll(CSR_HW_IF_CONFIG, HW_IF_NIC_READY, HW_IF_NIC_READY, 50);
        if ok {
            self.set(CSR_MBOX_SET, MBOX_OS_ALIVE);
        }
        ok
    }
    fn prepare_card(&self) -> bool {
        if self.set_hw_ready() {
            return true;
        }
        self.set(CSR_DBG_LINK_PWR_MGMT, RESET_LINK_PWR_MGMT_DISABLED);
        delay_us(1000);
        for _ in 0..10 {
            self.set(CSR_HW_IF_CONFIG, HW_IF_PREPARE);
            let end = now_us() + 150_000;
            while now_us() < end {
                if self.set_hw_ready() {
                    return true;
                }
                delay_us(200);
            }
            delay_us(25_000);
        }
        false
    }
    /// Is ASPM L1 on in the PCIe link control register? (Then the chip's L0s is set off.)
    fn aspm_l1(&self) -> bool {
        let (b, d, f) = self.at;
        let mut p = (crate::pci::cfg_read32(b, d, f, 0x34) & 0xFC) as u8;
        for _ in 0..48 {
            if p < 0x40 {
                return false;
            }
            let h = crate::pci::cfg_read32(b, d, f, p);
            if h & 0xFF == 0x10 {
                let Some(lc) = p.checked_add(0x10) else { return false };
                return crate::pci::cfg_read32(b, d, f, lc) & 0x2 != 0;
            }
            p = ((h >> 8) & 0xFC) as u8;
        }
        false
    }
    fn apm_init(&self) -> Result<(), &'static str> {
        self.set(CSR_GIO_CHICKEN_BITS, GIO_CHICKEN_DIS_L0S_EXIT_TIMER | GIO_CHICKEN_L1A_NO_L0S_RX);
        self.set(CSR_DBG_HPET_MEM, HPET_MEM_VAL);
        self.set(CSR_HW_IF_CONFIG, HW_IF_HAP_WAKE_L1A);
        if self.aspm_l1() {
            self.set(CSR_GIO, GIO_L0S);
        } else {
            self.clear(CSR_GIO, GIO_L0S);
        }
        self.set(CSR_GP_CNTRL, GP_INIT_DONE);
        if !self.poll(CSR_GP_CNTRL, GP_MAC_CLOCK_READY, GP_MAC_CLOCK_READY, 25_000) {
            return Err("the MAC clock did not come up");
        }
        if !self.lock() {
            return Err("no MAC access (the chip did not wake)");
        }
        self.prph_w(APMG_CLK_EN, APMG_CLK_DMA_CLK_RQT);
        delay_us(20);
        self.prph_set(APMG_PCIDEV_STT, APMG_PCIDEV_STT_L1_ACT_DIS);
        self.prph_w(APMG_RTC_INT_STT, APMG_RTC_INT_STT_RFKILL);
        self.unlock();
        Ok(())
    }
    fn start_hw(&mut self) -> Result<(), &'static str> {
        if !self.prepare_card() {
            return Err("the card did not say NIC_READY");
        }
        self.w(CSR_RESET, RESET_SW_RESET);
        delay_us(5000);
        self.apm_init()?;
        self.w(CSR_INT_MASK, 0);
        self.w(CSR_INT, 0xFFFF_FFFF);
        self.rfkill = self.r(CSR_GP_CNTRL) & GP_HW_RF_KILL_SW == 0;
        Ok(())
    }
    fn stop_device(&mut self) {
        self.w(CSR_INT_MASK, 0);
        self.w(CSR_INT, 0xFFFF_FFFF);
        if self.lock() {
            for ch in 0..8 {
                self.w(fh_tx_config(ch), 0);
                let idle = (1 << ch) << 16;
                self.poll(FH_TSSR_TX_STATUS, idle, idle, 4000);
            }
            self.w(FH_RCSR_CHNL0_CONFIG, 0);
            self.poll(FH_RSSR_RX_STATUS, RSSR_CHNL0_IDLE, RSSR_CHNL0_IDLE, 10_000);
            self.prph_w(APMG_CLK_DIS, APMG_CLK_DMA_CLK_RQT);
            self.unlock();
        }
        delay_us(5);
        self.set(CSR_RESET, RESET_STOP_MASTER);
        self.poll(CSR_RESET, RESET_MASTER_DISABLED, RESET_MASTER_DISABLED, 100);
        self.clear(CSR_GP_CNTRL, GP_INIT_DONE);
        self.w(CSR_RESET, RESET_SW_RESET);
        delay_us(5000);
        self.w(CSR_INT_MASK, 0);
        self.w(CSR_INT, 0xFFFF_FFFF);
        // the rings start empty next time
        for r in [self.mem.tfds, self.mem.cmd, self.mem.bc, self.mem.stat, self.mem.rxb] {
            r.bytes().fill(0);
        }
        self.cmd_cur = 0;
        self.rx_read = 0;
        self.up = false;
    }

    // -- rings --
    fn nic_init(&mut self) -> Result<(), &'static str> {
        self.apm_init()?;
        if !self.lock() {
            return Err("no MAC access for the NIC set-up");
        }
        self.prph_w(APMG_PS_CTRL, self.prph_r(APMG_PS_CTRL) & !APMG_PS_CTRL_MSK_PWR_SRC); // power from VMAIN
        self.unlock();
        // the MAC's step and dash from CSR_HW_REV, the radio's type, step and dash from the firmware's PHY_SKU
        let (rev, phy) = (self.hw_rev, self.phy_sku);
        let v = (rev & 0x3) | (((rev >> 2) & 0x3) << 2) | ((phy & 0x3) << 10) | (((phy >> 4) & 0x3) << 12) | (((phy >> 2) & 0x3) << 14);
        self.w(CSR_HW_IF_CONFIG, (self.r(CSR_HW_IF_CONFIG) & !HW_IF_CONFIG_FIELDS) | v | HW_IF_MAC_SI | HW_IF_RADIO_SI);
        if !self.lock() {
            return Err("no MAC access for the NIC set-up");
        }
        self.prph_set(APMG_PS_CTRL, APMG_PS_CTRL_EARLY_PWR_OFF_RESET_DIS);
        // receive: 64 descriptors, each the address of a 4 KiB buffer shifted by 8
        let rbd = self.mem.rbd.bytes();
        for i in 0..RX_COUNT {
            let a = ((self.mem.rxb.pa + i * PAGE) >> 8) as u32;
            rbd[4 * i..4 * i + 4].copy_from_slice(&a.to_le_bytes());
        }
        fence(Ordering::SeqCst);
        self.w(FH_RCSR_CHNL0_CONFIG, 0);
        self.w(FH_RSCSR_CHNL0_STTS_WPTR, (self.mem.stat.pa >> 4) as u32);
        self.w(FH_RSCSR_CHNL0_RBDCB_BASE, (self.mem.rbd.pa >> 8) as u32);
        self.w(FH_RCSR_CHNL0_CONFIG, RX_CONFIG);
        self.w(CSR_INT_COALESCING, INT_COALESCING_DEF);
        // transmit: the scheduler off until ALIVE, the keep-warm page, the command queue's ring
        self.prph_w(SCD_TXFACT, 0);
        self.w(FH_KW_MEM_ADDR, (self.mem.kw.pa >> 4) as u32);
        self.w(fh_cbbc_queue(CMD_QUEUE), (self.mem.tfds.pa >> 8) as u32);
        self.prph_set(SCD_GP_CTRL, SCD_GP_AUTO_ACTIVE_MODE | SCD_GP_ENABLE_31_QUEUES);
        self.unlock();
        self.w(FH_RSCSR_CHNL0_WPTR, 8);
        self.set(CSR_MAC_SHADOW_REG_CTRL, SHADOW_REG_CTRL_ALL);
        Ok(())
    }

    /// One packet buffer after another from where we last read to where the chip has closed, each walked; `f` sees
    /// every packet and returns true to stop waiting. Then the chip is told which descriptors are free again.
    fn rx_poll(&mut self, f: &mut dyn FnMut(&mut Self, &Packet<'static>) -> bool) -> bool {
        // SAFETY: the receive status frame is ours (DMA memory the chip writes); a volatile read of its first u16.
        let closed = unsafe { read_volatile(self.mem.stat.pa as *const u16) } as usize & 0xFFF;
        let closed = closed % RX_COUNT;
        let mut hit = false;
        let mut moved = false;
        while self.rx_read != closed {
            let at = self.mem.rxb.pa + self.rx_read * PAGE;
            // SAFETY: one of our receive buffers; the chip has closed it, so it does not write it now.
            let buf: &'static [u8] = unsafe { core::slice::from_raw_parts(at as *const u8, PAGE) };
            iwm_rx::walk(buf, |p| {
                if !hit {
                    hit = f(&mut *self, &p);
                }
            });
            // SAFETY: as above; the buffer goes back to the chip, cleared of its first packet header.
            unsafe { core::ptr::write_bytes(at as *mut u8, 0, 8) };
            self.rx_read = (self.rx_read + 1) % RX_COUNT;
            moved = true;
        }
        if moved {
            let last = if closed == 0 { RX_COUNT - 1 } else { closed - 1 };
            self.w(FH_RSCSR_CHNL0_WPTR, (last & !7) as u32);
        }
        hit
    }

    /// Notifications the chip sends on its own, whatever we are waiting for.
    fn note(&mut self, p: &Packet<'static>) {
        match p.code {
            iwm_rx::INIT_COMPLETE_NOTIF => self.init_complete = true,
            iwm_rx::CALIB_RES_NOTIF_PHY_DB if iwm_rx::parse_phy_db(p.data).is_ok() => self.calib_results += 1,
            iwm_rx::REPLY_ERROR => self.errors += 1,
            _ => {}
        }
    }

    /// Poll the receive ring for at most `us`, until `want` says a packet is the one.
    fn wait(&mut self, us: u64, want: &mut dyn FnMut(&mut Self, &Packet<'static>) -> bool) -> bool {
        let end = now_us() + us;
        loop {
            let got = self.rx_poll(&mut |d: &mut Self, p: &Packet<'static>| {
                d.note(p);
                want(d, p)
            });
            if got {
                return true;
            }
            if now_us() >= end {
                return false;
            }
            delay_us(50);
        }
    }

    // -- firmware upload --
    fn load_chunk(&self, dst: u32, len: usize) -> bool {
        let pa = self.mem.stage.pa as u64;
        self.w(CSR_INT, INT_FH_TX);
        if !self.lock() {
            return false;
        }
        self.w(fh_tx_config(SRVC_CHNL), 0);
        self.w(FH_SRVC_CHNL_SRAM_ADDR, dst);
        self.w(fh_tfdib_ctrl0(SRVC_CHNL), pa as u32);
        self.w(fh_tfdib_ctrl1(SRVC_CHNL), (((pa >> 32) as u32) << 28) | len as u32);
        self.w(fh_tx_buf_sts(SRVC_CHNL), TX_BUF_STS_ONE_TFDB);
        self.w(fh_tx_config(SRVC_CHNL), TX_CONFIG_ENABLE | TX_CONFIG_CIRQ_HOST_ENDTFD);
        self.unlock();
        let ok = self.poll(CSR_INT, INT_FH_TX, INT_FH_TX, 1_000_000);
        self.w(CSR_INT, INT_FH_TX);
        self.w(CSR_FH_INT_STATUS, self.r(CSR_FH_INT_STATUS));
        ok
    }

    /// Upload one image of `fw` (from `file`), start its CPU and wait for ALIVE.
    fn load_image(&mut self, file: &[u8], fw: &iwm_fw::Fw, img: Image, name: &str) -> Result<iwm_rx::Alive, ()> {
        self.w(CSR_INT, 0xFFFF_FFFF);
        if let Err(why) = self.nic_init() {
            step!("{} image: {}", name, why);
            self.say_regs();
            return Err(());
        }
        self.w(CSR_UCODE_DRV_GP1_CLR, GP1_SW_BIT_RFKILL | GP1_CMD_BLOCKED);
        self.w(CSR_INT, 0xFFFF_FFFF);
        self.w(CSR_INT_MASK, INT_INI_SET_MASK);
        let (mut sections, mut bytes, mut nchunks) = (0usize, 0usize, 0usize);
        for s in fw.image(img).iter() {
            if s.is_separator() {
                break; // one CPU: its sections end at the first separator
            }
            let data = fw.bytes(file, s);
            for c in iwm_rx::chunks(s.devoff, data.len(), FH_TB_MAX) {
                self.mem.stage.bytes()[..c.len].copy_from_slice(&data[c.from..c.from + c.len]);
                fence(Ordering::SeqCst);
                let ext = (FW_MEM_EXTENDED_START..=FW_MEM_EXTENDED_END).contains(&c.dst);
                if ext && self.lock() {
                    self.prph_set(LMPM_CHICK, LMPM_CHICK_EXTENDED_ADDR_SPACE);
                    self.unlock();
                }
                let ok = self.load_chunk(c.dst, c.len);
                if ext && self.lock() {
                    self.prph_clear(LMPM_CHICK, LMPM_CHICK_EXTENDED_ADDR_SPACE);
                    self.unlock();
                }
                if !ok {
                    step!("{} image: the DMA of {} bytes to {:#x} did not finish (1 s)", name, c.len, c.dst);
                    self.say_regs();
                    return Err(());
                }
                nchunks += 1;
                bytes += c.len;
            }
            sections += 1;
        }
        self.mem.stage.bytes().fill(0);
        step!("{} image: {} sections, {} bytes in {} DMA transfers", name, sections, bytes, nchunks);
        // run it: the CPU leaves reset, and its first word is ALIVE
        self.w(CSR_RESET, 0);
        let mut alive = None;
        let got = self.wait(1_000_000, &mut |_d: &mut Self, p: &Packet<'static>| {
            if p.code == iwm_rx::ALIVE {
                alive = Some(iwm_rx::parse_alive(p.data));
                true
            } else {
                false
            }
        });
        match (got, alive) {
            (true, Some(Ok(a))) if a.ok() => {
                step!("{} image: ALIVE (layout {}, firmware {}.{}, scheduler at {:#x})", name, a.layout, a.ucode_major, a.ucode_minor, a.scd_base);
                Ok(a)
            }
            (true, Some(Ok(a))) => {
                step!("{} image: ALIVE with status {:#06x}, not 0xcafe (error table at {:#x})", name, a.status, a.error_table);
                self.say_regs();
                Err(())
            }
            (true, _) => {
                step!("{} image: an ALIVE of a length no layout has", name);
                Err(())
            }
            (false, _) => {
                step!("{} image: no ALIVE within 1 s", name);
                self.say_regs();
                Err(())
            }
        }
    }

    /// The scheduler and the command queue, once the firmware is alive.
    fn post_alive(&mut self) -> Result<(), &'static str> {
        if !self.lock() {
            return Err("no MAC access after ALIVE");
        }
        let base = self.prph_r(SCD_SRAM_BASE_ADDR);
        self.w(HBUS_TARG_MEM_WADDR, base + SCD_CONTEXT_OFFSET);
        for _ in (SCD_CONTEXT_OFFSET..SCD_SRAM_CLEAR_END).step_by(4) {
            self.w(HBUS_TARG_MEM_WDAT, 0); // the address moves on by itself
        }
        self.prph_w(SCD_DRAM_BASE_ADDR, (self.mem.bc.pa >> 10) as u32);
        self.prph_w(SCD_CHAINEXT_EN, 0);
        let q = CMD_QUEUE;
        self.prph_w(scd_queue_status_bits(q), 1 << SCD_STTS_ACT_EN);
        self.prph_clear(SCD_AGGR_SEL, 1 << q);
        self.w(HBUS_TARG_WRPTR, q << 8);
        self.prph_w(scd_queue_rdptr(q), 0);
        let ctx = base + SCD_CONTEXT_OFFSET + 8 * q;
        self.mem_w(ctx, 0);
        self.mem_w(ctx + 4, (SCD_FRAME_LIMIT & 0x7F) | ((SCD_FRAME_LIMIT << 16) & 0x007F_0000));
        self.prph_w(scd_queue_status_bits(q), (1 << SCD_STTS_ACTIVE) | TX_FIFO_CMD | (1 << SCD_STTS_WSL) | SCD_STTS_REG_MSK);
        self.prph_set(SCD_EN_CTRL, 1 << q);
        self.prph_w(SCD_TXFACT, 0xFF);
        for ch in 0..8 {
            self.w(fh_tx_config(ch), TX_CONFIG_ENABLE | TX_CONFIG_CREDIT_ENABLE);
        }
        self.set(FH_TX_CHICKEN_BITS, TX_CHICKEN_SCD_AUTO_RETRY_EN);
        self.prph_clear(APMG_PCIDEV_STT, APMG_PCIDEV_STT_L1_ACT_DIS);
        self.unlock();
        Ok(())
    }

    /// Send one command and wait for its reply (1 s). The reply's data is kept in `reply`.
    fn command(&mut self, code: u8, payload: &[u8]) -> Result<(), &'static str> {
        let idx = self.cmd_cur;
        let cmd = self.mem.cmd.bytes();
        let Some(n) = iwm_rx::command(code, idx as u8, CMD_QUEUE as u8, payload, cmd) else {
            return Err("a command too long for its buffer");
        };
        let Some(t) = iwm_rx::tfd(&[(self.mem.cmd.pa as u64, n)]) else {
            return Err("a command outside the chip's DMA reach");
        };
        self.mem.tfds.bytes()[idx * TFD_SIZE..(idx + 1) * TFD_SIZE].copy_from_slice(&t);
        let bc = self.mem.bc.bytes();
        let qbase = CMD_QUEUE as usize * BC_ENTRIES * 2;
        let e = iwm_rx::byte_count(n).to_le_bytes();
        bc[qbase + idx * 2..qbase + idx * 2 + 2].copy_from_slice(&e);
        if idx < 64 {
            bc[qbase + (256 + idx) * 2..qbase + (256 + idx) * 2 + 2].copy_from_slice(&e);
        }
        fence(Ordering::SeqCst);
        if !self.lock() {
            return Err("no MAC access to send a command");
        }
        self.cmd_cur = (idx + 1) % TX_RING;
        self.w(HBUS_TARG_WRPTR, self.cmd_cur as u32 | (CMD_QUEUE << 8));
        let mut failed = false;
        let got = self.wait(1_000_000, &mut |d: &mut Self, p: &Packet<'static>| {
            if p.is_reply() && p.qid as u32 == CMD_QUEUE && p.idx as usize == idx {
                let k = p.data.len().min(d.reply.len());
                d.reply[..k].copy_from_slice(&p.data[..k]);
                d.reply_len = k;
                failed = p.flags & iwm_rx::CMD_FAILED != 0;
                true
            } else {
                false
            }
        });
        self.unlock();
        match (got, failed) {
            (false, _) => Err("no reply within 1 s"),
            (true, true) => Err("the chip says the command failed"),
            (true, false) => Ok(()),
        }
    }

    /// The first 2 KiB of an NVM section (all the fields we read lie there) into `out`. Returns the bytes read.
    fn nvm_read(&mut self, section: u16, out: &mut [u8; 2048]) -> Result<usize, &'static str> {
        self.command(iwm_rx::NVM_ACCESS_CMD, &iwm_rx::nvm_read_payload(section, 0, 2048))?;
        let r = self.reply;
        match iwm_rx::parse_nvm(&r[..self.reply_len]) {
            Ok(c) if c.section == section && c.offset == 0 => {
                out[..c.bytes.len()].copy_from_slice(c.bytes);
                Ok(c.bytes.len())
            }
            Ok(_) => Err("the answer is for another section or offset"),
            Err(iwm_rx::Why::BadStatus(_)) => Err("the chip says the section cannot be read"),
            Err(_) => Err("an answer the NVM layout does not allow"),
        }
    }

    /// INIT image: BT config, NVM, the calibration. REGULAR image after.
    fn bring_up(&mut self, file: &[u8]) -> bool {
        let fw = match iwm_fw::parse(file) {
            Ok(fw) => fw,
            Err(why) => {
                step!("the firmware file is not one this driver reads: {:?}", why);
                return false;
            }
        };
        self.phy_sku = fw.phy_sku.unwrap_or(0);
        step!(
            "firmware '{}', version {}.{}, {} records; PHY SKU {:#010x}",
            core::str::from_utf8(fw.name()).unwrap_or("?"),
            fw.ver >> 24,
            (fw.ver >> 16) & 0xFF,
            fw.records,
            self.phy_sku
        );
        if fw.num_cpu == Some(2) {
            step!("a firmware for two CPUs; this driver loads one-CPU images only");
            return false;
        }
        if let Err(why) = self.start_hw() {
            step!("power-up: {}", why);
            self.say_regs();
            return false;
        }
        step!("power-up: NIC ready, MAC clock up; radio switch {}", if self.rfkill { "OFF (RF-kill): the firmware still loads, the radio stays off" } else { "on" });
        if self.load_image(file, &fw, Image::Init, "INIT").is_err() {
            return false;
        }
        if let Err(why) = self.post_alive() {
            step!("INIT image: {}", why);
            return false;
        }
        // Bluetooth coexistence (the 7000 family wants it before the NVM): Wi-Fi mode, high-band retention
        if let Err(why) = self.command(iwm_rx::BT_CONFIG, &[3, 0, 0, 0, 0x10, 0, 0, 0]) {
            step!("BT_CONFIG: {}", why);
            return false;
        }
        let (mut hw, mut sw) = ([0u8; 2048], [0u8; 2048]);
        let hn = match self.nvm_read(iwm_rx::NVM_HW, &mut hw) {
            Ok(n) => n,
            Err(why) => {
                step!("NVM hardware section: {}", why);
                return false;
            }
        };
        let sn = match self.nvm_read(iwm_rx::NVM_SW, &mut sw) {
            Ok(n) => n,
            Err(why) => {
                step!("NVM software section: {}", why);
                return false;
            }
        };
        match iwm_rx::parse_nvm_info(&hw[..hn], &sw[..sn]) {
            Ok(info) => {
                let (c24, c5) = info.valid();
                let (tx, rx) = info.antennas();
                step!(
                    "NVM: MAC {:02x}:{:02x}:{:02x}:..:..:.. (the rest is not shown), version {:#06x}, SKU {:#06x}, {} channels at 2.4 GHz and {} at 5 GHz, antennas TX {:#x} RX {:#x}",
                    info.mac[0], info.mac[1], info.mac[2], info.version, info.sku, c24, c5, tx, rx
                );
                self.nvm = Some(info);
            }
            Err(_) => {
                step!("NVM: the sections are too short for the 7000 layout ({} and {} bytes)", hn, sn);
                return false;
            }
        }
        self.nvm_hw = hw;
        self.nvm_sw = sw;
        // smart FIFO off for the INIT image (state 3, everything else 0)
        let mut sf = [0u8; 92];
        sf[0] = 3;
        if let Err(why) = self.command(iwm_rx::REPLY_SF_CFG_CMD, &sf) {
            step!("SF_CFG: {}", why);
            return false;
        }
        // the antennas the firmware and the NVM both allow, then the PHY config with the INIT image's triggers
        let fw_tx = (self.phy_sku >> 16) & 0xF;
        let nvm_tx = self.nvm.map_or(0xF, |n| n.antennas().0 as u32);
        let tx = if fw_tx & nvm_tx != 0 { fw_tx & nvm_tx } else { fw_tx };
        if let Err(why) = self.command(iwm_rx::TX_ANT_CONFIGURATION_CMD, &tx.to_le_bytes()) {
            step!("TX_ANT_CONFIGURATION: {}", why);
            return false;
        }
        let (flow, event) = fw.image(Image::Init).calib.unwrap_or((0, 0));
        let mut phy = [0u8; 12];
        phy[..4].copy_from_slice(&self.phy_sku.to_le_bytes());
        phy[4..8].copy_from_slice(&flow.to_le_bytes());
        phy[8..12].copy_from_slice(&event.to_le_bytes());
        self.init_complete = false;
        self.calib_results = 0;
        if let Err(why) = self.command(iwm_rx::PHY_CONFIGURATION_CMD, &phy) {
            step!("PHY_CONFIGURATION: {}", why);
            return false;
        }
        let done = self.wait(4_000_000, &mut |d: &mut Self, _p: &Packet<'static>| d.init_complete && d.calib_results > 0);
        if !done && !(self.init_complete && self.calib_results > 0) {
            step!(
                "calibration: not finished within 4 s (INIT_COMPLETE {}, {} calibration results)",
                if self.init_complete { "seen" } else { "not seen" },
                self.calib_results
            );
            self.say_regs();
            return false;
        }
        step!("calibration: INIT_COMPLETE, {} calibration results", self.calib_results);
        // the regular image
        self.stop_device();
        if let Err(why) = self.start_hw() {
            step!("power-up for the regular image: {}", why);
            self.say_regs();
            return false;
        }
        if self.load_image(file, &fw, Image::Regular, "REGULAR").is_err() {
            return false;
        }
        if let Err(why) = self.post_alive() {
            step!("REGULAR image: {}", why);
            return false;
        }
        self.up = true;
        step!("the chip is up and idle (scanning comes with W6); {} firmware error reports seen", self.errors);
        true
    }
}

// ---- the commands -------------------------------------------------------------------------------------------------------

fn find() -> Option<((u8, u8, u8), u16)> {
    let at = crate::pci::find_by(255, |_, _, _, id| id & 0xFFFF == 0x8086 && IDS.contains(&((id >> 16) as u16)))?;
    let id = crate::pci::cfg_read32(at.0, at.1, at.2, 0);
    Some((at, (id >> 16) as u16))
}

fn frames_used() -> usize {
    crate::frame::used_frames()
}

/// `wifi up`
pub fn up() {
    if crate::report::capturing() {
        crate::kprintln!("wifi up: refused from the network; run it at this PC");
        return;
    }
    down_quiet();
    log_clear();
    let Some((at, device)) = find() else {
        step!("no Intel Wi-Fi chip AfriSovera drives on this PC (Wireless 7265 or 3165; `wifi` lists the chips)");
        return;
    };
    let (b, d, f) = at;
    let bar = crate::pci::bar_mem(b, d, f, 0);
    if bar == 0 || !crate::mmu::map_mmio(bar, 0x2000, "intel-wifi") {
        step!("8086:{:04x} at {:02x}:{:02x}.{} has no usable BAR0", device, b, d, f);
        return;
    }
    crate::pci::enable(b, d, f, 0x406); // memory decoding, bus mastering, INTx off (polled)
    // SAFETY: shell context, one command at a time.
    unsafe { *addr_of_mut!(FRAMES_BEFORE) = frames_used() };
    let mem = match alloc_all() {
        Ok(m) => m,
        Err(what) => {
            step!("not enough free memory in one piece for {}; nothing was changed", what);
            return;
        }
    };
    let mut dev = Dev {
        bar,
        at,
        device,
        hw_rev: 0,
        mem,
        cmd_cur: 0,
        rx_read: 0,
        rfkill: false,
        phy_sku: 0,
        init_complete: false,
        calib_results: 0,
        errors: 0,
        reply: [0; 2056],
        reply_len: 0,
        nvm_hw: [0; 2048],
        nvm_sw: [0; 2048],
        nvm: None,
        up: false,
    };
    dev.hw_rev = dev.r(CSR_HW_REV);
    let (bad, total, first) = dev.mem.dma_check();
    step!(
        "8086:{:04x} at {:02x}:{:02x}.{}, BAR0 {:#x}, CSR_HW_REV {:#06x} (type {:#05x}, step {}, dash {}); {} frames of DMA memory",
        device, b, d, f, bar, dev.hw_rev, dev.hw_rev & 0xFFF0, (dev.hw_rev >> 2) & 3, dev.hw_rev & 3, dev.mem.frames()
    );
    if bad == 0 {
        step!("dma_check: all {} regions page aligned and below 64 GiB (the chip's 36-bit reach)", total);
    } else {
        step!("dma_check: {} of {} regions outside the chip's reach (first: {}); stopped", bad, total, first);
        dev.mem.free_all();
        return;
    }
    let Some(name) = iwm_rx::firmware_for(device, dev.hw_rev) else {
        step!("no firmware file for this device and revision; stopped");
        dev.mem.free_all();
        return;
    };
    let Some(row) = crate::fwdb::by_name(name) else {
        step!("{} is not in the firmware list; stopped", name);
        dev.mem.free_all();
        return;
    };
    step!("firmware {} (picked by CSR_HW_REV); checking its size and sha256", name);
    let mut from = [0u8; 32];
    let pci = crate::fwdb::Dev::PciId { vendor: 0x8086, device };
    let mut reached = false;
    let r = crate::fwdb::send(row, pci, |file| {
        reached = true;
        dev.bring_up(file)
    }, &mut from);
    if let Err(why) = r {
        if !reached {
            step!("firmware {}: {}", name, why); // the store's own reason: missing, wrong size, wrong hash
        }
    }
    if dev.up {
        // SAFETY: shell context.
        unsafe { *addr_of_mut!(DEV) = Some(dev) };
    } else {
        dev.stop_device();
        dev.mem.free_all();
        step!("stopped; every frame given back ({} used before, {} now)", unsafe { *addr_of!(FRAMES_BEFORE) }, frames_used());
    }
}

fn down_quiet() -> bool {
    // SAFETY: shell context.
    let held = unsafe { (*addr_of_mut!(DEV)).take() };
    match held {
        Some(mut d) => {
            d.stop_device();
            d.mem.free_all();
            true
        }
        None => false,
    }
}

/// `wifi down`
pub fn down() {
    if down_quiet() {
        let (before, now) = (unsafe { *addr_of!(FRAMES_BEFORE) }, frames_used());
        crate::kprintln!(
            "wifi down: the chip is stopped and its DMA memory given back; frames used before wifi up {}, now {}: {}",
            before, now, if before == now { "equal" } else { "NOT equal" }
        );
    } else {
        crate::kprintln!("wifi down: the chip was not up");
    }
}

/// `wifi diag`: the last `wifi up`'s steps again, then the chip's state now.
pub fn diag() {
    // SAFETY: shell context.
    let log = unsafe { &*addr_of!(LOG) };
    if log.n == 0 {
        crate::kprintln!("wifi diag: `wifi up` has not run in this boot");
    } else {
        crate::kprintln!("wifi diag: the last wifi up, step by step:");
        for i in 0..log.n {
            crate::kprintln!("  {}", core::str::from_utf8(&log.text[i][..log.len[i]]).unwrap_or("?"));
        }
    }
    // SAFETY: shell context.
    match unsafe { (*addr_of!(DEV)).as_ref() } {
        Some(d) => {
            let (gp, hw, int, fh) = d.regs();
            crate::kprintln!(
                "wifi diag: up; CSR_HW_REV {:#06x}, GP_CNTRL {:#010x}, HW_IF_CONFIG {:#010x}, INT {:#010x}, FH_INT_STATUS {:#010x}; radio switch {}; {} frames of DMA memory held",
                d.hw_rev, gp, hw, int, fh, if gp & GP_HW_RF_KILL_SW == 0 { "OFF" } else { "on" }, d.mem.frames()
            );
        }
        None => crate::kprintln!("wifi diag: the chip is not up; frames used now {}", frames_used()),
    }
}
