//! `battery` (13 Nov 2026): the laptop battery, AC adapter, lid and embedded controller the firmware describes,
//! READ ONLY. The first step towards an AML interpreter: the DSDT and SSDTs are walked the way the ACPI grammar
//! lays out a definition block (Scope, Device, ThermalZone, Processor, PowerResource, Name, Method,
//! OperationRegion, Field, External, Alias, Mutex, Event), so every device gets its full path, its _HID and the
//! methods it has. Nothing is run: no method is evaluated and no register is touched. The charge level itself
//! comes only from the battery's _BST method, which waits for the interpreter.
//!
//! Limits, said plainly by the command: method bodies are skipped whole (by their length); an If, Else or While
//! block at definition level is not entered (its condition is an expression to run); an AML form this reader does
//! not know ends the block it is in, and the walk goes on after that block's end. Every read is inside the table.
//!
//! The interpreter's first slice (13 Nov 2026): a device's status (_STA) is worked out when the firmware gives it
//! without running anything: `Name(_STA, constant)`, a method whose whole body is `Return(constant)`, or
//! `Return(NAME)` where NAME is a constant integer Name found by the ACPI search rules (the device's scope, then each
//! parent up to the root). Anything else is said to need the interpreter. No register is read and nothing is run.
//!
//! The embedded controller's map (13 Nov 2026): its two I/O ports come from its `_CRS` resource template (the data
//! port first, then the command and status port, as the ACPI spec orders them), and every Field declared over an
//! EmbeddedControl region is read for its named units: each name with its byte, bit and width, found by the same
//! search rules (a Field may come before its region, or in another table's Scope). These are the registers a
//! battery's _BST and an adapter's _PSR read, so the interpreter can later go straight to them. Still nothing is
//! read from the controller itself.
//!
//! C1 (14 Nov 2026, AML_OPTIONS.md, ARCHITECT_REVIEWS 14 Nov): the pure parts moved to aml.rs (fuzz-host target
//! `aml_eval`), and the ACPI global lock is taken around each controller transaction when its _GLK asks for it (or it
//! has none and the firmware holds the lock): the spec's acquire and pending protocol on the FACS word, at most
//! 100 ms, GBL_RLS set when the firmware was waiting. No usable FACS when the lock is needed: the controller is not
//! touched.
//!
//! `battery ec` (13 Nov 2026): those named fields read through the controller's two ports with the ACPI read
//! handshake (command 0x80, then the address, every wait time-limited). Read only: 0x80 is the only command ever
//! sent. Nothing is sent when the status port reads 0xFF (no controller) or a byte is already waiting for another
//! reader, or when the _CRS names a port that belongs to another device. The ACPI global lock is not taken yet.
//!
//! The interpreter's second slice (13 Nov 2026): `battery ec` also works out each AC adapter's power source by
//! running its _PSR method with a small, read-only evaluator: Return, If and Else, Noop, the constants, And, Or and
//! XOr with no target, LAnd, LOr, LNot, LEqual, LGreater and LLess, constant integer Names, and the embedded
//! controller's named fields, read through the same handshake as above. Names are found from the method's own scope
//! by the ACPI search rules. Anything else (Store, a Local, a method call, a target to write) stops the evaluation and
//! says which form it met, so nothing is ever written.
//!
//! The interpreter's third slice (13 Nov 2026): Store, and Local0 to Local7, which live only while the method runs.
//! Store, And, Or and XOr may now put their result in a Local or the Debug object (Debug is dropped). A store into
//! anything else (a field, a Name, an Arg) still stops the evaluation and says so, and a Local read before anything
//! was stored in it stops it too, so nothing outside the method is ever written.
//!
//! The interpreter's fourth slice (13 Nov 2026): Packages of integers, so `battery ec` can run each battery's _BST
//! read only and show its state, rate, remaining capacity and voltage. A Package may be returned or stored in a
//! Local; Store into Index(LocalN, i) changes one element of a Package held in a Local, and DerefOf(Index(LocalN, i))
//! reads one. An Index of anything else (a named Package, an Arg) stops the evaluation and says so, as does an index
//! past the Package's end or a Package that declares more elements than it gives, so still nothing outside the
//! method is ever written.
//!
//! The interpreter's fifth slice (13 Nov 2026): named Packages, as most firmware writes its _BST
//! (Name(PBST, Package(4) {...}) beside the method, then Store into Index(PBST, i) and Return(PBST)). Every
//! `Name(X, Package)` of constant integers outside methods is kept from the tables. The first time a run uses one,
//! it gets its own copy for that run only: Index, DerefOf and Return work on the copy, and the namespace itself is
//! never changed, so the next run starts from the tables' values again. An Index of an Arg still stops.
//!
//! The interpreter's sixth slice (13 Nov 2026): strings, so `battery ec` can read each battery's _BIF or _BIX (its
//! units, design and last full capacity, design voltage, model, serial and type). A Package element, a named
//! Package's element and a value a Store copies may now be a string; a string where a number is needed still stops.
//! With the units and the last full charge known, the _BST line gives its units and a charge percentage is shown.
//! _BIX is used when a battery has both, as the ACPI spec asks; a `Name(_BIF, Package)` works as well as a method.
//!
//! The interpreter's slice after method calls with Args (13 Nov 2026): arithmetic, as helpers often scale the controller's raw values
//! (a capacity kept in tens of mAh, a rate split over two bytes). Add, Subtract, Multiply, ShiftLeft, ShiftRight,
//! Divide (remainder and quotient targets), Mod and Not, each with the same targets as And and Or, and each usable
//! as a statement. Integers wrap at 64 bits; a table of revision 1 would wrap at 32 bits in ACPICA, a known
//! difference. A Divide or Mod by Zero stops the evaluation and says so.
//!
//! The interpreter's loops (13 Nov 2026): While, Break and Continue, with Increment and Decrement of a Local or an
//! Arg, as helpers loop over bytes and wait on the controller. Every run may go round its loops at most EVAL_LOOPS
//! times in all (ACPICA's own limit), so a loop without end stops the evaluation and says so; the field reads inside
//! a loop still count towards EVAL_READS. A Break or Continue outside a While (also one in a method called from a
//! loop) stops it too. Increment or Decrement of anything but a Local or an Arg would write outside the method, so it
//! stops before anything is read.

use alloc::vec::Vec;

use crate::aml::*;
use core::sync::atomic::{AtomicU32, Ordering};


fn print_path(p: &[[u8; 4]]) {
    crate::kprint!("\\");
    for (k, s) in p.iter().enumerate() {
        if k > 0 {
            crate::kprint!(".");
        }
        crate::kprint!("{}", core::str::from_utf8(s).unwrap_or("????"));
    }
}

pub fn cmd(arg: &str) {
    match arg.trim() {
        "" => show(),
        "ec" => show_ec_read(),
        "selftest" => selftest(),
        _ => crate::kprintln!("battery: type battery, battery ec, or battery selftest"),
    }
}

/// Every DSDT and SSDT walked and settled; None (said on the console) when none could be read.
fn tables() -> Option<(Walk, bool, u32)> {
    let mut w = Walk::default();
    let (dsdt, ssdts) = crate::hwscan::aml_tables(&mut |t: &[u8]| walk_table(t, &mut w));
    if !dsdt && ssdts == 0 {
        crate::kprintln!("  tables: none could be read (no usable RSDT, XSDT, DSDT or SSDT)");
        return None;
    }
    settle(&mut w);
    Some((w, dsdt, ssdts as u32))
}

fn show() {
    crate::kprintln!("battery: what the firmware describes, read from its ACPI tables (nothing is run or changed)");
    let Some((w, dsdt, ssdts)) = tables() else { return };
    crate::kprintln!(
        "  tables: {} and {} SSDT(s) read: {} named objects, {} conditional block(s) not entered, {} cut short",
        if dsdt { "the DSDT" } else { "no DSDT" },
        ssdts,
        w.objects,
        w.conditional,
        w.cut
    );
    if w.cut > 0 {
        crate::kprintln!("  (a block is cut short at an AML form this reader does not know yet; the walk goes on after it)");
    }
    crate::kprintln!(
        "  status (_STA): {} device(s) have one: {} worked out without running anything, {} need the interpreter",
        w.sta_known + w.sta_run,
        w.sta_known,
        w.sta_run
    );
    let kinds = [
        (Kind::Battery, "battery"),
        (Kind::Ac, "AC adapter"),
        (Kind::Lid, "lid"),
        (Kind::Ec, "embedded controller"),
        (Kind::Thermal, "thermal zone"),
    ];
    for &(k, label) in &kinds {
        for d in w.devs.iter().filter(|d| d.kind == k) {
            crate::kprint!("  {}: ", label);
            print_path(&d.path);
            if !d.methods.is_empty() {
                crate::kprint!(", methods");
                for m in &d.methods {
                    crate::kprint!(" {}", core::str::from_utf8(m).unwrap_or("????"));
                }
            }
            if k == Kind::Ec {
                crate::kprint!(", {} region(s) in its own address space", d.ec_regions);
            }
            match d.sta {
                Sta::Absent => crate::kprint!(", no _STA (present by the ACPI rules)"),
                Sta::Value(v) => crate::kprint!(", status 0x{:02X} ({})", v, if v & 1 != 0 { "present" } else { "not present" }),
                _ => crate::kprint!(", status needs the interpreter"),
            }
            if d.cut {
                crate::kprint!(" (partly read)");
            }
            crate::kprintln!("");
            if k == Kind::Ec {
                show_ec(&w, d);
            }
        }
    }
    if w.dropped > 0 {
        crate::kprintln!("  {} more device(s) not listed (the list holds {})", w.dropped, MAX_KEPT);
    }
    if w.devs.iter().any(|d| d.kind == Kind::Battery) {
        crate::kprintln!("  charge: not read here; the battery gives it through its _BST method, which battery ec runs read only");
    } else {
        crate::kprintln!("  battery: none described (a desktop PC or a virtual machine has none)");
    }
}

/// The units of every Field over an EmbeddedControl region declared inside device `d`.
fn ec_units<'a>(w: &'a Walk, d: &Dev) -> Vec<&'a Unit> {
    w.ec_fields
        .iter()
        .filter(|(k, _, _)| w.regions.get(*k).is_some_and(|(q, _)| q.starts_with(&d.path)))
        .map(|(_, u, _)| u)
        .collect()
}

fn show_ec(w: &Walk, d: &Dev) {
    match d.crs {
        Crs::Absent => crate::kprintln!("    ports: no _CRS (the ECDT table may give them)"),
        Crs::Run => crate::kprintln!("    ports: its _CRS is a method, which needs the interpreter"),
        Crs::Ports(p, 2) => crate::kprintln!("    ports (from its _CRS): data 0x{:02X}, command and status 0x{:02X}", p[0], p[1]),
        Crs::Ports(_, n) => crate::kprintln!("    ports: its _CRS gives {} I/O port(s), not the two an embedded controller has", n),
    }
    let us = ec_units(w, d);
    if us.is_empty() {
        crate::kprintln!("    fields: none named in its own address space");
        return;
    }
    let lo = us.iter().map(|u| u.bit / 8).min().unwrap_or(0);
    let hi = us.iter().map(|u| (u.bit + u.bits.max(1) - 1) / 8).max().unwrap_or(0);
    crate::kprintln!("    fields: {} named in its own address space, bytes 0x{:02X} to 0x{:02X}", us.len(), lo, hi);
    for u in us.iter().take(MAX_SHOWN) {
        crate::kprint!("      {}: byte 0x{:02X}", core::str::from_utf8(&u.name).unwrap_or("????"), u.bit / 8);
        if u.bit % 8 != 0 {
            crate::kprint!(" bit {}", u.bit % 8);
        }
        if u.bits == 1 {
            crate::kprintln!(", 1 bit");
        } else {
            crate::kprintln!(", {} bits", u.bits);
        }
    }
    if us.len() > MAX_SHOWN {
        crate::kprintln!("      ({} more not listed)", us.len() - MAX_SHOWN);
    }
    if w.fields_dropped > 0 {
        crate::kprintln!("      ({} field unit(s) not kept, the list holds {})", w.fields_dropped, MAX_FIELDS);
    }
}

