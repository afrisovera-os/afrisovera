//! The desk's words, by id, in each language (LANGUAGES_PLAN.md in the OS notes; afriui.md section 27).
//!
//! Every string a person reads on the desk has an id here (`Msg`) and an English text. The drawing code asks for
//! `t(id)` or `fill(id, args, line)`, never a literal, so another language is one more table and no drawing change.
//!
//! - Sentences are whole, with numbered gaps `{0}` `{1}` `{2}`, so a language can put a number wherever its grammar
//!   wants. A gap that has no argument is left out; a `{` not followed by a digit and `}` is drawn as itself.
//! - What stays English on purpose: the `[desk]` serial lines and shell output (tests grade them), command words,
//!   file names, and the names AfriSovera, Mansa, Tonga and Nia.
//! - Static tables in .rodata: no heap, nothing parsed at run time.
//!
//! Stage 1 added the ids with English only (the desk drew exactly the pixels it drew before). Stage 2 adds the
//! switch (Settings, `desk lang <code>`, kept in afrisovera.conf) and Twi, every Twi string a Draft until a fluent
//! speaker checks it.

use core::sync::atomic::{AtomicU8, Ordering};

/// Is a string checked by a fluent speaker, or a machine draft? Every non-English string carries one.
#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Machine-drafted: Settings shows an ochre "draft" mark beside the language until a speaker checks it.
    Draft,
    /// Read and corrected by a fluent speaker (their initials and the date in a comment beside it).
    Checked,
}

/// The languages the desk can speak. The order is the order of the Settings chip.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Lang {
    En = 0,
    Tw = 1,
    Es = 2,
    Fr = 3,
    Pt = 4,
    Id = 5,
    De = 6,
    Tr = 7,
}

pub const LANGS: [Lang; 8] = [Lang::En, Lang::Tw, Lang::Es, Lang::Fr, Lang::Pt, Lang::Id, Lang::De, Lang::Tr];

/// Which way a language's text and the desk run (for the scripts engineer's mirrored desk, SHAPING_PLAN.md C2).
#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Ltr,
    Rtl,
}

#[allow(dead_code)]
impl Lang {
    /// The ISO 639 code, as written in afrisovera.conf and typed in `desk lang <code>`.
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Tw => "tw",
            Lang::Es => "es",
            Lang::Fr => "fr",
            Lang::Pt => "pt",
            Lang::Id => "id",
            Lang::De => "de",
            Lang::Tr => "tr",
        }
    }
    /// The language's name in itself, as the Settings chip shows it.
    #[allow(dead_code)]
    pub fn own_name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Tw => "Twi",
            Lang::Es => "Español",
            Lang::Fr => "Français",
            Lang::Pt => "Português",
            Lang::Id => "Bahasa Indonesia",
            Lang::De => "Deutsch",
            Lang::Tr => "Türkçe",
        }
    }
    /// Left to right for every language so far; Arabic, Urdu and Persian will say Rtl when they join the catalogue.
    pub fn dir(self) -> Dir {
        match self {
            Lang::En | Lang::Tw | Lang::Es | Lang::Fr | Lang::Pt | Lang::Id | Lang::De | Lang::Tr => Dir::Ltr,
        }
    }
    pub fn from_code(c: &str) -> Option<Lang> {
        LANGS.iter().copied().find(|l| l.code() == c)
    }
    /// The next language in the Settings chip's order, round to the first.
    pub fn next(self) -> Lang {
        LANGS[(self as usize + 1) % LANGS.len()]
    }
}

static CUR: AtomicU8 = AtomicU8::new(Lang::En as u8);

/// The language the desk draws in now.
pub fn current() -> Lang {
    let v = CUR.load(Ordering::Relaxed);
    LANGS.iter().copied().find(|l| *l as u8 == v).unwrap_or(Lang::En)
}

/// Choose the language the desk draws in (the caller redraws).
pub fn set(l: Lang) {
    CUR.store(l as u8, Ordering::Relaxed);
}

macro_rules! catalogue {
    ($($id:ident = $en:literal,)*) => {
        /// One id for every string a person reads on the desk.
        #[allow(dead_code)]
        #[derive(Clone, Copy, PartialEq, Eq)]
        pub enum Msg { $($id,)* }
        /// How many ids there are.
        pub const COUNT: usize = [$(Msg::$id),*].len();
        /// The ids' names, for `desk lang` and the speakers' sheet.
        #[allow(dead_code)]
        pub const IDS: [&str; COUNT] = [$(stringify!($id)),*];
        /// The English text of every id: the source every other language translates, and the fallback.
        pub static EN: [&str; COUNT] = [$($en),*];
        /// Every id in order, so a check can walk them.
        pub const ALL: [Msg; COUNT] = [$(Msg::$id),*];
    };
}

