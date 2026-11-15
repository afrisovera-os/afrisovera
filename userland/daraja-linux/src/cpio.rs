//! The bundle: a cpio archive in the "newc" format (magic "070701"), read-only, no allocation.
//!
//! Each entry is a 110-byte header of ASCII hex fields, the name (with its NUL) padded to 4 bytes from the header's
//! start, then the data padded to 4 bytes; "TRAILER!!!" ends the archive. Only regular files and folders are used;
//! anything else (links, devices) is skipped, never followed. Every size is checked against the archive before use.

/// What a lookup found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry<'a> {
    File(&'a [u8]),
    Dir,
}

/// Why an archive was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// a header is cut short or its magic is not "070701"
    BadHeader,
    /// a header field is not 8 hex digits
    BadNumber,
    /// a name or data runs past the end
    Truncated,
    /// no "TRAILER!!!" entry
    NoTrailer,
}

const HDR: usize = 110;
const S_IFMT: u32 = 0o170000;
const S_IFREG: u32 = 0o100000;
const S_IFDIR: u32 = 0o040000;

fn hex8(b: &[u8]) -> Result<u32, Error> {
    let mut v: u32 = 0;
    for &c in b {
        let d = match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => return Err(Error::BadNumber),
        };
        v = (v << 4) | d as u32;
    }
    Ok(v)
}

fn pad4(n: usize) -> Option<usize> {
    n.checked_add(3).map(|x| x & !3)
}

/// One archive entry, as it lies in the bytes.
struct Raw<'a> {
    name: &'a [u8],
    mode: u32,
    data: &'a [u8],
}

/// Walk the archive: calls `f` with each entry until it returns Some, or the trailer.
fn walk<'a, T>(ar: &'a [u8], mut f: impl FnMut(&Raw<'a>) -> Option<T>) -> Result<Option<T>, Error> {
    let mut at = 0usize;
    loop {
        let h = ar.get(at..at.checked_add(HDR).ok_or(Error::Truncated)?).ok_or(Error::BadHeader)?;
        if &h[0..6] != b"070701" {
            return Err(Error::BadHeader);
        }
        let mode = hex8(&h[14..22])?;
        let filesize = hex8(&h[54..62])? as usize;
        let namesize = hex8(&h[94..102])? as usize;
        if namesize == 0 {
            return Err(Error::Truncated);
        }
        let name_start = at + HDR;
        let name_end = name_start.checked_add(namesize).ok_or(Error::Truncated)?;
        let full = ar.get(name_start..name_end).ok_or(Error::Truncated)?;
        let name = &full[..namesize - 1]; // without the NUL
        let data_start = pad4(name_end).ok_or(Error::Truncated)?;
        let data_end = data_start.checked_add(filesize).ok_or(Error::Truncated)?;
        let data = ar.get(data_start..data_end).ok_or(Error::Truncated)?;
        if name == b"TRAILER!!!" {
            return Ok(None);
        }
        if let Some(t) = f(&Raw { name, mode, data }) {
            return Ok(Some(t));
        }
        at = pad4(data_end).ok_or(Error::Truncated)?;
    }
}

/// Check the whole archive once (every header, the trailer), before anything is served from it.
pub fn check(ar: &[u8]) -> Result<usize, Error> {
    let mut n = 0usize;
    match walk(ar, |_| -> Option<()> {
        n += 1;
        None
    }) {
        Ok(None) => Ok(n),
        Ok(Some(())) => Ok(n),
        Err(Error::BadHeader) if n > 0 => Err(Error::NoTrailer),
        Err(e) => Err(e),
    }
}

/// The archive's own name for `path`: no leading "/" or "./", no empty parts, never "..". None if the path tries to
/// leave the bundle or is not a plain path.
pub fn normalise<'o>(path: &[u8], out: &'o mut [u8; 4096]) -> Option<&'o [u8]> {
    let mut n = 0usize;
    for part in path.split(|&c| c == b'/') {
        match part {
            b"" | b"." => continue,
            b".." => return None,
            _ => {
                if part.contains(&0) {
                    return None;
                }
                if n > 0 {
                    *out.get_mut(n)? = b'/';
                    n += 1;
                }
                out.get_mut(n..n + part.len())?.copy_from_slice(part);
                n += part.len();
            }
        }
    }
    Some(&out[..n])
}

