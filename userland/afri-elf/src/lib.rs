//! afri-elf: a strict reader for static x86-64 ELF executables, for the Daraja Linux bridge.
//!
//! The bridge (userland/daraja-linux) loads a Linux program into its foreign process; the kernel never reads the file
//! (AFR-0001 7.7). This crate turns the file's bytes into a plan: which file bytes go to which addresses with which
//! permissions, where to start, and what the program's start-up needs to be told (its program headers and its
//! thread-local storage template). Anything it does not understand, or that would break a rule of ours, is refused
//! with a named reason; it never guesses.
//!
//! Scope (stage 0): 64-bit, little-endian, x86-64, `ET_EXEC` (linked at fixed addresses, no relocation), no program
//! interpreter, no dynamic section. Position-independent executables (`ET_DYN`, static-PIE) are stage L1.
//!
//! Our rules on top of the format:
//! - every segment lies inside the caller's area (for stage 0, the foreign area of AFR-0001 7.7);
//! - write and execute never together (W^X), on a segment and on the stack (`PT_GNU_STACK`);
//! - two loadable segments never share a page, since one page cannot carry two sets of permissions;
//! - the entry point is inside an executable segment;
//! - all arithmetic is checked: a size or offset that would overflow is refused, never wrapped.
//!
//! Written from:
//! - System V Application Binary Interface (gABI), chapter 4 "Object Files" (ELF header, e_ident) and chapter 5
//!   "Program Loading and Dynamic Linking" (program header, segment types, p_align, the PT_LOAD rules);
//! - System V AMD64 ABI (psABI), section 3.4 "Process Initialization" (auxiliary vector, AT_PHDR) and
//!   chapter 5 (EM_X86_64, the 4 KiB page);
//! - the GNU extensions' published values (PT_GNU_STACK 0x6474e551, PT_GNU_RELRO 0x6474e552, PT_GNU_PROPERTY
//!   0x6474e553) from the gABI's OS-specific range documentation.
//!
//! No Linux kernel or C library source was read (the clean-room rule, ARCHITECT_REVIEWS 14 Nov 2026).

#![no_std]
#![forbid(unsafe_code)]

use core::ops::Range;

/// The page size of x86-64 (psABI): mappings are made in whole pages.
pub const PAGE: u64 = 4096;
/// At most this many loadable segments (static programs have two to four).
pub const MAX_LOAD: usize = 8;
/// At most this many program headers are read.
pub const MAX_PHNUM: u16 = 32;

const EHDR_SIZE: usize = 64;
const PHDR_SIZE: usize = 56;

const ET_EXEC: u16 = 2;
const ET_DYN: u16 = 3;
const EM_X86_64: u16 = 62;

const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const PT_INTERP: u32 = 3;
const PT_PHDR: u32 = 6;
const PT_TLS: u32 = 7;
const PT_GNU_STACK: u32 = 0x6474_e551;

/// Segment permission bits (p_flags).
pub const PF_X: u32 = 1;
pub const PF_W: u32 = 2;
pub const PF_R: u32 = 4;

/// Why a file was refused. Each names one rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// shorter than an ELF header
    TooShort,
    /// not starting with 0x7F 'E' 'L' 'F'
    BadMagic,
    /// not ELFCLASS64
    Not64Bit,
    /// not ELFDATA2LSB
    NotLittleEndian,
    /// e_ident[EI_VERSION] or e_version is not 1
    BadVersion,
    /// an OS ABI other than System V (0) or GNU/Linux (3)
    BadOsAbi,
    /// e_machine is not EM_X86_64
    NotX86_64,
    /// ET_DYN: position-independent, needs relocation (stage L1)
    PositionIndependent,
    /// neither ET_EXEC nor ET_DYN (an object file, a core file)
    NotExecutable,
    /// e_ehsize or e_phentsize is not the 64-bit size
    BadHeaderSize,
    /// no program headers, or more than MAX_PHNUM
    BadPhnum,
    /// the program header table is not inside the file
    PhdrsOutsideFile,
    /// PT_INTERP: the program needs a dynamic loader (stage L2)
    NeedsInterpreter,
    /// PT_DYNAMIC: dynamic linking information (stage L2)
    Dynamic,
    /// more than MAX_LOAD loadable segments
    TooManySegments,
    /// no loadable segment at all
    NoSegments,
    /// p_filesz is larger than p_memsz
    FileSizeOverMemSize,
    /// a segment's bytes are not inside the file
    SegmentOutsideFile,
    /// p_align is not 0, 1 or a power of two at least the page size, or p_vaddr and p_offset disagree modulo it
    BadAlignment,
    /// a segment, the TLS template or the header table would wrap around the address space
    Overflow,
    /// a segment starts below the allowed area (a program linked at fixed low addresses, such as Linux's 0x400000)
    OutsideArea,
    /// a segment ends above the allowed area
    AboveArea,
    /// a loadable segment of no size (p_memsz 0)
    EmptySegment,
    /// a segment both writable and executable (W^X)
    WriteAndExecute,
    /// an executable stack was asked for (PT_GNU_STACK with PF_X)
    ExecutableStack,
    /// loadable segments not in ascending address order (the gABI requires it)
    Unordered,
    /// two loadable segments touch the same page
    SharedPage,
    /// the entry point is not inside an executable segment
    EntryNotExecutable,
    /// the program headers are not in any loaded segment and there is no PT_PHDR (AT_PHDR cannot be given)
    PhdrsNotLoaded,
    /// more than one PT_TLS, or a TLS template outside the loaded segments, or a bad TLS alignment
    BadTls,
}

