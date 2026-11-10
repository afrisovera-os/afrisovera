//! The few UEFI firmware calls the PC kernel needs before it takes the machine over.
//!
//! Hand-written, not a crate: the kernel has no dependencies and this is the whole of
//! the firmware interface it uses — find the memory map, find the screen, and leave
//! the firmware ("exit boot services"). After that the firmware is never called again.
//! Layouts are UEFI 2.x (x86-64, `efiapi` = the Microsoft x64 calling convention).

pub type Handle = *mut core::ffi::c_void;
pub type Status = usize;
pub const SUCCESS: Status = 0;

#[repr(C)]
pub struct TableHeader {
    pub signature: u64,
    pub revision: u32,
    pub header_size: u32,
    pub crc32: u32,
    pub reserved: u32,
}

#[repr(C)]
pub struct SystemTable {
    pub hdr: TableHeader,
    pub firmware_vendor: *const u16,
    pub firmware_revision: u32,
    pub console_in_handle: Handle,
    pub con_in: *mut core::ffi::c_void,
    pub console_out_handle: Handle,
    pub con_out: *mut core::ffi::c_void,
    pub stderr_handle: Handle,
    pub std_err: *mut core::ffi::c_void,
    pub runtime_services: *mut core::ffi::c_void,
    pub boot_services: *mut BootServices,
    pub n_config: usize,
    pub config_table: *mut core::ffi::c_void,
}

/// Boot services: the header, then a table of function pointers. Only the slots this
/// kernel calls are named; the index is the slot's position in the UEFI spec.
#[repr(C)]
pub struct BootServices {
    pub hdr: TableHeader,
    pub f: [usize; 44],
}
const GET_MEMORY_MAP: usize = 4;
const EXIT_BOOT_SERVICES: usize = 26;
const SET_WATCHDOG_TIMER: usize = 29;
const LOCATE_PROTOCOL: usize = 37;

type GetMemoryMapFn =
    extern "efiapi" fn(*mut usize, *mut u8, *mut usize, *mut usize, *mut u32) -> Status;
type ExitBootServicesFn = extern "efiapi" fn(Handle, usize) -> Status;
type SetWatchdogFn = extern "efiapi" fn(usize, u64, usize, *const u16) -> Status;
type LocateProtocolFn =
    extern "efiapi" fn(*const Guid, *mut core::ffi::c_void, *mut *mut core::ffi::c_void) -> Status;

#[repr(C)]
pub struct Guid(pub u32, pub u16, pub u16, pub [u8; 8]);

pub const GOP_GUID: Guid =
    Guid(0x9042a9de, 0x23dc, 0x4a38, [0x96, 0xfb, 0x7a, 0xde, 0xd0, 0x80, 0x51, 0x6a]);

/// One entry of the firmware's memory map. Entries are `desc_size` apart, which may be
/// larger than this struct — always step by `desc_size`, never by `size_of`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MemoryDescriptor {
    pub ty: u32,
    pub _pad: u32,
    pub phys_start: u64,
    pub virt_start: u64,
    pub pages: u64,
    pub attribute: u64,
}
/// Free RAM once the firmware is gone: conventional memory, plus what the loader and
/// boot services used (both become the kernel's after `exit_boot_services`).
pub fn usable_after_exit(ty: u32) -> bool {
    matches!(ty, 1 | 2 | 3 | 4 | 7)
}

#[repr(C)]
pub struct GraphicsOutput {
    pub query_mode: usize,
    pub set_mode: usize,
    pub blt: usize,
    pub mode: *const GopMode,
}
#[repr(C)]
pub struct GopMode {
    pub max_mode: u32,
    pub mode: u32,
    pub info: *const GopModeInfo,
    pub size_of_info: usize,
    pub fb_base: u64,
    pub fb_size: usize,
}
#[repr(C)]
pub struct GopModeInfo {
    pub version: u32,
    pub hres: u32,
    pub vres: u32,
    /// 0 = RGB-reserved, 1 = BGR-reserved (8 bits each), 2 = bitmask, 3 = no framebuffer.
    pub pixel_format: u32,
    pub masks: [u32; 4],
    pub stride: u32,
}

