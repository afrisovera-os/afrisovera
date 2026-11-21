//! Safe updates U4 (AFR-0010 section 8 steps 3 to 5; architect's OK, ARCHITECT_REVIEWS 14 Nov 2026): `updatetry`,
//! the trial boot, keep, `updatekeep`, `updateback` and `updatesetting`.
//!
//! - `updatetry` (on the main system) writes BootNext = the trial entry and restarts: the firmware starts the staged
//!   version ONCE.
//! - A boot is a TRIAL when, and only when, BootCurrent is the trial entry. On trial the data partition is READ ONLY
//!   (pc/disk.rs refuses), and the person is told so.
//! - Healthy = the boot self-tests passed, 2 minutes since power-on with the shell or the desk up, and a key or
//!   pointer event from the person. Then, with `updatesetting auto` (the default) the trial keeps itself; with
//!   `updatesetting ask` it waits for `updatekeep`.
//! - Keep: the main entry rewritten to name the new file FIRST, then the trial entry removed, then the record; then
//!   writes are allowed again and the person's changes since power-on are saved once.
//! - A failed self-test: the record says so, BootNext = the MAIN entry, restart: the old AfriSovera comes back and
//!   names what happened. A hang or a crash: the firmware's own next choice starts; the next main boot finds the
//!   "trial started" record with no outcome, says "version N did not start", and removes the trial entry.
//! - The record `AfriUpdate` (our own firmware variable) holds each slot's version, data format and SHA-256 and the
//!   last trial's state. It is advice only: every action recomputes the SHA from the EFI partition.

use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use crate::update::{self, RELEASE};

/// The data partition's format this kernel writes: 1 = AFRIVFS1 as on master. A2's format 2 (checksums, in unused
/// bytes) must set 2 when it merges.
pub const DATA_FORMAT: u16 = 1;

/// Whether a kernel that writes format `old` can read data in format `cur` (AFR-0010's data rule). Format 2 keeps
/// its checksums where format 1 ignores them (docs/vfs-format.md), so a format-1 kernel reads it. Format 3 (VFS V3,
/// AFRIVFS2) is read by no version before V3: `old >= cur` already says so.
fn readable_by(old: u16, cur: u16) -> bool {
    old >= cur || (old == 1 && cur == 2)
}

/// The data format of one volume's first sector: AFRIVFS1 is 1 (2 when byte 13 says 2: V2's checksums), AFRIVFS2 is
/// 3 (VFS V3); anything else 0.
fn format_of(s: &[u8; 512]) -> u16 {
    if &s[..8] == b"AFRIVFS2" {
        3
    } else if &s[..8] == b"AFRIVFS1" {
        if s[13] == 2 { 2 } else { 1 }
    } else {
        0
    }
}

/// The newest data format actually on this PC's internal disks (a whole-disk volume, or AfriSovera's partition), read
/// raw and read only. The drivers use only an AFRIVFS1 volume, so a disk V3 has turned into AFRIVFS2 is invisible to
/// this kernel's file system; it is read here directly (the architect's guard, 15 Nov 2026).
pub(crate) fn disk_format() -> u16 {
    let (ds, n) = crate::gpt::disks();
    let mut best = 0;
    for &(dk, cap, bs) in &ds[..n] {
        let mut s = [0u8; 512];
        if dk.read(0, &mut s) {
            best = best.max(format_of(&s));
        }
        if let Some(a) = crate::gpt::ours_first(dk, cap, bs) {
            if dk.read(a, &mut s) {
                best = best.max(format_of(&s));
            }
        }
    }
    best
}

// --- the record ---

const REC_NAME: &[u8] = b"AfriUpdate";
const REC_MAGIC: &[u8; 8] = b"AFRUPD01";
const REC_LEN: usize = 96;

/// The last trial's state.
#[derive(Clone, Copy, PartialEq)]
#[repr(u8)]
enum TrialState {
    None = 0,
    Started = 1,
    Kept = 2,
    FailedSelfTest = 3,
    /// the trial boot started and ran (written once at its power-on), not kept yet
    Ran = 4,
}

#[derive(Clone, Copy, Default)]
struct Slot {
    known: bool,
    version: u32,
    format: u16, // 0: not known yet (written by the kernel itself when it is kept)
    sha: [u8; 32],
}

#[derive(Clone, Copy)]
struct Rec {
    found: bool,
    ask: bool,
    state: TrialState,
    trial_slot: u8,
    trial_version: u32,
    slot: [Slot; 2],
}

fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

impl Rec {
    fn empty() -> Rec {
        Rec { found: false, ask: false, state: TrialState::None, trial_slot: 0, trial_version: 0, slot: [Slot::default(); 2] }
    }

    fn read() -> Rec {
        let mut b = [0u8; REC_LEN];
        let mut r = Rec::empty();
        match crate::uefi::get_ours(REC_NAME, &mut b) {
            Ok(n) if n == REC_LEN && &b[..8] == REC_MAGIC => {}
            _ => return r,
        }
        r.found = true;
        r.ask = b[8] == 1;
        r.state = match b[9] {
            1 => TrialState::Started,
            2 => TrialState::Kept,
            3 => TrialState::FailedSelfTest,
            4 => TrialState::Ran,
            _ => TrialState::None,
        };
        r.trial_slot = b[10] & 1;
        r.trial_version = u32le(&b, 12);
        for (k, s) in r.slot.iter_mut().enumerate() {
            let o = 16 + 40 * k;
            s.known = b[o + 6] == 1;
            s.version = u32le(&b, o);
            s.format = u16::from_le_bytes([b[o + 4], b[o + 5]]);
            s.sha.copy_from_slice(&b[o + 8..o + 40]);
        }
        r
    }

    /// Written once; a refusal (a full variable store) is said plainly and the update goes on without it.
    fn write(&self, cmd: &str) -> bool {
        let mut b = [0u8; REC_LEN];
        b[..8].copy_from_slice(REC_MAGIC);
        b[8] = self.ask as u8;
        b[9] = self.state as u8;
        b[10] = self.trial_slot;
        b[12..16].copy_from_slice(&self.trial_version.to_le_bytes());
        for (k, s) in self.slot.iter().enumerate() {
            let o = 16 + 40 * k;
            b[o..o + 4].copy_from_slice(&s.version.to_le_bytes());
            b[o + 4..o + 6].copy_from_slice(&s.format.to_le_bytes());
            b[o + 6] = s.known as u8;
            b[o + 8..o + 40].copy_from_slice(&s.sha);
        }
        let st = crate::uefi::set_ours(REC_NAME, &b);
        if st != crate::uefi::SUCCESS {
            crate::kprintln!(
                "{}: the firmware refused to keep AfriSovera's update record (status {:#x}; its variable store may be full); the update goes on without it, so versions cannot be named afterwards",
                cmd,
                st
            );
            return false;
        }
        true
    }
}

/// The slot index of a file the entries name (lower-case path): 0 afrisovera.efi, 1 afrisovera-b.efi.
fn slot_of(file: &[u8]) -> Option<usize> {
    match file {
        b"\\efi\\afrisovera\\afrisovera.efi" => Some(0),
        b"\\efi\\afrisovera\\afrisovera-b.efi" => Some(1),
        _ => None,
    }
}

/// The long name and the path of slot `k`.
fn slot_names(k: usize) -> (&'static str, &'static str) {
    if k == 0 {
        (crate::esp::stage::SLOT_A, "\\EFI\\afrisovera\\afrisovera.efi")
    } else {
        (crate::esp::stage::SLOT_B, "\\EFI\\afrisovera\\afrisovera-b.efi")
    }
}

/// For `updatestage` (U3): the record learns the spare slot's version and SHA, and the running slot's own version,
/// format and SHA if it did not know them. Advice only.
pub(crate) fn note_staged(spare_slot: usize, version: u32, sha: &[u8; 32], running_sha: Option<[u8; 32]>) {
    let mut r = Rec::read();
    r.slot[spare_slot] = Slot { known: true, version, format: 0, sha: *sha };
    let run = 1 - spare_slot;
    if let Some(s) = running_sha {
        r.slot[run] = Slot { known: true, version: RELEASE, format: DATA_FORMAT, sha: s };
    }
    if matches!(r.state, TrialState::Started | TrialState::FailedSelfTest | TrialState::Ran) {
        r.state = TrialState::None;
    }
    r.write("updatestage");
}

// --- this boot ---

/// 0: not a trial; 1: a trial, not decided; 2: kept; 3: waiting for updatekeep (asked once); 4: failed.
static TRIAL: AtomicU8 = AtomicU8::new(0);
/// The boot self-tests' verdict: 0 not yet, 1 passed, 2 failed.
static VERDICT: AtomicU8 = AtomicU8::new(0);
/// A key or pointer event from the person since the self-tests.
static USED: AtomicBool = AtomicBool::new(false);