/// One loadable segment, as the bridge maps it: `memsz` bytes from `vaddr`, the first `filesz` of them copied from
/// the file at `offset`, the rest zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Segment {
    pub vaddr: u64,
    pub memsz: u64,
    pub offset: u64,
    pub filesz: u64,
    /// PF_R, PF_W, PF_X
    pub flags: u32,
}

impl Segment {
    /// The whole pages the segment touches.
    pub fn pages(&self) -> Range<u64> {
        let start = self.vaddr & !(PAGE - 1);
        // checked when parsed: vaddr + memsz does not overflow, and rounding up stays inside the area
        let end = (self.vaddr + self.memsz + PAGE - 1) & !(PAGE - 1);
        start..end
    }
}

/// The thread-local storage template (PT_TLS): what the C library copies into each thread's block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tls {
    pub vaddr: u64,
    pub filesz: u64,
    pub memsz: u64,
    pub align: u64,
}

/// What the bridge needs to load and start the program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Image {
    pub entry: u64,
    segments: [Segment; MAX_LOAD],
    count: usize,
    /// for the auxiliary vector: AT_PHDR (where the program headers are in memory), AT_PHENT, AT_PHNUM
    pub phdr_vaddr: u64,
    pub phent: u64,
    pub phnum: u64,
    pub tls: Option<Tls>,
}

impl Image {
    /// The loadable segments, in ascending address order.
    pub fn segments(&self) -> &[Segment] {
        &self.segments[..self.count]
    }
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}
fn u64_at(b: &[u8], at: usize) -> u64 {
    let mut x = [0u8; 8];
    x.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(x)
}

