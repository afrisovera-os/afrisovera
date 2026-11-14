//! TPM command bytes and response parsing for the PC's TPM driver (F3 step 3, ADR 0008 Decision 1a). Pure
//! functions with no hardware access: `pc/tpm.rs` reads the device only through them, drbg-host includes this
//! file by `#[path]` for unit tests, and fuzz-host can point a target at `parse` (a parser of device input).
//!
//! The command list is closed (architect's condition 1, ARCHITECT_REVIEWS 13 Nov evening): `Cmd` has exactly
//! Startup (CLEAR / ST_CLEAR), GetRandom and, for TPM 1.2 only, the read-only GetCapability of the permanent and
//! volatile flags. Nothing here takes ownership, clears, changes a flag, writes NV or extends a PCR, and
//! `build` is the only way to make command bytes.
#![allow(dead_code)]

/// The response header: tag (2), size (4), return code (4).
pub const HDR: usize = 10;
/// The largest response the driver accepts (GetRandom of 32 bytes is 44 or 46; a flags structure is under 40).
pub const RSP_MAX: usize = 128;
/// The largest command `build` makes (GetCapability, 22 bytes).
pub const CMD_MAX: usize = 22;
/// Bytes asked for per GetRandom (a TPM 2.0 gives at most its largest digest, 32 with SHA-256).
pub const RANDOM_MAX: usize = 32;

/// TPM 1.2 return code: the TPM was never started (TPM_INVALID_POSTINIT).
pub const RC12_INVALID_POSTINIT: u32 = 0x26;
/// TPM 2.0 return code: the TPM was never started (TPM_RC_INITIALIZE).
pub const RC20_INITIALIZE: u32 = 0x100;

const TAG12_RQU: u16 = 0x00C1;
const TAG12_RSP: u16 = 0x00C4;
const TAG20_NO_SESSIONS: u16 = 0x8001;

const ORD12_STARTUP: u32 = 0x99;
const ORD12_GET_RANDOM: u32 = 0x46;
const ORD12_GET_CAPABILITY: u32 = 0x65;
const CC20_STARTUP: u32 = 0x144;
const CC20_GET_RANDOM: u32 = 0x17B;

const TPM12_ST_CLEAR: u16 = 1;
const TPM20_SU_CLEAR: u16 = 0;
const TPM12_CAP_FLAG: u32 = 4;
const TPM12_CAP_FLAG_PERMANENT: u32 = 0x108;
const TPM12_CAP_FLAG_VOLATILE: u32 = 0x109;
const TPM12_TAG_PERMANENT_FLAGS: u16 = 0x001F;
const TPM12_TAG_STCLEAR_FLAGS: u16 = 0x0020;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ver {
    V12,
    V20,
}

/// Which flags structure a TPM 1.2 GetCapability asks for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Flags {
    /// TPM_PERMANENT_FLAGS: its first boolean is `disable`.
    Permanent,
    /// TPM_STCLEAR_FLAGS: its first boolean is `deactivated`.
    Volatile,
}

/// Every command this driver can send. Closed: there is no other way to build command bytes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cmd {
    Startup,
    GetRandom(u16),
    GetCapFlags(Flags),
}

/// Every shape of `Cmd`, for the test that lists the ordinals. That test's exhaustive match (`shape` in
/// drbg-host/tests/tpm_wire.rs) fails to compile, or fails, until a new variant is listed here.
pub const ALL_CMDS: [Cmd; 4] =
    [Cmd::Startup, Cmd::GetRandom(RANDOM_MAX as u16), Cmd::GetCapFlags(Flags::Permanent), Cmd::GetCapFlags(Flags::Volatile)];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WireErr {
    /// The command does not exist for this TPM version (GetCapability on 2.0) or asks for too much.
    NoSuchCommand,
    /// Shorter than a header, or than its size field says.
    Short,
    /// The size field is below 10 or above RSP_MAX, or does not match the bytes read.
    BadSize,
    /// The tag is not the response tag of this version (2.0: sessions are never used).
    BadTag,
    /// The return code is not success.
    Rc(u32),
    /// The body does not have the shape of the answer asked for.
    BadBody,
}