/// The firmware's memory map, copied into a buffer the kernel owns.
pub struct MemoryMap {
    pub buf: *const u8,
    pub size: usize,
    pub key: usize,
    pub desc_size: usize,
}
impl MemoryMap {
    pub fn entries(&self) -> impl Iterator<Item = MemoryDescriptor> + '_ {
        let n = if self.desc_size == 0 { 0 } else { self.size / self.desc_size };
        // SAFETY: the firmware wrote `size` bytes of `desc_size`-strided descriptors.
        (0..n).map(move |i| unsafe {
            core::ptr::read_unaligned(self.buf.add(i * self.desc_size) as *const MemoryDescriptor)
        })
    }
}

/// Runtime services outlive `exit_boot_services`; until the kernel calls
/// SetVirtualAddressMap (it does not) they are called at their physical addresses.
#[repr(C)]
pub struct RuntimeServices {
    pub hdr: TableHeader,
    pub f: [usize; 14],
}
const RESET_SYSTEM: usize = 10;
const RESET_SHUTDOWN: u32 = 2;
const RESET_COLD: u32 = 0;
// Declared as returning: the spec says it does not, but a firmware that fails must not
// become undefined behaviour here; the callers then fall back to the hardware's own resets.
type ResetSystemFn = extern "efiapi" fn(u32, Status, usize, *const u8);

/// Power the machine off through the firmware (EfiResetShutdown).
pub fn shutdown(rt: *const RuntimeServices) -> ! {
    // SAFETY: slot RESET_SYSTEM has this signature per the UEFI spec; `rt` is the
    // runtime-services table saved from the system table before boot services exited.
    let f: ResetSystemFn = unsafe { core::mem::transmute((*rt).f[RESET_SYSTEM]) };
    f(RESET_SHUTDOWN, SUCCESS, 0, core::ptr::null());
    // the firmware did not switch the machine off: stop everything, say so
    crate::kprintln!("[pc] the firmware could not power off: everything is stopped, it is safe to switch the machine off");
    crate::cpu::halt_forever();
}

/// Restart the machine through the firmware (EfiResetCold): the PC shell's `reboot`.
pub fn reset_cold(rt: *const RuntimeServices) -> ! {
    // SAFETY: as `shutdown`.
    let f: ResetSystemFn = unsafe { core::mem::transmute((*rt).f[RESET_SYSTEM]) };
    f(RESET_COLD, SUCCESS, 0, core::ptr::null());
    // the firmware did not reset: the chipset's reset control (0xCF9: full reset), then
    // the keyboard controller's reset line, the two ways every PC can be restarted
    // SAFETY: port writes whose only effect is a machine reset.
    unsafe {
        core::arch::asm!("out dx, al", in("dx") 0xCF9u16, in("al") 0x02u8, options(nomem, nostack));
        core::arch::asm!("out dx, al", in("dx") 0xCF9u16, in("al") 0x06u8, options(nomem, nostack));
    }
    crate::timer::busywait_ms(50);
    // SAFETY: as above.
    unsafe { core::arch::asm!("out dx, al", in("dx") 0x64u16, in("al") 0xFEu8, options(nomem, nostack)) };
    crate::timer::busywait_ms(50);
    crate::kprintln!("[pc] the machine did not restart: it is safe to switch it off and on");
    crate::cpu::halt_forever();
}

pub struct Uefi {
    pub image: Handle,
    pub st: *mut SystemTable,
}

impl Uefi {
    fn bs(&self) -> &BootServices {
        // SAFETY: the firmware hands a valid system table to the image's entry point.
        unsafe { &*(*self.st).boot_services }
    }

