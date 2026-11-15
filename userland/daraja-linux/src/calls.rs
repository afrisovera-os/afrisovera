//! The Linux calls of stage 0 (COMPAT_STAGE0_PLAN section 16a), as pure logic over a `Host`, so every call and every
//! error is tested on the host with a fake one. The real Host (main.rs) is the kernel: fproc_read, fproc_write,
//! fproc_set_tls and the console.
//!
//! Numbers, arguments and errors from the Linux manual pages read(2), write(2), open(2), close(2), fstat(2),
//! lseek(2), ioctl_tty(2), writev(2), exit(2), arch_prctl(2), set_tid_address(2), exit_group(2), openat(2), and
//! errno(3) for the values. No Linux source was read (the clean-room rule).

use crate::cpio;

/// What the bridge can do to the foreign process and the world.
pub trait Host {
    /// Copy from the program's memory (fproc_read); Err on any unmapped page.
    fn read_mem(&mut self, addr: u64, buf: &mut [u8]) -> Result<(), ()>;
    /// Copy into the program's memory (fproc_write); Err on any page not writable.
    fn write_mem(&mut self, addr: u64, buf: &[u8]) -> Result<(), ()>;
    /// Output on fd 1 or 2 (the console's out1 and out2 lines).
    fn out(&mut self, fd: u8, bytes: &[u8]);
    /// The program's FS base (fproc_set_tls); Err if the kernel refused it.
    fn set_fs(&mut self, value: u64) -> Result<(), ()>;
}

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

