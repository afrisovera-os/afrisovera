//! The scan table (W2): the networks heard, one entry per BSSID, newest information kept, a fixed number of
//! entries (no allocation), aged out when not heard for a while. Pure: the caller gives the time in milliseconds.
#![allow(dead_code)]

use super::frame::{BssInfo, Mac};

pub const MAX_BSS: usize = 64;

#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub info: BssInfo,
    /// signal strength in dBm as the radio reported it (for example -45 strong, -85 weak)
    pub signal: i8,
    pub last_seen_ms: u64,
}

pub struct Table {
    slots: [Option<Entry>; MAX_BSS],
}

impl Table {
    pub const fn new() -> Self {
        Table { slots: [None; MAX_BSS] }
    }

    pub fn clear(&mut self) {
        self.slots = [None; MAX_BSS];
    }

    /// Record a beacon or probe response. A known BSSID is updated in place; a new one takes a free slot, or the
    /// slot of the weakest network heard if the table is full and the new one is stronger. Returns false if it
    /// was dropped.
    pub fn update(&mut self, info: BssInfo, signal: i8, now_ms: u64) -> bool {
        let e = Entry { info, signal, last_seen_ms: now_ms };
        if let Some(s) = self.slots.iter_mut().flatten().find(|s| s.info.bssid == info.bssid) {
            *s = e;
            return true;
        }
        if let Some(s) = self.slots.iter_mut().find(|s| s.is_none()) {
            *s = Some(e);
            return true;
        }
        let weakest = self
            .slots
            .iter_mut()
            .flatten()
            .min_by_key(|s| s.signal)
            .filter(|s| s.signal < signal);
        match weakest {
            Some(w) => {
                *w = e;
                true
            }
            None => false,
        }
    }

    /// Forget every network not heard for more than `max_age_ms`.
    pub fn age(&mut self, now_ms: u64, max_age_ms: u64) {
        for s in self.slots.iter_mut() {
            if matches!(s, Some(e) if now_ms.saturating_sub(e.last_seen_ms) > max_age_ms) {
                *s = None;
            }
        }
    }

    pub fn get(&self, bssid: &Mac) -> Option<&Entry> {
        self.slots.iter().flatten().find(|s| s.info.bssid == *bssid)
    }

    pub fn len(&self) -> usize {
        self.slots.iter().flatten().count()
    }

    /// The entries, strongest first, written into `out`; returns how many.
    pub fn sorted<'a>(&'a self, out: &mut [Option<&'a Entry>; MAX_BSS]) -> usize {
        let mut n = 0;
        for e in self.slots.iter().flatten() {
            out[n] = Some(e);
            n += 1;
        }
        // insertion sort by signal, strongest first (n is at most 64)
        for i in 1..n {
            let mut j = i;
            while j > 0 && out[j - 1].map_or(i8::MIN, |e| e.signal) < out[j].map_or(i8::MIN, |e| e.signal) {
                out.swap(j - 1, j);
                j -= 1;
            }
        }
        n
    }

    /// The strongest network with this name (exact bytes), if heard.
    pub fn best_for(&self, ssid: &[u8]) -> Option<&Entry> {
        self.slots.iter().flatten().filter(|s| s.info.ssid() == ssid).max_by_key(|s| s.signal)
    }
}

impl Default for Table {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::super::frame::{BssInfo, Security, RATES_MAX, RSN_MAX};
    use super::*;

    fn info(last: u8, ssid: &[u8]) -> BssInfo {
        let mut s = [0u8; 32];
        s[..ssid.len()].copy_from_slice(ssid);
        BssInfo {
            bssid: [2, 0, 0, 0, 0, last],
            ssid: s,
            ssid_len: ssid.len(),
            channel: 1,
            beacon_int: 100,
            capability: 1,
            security: Security::Open,
            rsn: [0; RSN_MAX],
            rsn_len: 0,
            rates: [0; RATES_MAX],
            n_rates: 0,
            n_basic_rates: 0,
        }
    }

    #[test]
    fn update_dedupe_age_sort() {
        let mut t = Table::new();
        assert!(t.update(info(1, b"a"), -70, 0));
        assert!(t.update(info(2, b"b"), -40, 0));
        assert!(t.update(info(1, b"a"), -60, 10)); // same BSSID: updated, not added
        assert_eq!(t.len(), 2);
        assert_eq!(t.get(&[2, 0, 0, 0, 0, 1]).unwrap().signal, -60);
        let mut out = [None; MAX_BSS];
        let n = t.sorted(&mut out);
        assert_eq!(n, 2);
        assert_eq!(out[0].unwrap().info.ssid(), b"b");
        t.age(5000, 1000);
        assert_eq!(t.len(), 0);
    }

    #[test]
    fn full_table_keeps_the_strongest() {
        let mut t = Table::new();
        for i in 0..MAX_BSS {
            assert!(t.update(info(i as u8, b"x"), -80, 0));
        }
        assert!(!t.update(info(200, b"weak"), -90, 0), "a weaker newcomer is dropped");
        assert!(t.update(info(201, b"strong"), -30, 0), "a stronger newcomer replaces the weakest");
        assert_eq!(t.len(), MAX_BSS);
        assert_eq!(t.best_for(b"strong").unwrap().signal, -30);
    }
}