catalogue! {
    // the window names (the title bar, the Circle's tiles)
    AppFiles = "Files",
    AppTerminal = "Terminal",
    AppSettings = "Settings",
    AppAbout = "About",
    AppCell = "Cell",
    // Settings
    SetDisplay = "Display",
    SetPixels = "{0} x {1} logical pixels",
    SetInput = "Input",
    SetPointer = "Pointer",
    SetTouch = "Touch",
    SetDesktop = "Desktop",
    SetWallpaper = "Monument wallpaper",
    SetClock24 = "24-hour clock",
    SetMotion = "Motion",
    SetNetwork = "Network",
    SetNoDevice = "No network device",
    SetNoAddress = "Link up, no address yet",
    SetConnected = "   Connected  {0}",
    SetSystem = "System",
    SetCores = "{0} core(s), {1} MiB memory",
    SetBooted = "Booted {0} time(s), up {1} s",
    // About
    AboutMotto = "Own your device - own your future",
    AboutKernel = "Kernel",
    AboutDesktop = "Desktop",
    AboutDevice = "Device",
    AboutMemory = "Memory",
    AboutKernelIs = "Mansa microkernel, Rust",
    AboutDesktopIs = "Tonga, Sovereign UI",
    AboutArm = "ARM64 (phone)",
    AboutPc = "x86-64 (PC)",
    AboutDeviceIs = "{0}, {1} core(s)",
    AboutMemoryIs = "{0} MiB, up {1} s",
    // Files
    FilesName = "Name",
    FilesSize = "Size",
    FilesNone = "No files yet",
    FilesBytes = "{0} bytes",
    FilesWasm = "WebAssembly program",
    FilesRunIt = "Run it from Terminal:",
    FilesBinary = "Binary data",
    FilesSelect = "Select a file",
    // the clock and power on the Spine and the bar
    ClockUtc = "UTC",
    ClockAm = "am",
    ClockPm = "pm",
    PowerAc = "AC",
    // the Circle
    CircleNoNet = "No network",
    CircleNoAddress = "Link up, no address",
    CircleSummary = "{0},  {1} MiB,  {2} files",
    CircleDevice = "This device",
    CircleUp = "Up {0} s.  Super or Esc closes the Circle.",
    // notices and the title bar
    NotesTitle = "Notifications",
    NotesNone = "Nothing new",
    NoteWelcome = "Welcome to AfriSovera",
    TitleUnsigned = "unsigned",
    TitleAnyway = "Close anyway",
    // the language switch (stage 2)
    SetLanguage = "Language",
    SetDraft = "draft",
    // the Terminal scrolled back (afriui.md section 28; the owner's HP, 14 Nov); English only for now
    TermBackOne = "1 line back, any key returns",
    TermBack = "{0} lines back, any key returns",
    // the power panel on the Spine (the owner's HP, 14 Nov: no way to shut down from the desk); English only for now
    PowerTitle = "Power",
    PowerOff = "Shut down",
    PowerRestart = "Restart",
    PowerOffAsk = "Shut down now?",
    PowerRestartAsk = "Restart now?",
    PowerUnsaved = "Open apps close without saving.",
    // the Circle's app tiles (afriui.md section 32)
    CircleBusy = "The Terminal is busy; try again soon.",
    PowerCancel = "Cancel",
}

/// The text of an id in the current language (English when the language has no text for it yet).
pub fn t(m: Msg) -> &'static [u8] {
    text_in(current(), m)
}

/// Safety words (the architect's condition 1, 13 Nov): what a person relies on to stay safe. A language shows its
/// own words for these only when a fluent speaker has checked them; until then the desk draws the English. Camera,
/// permission and password prompts, and the update trial notices join this list as they join the catalogue.
pub const SAFETY: [Msg; 8] = [
    Msg::TitleUnsigned,
    Msg::TitleAnyway,
    Msg::SetDraft,
    // the power panel's words that confirm or warn: a draft must never be what someone reads before switching off or
    // losing work (the architect, 14 Nov). PowerTitle and PowerCancel need not be.
    Msg::PowerOffAsk,
    Msg::PowerRestartAsk,
    Msg::PowerUnsaved,
    Msg::PowerOff,
    Msg::PowerRestart,
];

pub fn is_safety(m: Msg) -> bool {
    SAFETY.iter().any(|&s| s == m)
}

/// A language's own entry for an id, before any fallback: ("", Draft) when it has none.
fn entry(l: Lang, m: Msg) -> (&'static str, Status) {
    match l {
        Lang::En => (EN[m as usize], Status::Checked),
        Lang::Tw => TW[m as usize],
        Lang::Es => ES[m as usize],
        Lang::Fr => FR[m as usize],
        Lang::Pt => PT[m as usize],
        Lang::Id => ID[m as usize],
        Lang::De => DE[m as usize],
        Lang::Tr => TR[m as usize],
    }
}

/// The text of an id in a given language, falling back to English: when the language has no text yet, and for a
/// safety word that is still a draft.
pub fn text_in(l: Lang, m: Msg) -> &'static [u8] {
    let (own, st) = entry(l, m);
    let draft_safety = is_safety(m) && st != Status::Checked;
    (if own.is_empty() || draft_safety { EN[m as usize] } else { own }).as_bytes()
}

