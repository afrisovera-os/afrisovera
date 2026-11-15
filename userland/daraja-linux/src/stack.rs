//! The stack a Linux program finds at its entry point (System V AMD64 psABI 3.4.1, "Initial Stack and Register
//! State"), built into a buffer that will become the top of the foreign process's stack.
//!
//! From the stack pointer up: argc; argv[0..argc]; a null; envp[..]; a null; the auxiliary vector as (type, value)
//! pairs ending with AT_NULL. Above that, in the same buffer: the 16 random bytes AT_RANDOM points at, then the
//! argument and environment strings. The stack pointer is 16-byte aligned at entry, as the psABI requires.
//! Auxiliary vector types from the manual page getauxval(3).

/// The auxiliary vector types the bridge gives (getauxval(3)).
pub const AT_NULL: u64 = 0;
pub const AT_PHDR: u64 = 3;
pub const AT_PHENT: u64 = 4;
pub const AT_PHNUM: u64 = 5;
pub const AT_PAGESZ: u64 = 6;
pub const AT_BASE: u64 = 7;
pub const AT_FLAGS: u64 = 8;
pub const AT_ENTRY: u64 = 9;
pub const AT_UID: u64 = 11;
pub const AT_EUID: u64 = 12;
pub const AT_GID: u64 = 13;
pub const AT_EGID: u64 = 14;
pub const AT_SECURE: u64 = 23;
pub const AT_RANDOM: u64 = 25;

/// What the program's own image says (from afri-elf's Image).
#[derive(Clone, Copy, Debug)]
pub struct Aux {
    pub phdr: u64,
    pub phent: u64,
    pub phnum: u64,
    pub entry: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// the arguments, environment and vector do not fit in the buffer
    TooBig,
    /// a string holds a NUL byte (it would be cut short)
    NulInString,
    /// more than MAX_STRINGS arguments plus environment entries
    TooMany,
}

pub const MAX_STRINGS: usize = 256;
const AUX_PAIRS: usize = 14; // the 13 entries below plus AT_NULL