/// The real controller's ports, with the ACPI global lock taken around each transaction when the controller's
/// _GLK asks for it (ARCHITECT_REVIEWS 14 Nov, AML C1: a merge condition), or when there is no _GLK and the FACS
/// shows the lock held at that moment. The lock lives in the FACS (FADT FIRMWARE_CTRL / X_FIRMWARE_CTRL), checked
/// to lie in ACPI memory (hwscan::facs_lock). If the lock is needed and there is no usable FACS, the controller is
/// not touched.
struct Ports {
    /// the FACS lock word's address, when this controller needs the lock
    lock: Option<u64>,
    held: bool,
    /// times the firmware held the lock and we waited; times we gave it back to a waiting firmware
    waited: u32,
    signalled: u32,
}

/// The FACS lock word, through an atomic (the firmware's SMM code shares it).
struct Facs(u64);

impl LockWord for Facs {
    fn load(&self) -> u32 {
        // SAFETY: hwscan::facs_lock checked the FACS lies in ACPI memory in the kernel's 1:1 map; the word is 4-byte
        // aligned (offset 16 of a 64-byte-aligned table).
        unsafe { (*(self.0 as *const AtomicU32)).load(Ordering::SeqCst) }
    }
    fn cas(&self, old: u32, new: u32) -> bool {
        // SAFETY: as `load`.
        unsafe { (*(self.0 as *const AtomicU32)).compare_exchange(old, new, Ordering::SeqCst, Ordering::SeqCst).is_ok() }
    }
}

impl Ports {
    fn new() -> Ports {
        Ports { lock: None, held: false, waited: 0, signalled: 0 }
    }
}

impl EcIo for Ports {
    fn inb(&mut self, port: u16) -> u8 {
        // SAFETY: the port is this embedded controller's own, from its _CRS, checked by ec_port_refused (never the
        // keyboard controller, the PICs, PCI configuration or COM1); reached only from `battery ec`.
        unsafe { crate::cpu::inb(port) }
    }
    fn outb(&mut self, port: u16, v: u8) {
        // SAFETY: as `inb`; the only bytes written are the read command 0x80 and the address of the byte read.
        unsafe { crate::cpu::outb(port, v) }
    }
    fn device(&mut self, glk: Sta) -> Result<(), EcErr> {
        let facs = crate::hwscan::facs_lock();
        let need = match glk {
            Sta::Value(0) => false,
            Sta::Absent => facs.as_ref().is_ok_and(|&a| Facs(a).load() & 2 != 0),
            _ => true,
        };
        self.lock = None;
        if need {
            self.lock = Some(facs.map_err(|_| EcErr::NoLock)?);
        }
        Ok(())
    }
    fn begin(&mut self) -> bool {
        let Some(a) = self.lock else { return true };
        let w = Facs(a);
        let (t0, hz) = (crate::timer::cycles(), crate::timer::hz().max(1));
        loop {
            match glk_try(&w) {
                Some(true) => {
                    self.held = true;
                    return true;
                }
                Some(false) => self.waited += 1,
                None => return false,
            }
            // the firmware holds it: wait (at most 100 ms in all) until it lets go, then try again
            while w.load() & 2 != 0 {
                if crate::timer::cycles().wrapping_sub(t0) > hz / 10 {
                    return false;
                }
                core::hint::spin_loop();
            }
        }
    }
    fn end(&mut self) {
        if !self.held {
            return;
        }
        self.held = false;
        if let Some(a) = self.lock {
            if glk_release(&Facs(a)) == Some(true) {
                self.signalled += 1;
                crate::acpi::gbl_release(); // the firmware was waiting: GBL_RLS tells it the lock is free
            }
        }
    }
}

fn show_ec_read() {
    crate::kprintln!("battery ec: the embedded controller's named fields, read through its ports (read command 0x80 only, nothing is written to its memory)");
    let Some((w, _, _)) = tables() else { return };
    let mut any = false;
    for d in w.devs.iter().filter(|d| d.kind == Kind::Ec) {
        any = true;
        crate::kprint!("  embedded controller ");
        print_path(&d.path);
        crate::kprintln!(":");
        let (data, cmd) = match d.crs {
            Crs::Ports(p, 2) => (p[0], p[1]),
            _ => {
                crate::kprintln!("    not read: its two ports are not given plainly by its _CRS");
                continue;
            }
        };
        if data == cmd || ec_port_refused(data) || ec_port_refused(cmd) {
            crate::kprintln!("    not read: ports 0x{:02X} and 0x{:02X} are not an embedded controller's", data, cmd);
            continue;
        }
        let us: Vec<Unit> = ec_units(&w, d).into_iter().take(MAX_SHOWN).copied().collect();
        if us.is_empty() {
            crate::kprintln!("    not read: no fields named in its own address space");
            continue;
        }
        let mut port = Ports::new();
        if port.device(d.glk).is_err() {
            crate::kprintln!("    not read: it asks for the ACPI global lock, and there is no FACS to take it in, so nothing was sent");
            continue;
        }
        let (vals, bytes, stop) = ec_read_units(&mut port, data, cmd, &us);
        match stop {
            Some(EcErr::Dead) => {
                crate::kprintln!("    not read: nothing answers at port 0x{:02X} (it reads 0xFF), so nothing was sent", cmd);
                continue;
            }
            Some(EcErr::Waiting) => {
                crate::kprintln!("    not read: a byte is waiting there for another reader (the firmware), so nothing was sent");
                continue;
            }
            _ => {}
        }
        for (u, v) in us.iter().zip(vals.iter()) {
            let n = core::str::from_utf8(&u.name).unwrap_or("????");
            match *v {
                EcVal::Value(x) => crate::kprintln!("      {} = 0x{:X} ({}), byte 0x{:02X}, {} bit(s)", n, x, x, u.bit / 8, u.bits),
                EcVal::Wide => crate::kprintln!("      {}: not read (wider than 32 bits)", n),
                EcVal::Past => crate::kprintln!("      {}: not read (past the controller's 256 bytes)", n),
                EcVal::Skipped => {}
            }
        }
        match stop {
            Some(EcErr::Command(a)) => crate::kprintln!("    stopped at byte 0x{:02X}: the controller did not take the read command in time", a),
            Some(EcErr::Address(a)) => crate::kprintln!("    stopped at byte 0x{:02X}: the controller did not take the address in time", a),
            Some(EcErr::Answer(a)) => crate::kprintln!("    stopped at byte 0x{:02X}: the controller did not answer in time", a),
            Some(EcErr::Lock(a)) => crate::kprintln!("    stopped at byte 0x{:02X}: the ACPI global lock was not free within 100 ms", a),
            _ => {}
        }
        match port.lock {
            Some(_) => crate::kprintln!(
                "    {} byte(s) read, each under the ACPI global lock ({}; the firmware held it {} time(s), was told it was free {} time(s))",
                bytes, if matches!(d.glk, Sta::Absent) { "the firmware held it when we began" } else { "its _GLK asks for it" }, port.waited, port.signalled
            ),
            None => crate::kprintln!("    {} byte(s) read; the ACPI global lock was not needed (its _GLK is 0, or it has none and the lock was free)", bytes),
        }
    }
    if !any {
        crate::kprintln!("  embedded controller: none described");
    }
    show_psr(&w);
    show_bst(&w);
}

/// Why method `m` could not be worked out, as the rest of a console line.
fn say_why(m: &str, y: Why) {
    match y {
        Why::Op(x) => crate::kprintln!("{} uses AML form 0x{:02X}, which this interpreter does not run yet", m, x),
        Why::Name(s) => crate::kprintln!("{} refers to {}, which is neither a constant Name nor an embedded controller field", m, seg_str(&s)),
        Why::Read(s, EcErr::Dead) => crate::kprintln!("{} lives in the embedded controller, and nothing answers at its command port (it reads 0xFF)", seg_str(&s)),
        Why::Read(s, EcErr::Waiting) => crate::kprintln!("{} lives in the embedded controller, and a byte is waiting there for another reader", seg_str(&s)),
        Why::Read(s, EcErr::Lock(_)) => crate::kprintln!("reading {} needs the ACPI global lock, which was not free within 100 ms", seg_str(&s)),
        Why::Read(s, EcErr::NoLock) => crate::kprintln!("{} lives in an embedded controller that asks for the ACPI global lock, and there is no FACS to take it in", seg_str(&s)),
        Why::Read(s, _) => crate::kprintln!("reading {} from the embedded controller did not finish in time", seg_str(&s)),
        Why::NoController(s) => crate::kprintln!("{} is in an embedded controller whose two ports are not given plainly", seg_str(&s)),
        Why::Wide(s) => crate::kprintln!("{} is wider than 32 bits or past the controller's 256 bytes", seg_str(&s)),
        Why::Deep => crate::kprintln!("{} nests deeper than this interpreter goes", m),
        Why::NoReturn => crate::kprintln!("{} ends without returning a value", m),
        Why::Bad => crate::kprintln!("{} ends in the middle of a form", m),
        Why::Many => crate::kprintln!("{} reads more than {} fields", m, EVAL_READS),
        Why::Write(x) => crate::kprintln!("{} stores into something other than a Local (form 0x{:02X}), and nothing outside the method is ever written", m, x),
        Why::Unset(n) => crate::kprintln!("{} reads Local{} before storing anything in it", m, n),
        Why::Pkg => crate::kprintln!("{} uses a Package or a string where a number is needed", m),
        Why::Index => crate::kprintln!("{} uses Index on something other than a Package held in a Local, an Arg or a Name, or past its end", m),
        Why::NoArg(n) => crate::kprintln!("{} reads Arg{}, which it was not given", m, n),
        Why::Big(s) => crate::kprintln!("{} calls {}, a method longer than {} bytes, which this interpreter does not keep", m, seg_str(&s), MAX_BODY),
        Why::Calls => crate::kprintln!("{} calls methods more than {} deep", m, EVAL_CALLS),
        Why::Zero => crate::kprintln!("{} divides by Zero", m),
        Why::Loops => crate::kprintln!("{} goes round its loops more than {} times", m, EVAL_LOOPS),
        Why::NoWhile => crate::kprintln!("{} has a Break or Continue outside a While", m),
        Why::Short => crate::kprintln!("{} declares a Package with more elements than it gives", m),
    }
}