    /// Switch off the firmware's 5-minute watchdog, which would otherwise reset the
    /// machine under a kernel that has not exited boot services yet.
    pub fn disable_watchdog(&self) {
        // SAFETY: slot SET_WATCHDOG_TIMER has this signature per the UEFI spec.
        let f: SetWatchdogFn = unsafe { core::mem::transmute(self.bs().f[SET_WATCHDOG_TIMER]) };
        f(0, 0, 0, core::ptr::null());
    }

    /// Copy the current memory map into `buf`. The key it returns is what
    /// `exit_boot_services` must be handed; any firmware allocation after this call
    /// invalidates it.
    pub fn memory_map(&self, buf: &mut [u8]) -> Result<MemoryMap, Status> {
        let mut size = buf.len();
        let (mut key, mut desc_size, mut ver) = (0usize, 0usize, 0u32);
        // SAFETY: slot GET_MEMORY_MAP has this signature per the UEFI spec.
        let f: GetMemoryMapFn = unsafe { core::mem::transmute(self.bs().f[GET_MEMORY_MAP]) };
        let st = f(&mut size, buf.as_mut_ptr(), &mut key, &mut desc_size, &mut ver);
        if st != SUCCESS {
            return Err(st);
        }
        Ok(MemoryMap { buf: buf.as_ptr(), size, key, desc_size })
    }

    pub fn graphics(&self) -> Option<&'static GraphicsOutput> {
        let mut p: *mut core::ffi::c_void = core::ptr::null_mut();
        // SAFETY: slot LOCATE_PROTOCOL has this signature per the UEFI spec.
        let f: LocateProtocolFn = unsafe { core::mem::transmute(self.bs().f[LOCATE_PROTOCOL]) };
        if f(&GOP_GUID, core::ptr::null_mut(), &mut p) != SUCCESS || p.is_null() {
            return None;
        }
        // SAFETY: the firmware returned a GOP interface for the GOP GUID.
        Some(unsafe { &*(p as *const GraphicsOutput) })
    }

    pub fn exit_boot_services(&self, key: usize) -> Status {
        // SAFETY: slot EXIT_BOOT_SERVICES has this signature per the UEFI spec.
        let f: ExitBootServicesFn =
            unsafe { core::mem::transmute(self.bs().f[EXIT_BOOT_SERVICES]) };
        f(self.image, key)
    }
}

// --- the kernel's own boot file, kept for `installdisk` ---
//
// To make an internal disk start AfriSovera by itself, `installdisk` writes this very file
// (EFI\BOOT\BOOTX64.EFI) into an EFI system partition on it. After exit_boot_services
// nothing can read the USB stick through the firmware any more, so the file is read now,
// through the firmware's own file system on the device the kernel was loaded from, into
// pages of an OS-reserved memory type: the kernel's frame pool takes only the types
// `usable_after_exit` names, so they stay untouched (and inside the 4 GiB mapped 1:1).

const ALLOCATE_PAGES: usize = 2;
const HANDLE_PROTOCOL: usize = 16;
const ALLOC_MAX_ADDRESS: u32 = 1;
/// OS-reserved memory type (0x80000000 and up): never handed out by the kernel.
const MEM_OWN_IMAGE: u32 = 0x8000_0A50;
const OWN_MAX: usize = 16 << 20;

type AllocatePagesFn = extern "efiapi" fn(u32, u32, usize, *mut u64) -> Status;
type HandleProtocolFn = extern "efiapi" fn(Handle, *const Guid, *mut *mut core::ffi::c_void) -> Status;

const LOADED_IMAGE_GUID: Guid = Guid(0x5b1b31a1, 0x9562, 0x11d2, [0x8e, 0x3f, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b]);
const SIMPLE_FS_GUID: Guid = Guid(0x964e5b22, 0x6459, 0x11d2, [0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b]);

#[repr(C)]
struct LoadedImage {
    revision: u32,
    parent: Handle,
    system_table: *mut SystemTable,
    device: Handle,
    file_path: *const u8,
}

#[repr(C)]
struct SimpleFs {
    revision: u64,
    open_volume: extern "efiapi" fn(*mut SimpleFs, *mut *mut File) -> Status,
}

