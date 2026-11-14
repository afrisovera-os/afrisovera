//! F3 step 3: the PC TPM driver's wire module (kernel/src/pc/tpm_wire.rs). The command list is closed (architect's
//! condition 1), every command's bytes match the TPM 1.2 / 2.0 specifications, and the parser refuses every
//! malformed response shape before the driver would use a byte of it (condition 4).
use drbg_host::tpm_wire::*;
use std::collections::BTreeSet;

fn bytes(ver: Ver, cmd: Cmd) -> Vec<u8> {
    let mut b = [0u8; CMD_MAX];
    let n = build(ver, cmd, &mut b).unwrap();
    b[..n].to_vec()
}

fn hex(s: &str) -> Vec<u8> {
    let s: String = s.split_whitespace().collect();
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

/// Follow-up 3 (architect, TPM merge): every shape `Cmd` can take, as a match with no `_` arm. A new variant of
/// `Cmd` or `Flags` does not compile until it gets an arm here, with the next number; SHAPES (just below) is then
/// one more, and the closed-list test fails until ALL_CMDS holds the new shape too.
fn shape(c: Cmd) -> usize {
    match c {
        Cmd::Startup => 0,
        Cmd::GetRandom(_) => 1,
        Cmd::GetCapFlags(Flags::Permanent) => 2,
        Cmd::GetCapFlags(Flags::Volatile) => 3,
    }
}
const SHAPES: usize = 4;

#[test]
fn the_command_list_is_closed() {
    // Condition 1: exactly Startup, GetRandom and (1.2) GetCapability. Nothing else can be built.
    // ALL_CMDS must hold every shape (follow-up 3), and GetRandom is tried at every count a u16 can carry.
    let shapes: BTreeSet<usize> = ALL_CMDS.iter().map(|&c| shape(c)).collect();
    assert_eq!(shapes, (0..SHAPES).collect(), "ALL_CMDS misses a shape of Cmd");
    let every = ALL_CMDS.into_iter().chain((0..=u16::MAX).map(Cmd::GetRandom));
    let mut v12 = BTreeSet::new();
    let mut v20 = BTreeSet::new();
    for c in every {
        let mut b = [0u8; CMD_MAX];
        if let Ok(n) = build(Ver::V12, c, &mut b) {
            v12.insert(code_of(&b[..n]));
        }
        if let Ok(n) = build(Ver::V20, c, &mut b) {
            v20.insert(code_of(&b[..n]));
        }
    }
    assert_eq!(v12, BTreeSet::from([0x46, 0x65, 0x99]), "TPM 1.2: GetRandom, GetCapability, Startup only");
    assert_eq!(v20, BTreeSet::from([0x144, 0x17B]), "TPM 2.0: Startup, GetRandom only");
}

#[test]
fn command_bytes_match_the_specifications() {
    assert_eq!(bytes(Ver::V20, Cmd::Startup), hex("8001 0000000c 00000144 0000"));
    assert_eq!(bytes(Ver::V20, Cmd::GetRandom(32)), hex("8001 0000000c 0000017b 0020"));
    assert_eq!(bytes(Ver::V12, Cmd::Startup), hex("00c1 0000000c 00000099 0001"));
    assert_eq!(bytes(Ver::V12, Cmd::GetRandom(32)), hex("00c1 0000000e 00000046 00000020"));
    assert_eq!(bytes(Ver::V12, Cmd::GetCapFlags(Flags::Permanent)), hex("00c1 00000016 00000065 00000004 00000004 00000108"));
    assert_eq!(bytes(Ver::V12, Cmd::GetCapFlags(Flags::Volatile)), hex("00c1 00000016 00000065 00000004 00000004 00000109"));
    let mut b = [0u8; CMD_MAX];
    assert_eq!(build(Ver::V20, Cmd::GetCapFlags(Flags::Permanent), &mut b), Err(WireErr::NoSuchCommand));
    assert_eq!(build(Ver::V20, Cmd::GetRandom(0), &mut b), Err(WireErr::NoSuchCommand));
    assert_eq!(build(Ver::V12, Cmd::GetRandom(33), &mut b), Err(WireErr::NoSuchCommand));
}

fn rsp20(rc: u32, body: &[u8]) -> Vec<u8> {
    let mut v = vec![0x80, 0x01];
    v.extend_from_slice(&((10 + body.len()) as u32).to_be_bytes());
    v.extend_from_slice(&rc.to_be_bytes());
    v.extend_from_slice(body);
    v
}

fn rsp12(rc: u32, body: &[u8]) -> Vec<u8> {
    let mut v = vec![0x00, 0xc4];
    v.extend_from_slice(&((10 + body.len()) as u32).to_be_bytes());
    v.extend_from_slice(&rc.to_be_bytes());
    v.extend_from_slice(body);
    v
}

#[test]
fn good_random_responses_parse() {
    let mut body = vec![0x00, 0x20];
    body.extend((0..32u8).collect::<Vec<_>>());
    let r = rsp20(0, &body);
    let b = parse(Ver::V20, &r).unwrap();
    assert_eq!(random_bytes(Ver::V20, b, 32).unwrap(), &(0..32u8).collect::<Vec<_>>()[..]);
    let mut body = vec![0, 0, 0, 0x10];
    body.extend([7u8; 16]);
    let r = rsp12(0, &body);
    let b = parse(Ver::V12, &r).unwrap();
    assert_eq!(random_bytes(Ver::V12, b, 32).unwrap(), &[7u8; 16]);
}

#[test]
fn every_malformed_response_is_refused() {
    let good = {
        let mut body = vec![0x00, 0x20];
        body.extend([1u8; 32]);
        rsp20(0, &body)
    };
    // the header
    assert_eq!(rsp_size(&good[..9]), Err(WireErr::Short));
    assert_eq!(rsp_size(&hex("8001 00000009 00000000")), Err(WireErr::BadSize), "size below the header");
    assert_eq!(rsp_size(&hex("8001 00000081 00000000")), Err(WireErr::BadSize), "size above RSP_MAX (129)");
    assert_eq!(rsp_size(&hex("8001 ffffffff 00000000")), Err(WireErr::BadSize));
    assert_eq!(rsp_size(&hex("8001 00000080 00000000")), Ok(128));
    // the whole response
    assert_eq!(parse(Ver::V20, &good[..good.len() - 1]), Err(WireErr::Short));
    let mut longer = good.clone();
    longer.push(0);
    assert_eq!(parse(Ver::V20, &longer), Err(WireErr::BadSize));
    assert_eq!(parse(Ver::V12, &good), Err(WireErr::BadTag), "a 2.0 answer to a 1.2 driver");
    let mut sess = good.clone();
    sess[1] = 0x02;
    assert_eq!(parse(Ver::V20, &sess), Err(WireErr::BadTag), "sessions are never used");
    assert_eq!(parse(Ver::V20, &rsp20(RC20_INITIALIZE, &[])), Err(WireErr::Rc(0x100)));
    assert_eq!(parse(Ver::V12, &rsp12(RC12_INVALID_POSTINIT, &[])), Err(WireErr::Rc(0x26)));
    // the random bytes
    let b = parse(Ver::V20, &good).unwrap();
    assert_eq!(random_bytes(Ver::V20, b, 16), Err(WireErr::BadBody), "more than was asked for");
    assert_eq!(random_bytes(Ver::V20, &b[..b.len() - 1], 32), Err(WireErr::BadBody), "count larger than the bytes");
    assert_eq!(random_bytes(Ver::V20, &[0, 0], 32), Err(WireErr::BadBody), "zero bytes");
    assert_eq!(random_bytes(Ver::V20, &[0], 32), Err(WireErr::BadBody));
    assert_eq!(random_bytes(Ver::V12, &[0, 0, 0], 32), Err(WireErr::BadBody));
    assert_eq!(random_bytes(Ver::V12, &hex("ffffffff 01"), 32), Err(WireErr::BadBody), "a huge count");
}

#[test]
fn tpm12_flags_are_read_strictly() {
    // TPM_PERMANENT_FLAGS: tag 0x001f, disable, ownership, deactivated, ...
    let on = hex("00000005 001f 00 01 00");
    let off = hex("00000005 001f 01 00 01");
    assert_eq!(first_flag(&on, Flags::Permanent), Ok(false));
    assert_eq!(first_flag(&off, Flags::Permanent), Ok(true));
    // TPM_STCLEAR_FLAGS: tag 0x0020, deactivated, ...
    assert_eq!(first_flag(&hex("00000004 0020 00 00"), Flags::Volatile), Ok(false));
    assert_eq!(first_flag(&hex("00000004 0020 01 00"), Flags::Volatile), Ok(true));
    // anything else cannot be read (so the TPM is not counted)
    assert_eq!(first_flag(&on, Flags::Volatile), Err(WireErr::BadBody), "wrong structure");
    assert_eq!(first_flag(&hex("00000005 001f 02 00 00"), Flags::Permanent), Err(WireErr::BadBody), "not a boolean");
    assert_eq!(first_flag(&hex("00000009 001f 00 00 00"), Flags::Permanent), Err(WireErr::BadBody), "size past the body");
    assert_eq!(first_flag(&hex("00000002 001f"), Flags::Permanent), Err(WireErr::BadBody));
    assert_eq!(first_flag(&[], Flags::Permanent), Err(WireErr::BadBody));
}

#[test]
fn no_input_makes_the_parser_panic() {
    // A small exhaustive-ish sweep: every length up to 48 with a few fill patterns, through every parser.
    for len in 0..48 {
        for fill in [0u8, 0x01, 0x80, 0xff] {
            let mut v = vec![fill; len];
            if len >= 6 {
                v[2..6].copy_from_slice(&(len as u32).to_be_bytes());
            }
            for ver in [Ver::V12, Ver::V20] {
                let _ = rsp_size(&v);
                if let Ok(b) = parse(ver, &v) {
                    let _ = random_bytes(ver, b, 32);
                }
                let _ = random_bytes(ver, &v, 32);
            }
            let _ = first_flag(&v, Flags::Permanent);
            let _ = first_flag(&v, Flags::Volatile);
        }
    }
}