fn show_psr(w: &Walk) {
    for d in w.devs.iter().filter(|d| d.kind == Kind::Ac) {
        crate::kprint!("  AC adapter ");
        print_path(&d.path);
        let Some(body) = d.psr.as_ref() else {
            crate::kprintln!(": no _PSR method to run");
            continue;
        };
        let mut scope = d.path.clone();
        scope.push(*b"_PSR");
        let mut port = Ports::new();
        let mut read = |k: usize, u: &Unit| ec_field(w, k, u, &mut port);
        match eval_int(w, &scope, body, &mut read) {
            Ok(0) => crate::kprintln!(": on battery (_PSR gives 0x0)"),
            Ok(v) => crate::kprintln!(": on mains power (_PSR gives 0x{:X})", v),
            Err(y) => {
                crate::kprint!(": power source not worked out: ");
                say_why("_PSR", y);
            }
        }
    }
}

/// A _BST or _BIF number with its unit: 0xFFFFFFFF means the battery does not know it (ACPI spec 10.2.2.11).
fn bst_num(v: u64, unit: &str) {
    if v == 0xFFFF_FFFF {
        crate::kprint!("unknown");
    } else {
        crate::kprint!("{} {}", v, unit);
    }
}

/// A battery's _BST package of four: state bits, present rate, remaining capacity, present voltage in mV. `units`
/// are the rate's and the capacity's units from _BIF or _BIX, when it was read.
fn show_bst_pkg(p: &[u64], units: Option<(&str, &str)>) {
    crate::kprint!(": state 0x{:X} (", p[0]);
    let words: [(u64, &str); 4] = [(1, "discharging"), (2, "charging"), (4, "critical"), (8, "charge limited")];
    let mut first = true;
    for &(bit, s) in words.iter() {
        if p[0] & bit != 0 {
            crate::kprint!("{}{}", if first { "" } else { ", " }, s);
            first = false;
        }
    }
    if first {
        crate::kprint!("neither charging nor discharging");
    }
    let (rate, cap) = units.unwrap_or(("mW or mA", "mWh or mAh"));
    crate::kprint!("), rate ");
    bst_num(p[1], rate);
    crate::kprint!(", remaining ");
    bst_num(p[2], cap);
    crate::kprint!(", voltage ");
    bst_num(p[3], "mV");
    crate::kprintln!("");
}

/// A string from the firmware, printable characters only.
fn print_text(s: &[u8]) {
    for &c in s.iter() {
        crate::kprint!("{}", if (0x20..0x7F).contains(&c) { c as char } else { '?' });
    }
}

fn show_info(n: &[u8; 4], i: &Info) {
    crate::kprint!("    info ({}): design capacity ", seg_str(n));
    bst_num(i.design, i.units.1);
    crate::kprint!(", last full charge ");
    bst_num(i.full, i.units.1);
    crate::kprint!(", design voltage ");
    bst_num(i.volts, "mV");
    if let Some(c) = i.cycles.filter(|&c| c != 0xFFFF_FFFF) {
        crate::kprint!(", {} charge cycles", c);
    }
    for (label, s) in ["model", "serial", "type"].iter().zip(i.text.iter()) {
        let s = s.trim_ascii();
        if !s.is_empty() {
            crate::kprint!(", {} ", label);
            print_text(s);
        }
    }
    crate::kprintln!("");
}

fn show_bst(w: &Walk) {
    for d in w.devs.iter().filter(|d| d.kind == Kind::Battery) {
        crate::kprint!("  battery ");
        print_path(&d.path);
        let mut port = Ports::new();
        let mut read = |k: usize, u: &Unit| ec_field(w, k, u, &mut port);
        let info = info_of(w, d, &mut read);
        let parsed = match &info {
            Some((n, Ok(Val::Pkg(p)))) => info_from(n == b"_BIX", p),
            _ => None,
        };
        let mut remaining = None;
        match d.bst.as_ref().map(|body| run_bat(w, d, *b"_BST", body, &mut read)) {
            None => crate::kprintln!(": no _BST method to run"),
            Some(Ok(Val::Pkg(p))) if p.len() == 4 => {
                let ints: Option<Vec<u64>> = p.iter().map(|e| if let El::Int(v) = e { Some(*v) } else { None }).collect();
                match ints {
                    Some(v) => {
                        show_bst_pkg(&v, parsed.as_ref().map(|i| i.units));
                        remaining = Some(v[2]);
                    }
                    None => crate::kprintln!(": charge not worked out: _BST gives a string where a number is needed"),
                }
            }
            Some(Ok(Val::Pkg(p))) => crate::kprintln!(": charge not worked out: _BST gives a Package of {} elements, not 4", p.len()),
            Some(Ok(Val::Int(v))) => crate::kprintln!(": charge not worked out: _BST gives the number 0x{:X}, not a Package", v),
            Some(Ok(Val::Str(_))) => crate::kprintln!(": charge not worked out: _BST gives a string, not a Package"),
            Some(Err(y)) => {
                crate::kprint!(": charge not worked out: ");
                say_why("_BST", y);
            }
        }
        match (&info, &parsed) {
            (None, _) => crate::kprintln!("    info: no _BIF or _BIX, so the units and the full capacity are not known"),
            (Some((n, _)), Some(i)) => show_info(n, i),
            (Some((n, Ok(Val::Pkg(p)))), None) => {
                crate::kprintln!("    info ({}): not worked out: it gives a Package of {} elements that is not the shape the ACPI spec gives", seg_str(n), p.len())
            }
            (Some((n, Ok(_))), None) => crate::kprintln!("    info ({}): not worked out: it gives a number or a string, not a Package", seg_str(n)),
            (Some((n, Err(y))), None) => {
                crate::kprint!("    info ({}): not worked out: ", seg_str(n));
                say_why(seg_str(n), *y);
            }
        }
        if let (Some(i), Some(r)) = (&parsed, remaining) {
            match charge_pct(r, i.full) {
                Some(pc) => crate::kprintln!("    charge: {}% of the last full charge", pc),
                None => crate::kprintln!("    charge: not worked out (the remaining or the last full charge is unknown)"),
            }
        }
    }
}

fn ec_selftest() -> bool {
    let unit = |name: &[u8; 4], bit: u32, bits: u32| Unit { name: *name, bit, bits };
    let us = [
        unit(b"BST0", 0, 8),
        unit(b"BRC0", 16, 16),
        unit(b"ACIN", 36, 1),
        unit(b"B1ST", 0, 8),
        unit(b"SERN", 40, 64),
        unit(b"FAR_", 0x100 * 8, 8),
        unit(b"NIBL", 36, 4),
    ];
    let mut m = MockEc::new();
    m.ram[0] = 0x5A;
    m.ram[2] = 0x34;
    m.ram[3] = 0x12;
    m.ram[4] = 0x9F;
    m.ram[5] = 0x77;
    let (v, bytes, stop) = ec_read_units(&mut m, 0x62, 0x66, &us);
    let good = stop.is_none()
        && !m.bad
        && bytes == 4
        && m.writes == 8
        && v == [
            EcVal::Value(0x5A),
            EcVal::Value(0x1234),
            EcVal::Value(1),
            EcVal::Value(0x5A),
            EcVal::Wide,
            EcVal::Past,
            EcVal::Value(0x9),
        ];
    let mut dead = MockEc::new();
    dead.dead = true;
    let (_, b, s) = ec_read_units(&mut dead, 0x62, 0x66, &us);
    let dead_ok = s == Some(EcErr::Dead) && b == 0 && dead.writes == 0;
    let mut stuck = MockEc::new();
    stuck.stuck = true;
    let (v, b, s) = ec_read_units(&mut stuck, 0x62, 0x66, &us);
    let stuck_ok = s == Some(EcErr::Command(0)) && b == 0 && stuck.writes == 0 && v.iter().all(|x| *x == EcVal::Skipped);
    let mut waiting = MockEc::new();
    waiting.ans = Some(0x11);
    let (_, b, s) = ec_read_units(&mut waiting, 0x62, 0x66, &us);
    let waiting_ok = s == Some(EcErr::Waiting) && b == 0 && waiting.writes == 0;
    // a byte for another reader turns up between two reads (architect, 14 Nov): the next read sends nothing
    let mut stale = MockEc::new();
    stale.ram[0] = 0x5A;
    stale.stale_after = Some(1);
    let (v, b, s) = ec_read_units(&mut stale, 0x62, 0x66, &us);
    let stale_ok = s == Some(EcErr::Waiting) && b == 1 && stale.writes == 2 && v[0] == EcVal::Value(0x5A) && v[1] == EcVal::Skipped;
    let refused = ec_port_refused(0x60) && ec_port_refused(0x3F8) && ec_port_refused(0xCFC) && !ec_port_refused(0x62) && !ec_port_refused(0x66);
    good && dead_ok && stuck_ok && waiting_ok && stale_ok && refused
}

// ---- selftest: small AML blocks built here, walked, and the result checked ----

fn cat(parts: &[&[u8]]) -> Vec<u8> {
    let mut v = Vec::new();
    for p in parts {
        v.extend_from_slice(p);
    }
    v
}

/// `op` PkgLength `rest`, with the PkgLength worked out.
fn enc(op: &[u8], rest: &[u8]) -> Vec<u8> {
    let c = rest.len();
    let mut v = Vec::from(op);
    if c + 1 < 64 {
        v.push((c + 1) as u8);
    } else {
        let l = c + 2;
        v.push(0x40 | (l & 0x0F) as u8);
        v.push((l >> 4) as u8);
    }
    v.extend_from_slice(rest);
    v
}

fn dev(name: &[u8], body: &[u8]) -> Vec<u8> {
    enc(&[0x5B, 0x82], &cat(&[name, body]))
}
fn method(name: &[u8]) -> Vec<u8> {
    enc(&[0x14], &cat(&[name, &[0x00, 0xA4, 0x01]]))
}
fn hid_eisa(b: [u8; 4]) -> Vec<u8> {
    cat(&[&[0x08], b"_HID", &[0x0C], &b])
}
fn hid_str(s: &[u8]) -> Vec<u8> {
    cat(&[&[0x08], b"_HID", &[0x0D], s, &[0x00]])
}

fn walk_body(body: &[u8]) -> Walk {
    let mut t = Vec::from([0u8; 36].as_slice());
    t.extend_from_slice(body);
    let mut w = Walk::default();
    walk_table(&t, &mut w);
    settle(&mut w);
    w
}

fn sta_is(w: &Walk, path: &[u8], want: Option<u64>) -> bool {
    w.devs.iter().any(|d| {
        path_is(&d.path, path)
            && match (d.sta, want) {
                (Sta::Value(v), Some(x)) => v == x,
                (Sta::Run, None) => true,
                _ => false,
            }
    })
}

fn path_is(p: &[[u8; 4]], s: &[u8]) -> bool {
    let mut v = Vec::from(b"\\".as_slice());
    for (k, seg) in p.iter().enumerate() {
        if k > 0 {
            v.push(b'.');
        }
        v.extend_from_slice(seg);
    }
    v == s
}

fn has(w: &Walk, k: Kind, path: &[u8], methods: &[&[u8; 4]], cut: bool) -> bool {
    w.devs.iter().any(|d| {
        d.kind == k && path_is(&d.path, path) && d.cut == cut && d.methods.len() == methods.len() && d.methods.iter().zip(methods).all(|(a, b)| a == *b)
    })
}

