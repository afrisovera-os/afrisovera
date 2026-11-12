//! `crate::virtio` on the PC: the shared ARM virtio driver (virtio_core, unchanged), with
//! its five block-device calls widened to the PC's own disks: NVMe (nvme.rs), then SATA
//! (ahci.rs), then a SATA disk in IDE mode (ide.rs), then a USB stick or disk (xhci.rs), then an SD card (sdhci.rs); a built-in eMMC (sdhci.rs) holding our files comes before a USB stick, as an internal disk. The first usable one, in that order after a virtio disk, holds the VFS.
//!
//! The shared VFS and shell reach the disk only through `crate::virtio::{blk_present,
//! read_sector, write_sector, blk_capacity, blk_selftest}`. Defining those five here
//! shadows the glob re-export, so a PC with a virtio disk uses it exactly as before, and a
//! PC without one (every real PC) stores files on its NVMe or SATA disk.

pub use crate::virtio_core::*;

pub fn blk_present() -> bool {
    crate::virtio_core::blk_present() || crate::nvme::present() || crate::ahci::present() || crate::ide::present() || crate::xhci::msc_present() || crate::sdhci::present()
}

pub fn blk_capacity() -> u64 {
    if crate::virtio_core::blk_present() {
        crate::virtio_core::blk_capacity()
    } else if crate::nvme::present() {
        crate::nvme::capacity()
    } else if crate::ahci::present() {
        crate::ahci::capacity()
    } else if crate::ide::present() {
        crate::ide::capacity()
    } else if crate::sdhci::internal() {
        crate::sdhci::capacity()
    } else if crate::xhci::msc_present() {
        crate::xhci::msc_capacity()
    } else {
        crate::sdhci::capacity()
    }
}

pub fn read_sector(n: u64, out: &mut [u8; 512]) -> bool {
    if crate::virtio_core::blk_present() {
        crate::virtio_core::read_sector(n, out)
    } else if crate::nvme::present() {
        crate::nvme::read_sector(n, out)
    } else if crate::ahci::present() {
        crate::ahci::read_sector(n, out)
    } else if crate::ide::present() {
        crate::ide::read_sector(n, out)
    } else if crate::sdhci::internal() {
        crate::sdhci::read_sector(n, out)
    } else if crate::xhci::msc_present() {
        crate::xhci::msc_read(n, out)
    } else {
        crate::sdhci::read_sector(n, out)
    }
}

pub fn write_sector(n: u64, data: &[u8; 512]) -> bool {
    if crate::virtio_core::blk_present() {
        crate::virtio_core::write_sector(n, data)
    } else if crate::nvme::present() {
        crate::nvme::write_sector(n, data)
    } else if crate::ahci::present() {
        crate::ahci::write_sector(n, data)
    } else if crate::ide::present() {
        crate::ide::write_sector(n, data)
    } else if crate::sdhci::internal() {
        crate::sdhci::write_sector(n, data)
    } else if crate::xhci::msc_present() {
        crate::xhci::msc_write(n, data)
    } else {
        crate::sdhci::write_sector(n, data)
    }
}

/// The shared write/read-back check, over whichever disk is in use.
pub fn blk_selftest() -> bool {
    if crate::virtio_core::blk_present() {
        return crate::virtio_core::blk_selftest();
    }
    if !crate::nvme::present() && !crate::ahci::present() && !crate::ide::present() && !crate::xhci::msc_present() && !crate::sdhci::present() {
        return false;
    }
    let (rd, wr, name): (fn(u64, &mut [u8; 512]) -> bool, fn(u64, &[u8; 512]) -> bool, &str) =
        if crate::nvme::present() {
            (crate::nvme::read_sector, crate::nvme::write_sector, "nvme")
        } else if crate::ahci::present() {
            (crate::ahci::read_sector, crate::ahci::write_sector, "ahci")
        } else if crate::ide::present() {
            (crate::ide::read_sector, crate::ide::write_sector, "ide")
        } else if crate::sdhci::internal() {
            (crate::sdhci::read_sector, crate::sdhci::write_sector, "sdhci")
        } else if crate::xhci::msc_present() {
            (crate::xhci::msc_read, crate::xhci::msc_write, "usb-storage")
        } else {
            (crate::sdhci::read_sector, crate::sdhci::write_sector, "sdhci")
        };
    let sector = 100u64;
    let mut pat = [0u8; 512];
    for (i, b) in pat.iter_mut().enumerate() {
        *b = (i as u8) ^ 0x5a;
    }
    let mut rb = [0u8; 512];
    let ok = wr(sector, &pat) && rd(sector, &mut rb) && rb == pat;
    crate::kprintln!(
        "[{}] blk self-test {} (wrote+read sector {})",
        name,
        if ok { "OK" } else { "FAILED" },
        sector
    );
    ok
}

// Sound: the shared calls `audio`, `say` and Nia's `speak()` use, widened the same way to
// a USB headset or speaker (xhci.rs, preferred when plugged in, as other systems switch to
// it) and the PC's own Intel HD Audio chip (hda.rs) when there is no virtio-sound device.

pub fn sound_present() -> bool {
    crate::virtio_core::sound_present() || crate::xhci::audio_present() || crate::hda::present()
}

pub fn sound_stream_begin() -> bool {
    if crate::virtio_core::sound_present() {
        crate::virtio_core::sound_stream_begin()
    } else if crate::xhci::audio_present() {
        crate::xhci::audio_stream_begin()
    } else {
        crate::hda::stream_begin()
    }
}

pub fn sound_stream_write(samples: &[i16]) -> bool {
    if crate::virtio_core::sound_present() {
        crate::virtio_core::sound_stream_write(samples)
    } else if crate::xhci::audio_present() {
        crate::xhci::audio_stream_write(samples)
    } else {
        crate::hda::stream_write(samples)
    }
}

pub fn sound_stream_end() -> bool {
    if crate::virtio_core::sound_present() {
        crate::virtio_core::sound_stream_end()
    } else if crate::xhci::audio_present() {
        crate::xhci::audio_stream_end()
    } else {
        crate::hda::stream_end()
    }
}

#[allow(dead_code)]
pub fn sound_play_pcm(samples: &[i16]) -> bool {
    sound_stream_begin() && sound_stream_write(samples) && sound_stream_end()
}

pub fn sound_play_tone() -> bool {
    if crate::virtio_core::sound_present() {
        crate::virtio_core::sound_play_tone()
    } else if crate::xhci::audio_present() {
        crate::xhci::audio_tone()
    } else {
        crate::hda::tone()
    }
}