fn be16(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

/// Command bytes for `cmd` into `out`; returns the length.
pub fn build(ver: Ver, cmd: Cmd, out: &mut [u8; CMD_MAX]) -> Result<usize, WireErr> {
    let (tag, code, n) = match (ver, cmd) {
        (Ver::V12, Cmd::Startup) => (TAG12_RQU, ORD12_STARTUP, 12),
        (Ver::V20, Cmd::Startup) => (TAG20_NO_SESSIONS, CC20_STARTUP, 12),
        (Ver::V12, Cmd::GetRandom(_)) => (TAG12_RQU, ORD12_GET_RANDOM, 14),
        (Ver::V20, Cmd::GetRandom(_)) => (TAG20_NO_SESSIONS, CC20_GET_RANDOM, 12),
        (Ver::V12, Cmd::GetCapFlags(_)) => (TAG12_RQU, ORD12_GET_CAPABILITY, 22),
        (Ver::V20, Cmd::GetCapFlags(_)) => return Err(WireErr::NoSuchCommand),
    };
    if let Cmd::GetRandom(k) = cmd {
        if k == 0 || k as usize > RANDOM_MAX {
            return Err(WireErr::NoSuchCommand);
        }
    }
    out.fill(0);
    out[0..2].copy_from_slice(&tag.to_be_bytes());
    out[2..6].copy_from_slice(&(n as u32).to_be_bytes());
    out[6..10].copy_from_slice(&code.to_be_bytes());
    match (ver, cmd) {
        (Ver::V12, Cmd::Startup) => out[10..12].copy_from_slice(&TPM12_ST_CLEAR.to_be_bytes()),
        (Ver::V20, Cmd::Startup) => out[10..12].copy_from_slice(&TPM20_SU_CLEAR.to_be_bytes()),
        (Ver::V12, Cmd::GetRandom(k)) => out[10..14].copy_from_slice(&(k as u32).to_be_bytes()),
        (Ver::V20, Cmd::GetRandom(k)) => out[10..12].copy_from_slice(&k.to_be_bytes()),
        (Ver::V12, Cmd::GetCapFlags(f)) => {
            out[10..14].copy_from_slice(&TPM12_CAP_FLAG.to_be_bytes());
            out[14..18].copy_from_slice(&4u32.to_be_bytes());
            let sub = match f {
                Flags::Permanent => TPM12_CAP_FLAG_PERMANENT,
                Flags::Volatile => TPM12_CAP_FLAG_VOLATILE,
            };
            out[18..22].copy_from_slice(&sub.to_be_bytes());
        }
        (Ver::V20, Cmd::GetCapFlags(_)) => unreachable!(),
    }
    Ok(n)
}

/// The total response size a header announces, checked to be HDR ..= RSP_MAX before any more is read
/// (architect's condition 4: the driver never reads past what it can hold).
pub fn rsp_size(hdr: &[u8]) -> Result<usize, WireErr> {
    if hdr.len() < HDR {
        return Err(WireErr::Short);
    }
    let n = be32(&hdr[2..6]) as usize;
    if n < HDR || n > RSP_MAX {
        return Err(WireErr::BadSize);
    }
    Ok(n)
}

/// A whole response: tag and size checked, the return code checked; returns the body after the header.
pub fn parse(ver: Ver, rsp: &[u8]) -> Result<&[u8], WireErr> {
    let n = rsp_size(rsp)?;
    if n != rsp.len() {
        return Err(if rsp.len() < n { WireErr::Short } else { WireErr::BadSize });
    }
    let want = match ver {
        Ver::V12 => TAG12_RSP,
        Ver::V20 => TAG20_NO_SESSIONS,
    };
    if be16(&rsp[0..2]) != want {
        return Err(WireErr::BadTag);
    }
    let rc = be32(&rsp[6..10]);
    if rc != 0 {
        return Err(WireErr::Rc(rc));
    }
    Ok(&rsp[HDR..])
}

/// The random bytes of a GetRandom response body: their count must be at least 1, at most what was asked for,
/// and exactly what follows the count.
pub fn random_bytes(ver: Ver, body: &[u8], asked: usize) -> Result<&[u8], WireErr> {
    let (k, start) = match ver {
        Ver::V12 if body.len() >= 4 => (be32(&body[0..4]) as usize, 4),
        Ver::V20 if body.len() >= 2 => (be16(&body[0..2]) as usize, 2),
        _ => return Err(WireErr::BadBody),
    };
    if k == 0 || k > asked || body.len() != start + k {
        return Err(WireErr::BadBody);
    }
    Ok(&body[start..])
}

/// The first boolean of a TPM 1.2 flags structure (`disable` for Permanent, `deactivated` for Volatile). The body
/// is respSize (4) then the structure; its tag must be the one asked for and the boolean must be 0 or 1. Anything
/// else cannot be read, and the caller then does not count the TPM (architect's answer to Q2).
pub fn first_flag(body: &[u8], which: Flags) -> Result<bool, WireErr> {
    if body.len() < 4 + 3 {
        return Err(WireErr::BadBody);
    }
    let k = be32(&body[0..4]) as usize;
    if k < 3 || body.len() != 4 + k {
        return Err(WireErr::BadBody);
    }
    let s = &body[4..];
    let tag = match which {
        Flags::Permanent => TPM12_TAG_PERMANENT_FLAGS,
        Flags::Volatile => TPM12_TAG_STCLEAR_FLAGS,
    };
    if be16(&s[0..2]) != tag {
        return Err(WireErr::BadBody);
    }
    match s[2] {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(WireErr::BadBody),
    }
}

/// The ordinal (1.2) or command code (2.0) of built command bytes.
pub fn code_of(cmd: &[u8]) -> u32 {
    be32(&cmd[6..10])
}