#[repr(C)]
struct File {
    revision: u64,
    open: extern "efiapi" fn(*mut File, *mut *mut File, *const u16, u64, u64) -> Status,
    close: extern "efiapi" fn(*mut File) -> Status,
    delete: usize,
    read: extern "efiapi" fn(*mut File, *mut usize, *mut u8) -> Status,
    write: usize,
    get_position: extern "efiapi" fn(*mut File, *mut u64) -> Status,
    set_position: extern "efiapi" fn(*mut File, u64) -> Status,
}

/// (address, bytes) of the boot file kept in memory; (0, 0) if it could not be read.
static mut OWN: (u64, usize) = (0, 0);

/// The boot file the kernel was started from, if it was kept: its bytes.
pub fn own_image() -> Option<&'static [u8]> {
    // SAFETY: written once before exit_boot_services; the pages are never reused.
    let (a, n) = unsafe { OWN };
    if a == 0 || n == 0 {
        return None;
    }
    // SAFETY: as above; the pages lie below 4 GiB, which the kernel maps 1:1.
    Some(unsafe { core::slice::from_raw_parts(a as *const u8, n) })
}

impl Uefi {
    /// Read the kernel's own boot file into reserved pages (see above). One line either way:
    /// booting goes on regardless, only `installdisk` needs the file.
    pub fn keep_own_image(&self) {
        // SAFETY: slots and layouts per the UEFI spec; every pointer the firmware returns is
        // checked for null before it is used.
        unsafe {
            let hp: HandleProtocolFn = core::mem::transmute(self.bs().f[HANDLE_PROTOCOL]);
            let mut li: *mut core::ffi::c_void = core::ptr::null_mut();
            if hp(self.image, &LOADED_IMAGE_GUID, &mut li) != SUCCESS || li.is_null() {
                crate::kprintln!("[pc] own boot file: no loaded-image protocol -- installdisk will not be possible");
                return;
            }
            let li = &*(li as *const LoadedImage);
            let mut fs: *mut core::ffi::c_void = core::ptr::null_mut();
            if li.device.is_null() || hp(li.device, &SIMPLE_FS_GUID, &mut fs) != SUCCESS || fs.is_null() {
                crate::kprintln!("[pc] own boot file: the boot device has no file system the firmware reads -- installdisk will not be possible");
                return;
            }
            // the path: the file-path nodes of the image's device path, else the default one
            let bs = b'\\' as u16;
            let mut path = [0u16; 128];
            let mut n = 0;
            let mut p = li.file_path;
            for _ in 0..16 {
                if p.is_null() {
                    break;
                }
                let (ty, sub) = (*p, *p.add(1));
                let len = u16::from_le_bytes([*p.add(2), *p.add(3)]) as usize;
                if ty == 0x7F || len < 4 {
                    break;
                }
                if ty == 4 && sub == 4 {
                    let mut k = 4;
                    while k + 1 < len && n < path.len() - 1 {
                        let c = u16::from_le_bytes([*p.add(k), *p.add(k + 1)]);
                        if c == 0 {
                            break;
                        }
                        // nodes may be split ("\EFI", "\BOOT\..."): one backslash between them
                        if !(c == bs && n > 0 && path[n - 1] == bs) {
                            path[n] = c;
                            n += 1;
                        }
                        k += 2;
                    }
                }
                p = p.add(len);
            }
            if n == 0 {
                for c in b"\\EFI\\BOOT\\BOOTX64.EFI" {
                    path[n] = *c as u16;
                    n += 1;
                }
            }
            path[n] = 0;
            let fs = fs as *mut SimpleFs;
            let mut root: *mut File = core::ptr::null_mut();
            if ((*fs).open_volume)(fs, &mut root) != SUCCESS || root.is_null() {
                crate::kprintln!("[pc] own boot file: the boot volume could not be opened -- installdisk will not be possible");
                return;
            }
            let mut f: *mut File = core::ptr::null_mut();
            if ((*root).open)(root, &mut f, path.as_ptr(), 1, 0) != SUCCESS || f.is_null() {
                ((*root).close)(root);
                crate::kprintln!("[pc] own boot file: it could not be opened -- installdisk will not be possible");
                return;
            }
            let mut size = 0u64;
            let sized = ((*f).set_position)(f, u64::MAX) == SUCCESS
                && ((*f).get_position)(f, &mut size) == SUCCESS
                && ((*f).set_position)(f, 0) == SUCCESS;
            if !sized || size == 0 || size as usize > OWN_MAX {
                ((*f).close)(f);
                ((*root).close)(root);
                crate::kprintln!("[pc] own boot file: its size could not be read ({} B) -- installdisk will not be possible", size);
                return;
            }
            let ap: AllocatePagesFn = core::mem::transmute(self.bs().f[ALLOCATE_PAGES]);
            let mut at: u64 = 0xFFFF_F000; // highest address allowed: below 4 GiB
            let pages = (size as usize).div_ceil(4096);
            if ap(ALLOC_MAX_ADDRESS, MEM_OWN_IMAGE, pages, &mut at) != SUCCESS || at == 0 {
                ((*f).close)(f);
                ((*root).close)(root);
                crate::kprintln!("[pc] own boot file: no memory for it -- installdisk will not be possible");
                return;
            }
            let mut got = 0usize;
            while got < size as usize {
                let mut chunk = (size as usize - got).min(1 << 20);
                if ((*f).read)(f, &mut chunk, (at as usize + got) as *mut u8) != SUCCESS || chunk == 0 {
                    break;
                }
                got += chunk;
            }
            ((*f).close)(f);
            ((*root).close)(root);
            if got != size as usize {
                crate::kprintln!("[pc] own boot file: read {} of {} B -- installdisk will not be possible", got, size);
                return;
            }
            OWN = (at, got);
            let mut shown = [0u8; 64];
            let k = n.min(shown.len());
            for i in 0..k {
                shown[i] = if path[i] < 0x80 { path[i] as u8 } else { b'?' };
            }
            crate::kprintln!(
                "[pc] own boot file {} ({} KiB) kept for installdisk",
                core::str::from_utf8(&shown[..k]).unwrap_or("?"),
                got / 1024
            );
        }
    }
}