/// Answer one call. `a` holds the six arguments in Linux's order (rdi, rsi, rdx, r10, r8, r9).
pub fn handle(h: &mut dyn Host, files: &mut Files, nr: u64, a: [u64; 6]) -> Action {
    let r = match nr {
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
    }
    const B: u64 = 0x100_0050_0000;
    fn fake() -> Fake {
        Fake { base: B, mem: std::vec![0; 8192], out1: Vec::new(), out2: Vec::new(), fs: 0 }
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
        put(&mut h, 0, b"hi\n");
        assert_eq!(handle(&mut h, &mut files, 1, [1, B, 3, 0, 0, 0]), Action::Ret(3));
        // writev: two pieces from an iovec at +64
        put(&mut h, 100, b"ab");
        put(&mut h, 200, b"cd");
        let mut iov = Vec::new();
        for (base, len) in [(B + 100, 2u64), (B + 200, 2u64)] {
            iov.extend_from_slice(&base.to_le_bytes());
            iov.extend_from_slice(&len.to_le_bytes());
        }
        put(&mut h, 64, &iov);
        assert_eq!(handle(&mut h, &mut files, 20, [2, B + 64, 2, 0, 0, 0]), Action::Ret(4));
        assert_eq!((h.out1.as_slice(), h.out2.as_slice()), (&b"hi\n"[..], &b"abcd"[..]));
        assert_eq!(handle(&mut h, &mut files, 1, [5, B, 3, 0, 0, 0]), Action::Ret(-EBADF));
        assert_eq!(handle(&mut h, &mut files, 1, [1, 0x10, 3, 0, 0, 0]), Action::Ret(-EFAULT));
    }

    #[test]
    fn open_read_lseek_close_from_the_bundle() {
        let mut h = fake();
        let b = bundle();
        let mut files = Files::new(&b);
        put(&mut h, 0, b"/data/../data/greeting.txt\0"); // ".." is refused by the bundle reader
        assert_eq!(handle(&mut h, &mut files, 2, [B, 0, 0, 0, 0, 0]), Action::Ret(-ENOENT));
        put(&mut h, 0, b"data/greeting.txt\0");
        let fd = match handle(&mut h, &mut files, 257, [(-100i64) as u64, B, 0, 0, 0, 0]) {
            Action::Ret(fd) => fd,
            other => panic!("{:?}", other),
        };
        assert_eq!(fd, 3);
        assert_eq!(handle(&mut h, &mut files, 0, [3, B + 1000, 100, 0, 0, 0]), Action::Ret(7));
        assert_eq!(&h.mem[1000..1007], b"Habari\n");
        assert_eq!(handle(&mut h, &mut files, 0, [3, B + 1000, 100, 0, 0, 0]), Action::Ret(0)); // at the end
        assert_eq!(handle(&mut h, &mut files, 8, [3, 2, 0, 0, 0, 0]), Action::Ret(2)); // SEEK_SET 2
        assert_eq!(handle(&mut h, &mut files, 0, [3, B + 1000, 3, 0, 0, 0]), Action::Ret(3));
        assert_eq!(&h.mem[1000..1003], b"bar");
        assert_eq!(handle(&mut h, &mut files, 8, [1, 0, 0, 0, 0, 0]), Action::Ret(-ESPIPE));
        assert_eq!(handle(&mut h, &mut files, 3, [3, 0, 0, 0, 0, 0]), Action::Ret(0));
        assert_eq!(handle(&mut h, &mut files, 0, [3, B + 1000, 3, 0, 0, 0]), Action::Ret(-EBADF));
        // write flags: read-only bundle
        assert_eq!(handle(&mut h, &mut files, 2, [B, 1, 0, 0, 0, 0]), Action::Ret(-EROFS));
        assert_eq!(handle(&mut h, &mut files, 2, [B, 0o100, 0, 0, 0, 0]), Action::Ret(-EROFS));
        // stdin is empty
        assert_eq!(handle(&mut h, &mut files, 0, [0, B, 10, 0, 0, 0]), Action::Ret(0));
    }

    #[test]
    fn the_table_fills_and_fstat_reports() {
        let mut h = fake();
        let b = bundle();
        let mut files = Files::new(&b);
        put(&mut h, 0, b"data/greeting.txt\0");
        for _ in 3..MAX_FDS {
            assert!(matches!(handle(&mut h, &mut files, 2, [B, 0, 0, 0, 0, 0]), Action::Ret(fd) if fd >= 3));
        }
        assert_eq!(handle(&mut h, &mut files, 2, [B, 0, 0, 0, 0, 0]), Action::Ret(-EMFILE));
        assert_eq!(handle(&mut h, &mut files, 5, [3, B + 2000, 0, 0, 0, 0]), Action::Ret(0));
        let size = u64::from_le_bytes(h.mem[2048..2056].try_into().unwrap());
        let mode = u32::from_le_bytes(h.mem[2024..2028].try_into().unwrap());
        assert_eq!((size, mode & 0o170000), (7, 0o100000));
    }

    #[test]
    fn the_rest_of_the_table() {
        let mut h = fake();
        let b = bundle();
        let mut files = Files::new(&b);
        assert_eq!(handle(&mut h, &mut files, 16, [1, 0x5413, B, 0, 0, 0]), Action::Ret(-ENOTTY));
        assert_eq!(handle(&mut h, &mut files, 158, [0x1002, B + 64, 0, 0, 0, 0]), Action::Ret(0));
        assert_eq!(h.fs, B + 64);
        assert_eq!(handle(&mut h, &mut files, 158, [0x1003, 0, 0, 0, 0, 0]), Action::Ret(-EINVAL));
        assert_eq!(handle(&mut h, &mut files, 218, [B, 0, 0, 0, 0, 0]), Action::Ret(1));
        assert_eq!(handle(&mut h, &mut files, 231, [0x107, 0, 0, 0, 0, 0]), Action::Exit(7));
        assert_eq!(handle(&mut h, &mut files, 60, [3, 0, 0, 0, 0, 0]), Action::Exit(3));
        assert_eq!(handle(&mut h, &mut files, 999, [0; 6]), Action::Ret(-ENOSYS));
        assert_eq!(handle(&mut h, &mut files, 257, [5, B, 0, 0, 0, 0]), Action::Ret(-EBADF));
    }
}
