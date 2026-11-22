//! The Linux calls of stage 0 (COMPAT_STAGE0_PLAN section 16a) and of stage L1.1 (COMPAT_L1_PLAN 4: memory,
//! identity, time, signals), as pure logic over a `Host`, so every call and every error is tested on the host with a
//! fake one. The real Host (bridge.rs) is the kernel: fproc_read, fproc_write, fproc_set_tls, fproc_map, fproc_unmap,
//! fproc_protect, the clock, sleep and the console.
//!
//! Numbers, arguments and errors from the Linux manual pages read(2), write(2), open(2), close(2), fstat(2),
//! lseek(2), ioctl_tty(2), writev(2), exit(2), arch_prctl(2), set_tid_address(2), exit_group(2), openat(2), brk(2),
//! mmap(2), munmap(2), mprotect(2), getpid(2), gettid(2), getuid(2), uname(2), clock_gettime(2), gettimeofday(2),
//! nanosleep(2), clock_nanosleep(2), rt_sigaction(2), sigprocmask(2), sigaltstack(2), sched_getaffinity(2),
//! sched_yield(2), getrlimit(2), clock_getres(2), and errno(3) for the values. No Linux source was read (the clean-room rule).

use crate::cpio;
use crate::mem;

/// What the bridge can do to the foreign process and the world. The memory calls go through `mem::Mapper`.
pub trait Host: mem::Mapper {
    /// Copy from the program's memory (fproc_read); Err on any unmapped page.
    fn read_mem(&mut self, addr: u64, buf: &mut [u8]) -> Result<(), ()>;
    /// Copy into the program's memory (fproc_write); Err on any page not writable.
    fn write_mem(&mut self, addr: u64, buf: &[u8]) -> Result<(), ()>;
    /// Output on fd 1 or 2 (the console's out1 and out2 lines).
    fn out(&mut self, fd: u8, bytes: &[u8]);
    /// The program's FS base (fproc_set_tls); Err if the kernel refused it.
    fn set_fs(&mut self, value: u64) -> Result<(), ()>;
    /// Nanoseconds since boot, whole microseconds, never backwards (call 79, CLOCK_PLAN).
    fn mono_ns(&mut self) -> u64;
    /// Nanoseconds since 1970: the clock read once at the bridge's start plus the monotonic time since.
    fn real_ns(&mut self) -> u64;
    /// Sleep about `ms` milliseconds (the kernel's sleep call).
    fn sleep_ms(&mut self, ms: u64);
    /// One console line from the bridge (a named refusal the program itself cannot see).
    fn note(&mut self, line: &[u8]);
}

/// The ids a program sees: fixed (COMPAT_L1_PLAN 4). The Linux reference runs as an unprivileged user with the same
/// uid and gid; the pid and tid are 1, the one process of L1.
pub const PID: i64 = 1;
pub const UID: i64 = 1000;
/// uname(2)'s six fields of 65 bytes. The release names the layer, so a program that prints it says where it runs.
pub const UNAME: [&[u8]; 6] = [b"Linux", b"afrisovera", b"6.1.0-daraja", b"#1 Daraja L1", b"x86_64", b"(none)"];

/// What happens after a call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// put this in rax and run on (a value, or -errno)
    Ret(i64),
    /// end the program with this code
    Exit(u64),
}

// errno(3)
pub const ENOENT: i64 = 2;
pub const EBADF: i64 = 9;
pub const EFAULT: i64 = 14;
pub const EINVAL: i64 = 22;
pub const EMFILE: i64 = 24;
pub const ENOTTY: i64 = 25;
pub const ESPIPE: i64 = 29;
pub const EROFS: i64 = 30;
pub const ENOSYS: i64 = 38;
pub const ENAMETOOLONG: i64 = 36;

const MAX_FDS: usize = 16;
const AT_FDCWD: i64 = -100;
const TIOCGWINSZ: u64 = 0x5413;
const ARCH_SET_FS: u64 = 0x1002;
const O_ACCMODE: u64 = 3;
const O_CREAT: u64 = 0o100;
const O_TRUNC: u64 = 0o1000;
/// Bytes moved per piece: reads and writes are cut into pieces of this size (the buffer lives on the stack).
const CHUNK: usize = 512;

/// One open bundle file: its bytes and the position.
#[derive(Clone, Copy)]
pub struct File<'a> {
    data: &'a [u8],
    pos: usize,
}

/// The program's open files. 0, 1 and 2 are the standard ones and never in the table.
pub struct Files<'a> {
    bundle: &'a [u8],
    open: [Option<File<'a>>; MAX_FDS],
}

impl<'a> Files<'a> {
    pub fn new(bundle: &'a [u8]) -> Self {
        Files { bundle, open: [None; MAX_FDS] }
    }
}

fn file<'f, 'a>(files: &'f mut Files<'a>, fd: u64) -> Option<&'f mut File<'a>> {
    let i = usize::try_from(fd).ok()?;
    if i < 3 {
        return None;
    }
    files.open.get_mut(i)?.as_mut()
}

/// Read a NUL-terminated path from the program, at most 4096 bytes.
fn read_path(h: &mut dyn Host, addr: u64, out: &mut [u8; 4096]) -> Result<usize, i64> {
    let mut n = 0usize;
    while n < out.len() {
        let mut b = [0u8; 1];
        h.read_mem(addr.checked_add(n as u64).ok_or(-EFAULT)?, &mut b).map_err(|_| -EFAULT)?;
        if b[0] == 0 {
            return Ok(n);
        }
        out[n] = b[0];
        n += 1;
    }
    Err(-ENAMETOOLONG)
}