// --- the firmware's boot entries, kept for `bootentries` (READ ONLY) ---
//
// Booting AfriSovera from a partition next to Windows needs one more firmware boot entry
// (Boot#### + BootOrder). Before anything is ever added, `bootentries` shows the entries as
// they are: read here, before boot services exit, with GetVariable / GetNextVariableName
// (the kernel does not call the firmware's variable services later). Nothing is written.

const GET_VARIABLE: usize = 6;
const GET_NEXT_VARIABLE_NAME: usize = 7;
const BUFFER_TOO_SMALL: Status = (1 << 63) | 5;
const NOT_FOUND: Status = (1 << 63) | 14;
type GetVariableFn = extern "efiapi" fn(*const u16, *const Guid, *mut u32, *mut usize, *mut u8) -> Status;
type GetNextVariableNameFn = extern "efiapi" fn(*mut usize, *mut u16, *mut Guid) -> Status;
/// EFI_GLOBAL_VARIABLE: BootOrder, BootCurrent and Boot#### live under it.
const GLOBAL_VARIABLE_GUID: Guid = Guid(0x8be4df61, 0x93ca, 0x11d2, [0xaa, 0x0d, 0x00, 0xe0, 0x98, 0x03, 0x2b, 0x8c]);

pub const BOOT_MAX: usize = 32;
pub const OPT_MAX: usize = 768;
pub const ORDER_MAX: usize = 64;

