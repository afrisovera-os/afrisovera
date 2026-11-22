//! The Intel 7000-series Wi-Fi chip's registers and bits (Wi-Fi W5). Facts only, from the published OpenBSD iwm
//! driver's if_iwmreg.h (ISC), written down in the notes folder's WIFI_W5_REGISTERS.md. Offsets are from BAR0.
#![allow(dead_code)]

// ---- CSR --------------------------------------------------------------------------------------------------------
pub const CSR_HW_IF_CONFIG: u32 = 0x000;
pub const CSR_INT_COALESCING: u32 = 0x004;
pub const CSR_INT: u32 = 0x008;
pub const CSR_INT_MASK: u32 = 0x00C;
pub const CSR_FH_INT_STATUS: u32 = 0x010;
pub const CSR_RESET: u32 = 0x020;
pub const CSR_GP_CNTRL: u32 = 0x024;
pub const CSR_HW_REV: u32 = 0x028;
pub const CSR_GIO: u32 = 0x03C;
pub const CSR_UCODE_DRV_GP1_CLR: u32 = 0x05C;
pub const CSR_MBOX_SET: u32 = 0x088;
pub const CSR_MAC_SHADOW_REG_CTRL: u32 = 0x0A8;
pub const CSR_GIO_CHICKEN_BITS: u32 = 0x100;
pub const CSR_DBG_HPET_MEM: u32 = 0x240;
pub const CSR_DBG_LINK_PWR_MGMT: u32 = 0x250;

pub const HW_IF_MAC_SI: u32 = 0x100;
pub const HW_IF_RADIO_SI: u32 = 0x200;
pub const HW_IF_HAP_WAKE_L1A: u32 = 0x0008_0000;
pub const HW_IF_NIC_READY: u32 = 0x0040_0000;
pub const HW_IF_PREPARE: u32 = 0x0800_0000;
/// The fields `nic_config` sets: MAC dash and step, PHY type, dash and step, and the two SI bits.
pub const HW_IF_CONFIG_FIELDS: u32 = 0x3 | 0xC | 0xC00 | 0x3000 | 0xC000 | HW_IF_MAC_SI | HW_IF_RADIO_SI;

pub const INT_FH_RX: u32 = 1 << 31;
pub const INT_HW_ERR: u32 = 1 << 29;
pub const INT_FH_TX: u32 = 1 << 27;
pub const INT_SW_ERR: u32 = 1 << 25;
pub const INT_RF_KILL: u32 = 1 << 7;
pub const INT_ALIVE: u32 = 1 << 0;
pub const INT_INI_SET_MASK: u32 = 0xBA00_008B;

pub const RESET_SW_RESET: u32 = 0x80;
pub const RESET_MASTER_DISABLED: u32 = 0x100;
pub const RESET_STOP_MASTER: u32 = 0x200;
pub const RESET_LINK_PWR_MGMT_DISABLED: u32 = 0x8000_0000;

pub const GP_MAC_CLOCK_READY: u32 = 0x1;
pub const GP_INIT_DONE: u32 = 0x4;
pub const GP_MAC_ACCESS_REQ: u32 = 0x8;
pub const GP_GOING_TO_SLEEP: u32 = 0x10;
/// 0 here means the radio's hardware switch is off (RF-kill).
pub const GP_HW_RF_KILL_SW: u32 = 0x0800_0000;

pub const GIO_L0S: u32 = 0x2;
pub const GIO_CHICKEN_L1A_NO_L0S_RX: u32 = 0x0080_0000;
pub const GIO_CHICKEN_DIS_L0S_EXIT_TIMER: u32 = 0x2000_0000;
pub const HPET_MEM_VAL: u32 = 0xFFFF_0000;
pub const MBOX_OS_ALIVE: u32 = 0x20;
pub const GP1_SW_BIT_RFKILL: u32 = 0x2;
pub const GP1_CMD_BLOCKED: u32 = 0x4;
pub const INT_COALESCING_DEF: u32 = 0x40;
pub const SHADOW_REG_CTRL_ALL: u32 = 0x800F_FFFF;

// ---- HBUS: periphery (PRPH) and chip memory (SRAM), indirect ------------------------------------------------------
pub const HBUS_TARG_MEM_RADDR: u32 = 0x40C;
pub const HBUS_TARG_MEM_WADDR: u32 = 0x410;
pub const HBUS_TARG_MEM_WDAT: u32 = 0x418;
pub const HBUS_TARG_MEM_RDAT: u32 = 0x41C;
pub const HBUS_TARG_PRPH_WADDR: u32 = 0x444;
pub const HBUS_TARG_PRPH_RADDR: u32 = 0x448;
pub const HBUS_TARG_PRPH_WDAT: u32 = 0x44C;
pub const HBUS_TARG_PRPH_RDAT: u32 = 0x450;
pub const HBUS_TARG_WRPTR: u32 = 0x460;
/// A PRPH address as the HBUS registers take it.
pub const fn prph_addr(a: u32) -> u32 {
    (a & 0x000F_FFFF) | (3 << 24)
}