/// Scope(\_SB) { Device(EC0_) {_HID PNP0C09, OperationRegion(ECOR, EmbeddedControl, 0, 0xFF),
///                               Field(ECOR) {ACIN 1, , 7, BRC0 16},
///                               Device(ADP0) {_HID "ACPI0003", _PSR: If (ACIN) {Return(One)} Else {Return(Zero)}}}
///                 Device(ADP1) {_PSR: If (LEqual(^^EC0_.ACIN, One)) {Return(One)} Return(Zero)}
///                 Device(ADP2) {_PSR: Return(And(PSRC, One, Zero))}   Name(PSRC, 3) declared after it
///                 Device(ADP3) {_PSR: Store(One, Local0) Return(Local0)}
///                 Device(ADP4) {_PSR: Return(MTHD)}   MTHD is never declared
///                 Device(ADP5) {_PSR: If (LNot(LLess(\_SB_.EC0_.BRC0, 0x10))) {Return(One)}}   may end with no Return
///                 Device(ADP6) {_PSR: Return(Or(PSRC, Zero, Local1))}   the Or's target is a Local
///                 Device(ADP7) {_PSR: Store(One, ACIN) Return(One)}   stops before writing a field
///                 Device(ADP8) {_PSR: Return(Local2)}   Local2 was never stored
///                 Device(ADP9) {_PSR: Store(\_SB_.EC0_.BRC0, Local0) Store(Local0, Debug) XOr(Local0, 0x10, Local3)
///                                     If (LEqual(Local3, Zero)) {Return(One)} Return(Local3)}   one read, used twice }
/// with the controller's fields given by a pretend reader.
fn psr_selftest() -> bool {
    let ac = hid_str(b"ACPI0003");
    let psr = |body: &[u8]| enc(&[0x14], &cat(&[b"_PSR", &[0x00], body]));
    let adp = |name: &[u8], body: &[u8]| dev(name, &cat(&[&ac, &psr(body)]));
    let ret = |v: u8| [0xA4, v];
    let region = cat(&[&[0x5B, 0x80], b"ECOR", &[0x03, 0x00, 0x0A, 0xFF]]);
    let field = enc(&[0x5B, 0x81], &cat(&[b"ECOR", &[0x01], b"ACIN", &[0x01, 0x00, 0x07], b"BRC0", &[0x10]]));
    let adp0 = adp(b"ADP0", &cat(&[&enc(&[0xA0], &cat(&[b"ACIN", &ret(1)])), &enc(&[0xA1], &ret(0))]));
    let ec0 = dev(b"EC0_", &cat(&[&hid_eisa(EC), &region, &field, &adp0]));
    let up = cat(&[b"^^", &[0x2E], b"EC0_ACIN"]);
    let adp1 = adp(b"ADP1", &cat(&[&enc(&[0xA0], &cat(&[&[0x93], &up, &[0x01], &ret(1)])), &ret(0)]));
    let adp2 = adp(b"ADP2", &cat(&[&[0xA4, 0x7B], b"PSRC", &[0x01, 0x00]]));
    let adp3 = adp(b"ADP3", &[0x70, 0x01, 0x60, 0xA4, 0x60]);
    let adp4 = adp(b"ADP4", &cat(&[&[0xA4], b"MTHD"]));
    let brc = cat(&[b"\\", &[0x2F, 0x03], b"_SB_EC0_BRC0"]);
    let adp5 = adp(b"ADP5", &enc(&[0xA0], &cat(&[&[0x92, 0x95], &brc, &[0x0A, 0x10], &ret(1)])));
    let adp6 = adp(b"ADP6", &cat(&[&[0xA4, 0x7D], b"PSRC", &[0x00, 0x61]]));
    let adp7 = adp(b"ADP7", &cat(&[&[0x70, 0x01], b"ACIN", &[0xA4, 0x01]]));
    let adp8 = adp(b"ADP8", &[0xA4, 0x62]);
    let if9 = enc(&[0xA0], &[0x93, 0x63, 0x00, 0xA4, 0x01]);
    let adp9 = adp(b"ADP9", &cat(&[&[0x70], &brc, &[0x60, 0x70, 0x60, 0x5B, 0x31, 0x7F, 0x60, 0x0A, 0x10, 0x63], &if9, &[0xA4, 0x63]]));
    let psrc = cat(&[&[0x08], b"PSRC", &[0x0A, 0x03]]);
    let sb = cat(&[b"\\", b"_SB_"]);
    let w = walk_body(&enc(&[0x10], &cat(&[&sb, &ec0, &adp1, &adp2, &adp3, &adp4, &adp5, &adp6, &adp7, &adp8, &adp9, &psrc])));
    let run = |path: &[u8], acin: u64, brc: u64, fail: bool| -> (Result<u64, Why>, u32) {
        let Some(d) = w.devs.iter().find(|d| path_is(&d.path, path)) else { return (Err(Why::Bad), 99) };
        let Some(body) = d.psr.as_ref() else { return (Err(Why::Bad), 99) };
        let mut scope = d.path.clone();
        scope.push(*b"_PSR");
        let mut n = 0u32;
        let mut read = |_: usize, u: &Unit| -> Result<u64, Why> {
            n += 1;
            match (&u.name, fail) {
                (_, true) => Err(Why::Read(u.name, EcErr::Dead)),
                (b"ACIN", _) if u.bit == 0 && u.bits == 1 => Ok(acin),
                (b"BRC0", _) if u.bit == 8 && u.bits == 16 => Ok(brc),
                _ => Err(Why::Bad),
            }
        };
        let r = eval_int(&w, &scope, body, &mut read);
        (r, n)
    };
    let adps = w.devs.iter().filter(|d| d.kind == Kind::Ac && d.psr.is_some()).count();
    let checks: [(&[u8], u64, u64, bool, (Result<u64, Why>, u32)); 15] = [
        (b"\\_SB_.EC0_.ADP0", 1, 0, false, (Ok(1), 1)),
        (b"\\_SB_.EC0_.ADP0", 0, 0, false, (Ok(0), 1)),
        (b"\\_SB_.ADP1", 1, 0, false, (Ok(1), 1)),
        (b"\\_SB_.ADP1", 0, 0, false, (Ok(0), 1)),
        (b"\\_SB_.ADP1", 1, 0, true, (Err(Why::Read(*b"ACIN", EcErr::Dead)), 1)),
        (b"\\_SB_.ADP2", 0, 0, false, (Ok(1), 0)),
        (b"\\_SB_.ADP3", 0, 0, false, (Ok(1), 0)),
        (b"\\_SB_.ADP4", 0, 0, false, (Err(Why::Name(*b"MTHD")), 0)),
        (b"\\_SB_.ADP5", 0, 0x20, false, (Ok(1), 1)),
        (b"\\_SB_.ADP5", 0, 0x05, false, (Err(Why::NoReturn), 1)),
        (b"\\_SB_.ADP6", 1, 0, false, (Ok(3), 0)),
        (b"\\_SB_.ADP7", 1, 0, false, (Err(Why::Write(b'A')), 0)),
        (b"\\_SB_.ADP8", 0, 0, false, (Err(Why::Unset(2)), 0)),
        (b"\\_SB_.ADP9", 0, 0x10, false, (Ok(1), 1)),
        (b"\\_SB_.ADP9", 0, 0x30, false, (Ok(0x20), 1)),
    ];
    let mut ok = w.cut == 0 && adps == 10;
    if !ok {
        crate::kprintln!("battery selftest:   power source walk: {} cut short, {} adapters with a _PSR (expected 0 and 10)", w.cut, adps);
    }
    for (k, (path, acin, brc, fail, want)) in checks.iter().enumerate() {
        let got = run(path, *acin, *brc, *fail);
        if got != *want {
            ok = false;
            crate::kprintln!("battery selftest:   power source check {}: got {:?}, expected {:?}", k + 1, got, want);
        }
    }
    ok
}