/// Build the stack into `buf`, whose last byte is just below the address `top` (so `buf[i]` lies at
/// `top - buf.len() + i`). Returns the stack pointer to start the program with.
pub fn build(buf: &mut [u8], top: u64, args: &[&[u8]], env: &[&[u8]], aux: &Aux, random: &[u8; 16]) -> Result<u64, Error> {
    if args.len() + env.len() > MAX_STRINGS {
        return Err(Error::TooMany);
    }
    if args.iter().chain(env.iter()).any(|s| s.contains(&0)) {
        return Err(Error::NulInString);
    }
    let len = buf.len() as u64;
    let base = top.checked_sub(len).ok_or(Error::TooBig)?; // the address of buf[0]
    let mut at = buf.len(); // fill downward from the top
    let put = |bytes: &[u8], at: &mut usize, buf: &mut [u8]| -> Result<u64, Error> {
        *at = at.checked_sub(bytes.len()).ok_or(Error::TooBig)?;
        buf[*at..*at + bytes.len()].copy_from_slice(bytes);
        Ok(base + *at as u64)
    };
    let random_at = put(random, &mut at, &mut *buf)?;
    // strings, each with its NUL; their addresses kept in two fixed tables
    let mut ptrs = [0u64; MAX_STRINGS];
    for (i, s) in args.iter().chain(env.iter()).enumerate() {
        put(&[0], &mut at, &mut *buf)?;
        ptrs[i] = put(s, &mut at, &mut *buf)?;
    }
    // the pointer block: argc, argv, null, envp, null, the vector
    let words = 1 + args.len() + 1 + env.len() + 1 + 2 * AUX_PAIRS;
    let block = words * 8;
    let sp = at.checked_sub(block).ok_or(Error::TooBig)?;
    // the stack pointer itself 16-byte aligned
    let sp = sp.checked_sub(((base + sp as u64) % 16) as usize).ok_or(Error::TooBig)?;
    let vector: [(u64, u64); AUX_PAIRS] = [
        (AT_PHDR, aux.phdr),
        (AT_PHENT, aux.phent),
        (AT_PHNUM, aux.phnum),
        (AT_PAGESZ, 4096),
        (AT_BASE, 0), // no interpreter
        (AT_FLAGS, 0),
        (AT_ENTRY, aux.entry),
        (AT_UID, 0),
        (AT_EUID, 0),
        (AT_GID, 0),
        (AT_EGID, 0),
        (AT_SECURE, 0),
        (AT_RANDOM, random_at),
        (AT_NULL, 0),
    ];
    let mut w = sp;
    let word = |v: u64, w: &mut usize, buf: &mut [u8]| {
        buf[*w..*w + 8].copy_from_slice(&v.to_le_bytes());
        *w += 8;
    };
    word(args.len() as u64, &mut w, &mut *buf);
    for &p in &ptrs[..args.len()] {
        word(p, &mut w, &mut *buf);
    }
    word(0, &mut w, &mut *buf);
    for &p in &ptrs[args.len()..args.len() + env.len()] {
        word(p, &mut w, &mut *buf);
    }
    word(0, &mut w, &mut *buf);
    for (t, v) in vector {
        word(t, &mut w, &mut *buf);
        word(v, &mut w, &mut *buf);
    }
    Ok(base + sp as u64)
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;

    const TOP: u64 = 0x100_8000_0000;
    const AUX: Aux = Aux { phdr: 0x100_0040_0040, phent: 56, phnum: 6, entry: 0x100_0040_1010 };

    fn rd(buf: &[u8], addr: u64) -> u64 {
        let i = (addr - (TOP - buf.len() as u64)) as usize;
        u64::from_le_bytes(buf[i..i + 8].try_into().unwrap())
    }
    fn cstr(buf: &[u8], addr: u64) -> &[u8] {
        let i = (addr - (TOP - buf.len() as u64)) as usize;
        let n = buf[i..].iter().position(|&c| c == 0).unwrap();
        &buf[i..i + n]
    }

    #[test]
    fn the_psabi_layout() {
        let mut buf = [0u8; 4096];
        let rnd = [7u8; 16];
        let sp = build(&mut buf, TOP, &[&b"hello0"[..], &b"one"[..], &b"two"[..]], &[], &AUX, &rnd).unwrap();
        assert_eq!(sp % 16, 0);
        assert_eq!(rd(&buf, sp), 3); // argc
        assert_eq!(cstr(&buf, rd(&buf, sp + 8)), b"hello0");
        assert_eq!(cstr(&buf, rd(&buf, sp + 16)), b"one");
        assert_eq!(cstr(&buf, rd(&buf, sp + 24)), b"two");
        assert_eq!(rd(&buf, sp + 32), 0); // end of argv
        assert_eq!(rd(&buf, sp + 40), 0); // empty envp
        // the vector
        let mut v = sp + 48;
        let mut seen = std::vec::Vec::new();
        loop {
            let (t, val) = (rd(&buf, v), rd(&buf, v + 8));
            seen.push((t, val));
            v += 16;
            if t == AT_NULL {
                break;
            }
        }
        assert!(seen.contains(&(AT_PHDR, AUX.phdr)));
        assert!(seen.contains(&(AT_PHNUM, 6)));
        assert!(seen.contains(&(AT_PAGESZ, 4096)));
        assert!(seen.contains(&(AT_ENTRY, AUX.entry)));
        let r = seen.iter().find(|p| p.0 == AT_RANDOM).unwrap().1;
        let i = (r - (TOP - buf.len() as u64)) as usize;
        assert_eq!(&buf[i..i + 16], &rnd);
        assert!(v <= r, "the block lies below the random bytes and the strings");
    }

    #[test]
    fn with_an_environment_and_refusals() {
        let mut buf = [0u8; 4096];
        let sp = build(&mut buf, TOP, &[&b"a"[..]], &[&b"K=V"[..]], &AUX, &[0; 16]).unwrap();
        assert_eq!(sp % 16, 0);
        assert_eq!(cstr(&buf, rd(&buf, sp + 24)), b"K=V");
        let mut small = [0u8; 200];
        assert_eq!(build(&mut small, TOP, &[&b"a"[..]], &[], &AUX, &[0; 16]), Err(Error::TooBig));
        assert_eq!(build(&mut buf, TOP, &[&b"a\0b"[..]], &[], &AUX, &[0; 16]), Err(Error::NulInString));
        let many = [&b"x"[..]; 300];
        assert_eq!(build(&mut buf, TOP, &many, &[], &AUX, &[0; 16]), Err(Error::TooMany));
        // every alignment of the top still gives an aligned stack pointer
        for off in 0..16u64 {
            let sp = build(&mut buf, TOP - off, &[&b"abc"[..]], &[], &AUX, &[0; 16]).unwrap();
            assert_eq!(sp % 16, 0);
        }
    }
}
