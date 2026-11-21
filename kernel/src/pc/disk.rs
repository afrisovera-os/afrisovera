//! `crate::virtio` on the PC: the shared ARM virtio driver (virtio_core, unchanged), with
//! its five block-device calls widened to the PC's own disks: NVMe (nvme.rs), then SATA
//! (ahci.rs), then a SATA disk in IDE mode (ide.rs), then a USB stick or disk (xhci.rs), then an SD card (sdhci.rs); a built-in eMMC (sdhci.rs) holding our files comes before a USB stick, as an internal disk. The first usable one, in that order after a virtio disk, holds the VFS.
//!
//! The shared VFS and shell reach the disk only through `crate::virtio::{blk_present,
//! read_sector, write_sector, blk_capacity, blk_selftest}`. Defining those five here
//! shadows the glob re-export, so a PC with a virtio disk uses it exactly as before, and a
//! PC without one (every real PC) stores files on its NVMe or SATA disk.

pub use crate::virtio_core::*;

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Safe updates U4: on a TRIAL boot the data partition is read only (pc/trial.rs). The shared VFS asks
/// `blk_present` before every save, so a save says there is nothing to write to; `write_sector` refuses too, as a
/// backstop for any other writer, and counts what it refused.
static READ_ONLY: AtomicBool = AtomicBool::new(false);
static REFUSED: AtomicU64 = AtomicU64::new(0);

pub fn set_read_only(on: bool) {
    READ_ONLY.store(on, Ordering::Relaxed);
}

/// Sector writes refused while on trial.
#[allow(dead_code)]
pub fn refused_writes() -> u64 {
    REFUSED.load(Ordering::Relaxed)
}

pub fn blk_present() -> bool {
    if READ_ONLY.load(Ordering::Relaxed) {
        crate::kprintln!("[pc] trial: the data partition is read only until this version is kept; nothing is saved");
        return false;
    }
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
    if READ_ONLY.load(Ordering::Relaxed) {
        REFUSED.fetch_add(1, Ordering::Relaxed);
        return false;
    }
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

/// AFR-0011 V2: every write acknowledged before this call is on the medium of the disk in use (AFR-0011 section
/// 9). Same order as the calls above. A disk whose driver has no flush yet answers false ("cannot confirm"), never
/// true; each driver's own blk_flush replaces its false in V2.
pub fn blk_flush() -> bool {
    // safe updates U4 (the architect's rule for U4 merging after V2): on a trial boot nothing reached the disk,
    // so nothing can be confirmed on it
    if READ_ONLY.load(Ordering::Relaxed) {
        return false;
    }
    if crate::virtio_core::blk_present() {
        crate::virtio_core::blk_flush()
    } else if crate::nvme::present() {
        crate::nvme::blk_flush()
    } else if crate::ahci::present() {
        crate::ahci::blk_flush()
    } else if crate::ide::present() {
        crate::ide::blk_flush()
    } else if crate::sdhci::internal() {
        crate::sdhci::blk_flush()
    } else if crate::xhci::msc_present() {
        // the driver engineer's msc_sync: the gathered writes, then SYNCHRONIZE CACHE(10) when the stick said WCE; a
        // write failed since the last call makes it Failed (sticky). Unknown and Refused are true here; V3 marks them
        // "cannot confirm saves" (the architect's V2 verdict).
        let r = crate::xhci::msc_sync();
        // SAFETY: scalar write, kernel context.
        unsafe { MSC_LAST = Some(r) };
        r != crate::xhci::MscSync::Failed
    } else {
        crate::sdhci::blk_flush()
    }
}

/// The test rig's `blkflush-hang` (fw_cfg `opt/afrisovera/blkflush-hang`): the next device flush gets no answer, so
/// the driver's timeout and reset path runs (the architect's rule 2). A real PC has no fw_cfg: always false.
pub fn flush_hang_test() -> bool {
    crate::qemu::fw_cfg_file(b"opt/afrisovera/blkflush-hang")
}

/// The USB stick's last `msc_sync` answer, for `blkflush`'s note.
static mut MSC_LAST: Option<crate::xhci::MscSync> = None;

/// A few words on how the last `blk_flush` was confirmed, for `blkflush` (empty for a disk with nothing to add).
pub fn blk_note() -> &'static str {
    use crate::xhci::MscSync::*;
    if blk_name() != "usb" {
        return "";
    }
    // SAFETY: scalar read.
    match unsafe { MSC_LAST } {
        Some(Flushed) => " (SYNCHRONIZE CACHE passed)",
        Some(WriteThrough) => " (the stick has no write cache)",
        Some(Unknown) => " (the stick did not say whether it caches: this disk cannot confirm saves)",
        Some(Refused) => " (the stick refused SYNCHRONIZE CACHE: this disk cannot confirm saves)",
        _ => "",
    }
}

/// Which disk `blk_flush` answers for (for `blkflush`).
pub fn blk_name() -> &'static str {
    if crate::virtio_core::blk_present() {
        "virtio"
    } else if crate::nvme::present() {
        "nvme"
    } else if crate::ahci::present() {
        "ahci"
    } else if crate::ide::present() {
        "ide"
    } else if crate::sdhci::internal() {
        "emmc"
    } else if crate::xhci::msc_present() {
        "usb"
    } else if crate::sdhci::present() {
        "sd"
    } else {
        "none"
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