/// The boot variables as the firmware had them at power-on.
pub struct BootVars {
    /// false: the variables could not be read (`why` says why)
    pub read: bool,
    pub why: &'static str,
    pub order: [u16; ORDER_MAX],
    pub n_order: usize,
    /// BootOrder longer than ORDER_MAX
    pub order_cut: bool,
    pub current: Option<u16>,
    /// BootNext: the entry the firmware starts ONCE at the next power-on, then deletes
    pub next: Option<u16>,
    /// Boot#### numbers and their raw EFI_LOAD_OPTION bytes, in the firmware's name order
    pub num: [u16; BOOT_MAX],
    pub len: [usize; BOOT_MAX],
    pub data: [[u8; OPT_MAX]; BOOT_MAX],
    pub n: usize,
    /// Boot#### entries not kept (more than BOOT_MAX, or longer than OPT_MAX)
    pub skipped: usize,
    /// read again after `bootentry` changed them (not as at power-on any more)
    pub changed: bool,
}

const EMPTY: BootVars = BootVars {
    read: false,
    why: "not read yet",
    order: [0; ORDER_MAX],
    n_order: 0,
    order_cut: false,
    current: None,
    next: None,
    num: [0; BOOT_MAX],
    len: [0; BOOT_MAX],
    data: [[0; OPT_MAX]; BOOT_MAX],
    n: 0,
    skipped: 0,
    changed: false,
};
static mut BOOTVARS: BootVars = EMPTY;

/// The boot variables read at power-on (see `keep_boot_entries`).
pub fn boot_vars() -> &'static BootVars {
    // SAFETY: written once before exit_boot_services, only read afterwards.
    unsafe { &*core::ptr::addr_of!(BOOTVARS) }
}

/// "Boot" + 4 upper-case hex digits -> the number.
fn boot_number(name: &[u16]) -> Option<u16> {
    if name.len() != 8 || name[..4].iter().zip(b"Boot").any(|(a, b)| *a != *b as u16) {
        return None;
    }
    let mut v = 0u16;
    for &c in &name[4..] {
        let d = match c {
            0x30..=0x39 => c - 0x30,
            0x41..=0x46 => c - 0x41 + 10,
            _ => return None,
        };
        v = v << 4 | d;
    }
    Some(v)
}

impl Uefi {
    /// Read BootOrder, BootCurrent and every Boot#### into BOOTVARS. One line either way:
    /// booting goes on regardless, only `bootentries` shows them.
    pub fn keep_boot_entries(&self) {
        // SAFETY: the runtime-services table from the system table.
        let rt = unsafe { (*self.st).runtime_services } as *const RuntimeServices;
        // SAFETY: before exit_boot_services every runtime service may be called.
        unsafe { read_boot_vars(rt) };
        let bv = boot_vars();
        if rt.is_null() {
            crate::kprintln!("[pc] boot entries: {} -- bootentries will say so", bv.why);
            return;
        }
        crate::kprintln!(
            "[pc] boot entries: {} read ({} in BootOrder{}), kept for bootentries",
            bv.n,
            bv.n_order,
            match bv.current {
                Some(_) => ", started from one of them",
                None => "",
            }
        );
    }
}