/// Scope(\_SB) { Device(EC0_) {_HID PNP0C09, OperationRegion(ECOR, EmbeddedControl, 0, 0xFF), Field(ECOR) {ACIN 1, , 7, BRC0 16}}
///               Device(BATA) {_BST: Return(Package(4) {0x02, 0x05DC, \_SB_.EC0_.BRC0, 0x2EE0})}
///               Device(BATB) {_BST: Store(Package(4) {0, 0, 0, 0}, Local0) Store(\_SB_.EC0_.BRC0, Index(Local0, 2))
///                                   Store(One, Index(Local0, Zero)) Return(Local0)}
///               Device(BATC) {_BST: Store(Package(4) {0, 0, 0x0BB8, 0}, Local0) Return(DerefOf(Index(Local0, 2)))}
///               Device(BATD) {_BST: Store(Package(2) {One, One}, Local0) Store(One, Index(Local0, 5)) Return(Local0)}
///               Device(BATE) {_BST: Return(Package(4) {One})}   declares more than it gives
///               Device(BATF) {_BST: Store(One, Index(PBST, Zero)) Store(BRC0, Index(PBST, 2)) Return(PBST)}
///                                   this run's copy of the named Package
///               Device(BATG) {_BST: Store(Package(1) {Zero}, Local0) Return(LEqual(Local0, Zero))}
///               Device(BATH) {_BST: Store(One, Index(Local1, Zero)) Return(One)}   Local1 was never stored
///               Device(BATI) {_BST: Return(PBST)}   run after BATF: the tables' values, BATF changed only its copy
///               Device(BATJ) {_BST: Store(One, Index(Arg0, Zero)) Return(One)}   an Arg holding no Package stops
///               Device(BATK) {_BST: Store(5, Index(\_SB_.PBST, One)) Return(DerefOf(Index(PBST, One)))}
///                                   the full path and the short name are the same copy
///               Device(BATL) {_BST: Return(PSTR)}   a Package holding a string is kept, the string with it
///               Name(PBST, Package(4) {Zero, Zero, Zero, Zero})  Name(PSTR, Package(1) {"A"}) }
/// with the controller's fields given by a pretend reader.
fn bst_selftest() -> bool {
    let bst = |body: &[u8]| enc(&[0x14], &cat(&[b"_BST", &[0x00], body]));
    let bat = |name: &[u8], body: &[u8]| dev(name, &cat(&[&hid_eisa(BAT), &bst(body)]));
    let pkg = |n: u8, els: &[u8]| enc(&[0x12], &cat(&[&[n], els]));
    let region = cat(&[&[0x5B, 0x80], b"ECOR", &[0x03, 0x00, 0x0A, 0xFF]]);
    let field = enc(&[0x5B, 0x81], &cat(&[b"ECOR", &[0x01], b"ACIN", &[0x01, 0x00, 0x07], b"BRC0", &[0x10]]));
    let ec0 = dev(b"EC0_", &cat(&[&hid_eisa(EC), &region, &field]));
    let brc = cat(&[b"\\", &[0x2F, 0x03], b"_SB_EC0_BRC0"]);
    let zeros = pkg(4, &[0x00, 0x00, 0x00, 0x00]);
    let bata = bat(b"BATA", &cat(&[&[0xA4], &pkg(4, &cat(&[&[0x0A, 0x02, 0x0B, 0xDC, 0x05], &brc, &[0x0B, 0xE0, 0x2E]]))]));
    let batb = bat(
        b"BATB",
        &cat(&[&[0x70], &zeros, &[0x60, 0x70], &brc, &[0x88, 0x60, 0x0A, 0x02, 0x00], &[0x70, 0x01, 0x88, 0x60, 0x00, 0x00], &[0xA4, 0x60]]),
    );
    let batc = bat(b"BATC", &cat(&[&[0x70], &pkg(4, &[0x00, 0x00, 0x0B, 0xB8, 0x0B, 0x00]), &[0x60, 0xA4, 0x83, 0x88, 0x60, 0x0A, 0x02, 0x00]]));
    let batd = bat(b"BATD", &cat(&[&[0x70], &pkg(2, &[0x01, 0x01]), &[0x60, 0x70, 0x01, 0x88, 0x60, 0x0A, 0x05, 0x00, 0xA4, 0x60]]));
    let bate = bat(b"BATE", &cat(&[&[0xA4], &pkg(4, &[0x01])]));
    let batf = bat(
        b"BATF",
        &cat(&[&[0x70, 0x01, 0x88], b"PBST", &[0x00, 0x00, 0x70], &brc, &[0x88], b"PBST", &[0x0A, 0x02, 0x00, 0xA4], b"PBST"]),
    );
    let batg = bat(b"BATG", &cat(&[&[0x70], &pkg(1, &[0x00]), &[0x60, 0xA4, 0x93, 0x60, 0x00]]));
    let bath = bat(b"BATH", &[0x70, 0x01, 0x88, 0x61, 0x00, 0x00, 0xA4, 0x01]);
    let bati = bat(b"BATI", &cat(&[&[0xA4], b"PBST"]));
    let batj = bat(b"BATJ", &[0x70, 0x01, 0x88, 0x68, 0x00, 0x00, 0xA4, 0x01]);
    let batk = bat(b"BATK", &cat(&[&[0x70, 0x0A, 0x05, 0x88, b'\\', 0x2E], b"_SB_PBST", &[0x01, 0x00, 0xA4, 0x83, 0x88], b"PBST", &[0x01, 0x00]]));
    let batl = bat(b"BATL", &cat(&[&[0xA4], b"PSTR"]));
    let names = cat(&[&[0x08], b"PBST", &zeros, &[0x08], b"PSTR", &pkg(1, &[0x0D, b'A', 0x00])]);
    let sb = cat(&[b"\\", b"_SB_"]);
    let w = walk_body(&enc(&[0x10], &cat(&[&sb, &ec0, &bata, &batb, &batc, &batd, &bate, &batf, &batg, &bath, &bati, &batj, &batk, &batl, &names])));
    let run = |path: &[u8], brc: u64, int: bool| -> (Result<Val, Why>, u32) {
        let Some(d) = w.devs.iter().find(|d| path_is(&d.path, path)) else { return (Err(Why::Bad), 99) };
        let Some(body) = d.bst.as_ref() else { return (Err(Why::Bad), 99) };
        let mut scope = d.path.clone();
        scope.push(*b"_BST");
        let mut n = 0u32;
        let mut read = |_: usize, u: &Unit| -> Result<u64, Why> {
            n += 1;
            match &u.name {
                b"BRC0" if u.bit == 8 && u.bits == 16 => Ok(brc),
                _ => Err(Why::Bad),
            }
        };
        let r = if int { eval_int(&w, &scope, body, &mut read).map(Val::Int) } else { eval_method(&w, &scope, body, &mut read) };
        (r, n)
    };
    let bats = w.devs.iter().filter(|d| d.kind == Kind::Battery && d.bst.is_some()).count();
    let p = |v: [u64; 4]| Val::Pkg(v.iter().map(|&x| El::Int(x)).collect());
    let checks: [(&[u8], u64, bool, (Result<Val, Why>, u32)); 13] = [
        (b"\\_SB_.BATA", 0x0BB8, false, (Ok(p([2, 1500, 3000, 12000])), 1)),
        (b"\\_SB_.BATA", 0x0BB8, true, (Err(Why::Pkg), 1)),
        (b"\\_SB_.BATB", 0x07D0, false, (Ok(p([1, 0, 2000, 0])), 1)),
        (b"\\_SB_.BATC", 0, false, (Ok(Val::Int(3000)), 0)),
        (b"\\_SB_.BATD", 0, false, (Err(Why::Index), 0)),
        (b"\\_SB_.BATE", 0, false, (Err(Why::Short), 0)),
        (b"\\_SB_.BATF", 0x0BB8, false, (Ok(p([1, 0, 3000, 0])), 1)),
        (b"\\_SB_.BATG", 0, false, (Err(Why::Pkg), 0)),
        (b"\\_SB_.BATH", 0, false, (Err(Why::Unset(1)), 0)),
        (b"\\_SB_.BATI", 0, false, (Ok(p([0, 0, 0, 0])), 0)),
        (b"\\_SB_.BATJ", 0, false, (Err(Why::Index), 0)),
        (b"\\_SB_.BATK", 0, false, (Ok(Val::Int(5)), 0)),
        (b"\\_SB_.BATL", 0, false, (Ok(Val::Pkg(Vec::from([El::Str(Vec::from(&b"A"[..]))]))), 0)),
    ];
    let mut ok = w.cut == 0 && bats == 12 && w.pkgs.len() == 2;
    if !ok {
        crate::kprintln!("battery selftest:   battery status walk: {} cut short, {} batteries with a _BST, {} named Packages kept (expected 0, 12 and 2)", w.cut, bats, w.pkgs.len());
    }
    for (k, (path, brc, int, want)) in checks.iter().enumerate() {
        let got = run(path, *brc, *int);
        if got != *want {
            ok = false;
            crate::kprintln!("battery selftest:   battery status check {}: got {:?}, expected {:?}", k + 1, got, want);
        }
    }
    ok
}

/// Scope(\_SB) { Device(EC0_) {_HID PNP0C09, OperationRegion(ECOR, EmbeddedControl, 0, 0xFF), Field(ECOR) {ACIN 1, , 7, BRC0 16}}
///               Device(BIFA) {_BIF: Store(\_SB_.EC0_.BRC0, Index(PBIF, 2)) Store("LION2", Index(PBIF, 11)) Return(PBIF)}
///               Device(BIXB) {Name(_BIX, Package(20) {0, 0, 50000, 45000, 1, 11100, 0, 0, 32, 0 x7, "M", "S", "T", "O"})}
///               Device(BIXC) {_BIX: Return(Package(4) {One, One, One, One})  _BIF: Return(One)}   _BIX wins, a short one
///               Device(BIFD) {_BIF: Return(Package(13) {Zero, One, 2 to 8, "A", "B", "C", "D"})}   strings in a run's Package
///               Device(BIFE) {_BIF: Return(DerefOf(Index(PBIF, 9)))}   a string where a number is needed
///               Device(BIFF) {}   no _BIF or _BIX
///               Name(PBIF, Package(13) {1, 6000, 0, 1, 12000, 600, 300, 64, 64, "AFRI-TEST", "0001", "LION", "AFRI"}) }
/// with the controller's BRC0 given by a pretend reader as 4500; and the charge percentage worked out.
fn bif_selftest() -> bool {
    let mth = |name: &[u8], body: &[u8]| enc(&[0x14], &cat(&[name, &[0x00], body]));
    let bat = |name: &[u8], body: &[u8]| dev(name, &cat(&[&hid_eisa(BAT), body]));
    let pkg = |n: u8, els: &[u8]| enc(&[0x12], &cat(&[&[n], els]));
    let st = |s: &[u8]| cat(&[&[0x0D], s, &[0x00]]);
    let region = cat(&[&[0x5B, 0x80], b"ECOR", &[0x03, 0x00, 0x0A, 0xFF]]);
    let field = enc(&[0x5B, 0x81], &cat(&[b"ECOR", &[0x01], b"ACIN", &[0x01, 0x00, 0x07], b"BRC0", &[0x10]]));
    let ec0 = dev(b"EC0_", &cat(&[&hid_eisa(EC), &region, &field]));
    let brc = cat(&[b"\\", &[0x2F, 0x03], b"_SB_EC0_BRC0"]);
    let pbif_els = cat(&[
        &[0x01, 0x0B, 0x70, 0x17, 0x00, 0x01, 0x0B, 0xE0, 0x2E, 0x0B, 0x58, 0x02, 0x0B, 0x2C, 0x01, 0x0A, 0x40, 0x0A, 0x40],
        &st(b"AFRI-TEST"),
        &st(b"0001"),
        &st(b"LION"),
        &st(b"AFRI"),
    ]);
    let pbif = cat(&[&[0x08], b"PBIF", &pkg(13, &pbif_els)]);
    let bifa = bat(
        b"BIFA",
        &mth(b"_BIF", &cat(&[&[0x70], &brc, &[0x88], b"PBIF", &[0x0A, 0x02, 0x00, 0x70], &st(b"LION2"), &[0x88], b"PBIF", &[0x0A, 0x0B, 0x00, 0xA4], b"PBIF"])),
    );
    let bix_els = cat(&[
        &[0x00, 0x00, 0x0B, 0x50, 0xC3, 0x0B, 0xC8, 0xAF, 0x01, 0x0B, 0x5C, 0x2B, 0x00, 0x00, 0x0A, 0x20],
        &[0x00; 7],
        &st(b"M"),
        &st(b"S"),
        &st(b"T"),
        &st(b"O"),
    ]);
    let bixb = bat(b"BIXB", &cat(&[&[0x08], b"_BIX", &pkg(20, &bix_els)]));
    let bixc = bat(b"BIXC", &cat(&[&mth(b"_BIX", &cat(&[&[0xA4], &pkg(4, &[0x01, 0x01, 0x01, 0x01])])), &mth(b"_BIF", &[0xA4, 0x01])]));
    let bifd_els = cat(&[&[0x00, 0x01, 0x0A, 0x02, 0x0A, 0x03, 0x0A, 0x04, 0x0A, 0x05, 0x0A, 0x06, 0x0A, 0x07, 0x0A, 0x08], &st(b"A"), &st(b"B"), &st(b"C"), &st(b"D")]);
    let bifd = bat(b"BIFD", &mth(b"_BIF", &cat(&[&[0xA4], &pkg(13, &bifd_els)])));
    let bife = bat(b"BIFE", &mth(b"_BIF", &cat(&[&[0xA4, 0x83, 0x88], b"PBIF", &[0x0A, 0x09, 0x00]])));
    let biff = bat(b"BIFF", &[]);
    let sb = cat(&[b"\\", b"_SB_"]);
    let w = walk_body(&enc(&[0x10], &cat(&[&sb, &ec0, &bifa, &bixb, &bixc, &bifd, &bife, &biff, &pbif])));
    let run = |path: &[u8]| -> Option<([u8; 4], Result<Val, Why>)> {
        let d = w.devs.iter().find(|d| path_is(&d.path, path))?;
        let mut read = |_: usize, u: &Unit| -> Result<u64, Why> {
            match &u.name {
                b"BRC0" if u.bit == 8 && u.bits == 16 => Ok(4500),
                _ => Err(Why::Bad),
            }
        };
        info_of(&w, d, &mut read)
    };
    let parse = |r: &Option<([u8; 4], Result<Val, Why>)>| match r {
        Some((n, Ok(Val::Pkg(p)))) => info_from(n == b"_BIX", p),
        _ => None,
    };
    let v = |s: &[u8]| Vec::from(s);
    let mut ok = w.cut == 0 && w.devs.iter().filter(|d| d.kind == Kind::Battery).count() == 6;
    let a = run(b"\\_SB_.BIFA");
    let want_a = Info { units: ("mA", "mAh"), design: 6000, full: 4500, volts: 12000, cycles: None, text: [v(b"AFRI-TEST"), v(b"0001"), v(b"LION2")] };
    let b = run(b"\\_SB_.BIXB");
    let want_b = Info { units: ("mW", "mWh"), design: 50000, full: 45000, volts: 11100, cycles: Some(32), text: [v(b"M"), v(b"S"), v(b"T")] };
    let c = run(b"\\_SB_.BIXC");
    let d = run(b"\\_SB_.BIFD");
    let want_d = Info { units: ("mW", "mWh"), design: 1, full: 2, volts: 4, cycles: None, text: [v(b"A"), v(b"B"), v(b"C")] };
    let e = run(b"\\_SB_.BIFE");
    let f = run(b"\\_SB_.BIFF");
    let checks = [
        ("a _BIF method changing its named copy", a.as_ref().map(|x| x.0) == Some(*b"_BIF") && parse(&a) == Some(want_a)),
        ("a Name(_BIX, Package)", b.as_ref().map(|x| x.0) == Some(*b"_BIX") && parse(&b) == Some(want_b)),
        ("_BIX before _BIF, a short Package", c.as_ref().map(|x| x.0) == Some(*b"_BIX") && matches!(&c, Some((_, Ok(_)))) && parse(&c).is_none()),
        ("strings in a run's Package", parse(&d) == Some(want_d)),
        ("a string as a number", matches!(e, Some((_, Err(Why::Pkg))))),
        ("no _BIF or _BIX", f.is_none()),
        (
            "percentages",
            charge_pct(3000, 4500) == Some(66)
                && charge_pct(5000, 4500) == Some(100)
                && charge_pct(3000, 0).is_none()
                && charge_pct(0xFFFF_FFFF, 4500).is_none()
                && charge_pct(10, 0xFFFF_FFFF).is_none(),
        ),
    ];
    for (k, (name, good)) in checks.iter().enumerate() {
        if !good {
            ok = false;
            crate::kprintln!("battery selftest:   battery info check {} ({}) failed", k + 1, name);
        }
    }
    ok
}