fn open(h: &mut dyn Host, files: &mut Files, path: u64, flags: u64) -> i64 {
    let mut raw = [0u8; 4096];
    let n = match read_path(h, path, &mut raw) {
        Ok(n) => n,
        Err(e) => return e,
    };
    if flags & O_ACCMODE != 0 || flags & (O_CREAT | O_TRUNC) != 0 {
        return -EROFS; // the bundle is read-only (stage 0)
    }
    let mut norm = [0u8; 4096];
    let Some(name) = cpio::normalise(&raw[..n], &mut norm) else { return -ENOENT };
    let data = match cpio::find(files.bundle, name) {
        Some(cpio::Entry::File(d)) => d,
        Some(cpio::Entry::Dir) => &[][..], // a folder opens (reads give EISDIR on Linux; stage 0 reads 0 bytes)
        None => return -ENOENT,
    };
    match (3..MAX_FDS).find(|&i| files.open[i].is_none()) {
        Some(i) => {
            files.open[i] = Some(File { data, pos: 0 });
            i as i64
        }
        None => -EMFILE,
    }
}

fn read(h: &mut dyn Host, files: &mut Files, fd: u64, buf: u64, count: u64) -> i64 {
    if fd == 0 {
        return 0; // an empty input, as /dev/null
    }
    let Some(f) = file(files, fd) else { return -EBADF };
    let left = f.data.len() - f.pos;
    let n = (count.min(left as u64)) as usize;
    let mut done = 0usize;
    while done < n {
        let k = (n - done).min(CHUNK);
        let piece = &f.data[f.pos + done..f.pos + done + k];
        if h.write_mem(buf + done as u64, piece).is_err() {
            if done == 0 {
                return -EFAULT;
            }
            break; // a partial read: what was copied counts
        }
        done += k;
    }
    f.pos += done;
    done as i64
}

fn write(h: &mut dyn Host, fd: u64, buf: u64, count: u64) -> i64 {
    if fd != 1 && fd != 2 {
        return -EBADF; // bundle files are read-only; fd 0 is an input
    }
    let mut done = 0u64;
    let mut piece = [0u8; CHUNK];
    while done < count {
        let k = (count - done).min(CHUNK as u64) as usize;
        if h.read_mem(buf + done, &mut piece[..k]).is_err() {
            if done == 0 {
                return -EFAULT;
            }
            break; // a partial write: what was written counts
        }
        h.out(fd as u8, &piece[..k]);
        done += k as u64;
    }
    done as i64
}

fn writev(h: &mut dyn Host, fd: u64, iov: u64, cnt: u64) -> i64 {
    if cnt > 1024 {
        return -EINVAL; // IOV_MAX
    }
    let mut total = 0i64;
    for i in 0..cnt {
        let mut e = [0u8; 16];
        if h.read_mem(iov + i * 16, &mut e).is_err() {
            return if total == 0 { -EFAULT } else { total };
        }
        let base = u64::from_le_bytes(e[0..8].try_into().unwrap_or([0; 8]));
        let len = u64::from_le_bytes(e[8..16].try_into().unwrap_or([0; 8]));
        let r = write(h, fd, base, len);
        if r < 0 {
            return if total == 0 { r } else { total };
        }
        total += r;
    }
    total
}

/// readv(2): each buffer filled in turn by `read`, stopping at the first short read (as musl's stdio expects: its
/// fread reads into the caller's buffer and its own in one call). Written from readv(2).
fn readv(h: &mut dyn Host, files: &mut Files, fd: u64, iov: u64, cnt: u64) -> i64 {
    if cnt > 1024 {
        return -EINVAL; // IOV_MAX
    }
    let mut total = 0i64;
    for i in 0..cnt {
        let mut e = [0u8; 16];
        if h.read_mem(iov + i * 16, &mut e).is_err() {
            return if total == 0 { -EFAULT } else { total };
        }
        let base = u64::from_le_bytes(e[0..8].try_into().unwrap_or([0; 8]));
        let len = u64::from_le_bytes(e[8..16].try_into().unwrap_or([0; 8]));
        let r = read(h, files, fd, base, len);
        if r < 0 {
            return if total == 0 { r } else { total };
        }
        total += r;
        if (r as u64) < len {
            break; // a short read: the end of the file, or all there is for now
        }
    }
    total
}