/// Look up `path` (already normalised; "" is the bundle's root folder).
pub fn find<'a>(ar: &'a [u8], path: &[u8]) -> Option<Entry<'a>> {
    if path.is_empty() {
        return Some(Entry::Dir);
    }
    walk(ar, |r| {
        let name = r.name.strip_prefix(b"./").unwrap_or(r.name);
        if name != path {
            return None;
        }
        match r.mode & S_IFMT {
            S_IFREG => Some(Some(Entry::File(r.data))),
            S_IFDIR => Some(Some(Entry::Dir)),
            _ => Some(None), // links and devices: present, but never served
        }
    })
    .ok()
    .flatten()
    .flatten()
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    fn entry(out: &mut Vec<u8>, name: &str, mode: u32, data: &[u8]) {
        let namesize = name.len() + 1;
        let h = std::format!(
            "070701{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}",
            1, mode, 0, 0, 1, 0, data.len(), 0, 0, 0, 0, namesize, 0
        );
        out.extend_from_slice(h.as_bytes());
        out.extend_from_slice(name.as_bytes());
        out.push(0);
        while out.len() % 4 != 0 {
            out.push(0);
        }
        out.extend_from_slice(data);
        while out.len() % 4 != 0 {
            out.push(0);
        }
    }
    fn bundle() -> Vec<u8> {
        let mut a = Vec::new();
        entry(&mut a, ".", 0o040755, b"");
        entry(&mut a, "data", 0o040755, b"");
        entry(&mut a, "data/greeting.txt", 0o100644, b"Habari\n");
        entry(&mut a, "hello0", 0o100755, b"\x7fELF...");
        entry(&mut a, "link", 0o120777, b"data/greeting.txt");
        entry(&mut a, "TRAILER!!!", 0, b"");
        a
    }
    fn norm(p: &str) -> Option<Vec<u8>> {
        let mut b = [0u8; 4096];
        normalise(p.as_bytes(), &mut b).map(|s| s.to_vec())
    }

    #[test]
    fn reads_files_and_folders() {
        let a = bundle();
        assert_eq!(check(&a), Ok(5));
        assert_eq!(find(&a, b"data/greeting.txt"), Some(Entry::File(b"Habari\n")));
        assert_eq!(find(&a, b"data"), Some(Entry::Dir));
        assert_eq!(find(&a, b""), Some(Entry::Dir));
        assert_eq!(find(&a, b"missing"), None);
        assert_eq!(find(&a, b"link"), None); // a symbolic link is never followed or served
    }

    #[test]
    fn paths_never_leave_the_bundle() {
        assert_eq!(norm("/data//./greeting.txt").as_deref(), Some(&b"data/greeting.txt"[..]));
        assert_eq!(norm("./hello0").as_deref(), Some(&b"hello0"[..]));
        assert_eq!(norm("/").as_deref(), Some(&b""[..]));
        assert_eq!(norm("data/../../etc/passwd"), None);
        assert_eq!(norm(".."), None);
    }

    #[test]
    fn refuses_broken_archives() {
        let a = bundle();
        assert_eq!(check(&a[..50]), Err(Error::BadHeader));
        let mut b = a.clone();
        b[0] = b'1';
        assert_eq!(check(&b), Err(Error::BadHeader));
        let mut c = a.clone();
        c[60] = b'z'; // inside the first header's filesize field
        assert_eq!(check(&c), Err(Error::BadNumber));
        let cut = a.len() - 120; // the trailer gone
        assert!(check(&a[..cut]).is_err());
    }

    #[test]
    fn mutations_never_panic() {
        let a = bundle();
        let mut x: u64 = 0x2545_F491_4F6C_DD1D;
        for _ in 0..100_000 {
            let mut b = a.clone();
            for _ in 0..3 {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                let i = (x % b.len() as u64) as usize;
                b[i] = (x >> 32) as u8;
            }
            let _ = check(&b);
            let _ = find(&b, b"data/greeting.txt");
        }
    }
}