/// Scope(\_SB) { Device(EC0_) {_HID PNP0C09, OperationRegion(ECOR, EmbeddedControl, 0, 0xFF), Field(ECOR) {ACIN 1, , 7, BRC0 16}}
///               Method(GBST, 2) {Store(Arg0, Index(Arg1, Zero)) Store(\_SB_.EC0_.BRC0, Index(Arg1, 2)) Return(Arg1)}
///               Method(BUMP, 1) {Store(One, Arg0) Return(Arg0)}     Method(RDA1) {Return(Arg1)}
///               Method(LOOP) {Return(LOOP())}     Method(NORT) {Store(One, Local0)}     Method(HUGE) {1100 Noops}
///               Method(SETL) {Store(9, Local0) Return(One)}     Method(ADDA, 2) {Return(Or(Arg0, Arg1))}
///               Device(CALA) {_BST: Return(GBST(2, PBST))}   a helper filling a Package passed to it
///               Device(CALB) {_BST: Store(Zero, Local0) BUMP(Local0) Return(Local0)}   a call as a statement; Args are copies
///               Device(CALC) {_BST: Return(RDA1())}   an Arg it was not given
///               Device(CALD) {_BST: Return(LOOP())}   calls without end
///               Device(CALE) {_BST: Return(NORT())}   a helper that returns nothing
///               Device(CALF) {_BST: Return(HUGE())}   a helper too long to keep
///               Device(CALG) {_BST: Store(5, Local0) Store(SETL(), Local1) Return(Local0)}   the caller's Locals kept
///               Device(CALI) {_BST: Return(LEqual(ADDA(0x10, One), 0x11))}   a call inside an expression
///               Name(PBST, Package(4) {Zero, Zero, Zero, Zero}) }
/// with the controller's BRC0 given by a pretend reader.
fn call_selftest() -> bool {
    let mth = |name: &[u8], flags: u8, body: &[u8]| enc(&[0x14], &cat(&[name, &[flags], body]));
    let bat = |name: &[u8], body: &[u8]| dev(name, &cat(&[&hid_eisa(BAT), &mth(b"_BST", 0, body)]));
    let region = cat(&[&[0x5B, 0x80], b"ECOR", &[0x03, 0x00, 0x0A, 0xFF]]);
    let field = enc(&[0x5B, 0x81], &cat(&[b"ECOR", &[0x01], b"ACIN", &[0x01, 0x00, 0x07], b"BRC0", &[0x10]]));
    let ec0 = dev(b"EC0_", &cat(&[&hid_eisa(EC), &region, &field]));
    let brc = cat(&[b"\\", &[0x2F, 0x03], b"_SB_EC0_BRC0"]);
    let helpers = cat(&[
        &mth(b"GBST", 2, &cat(&[&[0x70, 0x68, 0x88, 0x69, 0x00, 0x00, 0x70], &brc, &[0x88, 0x69, 0x0A, 0x02, 0x00, 0xA4, 0x69]])),
        &mth(b"BUMP", 1, &[0x70, 0x01, 0x68, 0xA4, 0x68]),
        &mth(b"RDA1", 0, &[0xA4, 0x69]),
        &mth(b"LOOP", 0, &cat(&[&[0xA4], b"LOOP"])),
        &mth(b"NORT", 0, &[0x70, 0x01, 0x60]),
        &mth(b"HUGE", 0, &[0xA3; 1100]),
        &mth(b"SETL", 0, &[0x70, 0x0A, 0x09, 0x60, 0xA4, 0x01]),
        &mth(b"ADDA", 2, &[0xA4, 0x7D, 0x68, 0x69, 0x00]),
    ]);
    let devs = cat(&[
        &bat(b"CALA", &cat(&[&[0xA4], b"GBST", &[0x0A, 0x02], b"PBST"])),
        &bat(b"CALB", &cat(&[&[0x70, 0x00, 0x60], b"BUMP", &[0x60, 0xA4, 0x60]])),
        &bat(b"CALC", &cat(&[&[0xA4], b"RDA1"])),
        &bat(b"CALD", &cat(&[&[0xA4], b"LOOP"])),
        &bat(b"CALE", &cat(&[&[0xA4], b"NORT"])),
        &bat(b"CALF", &cat(&[&[0xA4], b"HUGE"])),
        &bat(b"CALG", &cat(&[&[0x70, 0x0A, 0x05, 0x60, 0x70], b"SETL", &[0x61, 0xA4, 0x60]])),
        &bat(b"CALI", &cat(&[&[0xA4, 0x93], b"ADDA", &[0x0A, 0x10, 0x01, 0x0A, 0x11]])),
    ]);
    let names = cat(&[&[0x08], b"PBST", &enc(&[0x12], &[0x04, 0x00, 0x00, 0x00, 0x00])]);
    let sb = cat(&[b"\\", b"_SB_"]);
    let w = walk_body(&enc(&[0x10], &cat(&[&sb, &ec0, &helpers, &devs, &names])));
    let run = |path: &[u8]| -> (Result<Val, Why>, u32) {
        let Some(d) = w.devs.iter().find(|d| path_is(&d.path, path)) else { return (Err(Why::Bad), 99) };
        let Some(body) = d.bst.as_ref() else { return (Err(Why::Bad), 99) };
        let mut n = 0u32;
        let mut read = |_: usize, u: &Unit| -> Result<u64, Why> {
            n += 1;
            match &u.name {
                b"BRC0" if u.bit == 8 && u.bits == 16 => Ok(3000),
                _ => Err(Why::Bad),
            }
        };
        let r = run_bat(&w, d, *b"_BST", body, &mut read);
        (r, n)
    };
    let p = |v: [u64; 4]| Val::Pkg(v.iter().map(|&x| El::Int(x)).collect());
    let checks: [(&[u8], (Result<Val, Why>, u32)); 8] = [
        (b"\\_SB_.CALA", (Ok(p([2, 0, 3000, 0])), 1)),
        (b"\\_SB_.CALB", (Ok(Val::Int(0)), 0)),
        (b"\\_SB_.CALC", (Err(Why::NoArg(1)), 0)),
        (b"\\_SB_.CALD", (Err(Why::Calls), 0)),
        (b"\\_SB_.CALE", (Err(Why::NoReturn), 0)),
        (b"\\_SB_.CALF", (Err(Why::Big(*b"HUGE")), 0)),
        (b"\\_SB_.CALG", (Ok(Val::Int(5)), 0)),
        (b"\\_SB_.CALI", (Ok(Val::Int(u64::MAX)), 0)),
    ];
    let kept = w.callees.iter().filter(|(_, _, b)| b.is_some()).count();
    let huge = w.callees.iter().any(|(q, _, b)| path_is(q, b"\\_SB_.HUGE") && b.is_none());
    let gbst = w.callees.iter().any(|(q, n, _)| path_is(q, b"\\_SB_.GBST") && *n == 2);
    let mut ok = w.cut == 0 && w.callees.len() == 16 && kept == 15 && huge && gbst;
    if !ok {
        crate::kprintln!("battery selftest:   method calls walk: {} cut short, {} methods, {} kept (expected 0, 16 and 15), HUGE dropped {}, GBST with 2 args {}", w.cut, w.callees.len(), kept, huge, gbst);
    }
    for (k, (path, want)) in checks.iter().enumerate() {
        let got = run(path);
        if got != *want {
            ok = false;
            crate::kprintln!("battery selftest:   method calls check {}: got {:?}, expected {:?}", k + 1, got, want);
        }
    }
    ok
}

