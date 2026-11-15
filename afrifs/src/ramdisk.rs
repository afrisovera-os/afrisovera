//! A disk in RAM for the tests and the fuzz targets: sparse (only written sectors take memory), with a log of every
//! write and flush, so the crash harness can rebuild any state a power cut could leave (tests/crash.rs).

use crate::{Disk, SECTOR};
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

/// One event the disk saw.
#[derive(Clone, Debug)]
pub enum Event {
    /// A write of whole sectors starting at `sector`.
    Write { sector: u64, data: Vec<u8> },
    Flush,
}

#[derive(Clone, Default)]
pub struct RamDisk {
    sectors: u64,
    data: BTreeMap<u64, [u8; SECTOR]>,
    /// Every write and flush, when `record` is on.
    pub log: Vec<Event>,
    pub record: bool,
    /// Fail every write from this many writes on (fault injection); None: never.
    pub fail_after_writes: Option<usize>,
    /// Fail every flush (a device that refuses to flush).
    pub fail_flush: bool,
    pub writes: usize,
    pub reads: usize,
}

impl RamDisk {
    pub fn new(sectors: u64) -> RamDisk {
        RamDisk { sectors, ..Default::default() }
    }

    /// The same disk with the given sectors written over it (no log).
    pub fn with_sectors(&self, over: &BTreeMap<u64, [u8; SECTOR]>) -> RamDisk {
        let mut d = RamDisk::new(self.sectors);
        d.data = self.data.clone();
        for (k, v) in over {
            d.data.insert(*k, *v);
        }
        d
    }

    pub fn sector(&self, s: u64) -> [u8; SECTOR] {
        self.data.get(&s).copied().unwrap_or([0u8; SECTOR])
    }

    pub fn set_sector(&mut self, s: u64, v: [u8; SECTOR]) {
        if v.iter().all(|&b| b == 0) {
            self.data.remove(&s);
        } else {
            self.data.insert(s, v);
        }
    }

    /// The written sectors, in order (the image without its zeros).
    pub fn written(&self) -> &BTreeMap<u64, [u8; SECTOR]> {
        &self.data
    }
}

impl Disk for RamDisk {
    fn sectors(&self) -> u64 {
        self.sectors
    }

    fn read(&mut self, sector: u64, buf: &mut [u8]) -> bool {
        self.reads += 1;
        let n = (buf.len() / SECTOR) as u64;
        if buf.len() % SECTOR != 0 || sector.checked_add(n).map_or(true, |e| e > self.sectors) {
            return false;
        }
        for i in 0..n {
            let s = self.sector(sector + i);
            buf[i as usize * SECTOR..(i as usize + 1) * SECTOR].copy_from_slice(&s);
        }
        true
    }

    fn write(&mut self, sector: u64, buf: &[u8]) -> bool {
        let n = (buf.len() / SECTOR) as u64;
        if buf.len() % SECTOR != 0 || sector.checked_add(n).map_or(true, |e| e > self.sectors) {
            return false;
        }
        if let Some(limit) = self.fail_after_writes {
            if self.writes >= limit {
                return false;
            }
        }
        self.writes += 1;
        for i in 0..n {
            let mut s = [0u8; SECTOR];
            s.copy_from_slice(&buf[i as usize * SECTOR..(i as usize + 1) * SECTOR]);
            self.set_sector(sector + i, s);
        }
        if self.record {
            self.log.push(Event::Write { sector, data: buf.to_vec() });
        }
        true
    }

    fn flush(&mut self) -> bool {
        if self.fail_flush {
            return false;
        }
        if self.record {
            self.log.push(Event::Flush);
        }
        true
    }
}