/// BOOTVARS filled from the firmware: at power-on, and again (`reread_boot_entries`) after
/// `bootentry` changed them.
///
/// SAFETY: `rt` is the firmware's runtime-services table (or null); the caller makes sure its
/// variable services may be called now (before exit_boot_services, or after it with the
/// firmware's memory still mapped 1:1, which this kernel never changes).
unsafe fn read_boot_vars(rt: *const RuntimeServices) {
        // SAFETY: slots and signatures per the UEFI spec; every buffer handed to the firmware
        // is ours and its size is passed.
        unsafe {
            let bv = &mut *core::ptr::addr_of_mut!(BOOTVARS);
            *bv = EMPTY;
            if rt.is_null() {
                bv.why = "the firmware has no runtime services";
                return;
            }
            let gv: GetVariableFn = core::mem::transmute((*rt).f[GET_VARIABLE]);
            let gn: GetNextVariableNameFn = core::mem::transmute((*rt).f[GET_NEXT_VARIABLE_NAME]);
            let var = |name: &[u8], buf: &mut [u8]| -> Result<usize, Status> {
                let mut w = [0u16; 16];
                for (i, c) in name.iter().enumerate().take(15) {
                    w[i] = *c as u16;
                }
                let mut size = buf.len();
                let mut attr = 0u32;
                let st = gv(w.as_ptr(), &GLOBAL_VARIABLE_GUID, &mut attr, &mut size, buf.as_mut_ptr());
                if st == SUCCESS { Ok(size.min(buf.len())) } else { Err(st) }
            };
            let mut ob = [0u8; ORDER_MAX * 2];
            match var(b"BootOrder", &mut ob) {
                Ok(n) => {
                    bv.n_order = n / 2;
                    for i in 0..bv.n_order {
                        bv.order[i] = u16::from_le_bytes([ob[2 * i], ob[2 * i + 1]]);
                    }
                }
                Err(BUFFER_TOO_SMALL) => bv.order_cut = true,
                Err(_) => {}
            }
            let mut cb = [0u8; 2];
            if let Ok(2) = var(b"BootCurrent", &mut cb) {
                bv.current = Some(u16::from_le_bytes(cb));
            }
            let mut nb = [0u8; 2];
            if let Ok(2) = var(b"BootNext", &mut nb) {
                bv.next = Some(u16::from_le_bytes(nb));
            }
            // every variable: the Boot#### ones under the global GUID are kept
            let mut name = [0u16; 128];
            let mut guid = Guid(0, 0, 0, [0; 8]);
            let mut seen = 0;
            loop {
                seen += 1;
                if seen > 4096 {
                    break; // a firmware that never says NOT_FOUND
                }
                let mut size = core::mem::size_of_val(&name);
                let st = gn(&mut size, name.as_mut_ptr(), &mut guid);
                if st == NOT_FOUND {
                    break;
                }
                if st != SUCCESS {
                    // a name longer than ours: the walk cannot go on past it
                    if bv.n == 0 && bv.n_order == 0 {
                        bv.why = "the firmware's variable list could not be walked";
                    }
                    break;
                }
                let g = &guid;
                let global = g.0 == GLOBAL_VARIABLE_GUID.0
                    && g.1 == GLOBAL_VARIABLE_GUID.1
                    && g.2 == GLOBAL_VARIABLE_GUID.2
                    && g.3 == GLOBAL_VARIABLE_GUID.3;
                let len = name.iter().position(|c| *c == 0).unwrap_or(name.len());
                let Some(num) = boot_number(&name[..len]) else { continue };
                if !global {
                    continue;
                }
                if bv.n == BOOT_MAX {
                    bv.skipped += 1;
                    continue;
                }
                let mut sz = OPT_MAX;
                let mut attr = 0u32;
                let i = bv.n;
                let st = gv(name.as_ptr(), &GLOBAL_VARIABLE_GUID, &mut attr, &mut sz, bv.data[i].as_mut_ptr());
                if st != SUCCESS || sz < 6 {
                    bv.skipped += 1;
                    continue;
                }
                bv.num[i] = num;
                bv.len[i] = sz;
                bv.n += 1;
            }
            bv.read = bv.n > 0 || bv.n_order > 0;
            if !bv.read && bv.why == "not read yet" {
                bv.why = "the firmware has no boot entries";
            }
        }
}

// --- `bootentry`: AfriSovera's own firmware boot entry, written on the owner's code ---
//
// The ONLY firmware variables this kernel ever writes: one Boot#### (AfriSovera's),
// BootOrder (that number added at its END, or taken out again) and BootNext (`bootnext`:
// AfriSovera's number, started once at the next power-on; the firmware deletes it then). Called after
// exit_boot_services through the runtime services at their physical addresses, which the
// kernel's 1:1 map keeps valid (RuntimeServicesCode/Data are never handed out as frames:
// `usable_after_exit` leaves types 5 and 6 out), with interrupts masked for the call.

const SET_VARIABLE: usize = 8;
type SetVariableFn = extern "efiapi" fn(*const u16, *const Guid, u32, usize, *const u8) -> Status;
/// NON_VOLATILE | BOOTSERVICE_ACCESS | RUNTIME_ACCESS: what Boot#### and BootOrder carry.
const VAR_NV_BS_RT: u32 = 7;
pub const ERR_NOT_FOUND: Status = NOT_FOUND;