/// Scope(\_SB) { Device(EC0_) {_HID PNP0C09, OperationRegion(ECOR, EmbeddedControl, 0, 0xFF), Field(ECOR) {ACIN 1, , 7, BRC0 16}}
///               Device(ARA_) {_BST: Return(Add(0x10, 0x20))}
///               Device(ARB_) {_BST: Return(Subtract(One, 2))}   wraps at 64 bits
///               Device(ARC_) {_BST: Store(3000, Local0) Multiply(Local0, 10, Local1) Return(Local1)}   a statement with a target
///               Device(ARD_) {_BST: Return(Or(ShiftLeft(One, 0x40), ShiftRight(3000, 4)))}   a shift by 64 gives Zero
///               Device(ARE_) {_BST: Divide(3001, 10, Local0, Local1) Return(Add(Multiply(Local1, 10), Local0))}
///               Device(ARF_) {_BST: Return(Divide(One, Zero, Local0, Local1))}   by Zero
///               Device(ARG_) {_BST: Return(Mod(3000, 7))}
///               Device(ARH_) {_BST: Return(And(Not(0x0F), 0xFF))}
///               Device(ARI_) {_BST: Return(Subtract(\_SB_.EC0_.BRC0, 100))}   a controller field scaled }
/// with the controller's BRC0 given by a pretend reader.
fn arith_selftest() -> bool {
    let mth = |name: &[u8], body: &[u8]| enc(&[0x14], &cat(&[name, &[0x00], body]));
    let bat = |name: &[u8], body: &[u8]| dev(name, &cat(&[&hid_eisa(BAT), &mth(b"_BST", body)]));
    let region = cat(&[&[0x5B, 0x80], b"ECOR", &[0x03, 0x00, 0x0A, 0xFF]]);
    let field = enc(&[0x5B, 0x81], &cat(&[b"ECOR", &[0x01], b"ACIN", &[0x01, 0x00, 0x07], b"BRC0", &[0x10]]));
    let ec0 = dev(b"EC0_", &cat(&[&hid_eisa(EC), &region, &field]));
    let brc = cat(&[b"\\", &[0x2F, 0x03], b"_SB_EC0_BRC0"]);
    let devs = cat(&[
        &bat(b"ARA_", &[0xA4, 0x72, 0x0A, 0x10, 0x0A, 0x20, 0x00]),
        &bat(b"ARB_", &[0xA4, 0x74, 0x01, 0x0A, 0x02, 0x00]),
        &bat(b"ARC_", &[0x70, 0x0B, 0xB8, 0x0B, 0x60, 0x77, 0x60, 0x0A, 0x0A, 0x61, 0xA4, 0x61]),
        &bat(b"ARD_", &[0xA4, 0x7D, 0x79, 0x01, 0x0A, 0x40, 0x00, 0x7A, 0x0B, 0xB8, 0x0B, 0x0A, 0x04, 0x00, 0x00]),
        &bat(b"ARE_", &[0x78, 0x0B, 0xB9, 0x0B, 0x0A, 0x0A, 0x60, 0x61, 0xA4, 0x72, 0x77, 0x61, 0x0A, 0x0A, 0x00, 0x60, 0x00]),
        &bat(b"ARF_", &[0xA4, 0x78, 0x01, 0x00, 0x60, 0x61]),
        &bat(b"ARG_", &[0xA4, 0x85, 0x0B, 0xB8, 0x0B, 0x0A, 0x07, 0x00]),
        &bat(b"ARH_", &[0xA4, 0x7B, 0x80, 0x0A, 0x0F, 0x00, 0x0A, 0xFF, 0x00]),
        &bat(b"ARI_", &cat(&[&[0xA4, 0x74], &brc, &[0x0A, 0x64, 0x00]])),
    ]);
    let sb = cat(&[b"\\", b"_SB_"]);
    let w = walk_body(&enc(&[0x10], &cat(&[&sb, &ec0, &devs])));
    let run = |path: &[u8]| -> (Result<Val, Why>, u32) {
        let Some(d) = w.devs.iter().find(|d| path_is(&d.path, path)) else { return (Err(Why::Bad), 99) };
        let Some(body) = d.bst.as_ref() else { return (Err(Why::Bad), 99) };
        let mut n = 0u32;
        let mut read = |_: usize, u: &Unit| -> Result<u64, Why> {
            n += 1;
            match &u.name {
                b"BRC0" if u.bit == 8 && u.bits == 16 => Ok(3000),
                _ => Err(Why::Bad),
            }
        };
        let r = run_bat(&w, d, *b"_BST", body, &mut read);
        (r, n)
    };
    let checks: [(&[u8], (Result<Val, Why>, u32)); 9] = [
        (b"\\_SB_.ARA_", (Ok(Val::Int(0x30)), 0)),
        (b"\\_SB_.ARB_", (Ok(Val::Int(u64::MAX)), 0)),
        (b"\\_SB_.ARC_", (Ok(Val::Int(30000)), 0)),
        (b"\\_SB_.ARD_", (Ok(Val::Int(187)), 0)),
        (b"\\_SB_.ARE_", (Ok(Val::Int(3001)), 0)),
        (b"\\_SB_.ARF_", (Err(Why::Zero), 0)),
        (b"\\_SB_.ARG_", (Ok(Val::Int(4)), 0)),
        (b"\\_SB_.ARH_", (Ok(Val::Int(0xF0)), 0)),
        (b"\\_SB_.ARI_", (Ok(Val::Int(2900)), 1)),
    ];
    let mut ok = w.cut == 0;
    if !ok {
        crate::kprintln!("battery selftest:   arithmetic walk: {} cut short (expected 0)", w.cut);
    }
    for (k, (path, want)) in checks.iter().enumerate() {
        let got = run(path);
        if got != *want {
            ok = false;
            crate::kprintln!("battery selftest:   arithmetic check {}: got {:?}, expected {:?}", k + 1, got, want);
        }
    }
    ok
}

/// Scope(\_SB) { Device(EC0_) {_HID PNP0C09, OperationRegion(ECOR, EmbeddedControl, 0, 0xFF), Field(ECOR) {ACIN 1, , 7, BRC0 16}}
///               Method(BRKM) {Break}
///               Device(LPA_) {_BST: Store(Zero, Local0) While(LLess(Local0, 10)) {Increment(Local0)} Return(Local0)}
///               Device(LPB_) {_BST: Store(10, Local0) Store(Zero, Local1)
///                                   While(Local0) {Decrement(Local0) Add(Local1, 3, Local1)} Return(Local1)}
///               Device(LPC_) {_BST: Store(Zero, Local0) While(One) {Increment(Local0) If(LEqual(Local0, 5)) {Break}} Return(Local0)}
///               Device(LPD_) {_BST: Store(Zero, Local0) Store(Zero, Local1) While(LLess(Local0, 10)) {Increment(Local0)
///                                   If(And(Local0, One)) {Continue} Add(Local1, Local0, Local1)} Return(Local1)}   the even ones
///               Device(LPE_) {_BST: While(One) {Return(0x2A)}}
///               Device(LPF_) {_BST: While(One) {Noop} Return(Zero)}   without end
///               Device(LPG_) {_BST: Break Return(Zero)}   outside a While
///               Device(LPH_) {_BST: Increment(\_SB_.EC0_.BRC0) Return(Zero)}   would write the controller: nothing read
///               Device(LPI_) {_BST: Store(Zero, Local1) While(LGreater(\_SB_.EC0_.BRC0, Local1)) {Add(Local1, 1000, Local1)} Return(Local1)}
///               Device(LPJ_) {_BST: Store(Zero, Local0) Decrement(Local0) Return(Local0)}   wraps at 64 bits
///               Device(LPK_) {_BST: While(One) {BRKM()} Return(Zero)}   a Break in a called method does not reach the loop }
/// with the controller's BRC0 given by a pretend reader.
fn loop_selftest() -> bool {
    let mth = |name: &[u8], body: &[u8]| enc(&[0x14], &cat(&[name, &[0x00], body]));
    let bat = |name: &[u8], body: &[u8]| dev(name, &cat(&[&hid_eisa(BAT), &mth(b"_BST", body)]));
    let region = cat(&[&[0x5B, 0x80], b"ECOR", &[0x03, 0x00, 0x0A, 0xFF]]);
    let field = enc(&[0x5B, 0x81], &cat(&[b"ECOR", &[0x01], b"ACIN", &[0x01, 0x00, 0x07], b"BRC0", &[0x10]]));
    let ec0 = dev(b"EC0_", &cat(&[&hid_eisa(EC), &region, &field]));
    let brc = cat(&[b"\\", &[0x2F, 0x03], b"_SB_EC0_BRC0"]);
    let wh = |body: &[u8]| enc(&[0xA2], body);
    let devs = cat(&[
        &mth(b"BRKM", &[0xA5]),
        &bat(b"LPA_", &cat(&[&[0x70, 0x00, 0x60], &wh(&[0x95, 0x60, 0x0A, 0x0A, 0x75, 0x60]), &[0xA4, 0x60]])),
        &bat(b"LPB_", &cat(&[&[0x70, 0x0A, 0x0A, 0x60, 0x70, 0x00, 0x61], &wh(&[0x60, 0x76, 0x60, 0x72, 0x61, 0x0A, 0x03, 0x61]), &[0xA4, 0x61]])),
        &bat(b"LPC_", &cat(&[&[0x70, 0x00, 0x60], &wh(&cat(&[&[0x01, 0x75, 0x60], &enc(&[0xA0], &[0x93, 0x60, 0x0A, 0x05, 0xA5])])), &[0xA4, 0x60]])),
        &bat(
            b"LPD_",
            &cat(&[
                &[0x70, 0x00, 0x60, 0x70, 0x00, 0x61],
                &wh(&cat(&[&[0x95, 0x60, 0x0A, 0x0A, 0x75, 0x60], &enc(&[0xA0], &[0x7B, 0x60, 0x01, 0x00, 0x9F]), &[0x72, 0x61, 0x60, 0x61]])),
                &[0xA4, 0x61],
            ]),
        ),
        &bat(b"LPE_", &wh(&[0x01, 0xA4, 0x0A, 0x2A])),
        &bat(b"LPF_", &cat(&[&wh(&[0x01, 0xA3]), &[0xA4, 0x00]])),
        &bat(b"LPG_", &[0xA5, 0xA4, 0x00]),
        &bat(b"LPH_", &cat(&[&[0x75], &brc, &[0xA4, 0x00]])),
        &bat(b"LPI_", &cat(&[&[0x70, 0x00, 0x61], &wh(&cat(&[&[0x94], &brc, &[0x61, 0x72, 0x61, 0x0B, 0xE8, 0x03, 0x61]])), &[0xA4, 0x61]])),
        &bat(b"LPJ_", &[0x70, 0x00, 0x60, 0x76, 0x60, 0xA4, 0x60]),
        &bat(b"LPK_", &cat(&[&wh(&cat(&[&[0x01], b"BRKM"])), &[0xA4, 0x00]])),
    ]);
    let sb = cat(&[b"\\", b"_SB_"]);
    let w = walk_body(&enc(&[0x10], &cat(&[&sb, &ec0, &devs])));
    let run = |path: &[u8]| -> (Result<Val, Why>, u32) {
        let Some(d) = w.devs.iter().find(|d| path_is(&d.path, path)) else { return (Err(Why::Bad), 99) };
        let Some(body) = d.bst.as_ref() else { return (Err(Why::Bad), 99) };
        let mut n = 0u32;
        let mut read = |_: usize, u: &Unit| -> Result<u64, Why> {
            n += 1;
            match &u.name {
                b"BRC0" if u.bit == 8 && u.bits == 16 => Ok(3000),
                _ => Err(Why::Bad),
            }
        };
        let r = run_bat(&w, d, *b"_BST", body, &mut read);
        (r, n)
    };
    let checks: [(&[u8], (Result<Val, Why>, u32)); 11] = [
        (b"\\_SB_.LPA_", (Ok(Val::Int(10)), 0)),
        (b"\\_SB_.LPB_", (Ok(Val::Int(30)), 0)),
        (b"\\_SB_.LPC_", (Ok(Val::Int(5)), 0)),
        (b"\\_SB_.LPD_", (Ok(Val::Int(30)), 0)),
        (b"\\_SB_.LPE_", (Ok(Val::Int(0x2A)), 0)),
        (b"\\_SB_.LPF_", (Err(Why::Loops), 0)),
        (b"\\_SB_.LPG_", (Err(Why::NoWhile), 0)),
        (b"\\_SB_.LPH_", (Err(Why::Write(b'\\')), 0)),
        (b"\\_SB_.LPI_", (Ok(Val::Int(3000)), 4)),
        (b"\\_SB_.LPJ_", (Ok(Val::Int(u64::MAX)), 0)),
        (b"\\_SB_.LPK_", (Err(Why::NoWhile), 0)),
    ];
    let mut ok = w.cut == 0;
    if !ok {
        crate::kprintln!("battery selftest:   loops walk: {} cut short (expected 0)", w.cut);
    }
    for (k, (path, want)) in checks.iter().enumerate() {
        let got = run(path);
        if got != *want {
            ok = false;
            crate::kprintln!("battery selftest:   loops check {}: got {:?}, expected {:?}", k + 1, got, want);
        }
    }
    ok
}