/// Answer one call. `a` holds the six arguments in Linux's order (rdi, rsi, rdx, r10, r8, r9).
pub fn handle(h: &mut dyn Host, files: &mut Files, m: &mut mem::Memory, nr: u64, a: [u64; 6]) -> Action {
    let r = match nr {
        // memory (L1.1): the bookkeeping is mem.rs; the kernel's fproc calls do the pages
        9 => m.mmap(h, a[0], a[1], a[2], a[3], a[4] as i64),
        10 => {
            let r = m.mprotect(h, a[0], a[1], a[2]);
            if m.back_from_none {
                m.back_from_none = false;
                h.note(b"[daraja] mprotect back from PROT_NONE is not supported: the pages were freed");
            }
            r
        }
        11 => m.munmap(h, a[0], a[1]),
        12 => m.brk(h, a[0]) as i64,
        // signals (L1.1): recorded as accepted, never delivered; a crash is still a FAULT
        13 => zero_out(h, a[2], 32),  // rt_sigaction: the old action, if asked, is "default"
        14 => zero_out(h, a[2], 8),   // rt_sigprocmask: the old mask, if asked, is empty
        131 => {
            // sigaltstack: none set; the old one, if asked, says SS_DISABLE (2)
            let mut ss = [0u8; 24];
            ss[8..12].copy_from_slice(&2u32.to_le_bytes());
            if a[1] != 0 && h.write_mem(a[1], &ss).is_err() { -EFAULT } else { 0 }
        }
        // identity (L1.1): fixed values
        39 | 186 => PID, // getpid, gettid
        110 => 0,        // getppid: started by the kernel, as a process started by init would see 0
        102 | 104 | 107 | 108 => UID, // getuid, getgid, geteuid, getegid
        63 => uname(h, a[0]),
        204 => {
            // sched_getaffinity(pid, len, mask): one CPU; Linux gives the bytes written, a multiple of 8
            if a[1] < 8 {
                -EINVAL
            } else {
                let mut mask = [0u8; 8];
                mask[0] = 1;
                if h.write_mem(a[2], &mask).is_err() { -EFAULT } else { 8 }
            }
        }
        302 => prlimit(h, a[1], a[3]),
        24 => 0, // sched_yield
        // time (call 79, CLOCK_PLAN 3): whole microseconds; realtime is the start's clock plus the monotonic time
        228 => match clock_ns(h, a[0]) {
            Some(ns) => if h.write_mem(a[1], &timespec(ns)).is_err() { -EFAULT } else { 0 },
            None => -EINVAL,
        },
        // clock_getres: 1 microsecond for every clock this bridge knows (the coarse-timer rule)
        229 => match clock_ns(h, a[0]) {
            Some(_) => if a[1] != 0 && h.write_mem(a[1], &timespec(1_000)).is_err() { -EFAULT } else { 0 },
            None => -EINVAL,
        },
        96 => {
            let ns = h.real_ns();
            let mut tv = [0u8; 16];
            tv[0..8].copy_from_slice(&(ns / 1_000_000_000).to_le_bytes());
            tv[8..16].copy_from_slice(&(ns % 1_000_000_000 / 1_000).to_le_bytes());
            if a[0] != 0 && h.write_mem(a[0], &tv).is_err() {
                -EFAULT
            } else if a[1] != 0 && h.write_mem(a[1], &[0u8; 8]).is_err() {
                -EFAULT
            } else {
                0
            }
        }
        35 => sleep(h, a[0], None),
        // clock_nanosleep(clock, flags, req, rem): TIMER_ABSTIME is bit 0; an unknown clock is EINVAL either way
        230 => match clock_ns(h, a[0]) {
            None => -EINVAL,
            Some(_) => sleep(h, a[2], if a[1] & 1 != 0 { Some(a[0]) } else { None }),
        },
        0 => read(h, files, a[0], a[1], a[2]),
        1 => write(h, a[0], a[1], a[2]),
        2 => open(h, files, a[0], a[1]),
        3 => match usize::try_from(a[0]).ok().filter(|&i| (3..MAX_FDS).contains(&i)) {
            Some(i) if files.open[i].is_some() => {
                files.open[i] = None;
                0
            }
            _ if a[0] < 3 => 0, // closing a standard fd: allowed, and it stays usable in stage 0
            _ => -EBADF,
        },
        5 => fstat(h, files, a[0], a[1]),
        8 => match file(files, a[0]) {
            Some(f) => {
                let base = match a[2] {
                    0 => 0i64,
                    1 => f.pos as i64,
                    2 => f.data.len() as i64,
                    _ => return Action::Ret(-EINVAL),
                };
                let to = base.checked_add(a[1] as i64).filter(|&v| v >= 0);
                match to {
                    Some(v) => {
                        f.pos = (v as usize).min(f.data.len());
                        v
                    }
                    None => -EINVAL,
                }
            }
            None if a[0] < 3 => -ESPIPE,
            None => -EBADF,
        },
        16 => {
            let _ = TIOCGWINSZ; // any request on any fd: not a terminal, as Linux says when output is a pipe or file
            -ENOTTY
        }
        19 => readv(h, files, a[0], a[1], a[2]),
        20 => writev(h, a[0], a[1], a[2]),
        60 | 231 => return Action::Exit(a[0] & 0xFF),
        158 => {
            if a[0] == ARCH_SET_FS {
                if h.set_fs(a[1]).is_ok() { 0 } else { -EINVAL }
            } else {
                -EINVAL
            }
        }
        218 => 1, // set_tid_address: the one thread's id
        257 => {
            if a[0] as i64 == AT_FDCWD {
                open(h, files, a[1], a[2])
            } else {
                -EBADF
            }
        }
        _ => -ENOSYS,
    };
    Action::Ret(r)
}

/// Write `len` zero bytes at `addr` if `addr` is not 0: 0, or -EFAULT.
fn zero_out(h: &mut dyn Host, addr: u64, len: usize) -> i64 {
    let z = [0u8; 32];
    if addr != 0 && h.write_mem(addr, &z[..len.min(32)]).is_err() { -EFAULT } else { 0 }
}