/// Read `file` as a static x86-64 executable whose segments must all lie inside `area`.
pub fn parse(file: &[u8], area: Range<u64>) -> Result<Image, Error> {
    if file.len() < EHDR_SIZE {
        return Err(Error::TooShort);
    }
    if file[0..4] != [0x7F, b'E', b'L', b'F'] {
        return Err(Error::BadMagic);
    }
    if file[4] != 2 {
        return Err(Error::Not64Bit);
    }
    if file[5] != 1 {
        return Err(Error::NotLittleEndian);
    }
    if file[6] != 1 {
        return Err(Error::BadVersion);
    }
    if file[7] != 0 && file[7] != 3 {
        return Err(Error::BadOsAbi);
    }
    match u16_at(file, 16) {
        ET_EXEC => {}
        ET_DYN => return Err(Error::PositionIndependent),
        _ => return Err(Error::NotExecutable),
    }
    if u16_at(file, 18) != EM_X86_64 {
        return Err(Error::NotX86_64);
    }
    if u32_at(file, 20) != 1 {
        return Err(Error::BadVersion);
    }
    let entry = u64_at(file, 24);
    let phoff = u64_at(file, 32);
    if u16_at(file, 52) as usize != EHDR_SIZE || u16_at(file, 54) as usize != PHDR_SIZE {
        return Err(Error::BadHeaderSize);
    }
    let phnum = u16_at(file, 56);
    if phnum == 0 || phnum > MAX_PHNUM {
        return Err(Error::BadPhnum);
    }
    let table_len = phnum as u64 * PHDR_SIZE as u64;
    let table_end = phoff.checked_add(table_len).ok_or(Error::Overflow)?;
    if table_end > file.len() as u64 {
        return Err(Error::PhdrsOutsideFile);
    }

    let mut img = Image {
        entry,
        segments: [Segment::default(); MAX_LOAD],
        count: 0,
        phdr_vaddr: 0,
        phent: PHDR_SIZE as u64,
        phnum: phnum as u64,
        tls: None,
    };
    let mut pt_phdr: Option<u64> = None;
    for i in 0..phnum as usize {
        let at = phoff as usize + i * PHDR_SIZE;
        let p_type = u32_at(file, at);
        let p_flags = u32_at(file, at + 4);
        let p_offset = u64_at(file, at + 8);
        let p_vaddr = u64_at(file, at + 16);
        let p_filesz = u64_at(file, at + 32);
        let p_memsz = u64_at(file, at + 40);
        let p_align = u64_at(file, at + 48);
        match p_type {
            PT_INTERP => return Err(Error::NeedsInterpreter),
            PT_DYNAMIC => return Err(Error::Dynamic),
            PT_GNU_STACK => {
                if p_flags & PF_X != 0 {
                    return Err(Error::ExecutableStack);
                }
            }
            PT_PHDR => pt_phdr = Some(p_vaddr),
            PT_TLS => {
                if img.tls.is_some() || p_filesz > p_memsz || (p_align > 1 && !p_align.is_power_of_two()) {
                    return Err(Error::BadTls);
                }
                p_vaddr.checked_add(p_memsz).ok_or(Error::Overflow)?;
                img.tls = Some(Tls { vaddr: p_vaddr, filesz: p_filesz, memsz: p_memsz, align: p_align.max(1) });
            }
            PT_LOAD => {
                if img.count == MAX_LOAD {
                    return Err(Error::TooManySegments);
                }
                if p_filesz > p_memsz {
                    return Err(Error::FileSizeOverMemSize);
                }
                let file_end = p_offset.checked_add(p_filesz).ok_or(Error::Overflow)?;
                if file_end > file.len() as u64 {
                    return Err(Error::SegmentOutsideFile);
                }
                if p_align > 1 && (!p_align.is_power_of_two() || p_align < PAGE) {
                    return Err(Error::BadAlignment);
                }
                // the gABI: p_vaddr and p_offset agree modulo the alignment; we need it modulo the page at least
                if p_vaddr % PAGE != p_offset % PAGE || (p_align > 1 && p_vaddr % p_align != p_offset % p_align) {
                    return Err(Error::BadAlignment);
                }
                if p_flags & PF_W != 0 && p_flags & PF_X != 0 {
                    return Err(Error::WriteAndExecute);
                }
                let end = p_vaddr.checked_add(p_memsz).ok_or(Error::Overflow)?;
                let end_page = end.checked_add(PAGE - 1).ok_or(Error::Overflow)? & !(PAGE - 1);
                if p_memsz == 0 {
                    return Err(Error::EmptySegment);
                }
                if p_vaddr < area.start {
                    return Err(Error::OutsideArea);
                }
                if end_page > area.end {
                    return Err(Error::AboveArea);
                }
                let seg = Segment { vaddr: p_vaddr, memsz: p_memsz, offset: p_offset, filesz: p_filesz, flags: p_flags };
                if let Some(prev) = img.segments[..img.count].last() {
                    if seg.vaddr < prev.vaddr {
                        return Err(Error::Unordered);
                    }
                    if seg.pages().start < prev.pages().end {
                        return Err(Error::SharedPage);
                    }
                }
                img.segments[img.count] = seg;
                img.count += 1;
            }
            _ => {} // the gABI: a loader ignores segment types it does not use (notes, GNU_RELRO, GNU_PROPERTY)
        }
    }
    if img.count == 0 {
        return Err(Error::NoSegments);
    }
    // a copy of the segment table, so `img` can still be written below
    let table = img.segments;
    let segs = &table[..img.count];
    if !segs.iter().any(|s| s.flags & PF_X != 0 && entry >= s.vaddr && entry < s.vaddr + s.memsz) {
        return Err(Error::EntryNotExecutable);
    }
    // AT_PHDR: PT_PHDR if present (it must lie in a loaded segment), else the table found through the file offsets
    let in_loaded = |va: u64, len: u64| {
        segs.iter().any(|s| va >= s.vaddr && va.checked_add(len).is_some_and(|e| e <= s.vaddr + s.filesz.min(s.memsz)))
    };
    img.phdr_vaddr = match pt_phdr {
        Some(va) if in_loaded(va, table_len) => va,
        Some(_) => return Err(Error::PhdrsNotLoaded),
        None => segs
            .iter()
            .find(|s| phoff >= s.offset && table_end <= s.offset + s.filesz)
            .map(|s| s.vaddr + (phoff - s.offset))
            .ok_or(Error::PhdrsNotLoaded)?,
    };
    if let Some(t) = img.tls {
        if !in_loaded(t.vaddr, t.filesz) {
            return Err(Error::BadTls);
        }
    }
    Ok(img)
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    const AREA: Range<u64> = 0x100_0000_0000..0x200_0000_0000;
    const BASE: u64 = 0x100_0040_0000;

    struct Ph {
        ty: u32,
        flags: u32,
        offset: u64,
        vaddr: u64,
        filesz: u64,
        memsz: u64,
        align: u64,
    }

    /// A file: header, program headers right after it (at 64), then `body` bytes up to `len`.
    fn build(etype: u16, entry: u64, phs: &[Ph], len: usize) -> Vec<u8> {
        let mut f = std::vec![0u8; len.max(64 + phs.len() * 56)];
        f[0..4].copy_from_slice(&[0x7F, b'E', b'L', b'F']);
        f[4] = 2;
        f[5] = 1;
        f[6] = 1;
        f[16..18].copy_from_slice(&etype.to_le_bytes());
        f[18..20].copy_from_slice(&EM_X86_64.to_le_bytes());
        f[20..24].copy_from_slice(&1u32.to_le_bytes());
        f[24..32].copy_from_slice(&entry.to_le_bytes());
        f[32..40].copy_from_slice(&64u64.to_le_bytes());
        f[52..54].copy_from_slice(&64u16.to_le_bytes());
        f[54..56].copy_from_slice(&56u16.to_le_bytes());
        f[56..58].copy_from_slice(&(phs.len() as u16).to_le_bytes());
        for (i, p) in phs.iter().enumerate() {
            let a = 64 + i * 56;
            f[a..a + 4].copy_from_slice(&p.ty.to_le_bytes());
            f[a + 4..a + 8].copy_from_slice(&p.flags.to_le_bytes());
            f[a + 8..a + 16].copy_from_slice(&p.offset.to_le_bytes());
            f[a + 16..a + 24].copy_from_slice(&p.vaddr.to_le_bytes());
            f[a + 24..a + 32].copy_from_slice(&p.vaddr.to_le_bytes());
            f[a + 32..a + 40].copy_from_slice(&p.filesz.to_le_bytes());
            f[a + 40..a + 48].copy_from_slice(&p.memsz.to_le_bytes());
            f[a + 48..a + 56].copy_from_slice(&p.align.to_le_bytes());
        }
        f
    }

    /// The shape of a small static musl program: a read-only first segment holding the headers, code, read-only
    /// data, and data plus bss, each on its own pages; a TLS template in the data; a non-executable stack.
    fn good_phs() -> Vec<Ph> {
        std::vec![
            Ph { ty: PT_LOAD, flags: PF_R, offset: 0, vaddr: BASE, filesz: 0x200, memsz: 0x200, align: 0x1000 },
            Ph { ty: PT_LOAD, flags: PF_R | PF_X, offset: 0x1000, vaddr: BASE + 0x1000, filesz: 0x800, memsz: 0x800, align: 0x1000 },
            Ph { ty: PT_LOAD, flags: PF_R, offset: 0x2000, vaddr: BASE + 0x2000, filesz: 0x100, memsz: 0x100, align: 0x1000 },
            Ph { ty: PT_LOAD, flags: PF_R | PF_W, offset: 0x3000, vaddr: BASE + 0x3000, filesz: 0x80, memsz: 0x2400, align: 0x1000 },
            Ph { ty: PT_TLS, flags: PF_R, offset: 0x3000, vaddr: BASE + 0x3000, filesz: 0x10, memsz: 0x20, align: 8 },
            Ph { ty: PT_GNU_STACK, flags: PF_R | PF_W, offset: 0, vaddr: 0, filesz: 0, memsz: 0, align: 16 },
        ]
    }
    fn good() -> Vec<u8> {
        build(ET_EXEC, BASE + 0x1010, &good_phs(), 0x3080)
    }
    fn with(f: impl FnOnce(&mut Vec<Ph>)) -> Result<Image, Error> {
        let mut phs = good_phs();
        f(&mut phs);
        parse(&build(ET_EXEC, BASE + 0x1010, &phs, 0x3080), AREA)
    }

    #[test]
    fn a_good_static_program() {
        let img = parse(&good(), AREA).unwrap();
        assert_eq!(img.entry, BASE + 0x1010);
        assert_eq!(img.segments().len(), 4);
        assert_eq!(img.segments()[3].pages(), BASE + 0x3000..BASE + 0x6000);
        assert_eq!(img.phdr_vaddr, BASE + 64); // the table at file offset 64, inside the first segment
        assert_eq!((img.phent, img.phnum), (56, 6));
        assert_eq!(img.tls, Some(Tls { vaddr: BASE + 0x3000, filesz: 0x10, memsz: 0x20, align: 8 }));
    }

    #[test]
    fn header_refusals() {
        let g = good();
        assert_eq!(parse(&g[..63], AREA), Err(Error::TooShort));
        let m = |i: usize, v: u8| {
            let mut f = g.clone();
            f[i] = v;
            parse(&f, AREA)
        };
        assert_eq!(m(1, b'X'), Err(Error::BadMagic));
        assert_eq!(m(4, 1), Err(Error::Not64Bit));
        assert_eq!(m(5, 2), Err(Error::NotLittleEndian));
        assert_eq!(m(6, 0), Err(Error::BadVersion));
        assert_eq!(m(7, 9), Err(Error::BadOsAbi));
        assert!(m(7, 3).is_ok()); // GNU/Linux OS ABI is fine
        assert_eq!(m(16, 3), Err(Error::PositionIndependent));
        assert_eq!(m(16, 1), Err(Error::NotExecutable));
        assert_eq!(m(18, 183), Err(Error::NotX86_64)); // EM_AARCH64
        assert_eq!(m(20, 2), Err(Error::BadVersion));
        assert_eq!(m(54, 32), Err(Error::BadHeaderSize));
        assert_eq!(m(56, 0), Err(Error::BadPhnum));
        assert_eq!(m(56, 200), Err(Error::BadPhnum));
        let mut f = g.clone();
        f[32..40].copy_from_slice(&(g.len() as u64 - 10).to_le_bytes());
        assert_eq!(parse(&f, AREA), Err(Error::PhdrsOutsideFile));
        f[32..40].copy_from_slice(&u64::MAX.to_le_bytes());
        assert_eq!(parse(&f, AREA), Err(Error::Overflow));
    }

    #[test]
    fn segment_refusals() {
        assert_eq!(with(|p| p[1].flags = PF_R | PF_W | PF_X), Err(Error::WriteAndExecute));
        assert_eq!(with(|p| p[5].flags = PF_R | PF_W | PF_X), Err(Error::ExecutableStack));
        assert_eq!(with(|p| p[3].filesz = 0x3000), Err(Error::FileSizeOverMemSize));
        assert_eq!(with(|p| p[3].offset = 0x4000), Err(Error::SegmentOutsideFile));
        assert_eq!(with(|p| p[2].vaddr = BASE + 0x2008), Err(Error::BadAlignment));
        assert_eq!(with(|p| p[2].align = 0x800), Err(Error::BadAlignment));
        assert_eq!(with(|p| p[2].align = 0x3000), Err(Error::BadAlignment));
        assert_eq!(with(|p| { p[3].vaddr = u64::MAX - 0xFFF; p[3].offset = 0x3000 }), Err(Error::Overflow));
        // below and above the area are told apart, so the bridge names the right cause (the architect, 21 Nov)
        assert_eq!(with(|p| p[3].vaddr = 0x200_0000_0000), Err(Error::AboveArea)); // past the area's end
        assert_eq!(with(|p| { p[3].vaddr = 0x1FF_FFFF_F000; p[3].offset = 0x3000 }), Err(Error::AboveArea)); // ends past it
        assert_eq!(with(|p| p[0].vaddr = 0x40_0000), Err(Error::OutsideArea)); // the usual Linux address: below ours
        assert_eq!(with(|p| { p[2].filesz = 0; p[2].memsz = 0 }), Err(Error::EmptySegment));
        assert_eq!(with(|p| p[2].vaddr = BASE + 0x1000), Err(Error::SharedPage));
        assert_eq!(with(|p| p[2].vaddr = BASE), Err(Error::Unordered));
        assert_eq!(with(|p| p[0].ty = PT_INTERP), Err(Error::NeedsInterpreter));
        assert_eq!(with(|p| p[5].ty = PT_DYNAMIC), Err(Error::Dynamic));
        assert_eq!(with(|p| p[1].flags = PF_R), Err(Error::EntryNotExecutable));
        assert_eq!(with(|p| p[3].memsz = 0), Err(Error::FileSizeOverMemSize));
        assert_eq!(with(|p| for x in p.iter_mut() { x.ty = 0x7000_0000 }), Err(Error::NoSegments));
    }

    #[test]
    fn program_headers_and_tls() {
        // the table not inside any loaded segment, and no PT_PHDR
        assert_eq!(with(|p| { p[0].offset = 0x1000; p[0].vaddr = BASE; p[1].offset = 0x1000 }), Err(Error::PhdrsNotLoaded));
        // an explicit PT_PHDR wins, and must lie in a loaded segment
        let ok = with(|p| p.push(Ph { ty: PT_PHDR, flags: PF_R, offset: 64, vaddr: BASE + 64, filesz: 0, memsz: 0, align: 8 }));
        assert_eq!(ok.unwrap().phdr_vaddr, BASE + 64);
        let bad = with(|p| p.push(Ph { ty: PT_PHDR, flags: PF_R, offset: 64, vaddr: BASE + 0x5000, filesz: 0, memsz: 0, align: 8 }));
        assert_eq!(bad, Err(Error::PhdrsNotLoaded));
        assert_eq!(with(|p| p[4].align = 12), Err(Error::BadTls));
        assert_eq!(with(|p| p[4].vaddr = BASE + 0x5000), Err(Error::BadTls));
        assert_eq!(
            with(|p| p.push(Ph { ty: PT_TLS, flags: PF_R, offset: 0x3000, vaddr: BASE + 0x3000, filesz: 0, memsz: 0, align: 8 })),
            Err(Error::BadTls)
        );
    }

    #[test]
    fn more_than_max_load_segments() {
        let phs: Vec<Ph> = (0..9u64)
            .map(|i| Ph { ty: PT_LOAD, flags: PF_R | PF_X, offset: i * 0x1000, vaddr: BASE + i * 0x1000, filesz: 0x10, memsz: 0x10, align: 0x1000 })
            .collect();
        assert_eq!(parse(&build(ET_EXEC, BASE, &phs, 0x9000), AREA), Err(Error::TooManySegments));
    }

    /// A small fuzz in the unit tests: random byte changes to a good file never panic, and whatever is accepted
    /// keeps every rule (inside the area, W^X, no shared pages, entry executable, segment bytes inside the file).
    #[test]
    fn mutations_never_panic_and_accepted_files_keep_the_rules() {
        let g = good();
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let mut accepted = 0;
        for _ in 0..200_000 {
            let mut f = g.clone();
            for _ in 0..(next() % 4 + 1) {
                let i = (next() % 400) as usize; // the headers, where the rules live
                f[i] = next() as u8;
            }
            if next() % 16 == 0 {
                let n = (next() % f.len() as u64) as usize;
                f.truncate(n);
            }
            if let Ok(img) = parse(&f, AREA) {
                accepted += 1;
                let segs = img.segments();
                for (k, s) in segs.iter().enumerate() {
                    assert!(s.pages().start >= AREA.start && s.pages().end <= AREA.end);
                    assert!(!(s.flags & PF_W != 0 && s.flags & PF_X != 0));
                    assert!(s.offset + s.filesz <= f.len() as u64);
                    if k > 0 {
                        assert!(segs[k - 1].pages().end <= s.pages().start);
                    }
                }
                assert!(segs.iter().any(|s| s.flags & PF_X != 0 && img.entry >= s.vaddr && img.entry < s.vaddr + s.memsz));
            }
        }
        assert!(accepted > 0, "the mutations should leave some files valid");
    }
}