fn selftest() {
    let sb = cat(&[b"\\", b"_SB_"]);
    let mut pass = 0;
    let mut case = |name: &str, ok: bool| {
        crate::kprintln!("battery selftest: {}: {}", name, if ok { "ok" } else { "FAILED" });
        pass += ok as u32;
    };

    // Scope(\_SB) { Device(BAT0) {_HID PNP0C0A, _UID One, _STA, _BST} Device(ADP1) {_HID "ACPI0003", _PSR}
    //               Device(LID0) {_HID PNP0C0D, _LID} }
    let bat0 = dev(b"BAT0", &cat(&[&hid_eisa(BAT), &[0x08], b"_UID", &[0x01], &method(b"_STA"), &method(b"_BST")]));
    let adp1 = dev(b"ADP1", &cat(&[&hid_str(b"ACPI0003"), &method(b"_PSR")]));
    let lid0 = dev(b"LID0", &cat(&[&hid_eisa(LID), &method(b"_LID")]));
    let w = walk_body(&enc(&[0x10], &cat(&[&sb, &bat0, &adp1, &lid0])));
    case(
        "three devices",
        w.cut == 0
            && w.devs.len() == 3
            && has(&w, Kind::Battery, b"\\_SB_.BAT0", &[b"_STA", b"_BST"], false)
            && has(&w, Kind::Ac, b"\\_SB_.ADP1", &[b"_PSR"], false)
            && has(&w, Kind::Lid, b"\\_SB_.LID0", &[b"_LID"], false),
    );

    // Scope(\_SB.PCI0) as a dual name { Device(EC0) {_HID PNP0C09, OperationRegion(ECOR, EmbeddedControl, 0, 0xFF),
    //                                    Field(ECOR) {BST0, 8}, Device(BAT1) {_HID "PNP0C0A"} } }
    let region = cat(&[&[0x5B, 0x80], b"ECOR", &[0x03, 0x00, 0x0A, 0xFF]]);
    let field = enc(&[0x5B, 0x81], &cat(&[b"ECOR", &[0x01], b"BST0", &[0x08]]));
    let bat1 = dev(b"BAT1", &hid_str(b"PNP0C0A"));
    let ec0 = dev(b"EC0_", &cat(&[&hid_eisa(EC), &region, &field, &bat1]));
    let w = walk_body(&enc(&[0x10], &cat(&[b"\\", &[0x2E], b"_SB_PCI0", &ec0])));
    case(
        "nested controller",
        w.cut == 0
            && w.devs.len() == 2
            && has(&w, Kind::Battery, b"\\_SB_.PCI0.EC0_.BAT1", &[], false)
            && w.devs.iter().any(|d| d.kind == Kind::Ec && path_is(&d.path, b"\\_SB_.PCI0.EC0_") && d.ec_regions == 1),
    );

    // an unknown form inside BAT2 ends BAT2 only; LID1 after it is still read
    let bat2 = dev(b"BAT2", &cat(&[&hid_eisa(BAT), &[0x5B, 0x99, 0x00, 0x00], &method(b"_BST")]));
    let lid1 = dev(b"LID1", &hid_eisa(LID));
    let w = walk_body(&enc(&[0x10], &cat(&[&sb, &bat2, &lid1])));
    case(
        "unknown form",
        w.cut == 1 && w.devs.len() == 2 && has(&w, Kind::Battery, b"\\_SB_.BAT2", &[], true) && has(&w, Kind::Lid, b"\\_SB_.LID1", &[], false),
    );

    // If (One) { Device(BATX) {_HID PNP0C0A} }: not entered, no battery
    let batx = dev(b"BATX", &hid_eisa(BAT));
    let w = walk_body(&enc(&[0x10], &cat(&[&sb, &enc(&[0xA0], &cat(&[&[0x01], &batx]))])));
    case("conditional", w.cut == 0 && w.conditional == 1 && w.devs.is_empty());

    // a Scope whose length runs past the table's end: cut, nothing read past it
    let w = walk_body(&[0x10, 0x3F, b'\\', b'_', b'S']);
    case("cut off", w.cut == 1 && w.devs.is_empty() && w.objects == 0);

    // Scope(\_GPE) { Device(\_SB.LID9) {_HID PNP0C0D}  Scope(^_TZ) { ThermalZone(TZ01) {_TMP} } }
    let lid9 = enc(&[0x5B, 0x82], &cat(&[b"\\", &[0x2E], b"_SB_LID9", &hid_eisa(LID)]));
    let tz = enc(&[0x5B, 0x85], &cat(&[b"TZ01", &method(b"_TMP")]));
    let up = enc(&[0x10], &cat(&[b"^_TZ_", &tz]));
    let w = walk_body(&enc(&[0x10], &cat(&[b"\\_GPE", &lid9, &up])));
    case(
        "root and parent names",
        w.cut == 0 && has(&w, Kind::Lid, b"\\_SB_.LID9", &[], false) && has(&w, Kind::Thermal, b"\\_TZ_.TZ01", &[b"_TMP"], false),
    );

    // Scope(\_SB) { Device(BAT3) {_HID PNP0C0A, Method(_STA) {Return(0x1F)}}
    //               Device(ADP3) {_HID "ACPI0003", Name(_STA, 0x0F)}
    //               Device(LID3) {_HID PNP0C0D, Method(_STA) {Return(LSTA)}}   LSTA is declared after it, one level up
    //               Device(EC3_) {_HID PNP0C09, Method(_STA) {Return(And(ECST, One))}}   needs the interpreter
    //               Device(BAT5) {_HID PNP0C0A, Method(_STA) {Return(NOPE)}}   NOPE is never declared
    //               ThermalZone(TZ3_) {}   no _STA
    //               Name(LSTA, Zero) }
    let ret = |name: &[u8], body: &[u8]| enc(&[0x14], &cat(&[name, &[0x00, 0xA4], body]));
    let bat3 = dev(b"BAT3", &cat(&[&hid_eisa(BAT), &ret(b"_STA", &[0x0A, 0x1F])]));
    let adp3 = dev(b"ADP3", &cat(&[&hid_str(b"ACPI0003"), &[0x08], b"_STA", &[0x0A, 0x0F]]));
    let lid3 = dev(b"LID3", &cat(&[&hid_eisa(LID), &ret(b"_STA", b"LSTA")]));
    let ec3 = dev(b"EC3_", &cat(&[&hid_eisa(EC), &ret(b"_STA", &cat(&[&[0x7B], b"ECST", &[0x01, 0x00]]))]));
    let bat5 = dev(b"BAT5", &cat(&[&hid_eisa(BAT), &ret(b"_STA", b"NOPE")]));
    let tz3 = enc(&[0x5B, 0x85], b"TZ3_");
    let lsta = cat(&[&[0x08], b"LSTA", &[0x00]]);
    let w = walk_body(&enc(&[0x10], &cat(&[&sb, &bat3, &adp3, &lid3, &ec3, &bat5, &tz3, &lsta])));
    case(
        "status values",
        w.cut == 0
            && w.devs.len() == 6
            && w.sta_known == 3
            && w.sta_run == 2
            && sta_is(&w, b"\\_SB_.BAT3", Some(0x1F))
            && sta_is(&w, b"\\_SB_.ADP3", Some(0x0F))
            && sta_is(&w, b"\\_SB_.LID3", Some(0))
            && sta_is(&w, b"\\_SB_.EC3_", None)
            && sta_is(&w, b"\\_SB_.BAT5", None)
            && w.devs.iter().any(|d| path_is(&d.path, b"\\_SB_.TZ3_") && matches!(d.sta, Sta::Absent)),
    );

    // Device(EC9_) {_HID PNP0C09, Name(_CRS, ResourceTemplate {IO 0x62, IO 0x66}),
    //               Field(ECR9) {AccessAs, BST0 8, Offset(2), BRC0 16, , 4, ACIN 1},   before its region
    //               OperationRegion(ECR9, EmbeddedControl, 0, 0xFF),
    //               OperationRegion(MEM0, SystemMemory, 0xF0000, 16), Field(MEM0) {XXXX 32} }   not the EC's
    // Scope(\_SB.EC9_) {Field(ECR9) {B1ST 8}}   after its region, from another scope
    let io = |p: u8| [0x47, 0x01, p, 0x00, p, 0x00, 0x00, 0x01];
    let res = cat(&[&io(0x62), &io(0x66), &[0x79, 0x00]]);
    let crs = cat(&[&[0x08], b"_CRS", &enc(&[0x11], &cat(&[&[0x0A, res.len() as u8], &res]))]);
    let f9 = enc(
        &[0x5B, 0x81],
        &cat(&[b"ECR9", &[0x01, 0x01, 0x01, 0x00], b"BST0", &[0x08, 0x00, 0x08], b"BRC0", &[0x10, 0x00, 0x04], b"ACIN", &[0x01]]),
    );
    let r9 = cat(&[&[0x5B, 0x80], b"ECR9", &[0x03, 0x00, 0x0A, 0xFF]]);
    let mem = cat(&[&[0x5B, 0x80], b"MEM0", &[0x00, 0x0C, 0x00, 0x00, 0x0F, 0x00, 0x0A, 0x10]]);
    let fm = enc(&[0x5B, 0x81], &cat(&[b"MEM0", &[0x01], b"XXXX", &[0x20]]));
    let ec9 = dev(b"EC9_", &cat(&[&hid_eisa(EC), &crs, &f9, &r9, &mem, &fm]));
    let more = enc(&[0x10], &cat(&[b"\\", &[0x2E], b"_SB_EC9_", &enc(&[0x5B, 0x81], &cat(&[b"ECR9", &[0x01], b"B1ST", &[0x08]]))]));
    let w = walk_body(&cat(&[&enc(&[0x10], &cat(&[&sb, &ec9])), &more]));
    let ok = match w.devs.iter().find(|d| path_is(&d.path, b"\\_SB_.EC9_")) {
        Some(d) => {
            let us = ec_units(&w, d);
            let unit = |n: &[u8; 4], bit: u32, bits: u32| us.iter().any(|u| &u.name == n && u.bit == bit && u.bits == bits);
            d.ec_regions == 1
                && matches!(d.crs, Crs::Ports([0x62, 0x66], 2))
                && us.len() == 4
                && unit(b"BST0", 0, 8)
                && unit(b"BRC0", 16, 16)
                && unit(b"ACIN", 36, 1)
                && unit(b"B1ST", 0, 8)
        }
        None => false,
    };
    case("controller ports and fields", w.cut == 0 && ok && w.pending.is_empty());

    case("controller reads", ec_selftest());

    case("power source", psr_selftest());

    case("battery status", bst_selftest());

    case("battery info", bif_selftest());

    case("method calls", call_selftest());

    case("arithmetic", arith_selftest());

    case("loops", loop_selftest());

    crate::kprintln!("battery selftest: {} of 15", pass);
}