/// uname(2): six NUL-padded fields of 65 bytes.
fn uname(h: &mut dyn Host, out: u64) -> i64 {
    let mut u = [0u8; 390];
    for (i, f) in UNAME.iter().enumerate() {
        u[i * 65..i * 65 + f.len()].copy_from_slice(f);
    }
    if h.write_mem(out, &u).is_err() { -EFAULT } else { 0 }
}

/// prlimit64(pid, resource, new, old): fixed limits (the old ones written if asked); setting one is accepted and
/// changes nothing (L1 has no limit to enforce beyond the pair's own caps).
fn prlimit(h: &mut dyn Host, res: u64, old: u64) -> i64 {
    const INF: u64 = u64::MAX;
    let (cur, max) = match res {
        3 => (8 << 20, INF),         // RLIMIT_STACK: 8 MiB, as a Linux default
        7 => (MAX_FDS as u64, MAX_FDS as u64), // RLIMIT_NOFILE: the bridge's own table
        _ if res < 16 => (INF, INF),
        _ => return -EINVAL,
    };
    let mut l = [0u8; 16];
    l[0..8].copy_from_slice(&cur.to_le_bytes());
    l[8..16].copy_from_slice(&max.to_le_bytes());
    if old != 0 && h.write_mem(old, &l).is_err() { -EFAULT } else { 0 }
}

/// The clocks of clock_gettime(2), clock_getres(2) and clock_nanosleep(2), by their Linux ids (a C int, so the low 32
/// bits): REALTIME (0), REALTIME_COARSE (5), REALTIME_ALARM (8) and TAI (11) are the bridge's realtime; MONOTONIC (1),
/// MONOTONIC_RAW (4), MONOTONIC_COARSE (6), BOOTTIME (7) and BOOTTIME_ALARM (9) are call 79; PROCESS_CPUTIME_ID (2)
/// and THREAD_CPUTIME_ID (3) are call 79 too, for now (one process, one thread in L1). Any other id: None (EINVAL).
fn clock_ns(h: &mut dyn Host, clk: u64) -> Option<u64> {
    match clk as u32 as i32 {
        0 | 5 | 8 | 11 => Some(h.real_ns()),
        1 | 2 | 3 | 4 | 6 | 7 | 9 => Some(h.mono_ns()),
        _ => None,
    }
}

/// A Linux `struct timespec` (seconds, nanoseconds; little-endian) for `ns`.
fn timespec(ns: u64) -> [u8; 16] {
    let mut ts = [0u8; 16];
    ts[0..8].copy_from_slice(&(ns / 1_000_000_000).to_le_bytes());
    ts[8..16].copy_from_slice(&(ns % 1_000_000_000).to_le_bytes());
    ts
}

/// nanosleep(req, rem) and clock_nanosleep's sleep: the request read from the program, slept in milliseconds
/// (rounded up, so a sleep is never shorter than asked). With `absolute` (a clock id) the request is a time on that
/// clock, slept until then.
fn sleep(h: &mut dyn Host, req: u64, absolute: Option<u64>) -> i64 {
    let mut ts = [0u8; 16];
    if h.read_mem(req, &mut ts).is_err() {
        return -EFAULT;
    }
    let sec = u64::from_le_bytes(ts[0..8].try_into().unwrap_or([0; 8]));
    let nsec = u64::from_le_bytes(ts[8..16].try_into().unwrap_or([0; 8]));
    if nsec >= 1_000_000_000 || sec > i64::MAX as u64 {
        return -EINVAL;
    }
    let want = sec.saturating_mul(1_000_000_000).saturating_add(nsec);
    let ns = match absolute {
        Some(clk) => want.saturating_sub(clock_ns(h, clk).unwrap_or(want)),
        None => want,
    };
    let ms = ns.div_ceil(1_000_000);
    if ms > 0 {
        h.sleep_ms(ms);
    }
    0
}