/// The test switch (compile time only, never at run time; the release guard refuses it): AFRI_TEST_FAIL.
const TEST_FAIL: &[u8] = match option_env!("AFRI_TEST_FAIL") {
    Some(s) => s.as_bytes(),
    None => b"",
};

const fn same(a: &[u8], b: &[u8]) -> bool {
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

// AFR-0010 section 7 and the architect's U4 Q5: test switches never reach a release build.
#[cfg(feature = "release")]
const _: () = assert!(
    TEST_FAIL.is_empty() && option_env!("AFRI_RELEASE").is_none(),
    "release build refused: a test switch (AFRI_TEST_FAIL or AFRI_RELEASE) is set (AFR-0010 section 7)"
);

/// Whether this boot is a trial (data read only).
pub fn on_trial() -> bool {
    matches!(TRIAL.load(Ordering::Relaxed), 1 | 3 | 4)
}

/// Right after update::boot_line: the test-only early crash (an empty IDT, then a fault: a triple fault).
pub fn early() {
    if same(TEST_FAIL, b"triplefault") {
        crate::kprintln!("[pc] TEST BUILD: AFRI_TEST_FAIL=triplefault, faulting now");
        // SAFETY: deliberate, test builds only: an IDT of length 0 makes the next exception a triple fault.
        unsafe {
            let idt: [u8; 10] = [0; 10];
            core::arch::asm!("lidt [{}]", "int3", in(reg) idt.as_ptr());
        }
    }
}

/// From main.rs: the boot self-tests' verdict.
pub fn verdict(ok: bool) {
    let ok = ok && !same(TEST_FAIL, b"selftest");
    if same(TEST_FAIL, b"selftest") {
        crate::kprintln!("[pc] TEST BUILD: AFRI_TEST_FAIL=selftest, the verdict is FAILED");
    }
    VERDICT.store(if ok { 1 } else { 2 }, Ordering::Relaxed);
}

/// A key or pointer event from the person (uart::inject, xhci::mouse_report; COM1 does not count).
pub fn input() {
    if VERDICT.load(Ordering::Relaxed) != 0 {
        USED.store(true, Ordering::Relaxed);
    }
}

/// After the data partition is loaded, before anything can be saved: decide trial or not, and act on the record.
pub fn at_boot() {
    let cur = crate::bootent::current();
    let trial = crate::bootent::trial_number();
    let mut r = Rec::read();
    if cur.is_some() && cur == trial {
        TRIAL.store(1, Ordering::Relaxed);
        crate::virtio::set_read_only(true);
        crate::kprintln!(
            "[pc] TRIAL of version {}: nothing you change is saved until this version is kept (updatekeep); a restart now goes back",
            RELEASE
        );
        crate::desk::notify("Trial of a new AfriSovera version: nothing you change is saved until it is kept.");
        if VERDICT.load(Ordering::Relaxed) == 2 {
            fail_selftest(&mut r);
        }
        // it started: so a later main boot says "not kept", not "did not start"
        if r.state == TrialState::Started {
            r.state = TrialState::Ran;
            r.write("[pc] trial");
        }
        return;
    }
    // the main system (or a stick): what the last trial left
    let Ok(m) = crate::bootent::main_exact() else { return };
    if cur != Some(m.num) {
        return;
    }
    match r.state {
        TrialState::Started => {
            crate::kprintln!(
                "[pc] updates: version {} did not start; this is still version {}. Its trial entry is taken out; the file stays staged (updatetry tries again, updatestage remove takes it out)",
                r.trial_version,
                RELEASE
            );
            let _ = crate::bootent::trial_remove();
            r.state = TrialState::None;
            r.write("[pc] updates");
        }
        TrialState::Ran => {
            crate::kprintln!(
                "[pc] updates: the trial of version {} ended without being kept (a restart or a power cut); this is still version {}; what was done during it was not saved; updatetry tries it again",
                r.trial_version,
                RELEASE
            );
            r.state = TrialState::None;
            r.write("[pc] updates");
        }
        TrialState::FailedSelfTest => {
            crate::kprintln!(
                "[pc] updates: the trial of version {} failed its self-tests; this is still version {} (updatestage remove takes it out)",
                r.trial_version,
                RELEASE
            );
            r.state = TrialState::None;
            r.write("[pc] updates");
        }
        _ => {}
    }
    // a power cut between the two writes of a keep leaves a trial entry naming the main file: taken out
    if let Some((f, n)) = crate::bootent::trial_file() {
        if f[..n] == m.file[..m.file_n] {
            crate::kprintln!("[pc] updates: a trial entry naming the running file was left by a cut-off keep; taken out");
            let _ = crate::bootent::trial_remove();
        }
    }
}

/// A trial whose self-tests failed: record it, BootNext = the main entry, restart (no automatic second try).
fn fail_selftest(r: &mut Rec) {
    TRIAL.store(4, Ordering::Relaxed);
    crate::kprintln!("[pc] TRIAL of version {}: the self-tests FAILED; not kept; going back to the main system", RELEASE);
    r.state = TrialState::FailedSelfTest;
    r.trial_version = RELEASE;
    r.write("[pc] trial");
    if let Ok(m) = crate::bootent::main_exact() {
        let st = crate::uefi::set_global(b"BootNext", &m.num.to_le_bytes());
        if st != crate::uefi::SUCCESS {
            crate::kprintln!("[pc] trial: BootNext could not be written (status {:#x}); the firmware's usual first choice starts", st);
        }
    }
    crate::kprintln!("[pc] trial: restarting");
    crate::psci::reboot();
}

/// The trial window: 2 minutes; the test rig (fw_cfg, never on a real PC) shortens it to 5 seconds.
fn window_ticks() -> u64 {
    if crate::qemu::fw_cfg_file(b"opt/afrisovera/trial-short") { 500 } else { 12_000 }
}

/// From the shell's idle loop (pc/uart.rs note_idle_wfi): keep when healthy.
pub fn idle() {
    if TRIAL.load(Ordering::Relaxed) != 1 {
        return;
    }
    if VERDICT.load(Ordering::Relaxed) != 1 || !USED.load(Ordering::Relaxed) || crate::sched::ticks() < window_ticks() {
        return;
    }
    if Rec::read().ask {
        TRIAL.store(3, Ordering::Relaxed);
        crate::kprintln!("\n[pc] TRIAL of version {} runs well: updatekeep keeps it; a restart goes back", RELEASE);
        crate::desk::notify("The new version runs well: type updatekeep to keep it, or restart to go back.");
        return;
    }
    keep("[pc] trial");
}

/// Keep: the main entry first, then the trial entry out, then the record; then writes allowed and the person's
/// changes saved once.
fn keep(cmd: &str) -> bool {
    let m = match crate::bootent::main_exact() {
        Ok(m) => m,
        Err(_) => {
            crate::kprintln!("{}: AfriSovera's main boot entry is not there exactly once; not kept", cmd);
            TRIAL.store(3, Ordering::Relaxed);
            return false;
        }
    };
    let Some((f, n)) = crate::bootent::trial_file() else {
        crate::kprintln!("{}: the trial entry is gone; not kept", cmd);
        TRIAL.store(3, Ordering::Relaxed);
        return false;
    };
    let Some(k) = slot_of(&f[..n]) else {
        crate::kprintln!("{}: the trial entry names a file that is not one of AfriSovera's two; not kept", cmd);
        TRIAL.store(3, Ordering::Relaxed);
        return false;
    };
    let (_, path) = slot_names(k);
    if let Err(why) = crate::bootent::main_rewrite(&m, path) {
        crate::kprintln!("{}: {}; not kept (a restart goes back)", cmd, why);
        TRIAL.store(3, Ordering::Relaxed);
        return false;
    }
    // test builds only (AFRI_TEST_FAIL=keepcut): the power cut between the keep's two writes, made exactly
    if same(TEST_FAIL, b"keepcut") {
        crate::kprintln!("[pc] TEST BUILD: AFRI_TEST_FAIL=keepcut, restarting between the keep's two writes");
        crate::psci::reboot();
    }
    let _ = crate::bootent::trial_remove();
    let mut r = Rec::read();
    let back = r.slot[1 - k];
    let sha = r.slot[k].sha;
    r.slot[k] = Slot { known: true, version: RELEASE, format: DATA_FORMAT, sha };
    r.state = TrialState::Kept;
    r.trial_version = RELEASE;
    r.write(cmd);
    TRIAL.store(2, Ordering::Relaxed);
    crate::virtio::set_read_only(false);
    let saved = crate::vfs::sync_to_disk();
    if back.known {
        crate::kprintln!("{}: updated to version {}; updateback returns to version {}", cmd, RELEASE, back.version);
    } else {
        crate::kprintln!("{}: updated to version {}; updateback returns to the version before it", cmd, RELEASE);
    }
    crate::kprintln!(
        "{}: {}",
        cmd,
        if saved { "your changes since this start are now saved" } else { "your changes since this start could not be saved (no data partition)" }
    );
    crate::desk::notify("AfriSovera is updated. Your changes since this start are saved.");
    true
}

// --- the commands ---

/// The SHA-256 of a slot's file read from the EFI partition now, and the partition, for try and back.
fn slot_sha(cmd: &str, m: &crate::bootent::Main, k: usize) -> Option<(crate::esp::stage::Esp, [u8; 32], u64)> {
    let esp = match crate::esp::stage::open(&m.guid) {
        Ok(e) => e,
        Err(why) => {
            crate::kprintln!("{}: {}; nothing written", cmd, why);
            return None;
        }
    };
    let (long, _) = slot_names(k);
    let live = match esp.live(long) {
        Ok(Some(l)) => l,
        Ok(None) => {
            crate::kprintln!("{}: {} is not in \\EFI\\afrisovera; nothing written", cmd, long);
            return None;
        }
        Err(why) => {
            crate::kprintln!("{}: \\EFI\\afrisovera: {}; nothing written", cmd, why);
            return None;
        }
    };
    match esp.read_back(long, live.size as u64) {
        Ok((sha, _)) => Some((esp, sha, live.size as u64)),
        Err(why) => {
            crate::kprintln!("{}: {}; nothing written", cmd, why);
            None
        }
    }
}

/// `updatetry [yes <code>]`
pub fn try_(given: Option<&str>) {
    const CMD: &str = "updatetry";
    if on_trial() {
        crate::kprintln!("{}: this boot is itself a trial; nothing written", CMD);
        return;
    }
    let Some((m, _spare, spare_path)) = update::main_and_spare(CMD) else { return };
    let Some(tnum) = crate::bootent::trial_number() else {
        crate::kprintln!("{}: nothing is staged (no trial entry; updatestage stages an update); nothing written", CMD);
        return;
    };
    let Some((tf, tn)) = crate::bootent::trial_file() else { return };
    let k = match slot_of(&tf[..tn]) {
        Some(k) if spare_path.eq_ignore_ascii_case(core::str::from_utf8(&tf[..tn]).unwrap_or("")) => k,
        _ => {
            crate::kprintln!("{}: the trial entry does not name the spare file; nothing written (updatestage again puts it right)", CMD);
            return;
        }
    };
    let r = Rec::read();
    let Some((esp, sha, _)) = slot_sha(CMD, &m, k) else { return };
    if !r.slot[k].known || r.slot[k].sha != sha {
        crate::kprintln!("{}: the staged file does not match what updatestage checked and recorded; nothing written (updatestage again)", CMD);
        return;
    }
    let v = r.slot[k].version;
    crate::kprintln!(
        "{}: the next start runs version {} ONCE, as a trial; the data is read only while it runs, and it is kept only when it starts well and you use it",
        CMD,
        v
    );
    // the architect's Q1: say before it happens what starts if the new version does not
    if let Some((first, d, dn)) = crate::bootent::first_choice() {
        if first != m.num {
            crate::kprintln!(
                "  if version {} fails to start, this PC starts \"{}\" instead; choose AfriSovera in the firmware's boot menu to come back",
                v,
                core::str::from_utf8(&d[..dn]).unwrap_or("?")
            );
        }
    }
    let Some(want) = esp.code(CMD.as_bytes(), v, &sha) else {
        crate::kprintln!("{}: a read FAILED; nothing written", CMD);
        return;
    };
    let want = want ^ (tnum as u32 & 0xFFFF);
    if !crate::esp::agreed(CMD, given, want) {
        return;
    }
    let mut r = r;
    r.state = TrialState::Started;
    r.trial_slot = k as u8;
    r.trial_version = v;
    r.write(CMD);
    let st = crate::uefi::set_global(b"BootNext", &tnum.to_le_bytes());
    let mut nb = [0u8; 2];
    if st != crate::uefi::SUCCESS || crate::uefi::get_global(b"BootNext", &mut nb) != Ok(2) || u16::from_le_bytes(nb) != tnum {
        crate::kprintln!("{}: BootNext could not be written (status {:#x}); nothing will start the trial", CMD, st);
        let mut r = Rec::read();
        r.state = TrialState::None;
        r.write(CMD);
        return;
    }
    crate::vfs::sync_to_disk();
    crate::kprintln!("{}: BootNext = the trial entry, read back; restarting now", CMD);
    crate::psci::reboot();
}

/// `updatekeep [yes <code>]` (on a trial waiting with `updatesetting ask`, or any healthy trial)
pub fn keep_cmd(given: Option<&str>) {
    const CMD: &str = "updatekeep";
    if !on_trial() || TRIAL.load(Ordering::Relaxed) == 4 {
        crate::kprintln!("{}: this boot is not a trial; nothing to keep", CMD);
        return;
    }
    if VERDICT.load(Ordering::Relaxed) != 1 {
        crate::kprintln!("{}: the self-tests did not pass; not kept", CMD);
        return;
    }
    let want = 0x4b00 | (RELEASE & 0xFF);
    crate::kprintln!("{}: version {} becomes this PC's AfriSovera; updateback returns to the version before it", CMD, RELEASE);
    if !crate::esp::agreed(CMD, given, want) {
        return;
    }
    keep(CMD);
}

/// `updateback [yes <code>]`: the main entry names the other boot file again (the owner's deliberate downgrade).
pub fn back(given: Option<&str>) {
    const CMD: &str = "updateback";
    if on_trial() {
        crate::kprintln!("{}: this boot is a trial; a restart goes back; nothing written", CMD);
        return;
    }
    let Some((m, _spare, spare_path)) = update::main_and_spare(CMD) else { return };
    let k = if spare_path.ends_with("-b.efi") { 1 } else { 0 };
    let r = Rec::read();
    let s = r.slot[k];
    if !s.known {
        crate::kprintln!("{}: the version in the other slot is not known (no record); nothing written", CMD);
        return;
    }
    // the architect's data guard: the older version must be able to read today's data, as it is ON THE DISK (a newer
    // file system than this kernel writes can already be there)
    let cur = DATA_FORMAT.max(disk_format());
    if cur >= 3 && s.format != 0 && !readable_by(s.format, cur) {
        crate::kprintln!(
            "{}: REFUSED: this disk now holds the new file system, which version {} cannot read; nothing written",
            CMD,
            s.version
        );
        return;
    }
    if s.format == 0 || !readable_by(s.format, cur) {
        crate::kprintln!(
            "{}: REFUSED: version {} writes data format {} and cannot read this PC's data in format {}; nothing written",
            CMD,
            s.version,
            s.format,
            cur
        );
        return;
    }
    let Some((esp, sha, _)) = slot_sha(CMD, &m, k) else { return };
    if sha != s.sha {
        crate::kprintln!("{}: REFUSED: the other boot file is not the version {} that was kept; nothing written", CMD, s.version);
        return;
    }
    crate::kprintln!(
        "{}: the next start runs version {} again (this is version {}); updateback again returns to version {}",
        CMD,
        s.version,
        RELEASE,
        RELEASE
    );
    let Some(want) = esp.code(CMD.as_bytes(), s.version, &sha) else {
        crate::kprintln!("{}: a read FAILED; nothing written", CMD);
        return;
    };
    if !crate::esp::agreed(CMD, given, want) {
        return;
    }
    if crate::bootent::trial_number().is_some() {
        let _ = crate::bootent::trial_remove();
    }
    match crate::bootent::main_rewrite(&m, spare_path) {
        Ok(()) => crate::kprintln!("{}: AfriSovera's boot entry now names {}, read back; it takes effect at the next restart", CMD, spare_path),
        Err(why) => crate::kprintln!("{}: {}", CMD, why),
    }
}

/// `updatesetting [auto|ask]`: whether a healthy trial keeps itself (auto, the default) or waits for updatekeep.
pub fn setting(arg: &str) {
    let mut r = Rec::read();
    match arg {
        "" => crate::kprintln!("updatesetting: {}", if r.ask { "ask (a healthy trial waits for updatekeep)" } else { "auto (a healthy trial keeps itself)" }),
        "auto" | "ask" => {
            r.ask = arg == "ask";
            if r.write("updatesetting") {
                crate::kprintln!("updatesetting: now {}", arg);
            }
        }
        _ => crate::kprintln!("updatesetting: use  updatesetting auto  or  updatesetting ask"),
    }
}