fn wide(name: &[u8]) -> [u16; 16] {
    let mut w = [0u16; 16];
    for (i, c) in name.iter().enumerate().take(15) {
        w[i] = *c as u16;
    }
    w
}

fn masked<T>(f: impl FnOnce() -> T) -> T {
    let on = crate::cpu::interrupts_on();
    crate::cpu::cli();
    let r = f();
    if on {
        crate::cpu::sti();
    }
    r
}

/// A global variable as the firmware has it now: Ok(bytes) / Err(status) (NOT_FOUND: none).
pub fn get_global(name: &[u8], buf: &mut [u8]) -> Result<usize, Status> {
    let rt = crate::runtime_services();
    if rt.is_null() {
        return Err(NOT_FOUND);
    }
    let w = wide(name);
    let mut size = buf.len();
    let mut attr = 0u32;
    // SAFETY: slot GET_VARIABLE per the UEFI spec; see the section comment for why it may be
    // called now; the buffer is ours and its size is passed.
    let st = masked(|| unsafe {
        let gv: GetVariableFn = core::mem::transmute((*rt).f[GET_VARIABLE]);
        gv(w.as_ptr(), &GLOBAL_VARIABLE_GUID, &mut attr, &mut size, buf.as_mut_ptr())
    });
    if st == SUCCESS { Ok(size.min(buf.len())) } else { Err(st) }
}

/// Write (or, with empty `data`, delete) a global variable, non-volatile. The status.
pub fn set_global(name: &[u8], data: &[u8]) -> Status {
    let rt = crate::runtime_services();
    if rt.is_null() {
        return NOT_FOUND;
    }
    let w = wide(name);
    let attr = if data.is_empty() { 0 } else { VAR_NV_BS_RT };
    // SAFETY: slot SET_VARIABLE per the UEFI spec; see the section comment.
    masked(|| unsafe {
        let sv: SetVariableFn = core::mem::transmute((*rt).f[SET_VARIABLE]);
        sv(w.as_ptr(), &GLOBAL_VARIABLE_GUID, attr, data.len(), data.as_ptr())
    })
}

// --- `rtcinfo`: the firmware's own clock (GetTime), to compare with the CMOS read ---

const GET_TIME: usize = 0;

/// EFI_TIME. `time_zone` is minutes from UTC, 2047 when unspecified (local time);
/// `daylight` bit 0: adjust for daylight saving, bit 1: daylight saving is in effect.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct EfiTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pad1: u8,
    pub nanosecond: u32,
    pub time_zone: i16,
    pub daylight: u8,
    pad2: u8,
}
type GetTimeFn = extern "efiapi" fn(*mut EfiTime, *mut u8) -> Status;

/// The firmware's clock now: Ok(time) / Err(status) (NOT_FOUND: no runtime services).
pub fn get_time() -> Result<EfiTime, Status> {
    let rt = crate::runtime_services();
    if rt.is_null() {
        return Err(NOT_FOUND);
    }
    let mut t = EfiTime::default();
    // SAFETY: slot GET_TIME per the UEFI spec, called as GetVariable is (see `bootentry`'s
    // section comment); the capabilities pointer is optional and passed as null.
    let st = masked(|| unsafe {
        let gt: GetTimeFn = core::mem::transmute((*rt).f[GET_TIME]);
        gt(&mut t, core::ptr::null_mut())
    });
    if st == SUCCESS { Ok(t) } else { Err(st) }
}

/// BOOTVARS read again from the firmware after a change (`bootentries` then says so).
pub fn reread_boot_entries() {
    let rt = crate::runtime_services();
    // SAFETY: see the section comment.
    masked(|| unsafe { read_boot_vars(rt) });
    // SAFETY: only the shell's one thread touches BOOTVARS after boot.
    unsafe { (*core::ptr::addr_of_mut!(BOOTVARS)).changed = true };
}