/// A language's table, indexed by id, from a list of (id, text, status): an id the list leaves out is "" (shown in
/// English and counted). Built at compile time.
const fn table(pairs: &[(Msg, &'static str, Status)]) -> [(&'static str, Status); COUNT] {
    let mut t = [("", Status::Draft); COUNT];
    let mut i = 0;
    while i < pairs.len() {
        t[pairs[i].0 as usize] = (pairs[i].1, pairs[i].2);
        i += 1;
    }
    t
}

use self::Status::Draft;

/// Twi (Akan, Asante), drafted by machine on 13 Nov 2026. Every string is a Draft until a fluent speaker reads it;
/// a checked one becomes `Checked` with the checker's initials and the date in a comment. Product names, the
/// architecture names and UTC stay as they are; am, pm and AC are left out (shown in English) until a speaker says
/// what the clock and the plug should read.
static TW: [(&str, Status); COUNT] = table(&[
    (Msg::AppFiles, "Nkrataa", Draft),
    (Msg::AppTerminal, "Terminal", Draft),
    (Msg::AppSettings, "Nhyehyɛe", Draft),
    (Msg::AppAbout, "Ɛho nsɛm", Draft),
    (Msg::AppCell, "Cell", Draft),
    (Msg::SetDisplay, "Hwɛbea", Draft),
    (Msg::SetPixels, "{0} x {1} pixels", Draft),
    (Msg::SetInput, "Nhyɛmu", Draft),
    (Msg::SetPointer, "Pointa", Draft),
    (Msg::SetTouch, "Nsa ka", Draft),
    (Msg::SetDesktop, "Adwumayɛ pon", Draft),
    (Msg::SetWallpaper, "Nkaedum mfoni", Draft),
    (Msg::SetClock24, "Dɔnhwerew 24", Draft),
    (Msg::SetMotion, "Nkankɔ", Draft),
    (Msg::SetNetwork, "Nkitahodi", Draft),
    (Msg::SetNoDevice, "Nkitahodi afiri biara nni hɔ", Draft),
    (Msg::SetNoAddress, "Ɛkeka ho, nanso address nni hɔ", Draft),
    (Msg::SetConnected, "   Ɛkeka ho  {0}", Draft),
    (Msg::SetSystem, "Sistem", Draft),
    (Msg::SetCores, "Core {0}, memory {1} MiB", Draft),
    (Msg::SetBooted, "Wɔabue no mpɛn {0}, ɛreyɛ adwuma {1} s", Draft),
    (Msg::AboutMotto, "Wo afiri yɛ wo dea, wo daakye yɛ wo dea", Draft),
    (Msg::AboutKernel, "Kernel", Draft),
    (Msg::AboutDesktop, "Adwumayɛ pon", Draft),
    (Msg::AboutDevice, "Afiri", Draft),
    (Msg::AboutMemory, "Memory", Draft),
    (Msg::AboutKernelIs, "Mansa microkernel, Rust", Draft),
    (Msg::AboutDesktopIs, "Tonga, Sovereign UI", Draft),
    (Msg::AboutArm, "ARM64 (fon)", Draft),
    (Msg::AboutPc, "x86-64 (kɔmputa)", Draft),
    (Msg::AboutDeviceIs, "{0}, core {1}", Draft),
    (Msg::AboutMemoryIs, "{0} MiB, adwuma {1} s", Draft),
    (Msg::FilesName, "Din", Draft),
    (Msg::FilesSize, "Kɛseɛ", Draft),
    (Msg::FilesNone, "Fael biara nni hɔ", Draft),
    (Msg::FilesBytes, "byte {0}", Draft),
    (Msg::FilesWasm, "WebAssembly dwumadi", Draft),
    (Msg::FilesRunIt, "Bue no wɔ Terminal mu:", Draft),
    (Msg::FilesBinary, "Binary data", Draft),
    (Msg::FilesSelect, "Yi fael bi", Draft),
    (Msg::ClockUtc, "UTC", Draft),
    (Msg::CircleNoNet, "Nkitahodi nni hɔ", Draft),
    (Msg::CircleNoAddress, "Ɛkeka ho, address nni hɔ", Draft),
    (Msg::CircleSummary, "{0},  {1} MiB,  fael {2}", Draft),
    (Msg::CircleDevice, "Afiri yi", Draft),
    (Msg::CircleUp, "Adwuma {0} s.  Super anaa Esc to mu.", Draft),
    (Msg::NotesTitle, "Nkra", Draft),
    (Msg::NotesNone, "Biribi foforo nni hɔ", Draft),
    (Msg::NoteWelcome, "Akwaaba wɔ AfriSovera", Draft),
    (Msg::TitleUnsigned, "wɔnhyɛɛ ase", Draft),
    (Msg::TitleAnyway, "To mu ara", Draft),
    (Msg::SetLanguage, "Kasa", Draft),
    (Msg::PowerCancel, "Gyae", Draft),
]);

/// Spanish (Español), machine drafts on 14 Nov 2026 by Helsinki-NLP/opus-mt-en-es at revision 5bc4493d463c (licence apache-2.0), made in the cloud by
/// tools/lang/draft.py. Every string is a Draft until a fluent speaker checks it; the safety words are drawn in
/// English until then whatever this table says.
static ES: [(&str, Status); COUNT] = table(&[
    (Msg::AppFiles, "Archivos", Draft),
    (Msg::AppSettings, "Configuración", Draft),
    (Msg::AppAbout, "Acerca de", Draft),
    (Msg::SetDisplay, "Visor", Draft),
    (Msg::SetPixels, "{0} x {1} píxeles lógicos", Draft),
    (Msg::SetInput, "Entrada", Draft),
    (Msg::SetPointer, "Puntero", Draft),
    (Msg::SetTouch, "Touch", Draft),
    (Msg::SetDesktop, "Escritorio", Draft),
    (Msg::SetWallpaper, "Fondo de pantalla del monumento", Draft),
    (Msg::SetClock24, "Reloj 24 horas", Draft),
    (Msg::SetMotion, "Moción", Draft),
    (Msg::SetNetwork, "Red", Draft),
    (Msg::SetNoDevice, "Ningún dispositivo de red", Draft),
    (Msg::SetNoAddress, "Enlace, no hay dirección todavía", Draft),
    (Msg::SetConnected, "   Conectado {0}", Draft),
    (Msg::SetSystem, "Sistema", Draft),
    (Msg::SetCores, "{0} núcleo(s), {1} MiB memoria", Draft),
    (Msg::SetBooted, "Arrancado {0} tiempo(s), hasta {1} s", Draft),
    (Msg::AboutMotto, "Posee tu dispositivo - posee tu futuro", Draft),
    (Msg::AboutDesktop, "Escritorio", Draft),
    (Msg::AboutDevice, "Dispositivo", Draft),
    (Msg::AboutMemory, "Memoria", Draft),
    (Msg::AboutDeviceIs, "{0}, {1} núcleo(s)", Draft),
    (Msg::AboutMemoryIs, "{0} MiB, up {1} s", Draft),
    (Msg::FilesName, "Nombre", Draft),
    (Msg::FilesSize, "Tamaño", Draft),
    (Msg::FilesNone, "Aún no hay archivos.", Draft),
    (Msg::FilesBytes, "{0} bytes", Draft),
    (Msg::FilesWasm, "Programa WebAssembly", Draft),
    (Msg::FilesRunIt, "Ejecutarlo desde Terminal:", Draft),
    (Msg::FilesBinary, "Datos binarios", Draft),
    (Msg::FilesSelect, "Seleccione un archivo", Draft),
    (Msg::ClockAm, "- Sí.", Draft),
    (Msg::ClockPm, "pm", Draft),
    (Msg::CircleNoNet, "Sin red", Draft),
    (Msg::CircleNoAddress, "Enlace, no hay dirección", Draft),
    (Msg::CircleSummary, "{0}, {1} MiB, {2} archivos", Draft),
    (Msg::CircleDevice, "Este dispositivo", Draft),
    (Msg::CircleUp, "Up {0} s. Super o Esc cierra el círculo.", Draft),
    (Msg::NotesTitle, "Notificaciones", Draft),
    (Msg::NotesNone, "Nada nuevo", Draft),
    (Msg::NoteWelcome, "Bienvenido a AfriSovera", Draft),
    (Msg::TitleUnsigned, "sin firmar", Draft),
    (Msg::TitleAnyway, "Cierra de todos modos.", Draft),
    (Msg::SetLanguage, "Idioma", Draft),
    (Msg::SetDraft, "proyecto", Draft),
    (Msg::TermBackOne, "1 línea atrás, cualquier clave devuelve", Draft),
    (Msg::TermBack, "{0} líneas atrás, cualquier clave devuelve", Draft),
]);

/// French (Français), machine drafts on 14 Nov 2026 by Helsinki-NLP/opus-mt-en-fr at revision dd7f6540a7a4 (licence apache-2.0), made in the cloud by
/// tools/lang/draft.py. Every string is a Draft until a fluent speaker checks it; the safety words are drawn in
/// English until then whatever this table says.
/// Left out (too wide for their place on the desk; the English shows until a speaker gives a shorter word): AboutMemoryIs, CircleUp, FilesRunIt, FilesWasm, SetPointer, SetTouch, TermBack, TermBackOne.
static FR: [(&str, Status); COUNT] = table(&[
    (Msg::AppFiles, "Fichiers", Draft),
    (Msg::AppSettings, "Paramètres", Draft),
    (Msg::AppAbout, "À propos", Draft),
    (Msg::SetDisplay, "Affichage", Draft),
    (Msg::SetPixels, "{0} x {1} pixels logiques", Draft),
    (Msg::SetInput, "Entrée", Draft),
    (Msg::SetDesktop, "Bureau", Draft),
    (Msg::SetWallpaper, "Papier peint monumental", Draft),
    (Msg::SetClock24, "24 heures sur 24", Draft),
    (Msg::SetMotion, "Motion", Draft),
    (Msg::SetNetwork, "Réseau", Draft),
    (Msg::SetNoDevice, "Pas de périphérique réseau", Draft),
    (Msg::SetNoAddress, "Lien vers le haut, pas d'adresse pour l'instant", Draft),
    (Msg::SetConnected, "   Connecté {0}", Draft),
    (Msg::SetSystem, "Système", Draft),
    (Msg::SetCores, "{0} cœur(s), {1} Mio de mémoire", Draft),
    (Msg::SetBooted, "Démarrage {0} fois, jusqu'à {1} s", Draft),
    (Msg::AboutMotto, "Posséder votre appareil - posséder votre futur", Draft),
    (Msg::AboutDesktop, "Bureau", Draft),
    (Msg::AboutDevice, "Périphérique", Draft),
    (Msg::AboutMemory, "Mémoire", Draft),
    (Msg::AboutDeviceIs, "{0}, {1} cœur(s)", Draft),
    (Msg::FilesName, "Dénomination", Draft),
    (Msg::FilesSize, "Taille", Draft),
    (Msg::FilesNone, "Pas encore de fichiers", Draft),
    (Msg::FilesBytes, "{0} octets", Draft),
    (Msg::FilesBinary, "Données binaires", Draft),
    (Msg::FilesSelect, "Sélectionnez un fichier", Draft),
    (Msg::ClockAm, "Je suis", Draft),
    (Msg::ClockPm, "pm", Draft),
    (Msg::CircleNoNet, "Pas de réseau", Draft),
    (Msg::CircleNoAddress, "Lien vers le haut, pas d'adresse", Draft),
    (Msg::CircleSummary, "{0}, {1} fichiers MiB, {2} fichiers", Draft),
    (Msg::CircleDevice, "Ce dispositif", Draft),
    (Msg::NotesTitle, "Notifications", Draft),
    (Msg::NotesNone, "Rien de nouveau", Draft),
    (Msg::NoteWelcome, "Bienvenue à AfriSovera", Draft),
    (Msg::TitleUnsigned, "non signé", Draft),
    (Msg::TitleAnyway, "Fermer de toute façon", Draft),
    (Msg::SetLanguage, "Langue", Draft),
    (Msg::SetDraft, "projet", Draft),
]);

/// Portuguese (Português), machine drafts on 14 Nov 2026 by Helsinki-NLP/opus-mt-tc-big-en-pt at revision 9f2863d807ec (licence cc-by-4.0), made in the cloud by
/// tools/lang/draft.py. Every string is a Draft until a fluent speaker checks it; the safety words are drawn in
/// English until then whatever this table says.
/// Left out (too wide for their place on the desk; the English shows until a speaker gives a shorter word): AppSettings, SetPointer.
static PT: [(&str, Status); COUNT] = table(&[
    (Msg::AppFiles, "Ficheiros", Draft),
    (Msg::AppAbout, "Acerca", Draft),
    (Msg::SetDisplay, "Mostrar", Draft),
    (Msg::SetPixels, "{0} x {1} pixels lógicos", Draft),
    (Msg::SetInput, "Entrada", Draft),
    (Msg::SetTouch, "Toque", Draft),
    (Msg::SetDesktop, "Área de trabalho", Draft),
    (Msg::SetWallpaper, "Papel de parede de monumento", Draft),
    (Msg::SetClock24, "Relógio 24 horas", Draft),
    (Msg::SetMotion, "Moção", Draft),
    (Msg::SetNetwork, "Rede", Draft),
    (Msg::SetNoDevice, "Nenhum dispositivo de rede", Draft),
    (Msg::SetNoAddress, "Link up, nenhum endereço ainda", Draft),
    (Msg::SetConnected, "   Conectado {0}", Draft),
    (Msg::SetSystem, "Sistema", Draft),
    (Msg::SetCores, "{0} núcleo(s), {1} MiB memória", Draft),
    (Msg::SetBooted, "Booted {0} vez(s), até {1} s", Draft),
    (Msg::AboutMotto, "Possuir o seu dispositivo - possuir o seu futuro", Draft),
    (Msg::AboutDesktop, "Área de trabalho", Draft),
    (Msg::AboutDevice, "Dispositivo", Draft),
    (Msg::AboutMemory, "Memória", Draft),
    (Msg::AboutDeviceIs, "{0}, {1} núcleo(s)", Draft),
    (Msg::AboutMemoryIs, "{0} MiB, até {1} s", Draft),
    (Msg::FilesName, "Nome", Draft),
    (Msg::FilesSize, "Tamanho", Draft),
    (Msg::FilesNone, "Nenhum arquivo ainda", Draft),
    (Msg::FilesBytes, "{0} bytes", Draft),
    (Msg::FilesWasm, "Programa WebAssembly", Draft),
    (Msg::FilesRunIt, "Execute-o a partir do terminal:", Draft),
    (Msg::FilesBinary, "Dados binários", Draft),
    (Msg::FilesSelect, "Selecione um arquivo", Draft),
    (Msg::ClockAm, "am", Draft),
    (Msg::ClockPm, "pm", Draft),
    (Msg::CircleNoNet, "Nenhuma rede", Draft),
    (Msg::CircleNoAddress, "Ligar, sem endereço", Draft),
    (Msg::CircleSummary, "{0}, {1} MiB, {2} arquivos", Draft),
    (Msg::CircleDevice, "Este dispositivo", Draft),
    (Msg::CircleUp, "Até {0} s. Super ou Esc fecha o Círculo.", Draft),
    (Msg::NotesTitle, "Notificações", Draft),
    (Msg::NotesNone, "Nada de novo", Draft),
    (Msg::NoteWelcome, "Bem-vindo ao AfriSovera", Draft),
    (Msg::TitleUnsigned, "unsigned", Draft),
    (Msg::TitleAnyway, "Fechar de qualquer maneira", Draft),
    (Msg::SetLanguage, "Língua", Draft),
    (Msg::SetDraft, "projecto", Draft),
    (Msg::TermBackOne, "1 linha para trás, qualquer chave retorna", Draft),
    (Msg::TermBack, "{0} linhas para trás, qualquer chave retorna", Draft),
]);

/// Indonesian (Bahasa Indonesia), machine drafts on 14 Nov 2026 by Helsinki-NLP/opus-mt-en-id at revision 6e4c52d61a6b (licence apache-2.0), made in the cloud by
/// tools/lang/draft.py. Every string is a Draft until a fluent speaker checks it; the safety words are drawn in
/// English until then whatever this table says.
/// Left out (too wide for their place on the desk; the English shows until a speaker gives a shorter word): AboutMotto, CircleUp, SetPointer.
static ID: [(&str, Status); COUNT] = table(&[
    (Msg::AppFiles, "Berkas", Draft),
    (Msg::AppSettings, "Pengaturan", Draft),
    (Msg::AppAbout, "Tentang", Draft),
    (Msg::SetDisplay, "Tampilan", Draft),
    (Msg::SetPixels, "{0} x {1} piksel logis", Draft),
    (Msg::SetInput, "Masukan", Draft),
    (Msg::SetTouch, "Sentuh", Draft),
    (Msg::SetDesktop, "Desktop", Draft),
    (Msg::SetWallpaper, "wallpaper monumen", Draft),
    (Msg::SetClock24, "24 jam", Draft),
    (Msg::SetMotion, "Gerak", Draft),
    (Msg::SetNetwork, "Jaringan", Draft),
    (Msg::SetNoDevice, "Tak ada perangkat jaringan", Draft),
    (Msg::SetNoAddress, "Link up, tidak ada alamat belum", Draft),
    (Msg::SetConnected, "   Terhubung {0}", Draft),
    (Msg::SetSystem, "Sistem", Draft),
    (Msg::SetCores, "{0} core 1936s), {1} MiB memori", Draft),
    (Msg::SetBooted, "Ditendang {0} kali di tahun 1964s), naik {1} s", Draft),
    (Msg::AboutDesktop, "Desktop", Draft),
    (Msg::AboutDevice, "Perangkat", Draft),
    (Msg::AboutMemory, "Memori", Draft),
    (Msg::AboutDeviceIs, "{0}, {1} core 1932s)", Draft),
    (Msg::AboutMemoryIs, "{0} MiB, naik {1} s", Draft),
    (Msg::FilesName, "Nama", Draft),
    (Msg::FilesSize, "Ukuran", Draft),
    (Msg::FilesNone, "Belum ada berkas", Draft),
    (Msg::FilesBytes, "{0} byte", Draft),
    (Msg::FilesWasm, "Program WebAssembly", Draft),
    (Msg::FilesRunIt, "Jalankan dari Terminal:", Draft),
    (Msg::FilesBinary, "Data binari", Draft),
    (Msg::FilesSelect, "Pilih sebuah berkas", Draft),
    (Msg::ClockAm, "am", Draft),
    (Msg::ClockPm, "pm", Draft),
    (Msg::CircleNoNet, "Tidak ada jaringan", Draft),
    (Msg::CircleNoAddress, "Link up, tidak ada alamat", Draft),
    (Msg::CircleSummary, "{0}, {1} MiB, {2} file", Draft),
    (Msg::CircleDevice, "Perangkat ini", Draft),
    (Msg::NotesTitle, "Pemberitahuan", Draft),
    (Msg::NotesNone, "Tidak ada yang baru", Draft),
    (Msg::NoteWelcome, "Selamat Datang di AfriSovera", Draft),
    (Msg::TitleUnsigned, "unsigned", Draft),
    (Msg::TitleAnyway, "Tetap tutup", Draft),
    (Msg::SetLanguage, "Bahasa", Draft),
    (Msg::SetDraft, "draft", Draft),
    (Msg::TermBackOne, "1 baris kembali, setiap kunci kembali", Draft),
    (Msg::TermBack, "{0} baris kembali, setiap kunci kembali", Draft),
]);

/// German (Deutsch), machine drafts on 14 Nov 2026 by Helsinki-NLP/opus-mt-en-de at revision 6183067f769a (licence cc-by-4.0), made in the cloud by
/// tools/lang/draft.py. Every string is a Draft until a fluent speaker checks it; the safety words are drawn in
/// English until then whatever this table says.
/// Left out (too wide for their place on the desk; the English shows until a speaker gives a shorter word): CircleUp, ClockPm, FilesRunIt, SetTouch, TermBack, TermBackOne.
static DE: [(&str, Status); COUNT] = table(&[
    (Msg::AppFiles, "Dateien", Draft),
    (Msg::AppSettings, "Einstellungen", Draft),
    (Msg::AppAbout, "Über", Draft),
    (Msg::SetDisplay, "Anzeige", Draft),
    (Msg::SetPixels, "{0} x {1} logische Pixel", Draft),
    (Msg::SetInput, "Eingabe", Draft),
    (Msg::SetPointer, "Zeiger", Draft),
    (Msg::SetDesktop, "Arbeitsfläche", Draft),
    (Msg::SetWallpaper, "Denkmal Tapete", Draft),
    (Msg::SetClock24, "24-Stunden-Uhr", Draft),
    (Msg::SetMotion, "Antrag", Draft),
    (Msg::SetNetwork, "Netzwerk", Draft),
    (Msg::SetNoDevice, "Kein Netzwerkgerät", Draft),
    (Msg::SetNoAddress, "Link up, noch keine Adresse", Draft),
    (Msg::SetConnected, "   Verbunden {0}", Draft),
    (Msg::SetSystem, "System", Draft),
    (Msg::SetCores, "{0} Kern(e), {1} MiB-Speicher", Draft),
    (Msg::SetBooted, "Gebootet {0} Zeit(e), bis {1} s", Draft),
    (Msg::AboutMotto, "Besitzen Sie Ihr Gerät - besitzen Sie Ihre Zukunft", Draft),
    (Msg::AboutDesktop, "Arbeitsfläche", Draft),
    (Msg::AboutDevice, "Gerät", Draft),
    (Msg::AboutMemory, "Speicher", Draft),
    (Msg::AboutDeviceIs, "{0}, {1} Kern(e)", Draft),
    (Msg::AboutMemoryIs, "{0} MiB, nach oben {1} s", Draft),
    (Msg::FilesName, "Bezeichnung", Draft),
    (Msg::FilesSize, "Größe", Draft),
    (Msg::FilesNone, "Noch keine Dateien", Draft),
    (Msg::FilesBytes, "{0} Bytes", Draft),
    (Msg::FilesWasm, "WebAssembly-Programm", Draft),
    (Msg::FilesBinary, "Binäre Daten", Draft),
    (Msg::FilesSelect, "Eine Datei auswählen", Draft),
    (Msg::ClockAm, "bin", Draft),
    (Msg::CircleNoNet, "Kein Netz", Draft),
    (Msg::CircleNoAddress, "Verlinken, keine Adresse", Draft),
    (Msg::CircleSummary, "{0}, {1} MiB, {2} Dateien", Draft),
    (Msg::CircleDevice, "Dieses Gerät", Draft),
    (Msg::NotesTitle, "Mitteilungen", Draft),
    (Msg::NotesNone, "Nichts Neues", Draft),
    (Msg::NoteWelcome, "Willkommen bei AfriSovera", Draft),
    (Msg::TitleUnsigned, "unsigniert", Draft),
    (Msg::TitleAnyway, "Schließen Sie sich sowieso.", Draft),
    (Msg::SetLanguage, "Sprache", Draft),
    (Msg::SetDraft, "Entwurf", Draft),
]);

/// Turkish (Türkçe), machine drafts on 14 Nov 2026 by Helsinki-NLP/opus-mt-tc-big-en-tr at revision e539fc16a8a1 (licence cc-by-4.0), made in the cloud by
/// tools/lang/draft.py. Every string is a Draft until a fluent speaker checks it; the safety words are drawn in
/// English until then whatever this table says.
/// Left out (too wide for their place on the desk; the English shows until a speaker gives a shorter word): CircleUp.
static TR: [(&str, Status); COUNT] = table(&[
    (Msg::AppFiles, "Dosyalar", Draft),
    (Msg::AppSettings, "Ayarlar", Draft),
    (Msg::AppAbout, "Hakkında", Draft),
    (Msg::SetDisplay, "Göster", Draft),
    (Msg::SetPixels, "{0} x {1} mantıksal piksel", Draft),
    (Msg::SetInput, "Giriş", Draft),
    (Msg::SetPointer, "İşaretçi", Draft),
    (Msg::SetTouch, "Dokun", Draft),
    (Msg::SetDesktop, "Masaüstü", Draft),
    (Msg::SetWallpaper, "Anıt duvar kağıdı", Draft),
    (Msg::SetClock24, "24 saat", Draft),
    (Msg::SetMotion, "Hareket", Draft),
    (Msg::SetNetwork, "Ağ", Draft),
    (Msg::SetNoDevice, "Ağ aygıtı yok", Draft),
    (Msg::SetNoAddress, "Bağlantı, henüz adres yok", Draft),
    (Msg::SetConnected, "   Bağlı {0}", Draft),
    (Msg::SetSystem, "Sistem", Draft),
    (Msg::SetCores, "{0} çekirdek(ler), {1} MiB bellek", Draft),
    (Msg::SetBooted, "Booted {0} kez, yukarı {1} s", Draft),
    (Msg::AboutMotto, "Cihazınıza sahip olun - geleceğinize sahip olun", Draft),
    (Msg::AboutDesktop, "Masaüstü", Draft),
    (Msg::AboutDevice, "Aygıt", Draft),
    (Msg::AboutMemory, "Bellek", Draft),
    (Msg::AboutDeviceIs, "{0}, {1} çekirdek(ler)", Draft),
    (Msg::AboutMemoryIs, "{0} MiB, yukarı {1} s", Draft),
    (Msg::FilesName, "İsim", Draft),
    (Msg::FilesSize, "Boyut", Draft),
    (Msg::FilesNone, "Henüz dosya yok", Draft),
    (Msg::FilesBytes, "{0} bayt", Draft),
    (Msg::FilesWasm, "WebAssembly programı", Draft),
    (Msg::FilesRunIt, "Terminal'den çalıştır:", Draft),
    (Msg::FilesBinary, "İkili veri", Draft),
    (Msg::FilesSelect, "Bir dosya seçin", Draft),
    (Msg::ClockAm, "ben", Draft),
    (Msg::ClockPm, "pm", Draft),
    (Msg::CircleNoNet, "Ağ yok", Draft),
    (Msg::CircleNoAddress, "Bağlan, adres yok", Draft),
    (Msg::CircleSummary, "{0}, {1} MiB, {2} dosya", Draft),
    (Msg::CircleDevice, "Bu cihaz", Draft),
    (Msg::NotesTitle, "Bildirimler", Draft),
    (Msg::NotesNone, "Yeni bir şey yok.", Draft),
    (Msg::NoteWelcome, "Afrika'ya Hoş Geldiniz", Draft),
    (Msg::TitleUnsigned, "imzasız", Draft),
    (Msg::TitleAnyway, "Yine de kapat.", Draft),
    (Msg::SetLanguage, "Dil", Draft),
    (Msg::SetDraft, "taslak", Draft),
    (Msg::TermBackOne, "1 satır geri, herhangi bir anahtar döner", Draft),
    (Msg::TermBack, "{0} satır geri, herhangi bir anahtar döner", Draft),
]);

/// One value for a gap in a sentence.
#[derive(Clone, Copy)]
pub enum Arg<'a> {
    N(u64),
    S(&'a [u8]),
}

/// Write the sentence for `m` with its gaps filled through `put` (a number is written as decimal digits).
pub fn fill_with(m: Msg, args: &[Arg], put: &mut dyn FnMut(&[u8])) {
    fill_text(t(m), args, put)
}

/// Fill the gaps of a template. `{d}` (one digit) takes `args[d]`; a gap with no argument writes nothing; any other
/// byte, a lone `{` included, is copied as it is. Never reads past the template.
pub fn fill_text(s: &[u8], args: &[Arg], put: &mut dyn FnMut(&[u8])) {
    let mut i = 0;
    let mut from = 0;
    while i < s.len() {
        if s[i] == b'{' && i + 2 < s.len() && s[i + 1].is_ascii_digit() && s[i + 2] == b'}' {
            put(&s[from..i]);
            match args.get((s[i + 1] - b'0') as usize) {
                Some(Arg::N(v)) => {
                    let mut d = [0u8; 20];
                    let mut k = d.len();
                    let mut v = *v;
                    loop {
                        k -= 1;
                        d[k] = b'0' + (v % 10) as u8;
                        v /= 10;
                        if v == 0 {
                            break;
                        }
                    }
                    put(&d[k..]);
                }
                Some(Arg::S(b)) => put(b),
                None => {}
            }
            i += 3;
            from = i;
        } else {
            i += 1;
        }
    }
    put(&s[from..]);
}

/// The language codes, for the `desk lang` usage line.
pub fn codes() -> &'static str {
    "en tw es fr pt id de tr"
}