/// fstat: a 144-byte struct stat (x86-64 layout from the stat(3type) manual page): only st_mode, st_size, st_blksize
/// and st_nlink are filled in; the rest 0.
fn fstat(h: &mut dyn Host, files: &mut Files, fd: u64, out: u64) -> i64 {
    let (mode, size) = if fd < 3 {
        (0o020620u32, 0u64) // a character device, as a console
    } else {
        match file(files, fd) {
            Some(f) => (0o100444u32, f.data.len() as u64),
            None => return -EBADF,
        }
    };
    let mut st = [0u8; 144];
    st[16..24].copy_from_slice(&1u64.to_le_bytes()); // st_nlink
    st[24..28].copy_from_slice(&mode.to_le_bytes()); // st_mode
    st[48..56].copy_from_slice(&size.to_le_bytes()); // st_size
    st[56..64].copy_from_slice(&4096u64.to_le_bytes()); // st_blksize
    if h.write_mem(out, &st).is_err() {
        return -EFAULT;
    }
    0
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    /// A fake foreign process: one flat memory from `base`, the console as two byte lists, the FS base.
    struct Fake {
        base: u64,
        mem: Vec<u8>,
        out1: Vec<u8>,
        out2: Vec<u8>,
        fs: u64,
        slept: u64,
        now: u64,
        notes: Vec<Vec<u8>>,
        pages: std::collections::BTreeMap<u64, u64>,
    }
    impl Host for Fake {
        fn read_mem(&mut self, addr: u64, buf: &mut [u8]) -> Result<(), ()> {
            let i = addr.checked_sub(self.base).ok_or(())? as usize;
            let s = self.mem.get(i..i + buf.len()).ok_or(())?;
            buf.copy_from_slice(s);
            Ok(())
        }
        fn write_mem(&mut self, addr: u64, buf: &[u8]) -> Result<(), ()> {
            let i = addr.checked_sub(self.base).ok_or(())? as usize;
            let s = self.mem.get_mut(i..i + buf.len()).ok_or(())?;
            s.copy_from_slice(buf);
            Ok(())
        }
        fn out(&mut self, fd: u8, bytes: &[u8]) {
            if fd == 1 { self.out1.extend_from_slice(bytes) } else { self.out2.extend_from_slice(bytes) }
        }
        fn set_fs(&mut self, value: u64) -> Result<(), ()> {
            self.fs = value;
            Ok(())
        }
        fn mono_ns(&mut self) -> u64 {
            self.now += 1_000;
            self.now
        }
        fn real_ns(&mut self) -> u64 {
            1_700_000_000_000_000_000 + self.mono_ns()
        }
        fn sleep_ms(&mut self, ms: u64) {
            self.slept += ms;
            self.now += ms * 1_000_000;
        }
        fn note(&mut self, line: &[u8]) {
            self.notes.push(line.to_vec());
        }
    }
    /// The pages, as the kernel would hold them: their rights by address.
    impl mem::Mapper for Fake {
        fn map(&mut self, a: u64, len: u64) -> Result<(), ()> {
            for p in (a..a + len).step_by(4096) {
                self.pages.insert(p, mem::PROT_READ | mem::PROT_WRITE);
            }
            Ok(())
        }
        fn unmap(&mut self, a: u64, len: u64) -> Result<(), ()> {
            for p in (a..a + len).step_by(4096) {
                self.pages.remove(&p);
            }
            Ok(())
        }
        fn protect(&mut self, a: u64, len: u64, prot: u64) -> Result<(), ()> {
            for p in (a..a + len).step_by(4096) {
                self.pages.insert(p, prot);
            }
            Ok(())
        }
    }
    fn memory() -> mem::Memory {
        mem::Memory::new(0x100_0000_0000, 0x100_0040_5000, 0x100_3FE0_0000)
    }
    const B: u64 = 0x100_0050_0000;
    fn fake() -> Fake {
        Fake {
            base: B,
            mem: std::vec![0; 8192],
            out1: Vec::new(),
            out2: Vec::new(),
            fs: 0,
            slept: 0,
            now: 5_000_000_000,
            notes: Vec::new(),
            pages: Default::default(),
        }
    }
    fn put(f: &mut Fake, at: u64, s: &[u8]) {
        f.write_mem(B + at, s).unwrap();
    }

    fn bundle() -> Vec<u8> {
        let mut a = Vec::new();
        let mut entry = |name: &str, mode: u32, data: &[u8]| {
            let h = std::format!(
                "070701{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}",
                1, mode, 0, 0, 1, 0, data.len(), 0, 0, 0, 0, name.len() + 1, 0
            );
            a.extend_from_slice(h.as_bytes());
            a.extend_from_slice(name.as_bytes());
            a.push(0);
            while a.len() % 4 != 0 { a.push(0) }
            a.extend_from_slice(data);
            while a.len() % 4 != 0 { a.push(0) }
        };
        entry("data", 0o040755, b"");
        entry("data/greeting.txt", 0o100644, b"Habari\n");
        entry("TRAILER!!!", 0, b"");
        a
    }

    #[test]
    fn write_and_writev_reach_the_console() {
        let mut h = fake();
        let b = bundle();
        let mut files = Files::new(&b);
        let mut m = memory();
        put(&mut h, 0, b"hi\n");
        assert_eq!(handle(&mut h, &mut files, &mut m, 1, [1, B, 3, 0, 0, 0]), Action::Ret(3));
        // writev: two pieces from an iovec at +64
        put(&mut h, 100, b"ab");
        put(&mut h, 200, b"cd");
        let mut iov = Vec::new();
        for (base, len) in [(B + 100, 2u64), (B + 200, 2u64)] {
            iov.extend_from_slice(&base.to_le_bytes());
            iov.extend_from_slice(&len.to_le_bytes());
        }
        put(&mut h, 64, &iov);
        assert_eq!(handle(&mut h, &mut files, &mut m, 20, [2, B + 64, 2, 0, 0, 0]), Action::Ret(4));
        assert_eq!((h.out1.as_slice(), h.out2.as_slice()), (&b"hi\n"[..], &b"abcd"[..]));
        assert_eq!(handle(&mut h, &mut files, &mut m, 1, [5, B, 3, 0, 0, 0]), Action::Ret(-EBADF));
        assert_eq!(handle(&mut h, &mut files, &mut m, 1, [1, 0x10, 3, 0, 0, 0]), Action::Ret(-EFAULT));
    }

    #[test]
    fn open_read_lseek_close_from_the_bundle() {
        let mut h = fake();
        let b = bundle();
        let mut files = Files::new(&b);
        let mut m = memory();
        put(&mut h, 0, b"/data/../data/greeting.txt\0"); // ".." is refused by the bundle reader
        assert_eq!(handle(&mut h, &mut files, &mut m, 2, [B, 0, 0, 0, 0, 0]), Action::Ret(-ENOENT));
        put(&mut h, 0, b"data/greeting.txt\0");
        let fd = match handle(&mut h, &mut files, &mut m, 257, [(-100i64) as u64, B, 0, 0, 0, 0]) {
            Action::Ret(fd) => fd,
            other => panic!("{:?}", other),
        };
        assert_eq!(fd, 3);
        assert_eq!(handle(&mut h, &mut files, &mut m, 0, [3, B + 1000, 100, 0, 0, 0]), Action::Ret(7));
        assert_eq!(&h.mem[1000..1007], b"Habari\n");
        assert_eq!(handle(&mut h, &mut files, &mut m, 0, [3, B + 1000, 100, 0, 0, 0]), Action::Ret(0)); // at the end
        assert_eq!(handle(&mut h, &mut files, &mut m, 8, [3, 2, 0, 0, 0, 0]), Action::Ret(2)); // SEEK_SET 2
        assert_eq!(handle(&mut h, &mut files, &mut m, 0, [3, B + 1000, 3, 0, 0, 0]), Action::Ret(3));
        assert_eq!(&h.mem[1000..1003], b"bar");
        // readv (musl's fread): two buffers, 3 then 100 bytes; the second read is short, so readv stops there
        assert_eq!(handle(&mut h, &mut files, &mut m, 8, [3, 0, 0, 0, 0, 0]), Action::Ret(0));
        put(&mut h, 3000, &(B + 1100).to_le_bytes());
        put(&mut h, 3008, &3u64.to_le_bytes());
        put(&mut h, 3016, &(B + 1200).to_le_bytes());
        put(&mut h, 3024, &100u64.to_le_bytes());
        assert_eq!(handle(&mut h, &mut files, &mut m, 19, [3, B + 3000, 2, 0, 0, 0]), Action::Ret(7));
        assert_eq!((&h.mem[1100..1103], &h.mem[1200..1204]), (&b"Hab"[..], &b"ari\n"[..]));
        assert_eq!(handle(&mut h, &mut files, &mut m, 19, [3, B + 3000, 2000, 0, 0, 0]), Action::Ret(-EINVAL));
        assert_eq!(handle(&mut h, &mut files, &mut m, 8, [1, 0, 0, 0, 0, 0]), Action::Ret(-ESPIPE));
        assert_eq!(handle(&mut h, &mut files, &mut m, 3, [3, 0, 0, 0, 0, 0]), Action::Ret(0));
        assert_eq!(handle(&mut h, &mut files, &mut m, 0, [3, B + 1000, 3, 0, 0, 0]), Action::Ret(-EBADF));
        // write flags: read-only bundle
        assert_eq!(handle(&mut h, &mut files, &mut m, 2, [B, 1, 0, 0, 0, 0]), Action::Ret(-EROFS));
        assert_eq!(handle(&mut h, &mut files, &mut m, 2, [B, 0o100, 0, 0, 0, 0]), Action::Ret(-EROFS));
        // stdin is empty
        assert_eq!(handle(&mut h, &mut files, &mut m, 0, [0, B, 10, 0, 0, 0]), Action::Ret(0));
    }

    #[test]
    fn the_table_fills_and_fstat_reports() {
        let mut h = fake();
        let b = bundle();
        let mut files = Files::new(&b);
        let mut m = memory();
        put(&mut h, 0, b"data/greeting.txt\0");
        for _ in 3..MAX_FDS {
            assert!(matches!(handle(&mut h, &mut files, &mut m, 2, [B, 0, 0, 0, 0, 0]), Action::Ret(fd) if fd >= 3));
        }
        assert_eq!(handle(&mut h, &mut files, &mut m, 2, [B, 0, 0, 0, 0, 0]), Action::Ret(-EMFILE));
        assert_eq!(handle(&mut h, &mut files, &mut m, 5, [3, B + 2000, 0, 0, 0, 0]), Action::Ret(0));
        let size = u64::from_le_bytes(h.mem[2048..2056].try_into().unwrap());
        let mode = u32::from_le_bytes(h.mem[2024..2028].try_into().unwrap());
        assert_eq!((size, mode & 0o170000), (7, 0o100000));
    }

    #[test]
    fn the_rest_of_the_table() {
        let mut h = fake();
        let b = bundle();
        let mut files = Files::new(&b);
        let mut m = memory();
        assert_eq!(handle(&mut h, &mut files, &mut m, 16, [1, 0x5413, B, 0, 0, 0]), Action::Ret(-ENOTTY));
        assert_eq!(handle(&mut h, &mut files, &mut m, 158, [0x1002, B + 64, 0, 0, 0, 0]), Action::Ret(0));
        assert_eq!(h.fs, B + 64);
        assert_eq!(handle(&mut h, &mut files, &mut m, 158, [0x1003, 0, 0, 0, 0, 0]), Action::Ret(-EINVAL));
        assert_eq!(handle(&mut h, &mut files, &mut m, 218, [B, 0, 0, 0, 0, 0]), Action::Ret(1));
        assert_eq!(handle(&mut h, &mut files, &mut m, 231, [0x107, 0, 0, 0, 0, 0]), Action::Exit(7));
        assert_eq!(handle(&mut h, &mut files, &mut m, 60, [3, 0, 0, 0, 0, 0]), Action::Exit(3));
        assert_eq!(handle(&mut h, &mut files, &mut m, 999, [0; 6]), Action::Ret(-ENOSYS));
        assert_eq!(handle(&mut h, &mut files, &mut m, 257, [5, B, 0, 0, 0, 0]), Action::Ret(-EBADF));
    }

    fn call(h: &mut Fake, m: &mut mem::Memory, nr: u64, a: [u64; 6]) -> i64 {
        let b = bundle();
        let mut files = Files::new(&b);
        match handle(h, &mut files, m, nr, a) {
            Action::Ret(v) => v,
            Action::Exit(c) => panic!("exit {}", c),
        }
    }

    #[test]
    fn l11_memory_calls_reach_the_pages() {
        let (mut h, mut m) = (fake(), memory());
        let b0 = call(&mut h, &mut m, 12, [0; 6]) as u64; // brk(0)
        assert_eq!(b0, 0x100_0040_5000);
        assert_eq!(call(&mut h, &mut m, 12, [b0 + 10_000, 0, 0, 0, 0, 0]) as u64, b0 + 10_000);
        assert_eq!(h.pages.len(), 3);
        // mmap(0, 8192, RW, PRIVATE|ANONYMOUS, -1, 0), then mprotect to R, then munmap
        let a = call(&mut h, &mut m, 9, [0, 8192, 3, 0x22, u64::MAX, 0]);
        assert!(a > 0, "mmap {}", a);
        assert_eq!(call(&mut h, &mut m, 10, [a as u64, 4096, 1, 0, 0, 0]), 0);
        assert_eq!(h.pages[&(a as u64)], 1);
        assert_eq!(call(&mut h, &mut m, 10, [a as u64, 4096, 3, 0, 0, 0]), -13); // adding WRITE: EACCES
        assert_eq!(call(&mut h, &mut m, 10, [a as u64 + 4096, 4096, 7, 0, 0, 0]), -13); // adding EXEC
        assert_eq!(call(&mut h, &mut m, 10, [a as u64 + 4096, 4096, 0, 0, 0, 0]), 0); // to none
        assert_eq!(call(&mut h, &mut m, 10, [a as u64 + 4096, 4096, 1, 0, 0, 0]), -12); // back from none
        assert_eq!(h.notes.len(), 1, "the named line for back-from-none");
        assert!(h.notes[0].ends_with(b"the pages were freed"));
        assert_eq!(call(&mut h, &mut m, 11, [a as u64, 8192, 0, 0, 0, 0]), 0);
        assert_eq!(h.pages.len(), 3); // the brk pages only
        assert_eq!(call(&mut h, &mut m, 9, [0, 4096, 5, 0x22, u64::MAX, 0]), -13); // no new code after the start
    }

    #[test]
    fn l11_identity_time_and_signals() {
        let (mut h, mut m) = (fake(), memory());
        assert_eq!(call(&mut h, &mut m, 39, [0; 6]), 1);
        assert_eq!(call(&mut h, &mut m, 186, [0; 6]), 1);
        assert_eq!(call(&mut h, &mut m, 110, [0; 6]), 0);
        for nr in [102, 104, 107, 108] {
            assert_eq!(call(&mut h, &mut m, nr, [0; 6]), 1000);
        }
        assert_eq!(call(&mut h, &mut m, 63, [B, 0, 0, 0, 0, 0]), 0);
        assert_eq!(&h.mem[0..6], b"Linux\0");
        assert_eq!(&h.mem[65 * 4..65 * 4 + 7], b"x86_64\0");
        assert_eq!(call(&mut h, &mut m, 63, [0x10, 0, 0, 0, 0, 0]), -EFAULT);
        // clock_gettime: REALTIME is the start's clock plus the monotonic time; MONOTONIC is call 79
        let ts = |h: &Fake, at: usize| {
            let s = u64::from_le_bytes(h.mem[at..at + 8].try_into().unwrap());
            let n = u64::from_le_bytes(h.mem[at + 8..at + 16].try_into().unwrap());
            s * 1_000_000_000 + n
        };
        assert_eq!(call(&mut h, &mut m, 228, [0, B + 512, 0, 0, 0, 0]), 0);
        assert_eq!(u64::from_le_bytes(h.mem[512..520].try_into().unwrap()), 1_700_000_005);
        assert_eq!(u64::from_le_bytes(h.mem[520..528].try_into().unwrap()), 1_000);
        assert_eq!(call(&mut h, &mut m, 228, [1, B + 512, 0, 0, 0, 0]), 0);
        let m1 = ts(&h, 512);
        assert_eq!(m1, 5_000_002_000);
        for clk in [2u64, 3, 4, 6, 7, 9] {
            assert_eq!(call(&mut h, &mut m, 228, [clk, B + 512, 0, 0, 0, 0]), 0, "clock {clk}");
            assert!(ts(&h, 512) > m1, "clock {clk} is monotonic");
        }
        for clk in [5u64, 8, 11] {
            assert_eq!(call(&mut h, &mut m, 228, [clk, B + 512, 0, 0, 0, 0]), 0, "clock {clk}");
            assert!(ts(&h, 512) > 1_700_000_000_000_000_000, "clock {clk} is realtime");
        }
        // an unknown clock (10, 12, a negative CPU-clock id) is EINVAL, and the buffer is not touched
        h.mem[512..528].copy_from_slice(&[0xAA; 16]);
        for clk in [10u64, 12, 0xFFFF_FFFE] {
            assert_eq!(call(&mut h, &mut m, 228, [clk, B + 512, 0, 0, 0, 0]), -EINVAL, "clock {clk}");
        }
        assert_eq!(&h.mem[512..528], &[0xAA; 16]);
        assert_eq!(call(&mut h, &mut m, 228, [1, 0x10, 0, 0, 0, 0]), -EFAULT);
        // only the low 32 bits name the clock (a C int)
        assert_eq!(call(&mut h, &mut m, 228, [0xDEAD_0000_0000_0001, B + 512, 0, 0, 0, 0]), 0);
        // clock_getres: 1 microsecond; a null pointer is allowed; an unknown clock EINVAL
        assert_eq!(call(&mut h, &mut m, 229, [1, B + 512, 0, 0, 0, 0]), 0);
        assert_eq!(ts(&h, 512), 1_000);
        assert_eq!(call(&mut h, &mut m, 229, [0, 0, 0, 0, 0, 0]), 0);
        assert_eq!(call(&mut h, &mut m, 229, [10, B + 512, 0, 0, 0, 0]), -EINVAL);
        // gettimeofday: seconds and microseconds of the realtime clock
        assert_eq!(call(&mut h, &mut m, 96, [B + 600, 0, 0, 0, 0, 0]), 0);
        assert_eq!(u64::from_le_bytes(h.mem[600..608].try_into().unwrap()), 1_700_000_005);
        let us = u64::from_le_bytes(h.mem[608..616].try_into().unwrap());
        assert!(us > 0 && us < 1_000_000, "tv_usec {us}");
        // nanosleep({1, 500 ms}): 1500 ms slept; a bad nsec is EINVAL
        h.mem[700..708].copy_from_slice(&1u64.to_le_bytes());
        h.mem[708..716].copy_from_slice(&500_000_000u64.to_le_bytes());
        assert_eq!(call(&mut h, &mut m, 35, [B + 700, 0, 0, 0, 0, 0]), 0);
        assert_eq!(h.slept, 1500);
        h.mem[708..716].copy_from_slice(&2_000_000_000u64.to_le_bytes());
        assert_eq!(call(&mut h, &mut m, 35, [B + 700, 0, 0, 0, 0, 0]), -EINVAL);
        // nanosleep of 1 ns sleeps 1 ms (rounded up, never shorter than asked)
        h.slept = 0;
        h.mem[700..708].copy_from_slice(&0u64.to_le_bytes());
        h.mem[708..716].copy_from_slice(&1u64.to_le_bytes());
        assert_eq!(call(&mut h, &mut m, 35, [B + 700, 0, 0, 0, 0, 0]), 0);
        assert_eq!(h.slept, 1);
        // clock_nanosleep(MONOTONIC, TIMER_ABSTIME, now + 250 ms): about 250 ms slept; a time already past: none
        h.slept = 0;
        let until = h.now + 250_000_000;
        h.mem[700..708].copy_from_slice(&(until / 1_000_000_000).to_le_bytes());
        h.mem[708..716].copy_from_slice(&(until % 1_000_000_000).to_le_bytes());
        assert_eq!(call(&mut h, &mut m, 230, [1, 1, B + 700, 0, 0, 0]), 0);
        assert_eq!(h.slept, 250);
        h.slept = 0;
        assert_eq!(call(&mut h, &mut m, 230, [1, 1, B + 700, 0, 0, 0]), 0);
        assert_eq!(h.slept, 0);
        // clock_nanosleep(REALTIME, TIMER_ABSTIME, a time in 2023): already past, none; an unknown clock: EINVAL
        h.mem[700..708].copy_from_slice(&1_700_000_000u64.to_le_bytes());
        h.mem[708..716].copy_from_slice(&0u64.to_le_bytes());
        assert_eq!(call(&mut h, &mut m, 230, [0, 1, B + 700, 0, 0, 0]), 0);
        assert_eq!(h.slept, 0);
        assert_eq!(call(&mut h, &mut m, 230, [10, 0, B + 700, 0, 0, 0]), -EINVAL);
        // signals: accepted, the old values written as "default" and "empty"
        h.mem[800..832].copy_from_slice(&[0xAA; 32]);
        assert_eq!(call(&mut h, &mut m, 13, [11, 0, B + 800, 8, 0, 0]), 0);
        assert!(h.mem[800..832].iter().all(|&b| b == 0));
        assert_eq!(call(&mut h, &mut m, 14, [0, 0, B + 900, 8, 0, 0]), 0);
        assert_eq!(call(&mut h, &mut m, 131, [0, B + 1000, 0, 0, 0, 0]), 0);
        assert_eq!(u32::from_le_bytes(h.mem[1008..1012].try_into().unwrap()), 2); // SS_DISABLE
        // one CPU; limits; an unknown resource
        assert_eq!(call(&mut h, &mut m, 204, [0, 128, B + 1100, 0, 0, 0]), 8);
        assert_eq!(h.mem[1100], 1);
        assert_eq!(call(&mut h, &mut m, 302, [0, 3, 0, B + 1200, 0, 0]), 0);
        assert_eq!(u64::from_le_bytes(h.mem[1200..1208].try_into().unwrap()), 8 << 20);
        assert_eq!(call(&mut h, &mut m, 302, [0, 99, 0, B + 1200, 0, 0]), -EINVAL);
        // the refused ones stay ENOSYS: fork, execve, pipe2, socket
        for nr in [57, 59, 293, 41] {
            assert_eq!(call(&mut h, &mut m, nr, [0; 6]), -ENOSYS);
        }
    }
}