// ---- FH: the DMA engine ---------------------------------------------------------------------------------------------
pub const FH_KW_MEM_ADDR: u32 = 0x197C;
pub const fn fh_cbbc_queue(q: u32) -> u32 {
    0x19D0 + 4 * q // queues 0 to 15
}
pub const SRVC_CHNL: u32 = 9;
pub const fn fh_tfdib_ctrl0(ch: u32) -> u32 {
    0x1900 + 8 * ch
}
pub const fn fh_tfdib_ctrl1(ch: u32) -> u32 {
    0x1904 + 8 * ch
}
pub const FH_SRVC_CHNL_SRAM_ADDR: u32 = 0x19C8;
pub const fn fh_tx_config(ch: u32) -> u32 {
    0x1D00 + 0x20 * ch
}
pub const fn fh_tx_buf_sts(ch: u32) -> u32 {
    0x1D00 + 0x20 * ch + 8
}
pub const TX_CONFIG_ENABLE: u32 = 0x8000_0000;
pub const TX_CONFIG_CREDIT_ENABLE: u32 = 0x8;
pub const TX_CONFIG_CIRQ_HOST_ENDTFD: u32 = 0x0010_0000;
pub const TX_BUF_STS_ONE_TFDB: u32 = (1 << 20) | (1 << 12) | 0x3;
pub const FH_TSSR_TX_STATUS: u32 = 0x1EB0;
pub const FH_TX_CHICKEN_BITS: u32 = 0x1E98;
pub const TX_CHICKEN_SCD_AUTO_RETRY_EN: u32 = 0x2;
/// The most one DMA transfer may carry.
pub const FH_TB_MAX: usize = 0x20000;
/// Firmware loaded into 0x40000..=0x57FFF needs LMPM_CHICK's extended-address bit set while it loads.
pub const FW_MEM_EXTENDED_START: u32 = 0x40000;
pub const FW_MEM_EXTENDED_END: u32 = 0x57FFF;
pub const LMPM_CHICK: u32 = 0xA0_1FF8;
pub const LMPM_CHICK_EXTENDED_ADDR_SPACE: u32 = 0x01;

pub const FH_RSCSR_CHNL0_STTS_WPTR: u32 = 0x1BC0;
pub const FH_RSCSR_CHNL0_RBDCB_BASE: u32 = 0x1BC4;
pub const FH_RSCSR_CHNL0_WPTR: u32 = 0x1BC8;
pub const FH_RCSR_CHNL0_CONFIG: u32 = 0x1C00;
pub const FH_RSSR_RX_STATUS: u32 = 0x1C44;
pub const RSSR_CHNL0_IDLE: u32 = 0x0100_0000;
/// RX channel 0 on: DMA enabled, 2^6 = 64 descriptors, 4 KiB buffers, interrupts to the host (we poll), the
/// receive timeout 0x11, IGNORE_RXF_EMPTY.
pub const RX_RING_LOG: u32 = 6;
pub const RX_CONFIG: u32 = 0x8000_0000 | (RX_RING_LOG << 20) | 0x1000 | (0x11 << 4) | 0x4;

// ---- PRPH ---------------------------------------------------------------------------------------------------------
pub const APMG_CLK_EN: u32 = 0x3004;
pub const APMG_CLK_DIS: u32 = 0x3008;
pub const APMG_PS_CTRL: u32 = 0x300C;
pub const APMG_PCIDEV_STT: u32 = 0x3010;
pub const APMG_RTC_INT_STT: u32 = 0x301C;
pub const APMG_CLK_DMA_CLK_RQT: u32 = 0x200;
pub const APMG_PS_CTRL_MSK_PWR_SRC: u32 = 0x0300_0000;
pub const APMG_PS_CTRL_EARLY_PWR_OFF_RESET_DIS: u32 = 0x0040_0000;
pub const APMG_PCIDEV_STT_L1_ACT_DIS: u32 = 0x800;
pub const APMG_RTC_INT_STT_RFKILL: u32 = 0x1000_0000;

pub const SCD_BASE: u32 = 0xA0_2C00;
pub const SCD_SRAM_BASE_ADDR: u32 = SCD_BASE;
pub const SCD_DRAM_BASE_ADDR: u32 = SCD_BASE + 0x08;
pub const SCD_TXFACT: u32 = SCD_BASE + 0x10;
pub const SCD_GP_CTRL: u32 = SCD_BASE + 0x1A8;
pub const SCD_CHAINEXT_EN: u32 = SCD_BASE + 0x244;
pub const SCD_AGGR_SEL: u32 = SCD_BASE + 0x248;
pub const SCD_EN_CTRL: u32 = SCD_BASE + 0x254;
pub const fn scd_queue_rdptr(q: u32) -> u32 {
    SCD_BASE + 0x68 + 4 * q
}
pub const fn scd_queue_status_bits(q: u32) -> u32 {
    SCD_BASE + 0x10C + 4 * q
}
pub const SCD_GP_ENABLE_31_QUEUES: u32 = 1 << 0;
pub const SCD_GP_AUTO_ACTIVE_MODE: u32 = 1 << 18;
pub const SCD_STTS_ACTIVE: u32 = 3;
pub const SCD_STTS_WSL: u32 = 4;
pub const SCD_STTS_ACT_EN: u32 = 19;
pub const SCD_STTS_REG_MSK: u32 = 0x017F_0000;
/// The scheduler's SRAM: queue contexts (8 bytes each), TX status, the translation table.
pub const SCD_CONTEXT_OFFSET: u32 = 0x600;
pub const SCD_SRAM_CLEAR_END: u32 = 0x808;
pub const SCD_FRAME_LIMIT: u32 = 64;

// ---- the command queue --------------------------------------------------------------------------------------------
/// The legacy command queue and its TX FIFO.
pub const CMD_QUEUE: u32 = 9;
pub const TX_FIFO_CMD: u32 = 7;
/// TFDs in a TX ring (the write pointer register's index is 8 bits).
pub const TX_RING: usize = 256;
pub const TFD_SIZE: usize = 128;
/// Byte-count table: 320 u16 entries per queue, for the 31 queues.
pub const BC_ENTRIES: usize = 320;
pub const BC_QUEUES: usize = 31;