/// For a language: (ids, drafts, checked, shown in English because there is no text yet). English counts its own
/// strings as checked.
pub fn counts(l: Lang) -> (usize, usize, usize, usize) {
    let t = match l {
        Lang::En => return (COUNT, 0, COUNT, 0),
        Lang::Tw => &TW,
        Lang::Es => &ES,
        Lang::Fr => &FR,
        Lang::Pt => &PT,
        Lang::Id => &ID,
        Lang::De => &DE,
        Lang::Tr => &TR,
    };
    let (mut draft, mut checked, mut english) = (0, 0, 0);
    for (k, &(s, st)) in t.iter().enumerate() {
        // a safety word not yet checked is drawn in English, so it counts as English until checked
        if s.is_empty() || (is_safety(ALL[k]) && st != Status::Checked) {
            english += 1;
        } else if st == Status::Checked {
            checked += 1;
        } else {
            draft += 1;
        }
    }
    (COUNT, draft, checked, english)
}

/// Check every language's strings: the same gaps as the English, no combining marks (U+0300 to U+036F; the desk does
/// not place them over a letter yet), no em or en dashes, no ellipsis character and no double hyphen (the owner's
/// writing law). Each problem goes to `bad(code, id, why)`.
pub fn check(bad: &mut dyn FnMut(&str, &str, &str)) {
    for &l in LANGS.iter() {
        for k in 0..COUNT {
            let s = text_in(l, ALL[k]);
            if gaps(s) != gaps(EN[k].as_bytes()) {
                bad(l.code(), IDS[k], "gaps differ from the English");
            }
            let st = core::str::from_utf8(s).unwrap_or("");
            if st.is_empty() && !s.is_empty() {
                bad(l.code(), IDS[k], "not UTF-8");
            }
            if st.chars().any(|c| ('\u{300}'..='\u{36F}').contains(&c)) {
                bad(l.code(), IDS[k], "a combining mark");
            }
            if st.chars().any(|c| c == '\u{2013}' || c == '\u{2014}' || c == '\u{2026}') || st.contains("--") {
                bad(l.code(), IDS[k], "a dash or ellipsis the writing law bans");
            }
            // a safety word is never drawn from a draft
            let (own, status) = entry(l, ALL[k]);
            if is_safety(ALL[k]) && status != Status::Checked && !own.is_empty() && own != EN[k] && s == own.as_bytes() && l != Lang::En {
                bad(l.code(), IDS[k], "a safety word drawn from a draft");
            }
        }
    }
}

/// The gaps a template uses, as a bit set (bit d for `{d}`), so a test can check every language uses the same gaps
/// as its English.
#[allow(dead_code)]
pub fn gaps(s: &[u8]) -> u16 {
    let mut g = 0u16;
    let mut i = 0;
    while i + 2 < s.len() {
        if s[i] == b'{' && s[i + 1].is_ascii_digit() && s[i + 2] == b'}' {
            g |= 1 << (s[i + 1] - b'0');
            i += 3;
        } else {
            i += 1;
        }
    }
    g
}
