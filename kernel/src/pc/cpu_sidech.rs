//! G13 (docs/security-model.md; architect, 22 Nov 2026): which CPU side channels this x86-64 CPU is open to, from CPUID
//! and IA32_ARCH_CAPABILITIES. Pure: the caller reads the hardware and passes the raw values, so fuzz-host tests the
//! parse on the host (fuzz-host/tests/cpu_sidech.rs). A CPU we cannot rule out counts as open.
//!
//! Sources: Intel's "Affected Processors" guidance and the SDM's IA32_ARCH_CAPABILITIES bits (RDCL_NO bit 0, MDS_NO
//! bit 5, TAA_NO bit 8; the MSR exists when CPUID.(EAX=7,ECX=0):EDX[29] is set); CPUID.7.0:EDX[10] MD_CLEAR (VERW
//! clears buffers), EDX[28] L1D_FLUSH, EBX[11] RTM and EBX[4] HLE (TSX). AMD processors are not affected by Meltdown,
//! L1TF or MDS by AMD's own statements; other vendors are not ruled out.

use core::fmt;

/// IA32_ARCH_CAPABILITIES. Read only when `edx7` bit 29 says it exists: reading it otherwise is a #GP.
pub const MSR_ARCH_CAPABILITIES: u32 = 0x10A;
/// CPUID.7.0:EDX bit 29: IA32_ARCH_CAPABILITIES exists.
pub const EDX7_ARCH_CAPABILITIES: u32 = 1 << 29;

const RDCL_NO: u64 = 1 << 0;
const MDS_NO: u64 = 1 << 5;
const TAA_NO: u64 = 1 << 8;
const EDX7_MD_CLEAR: u32 = 1 << 10;
const EDX7_L1D_FLUSH: u32 = 1 << 28;
const EBX7_HLE: u32 = 1 << 4;
const EBX7_RTM: u32 = 1 << 11;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Vendor {
    Intel,
    Amd,
    Other,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    /// the CPU says it is not affected
    No,
    /// the CPU says it is affected
    Open,
    /// the CPU does not say: treated as open
    NotRuledOut,
    /// the feature it needs is absent (TAA without TSX)
    NotApplicable,
}

impl State {
    /// Open or not ruled out: what every gate must treat as affected.
    pub fn affected(self) -> bool {
        matches!(self, State::Open | State::NotRuledOut)
    }
    fn word(self) -> &'static str {
        match self {
            State::No => "no",
            State::Open => "OPEN",
            State::NotRuledOut => "not ruled out",
            State::NotApplicable => "n/a",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Report {
    pub meltdown: State,
    pub mds: State,
    pub taa: State,
    pub l1tf: State,
    pub md_clear: bool,
    pub l1d_flush: bool,
    pub tsx: bool,
    /// the MSR's value, when it exists and was read
    pub arch_caps: Option<u64>,
}

/// `edx7`, `ebx7`: CPUID leaf 7 subleaf 0 EDX and EBX (0 when leaf 7 does not exist). `arch_caps`: MSR 0x10A, passed
/// only when `edx7` has bit 29 (the caller reads it only then).
pub fn assess(vendor: Vendor, edx7: u32, ebx7: u32, arch_caps: Option<u64>) -> Report {
    let tsx = ebx7 & (EBX7_RTM | EBX7_HLE) != 0;
    let md_clear = edx7 & EDX7_MD_CLEAR != 0;
    let l1d_flush = edx7 & EDX7_L1D_FLUSH != 0;
    // a value passed without the bit that says the MSR exists is ignored: it cannot have been read honestly
    let caps = if edx7 & EDX7_ARCH_CAPABILITIES != 0 { arch_caps } else { None };
    let (meltdown, mds, taa, l1tf) = match vendor {
        Vendor::Amd => (State::No, State::No, if tsx { State::NotRuledOut } else { State::NotApplicable }, State::No),
        Vendor::Other => (
            State::NotRuledOut,
            State::NotRuledOut,
            if tsx { State::NotRuledOut } else { State::NotApplicable },
            State::NotRuledOut,
        ),
        Vendor::Intel => match caps {
            None => (
                State::NotRuledOut,
                State::NotRuledOut,
                if tsx { State::NotRuledOut } else { State::NotApplicable },
                State::NotRuledOut,
            ),
            Some(c) => {
                let flag = |bit: u64| if c & bit != 0 { State::No } else { State::Open };
                let taa = if !tsx { State::NotApplicable } else { flag(TAA_NO) };
                // L1TF affects the same parts as Meltdown: RDCL_NO rules both out
                (flag(RDCL_NO), flag(MDS_NO), taa, flag(RDCL_NO))
            }
        },
    };
    Report { meltdown, mds, taa, l1tf, md_clear, l1d_flush, tsx, arch_caps: caps }
}

impl Report {
    /// Daraja's gate (architect, 22 Nov 2026): without KPTI, downloaded foreign programs run only when this is false.
    pub fn meltdown_affected(&self) -> bool {
        self.meltdown.affected()
    }
}

/// One line: `Meltdown OPEN (RDCL_NO 0), MDS OPEN (MDS_NO 0, MD_CLEAR 1), TAA n/a (no TSX), L1TF OPEN (L1D_FLUSH 1)`.
impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let bit = |b: u64| match self.arch_caps {
            Some(c) => if c & b != 0 { "1" } else { "0" },
            None => "?",
        };
        write!(f, "Meltdown {} (RDCL_NO {}), ", self.meltdown.word(), bit(RDCL_NO))?;
        write!(f, "MDS {} (MDS_NO {}, MD_CLEAR {}), ", self.mds.word(), bit(MDS_NO), self.md_clear as u8)?;
        if self.tsx {
            write!(f, "TAA {} (TAA_NO {}), ", self.taa.word(), bit(TAA_NO))?;
        } else {
            write!(f, "TAA {} (no TSX), ", self.taa.word())?;
        }
        write!(f, "L1TF {} (L1D_FLUSH {})", self.l1tf.word(), self.l1d_flush as u8)
    }
}
