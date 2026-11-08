//! Daraja — the **Android compatibility bridge** (scaffold v0).
//!
//! Daraja is how AfriSovera aims to meet users where they are: let an Android app
//! be *imported* and run — but under AfriSovera's rules, not Android's. The single
//! most important part of that, and the part built here, is the **security
//! boundary**: an Android app declares permissions in its manifest; Daraja
//! translates each one into an AfriSovera **capability** and a **policy** (auto /
//! ask-the-user / denied). So an imported Android app gets *only* the capabilities
//! the user grants — the same "malware can't breathe / no ambient authority" model
//! that native apps live under (docs/10).
//!
//! **Honest scope:** this parses an app's declared permissions (a `.apkmeta`
//! descriptor) and produces the capability decision. It does **not** execute
//! Android bytecode. A real bridge additionally needs an ART/Dalvik-class runtime,
//! a libc/Bionic shim, and HAL shims for vendor blobs (docs/11) — a large future
//! build-out that sits *behind* this permission gate. Nothing here pretends an
//! Android app actually runs; it decides what one *would be allowed to do*.

/// What AfriSovera does with a requested Android permission.
#[derive(Clone, Copy, PartialEq)]
enum Policy {
    Auto,   // harmless — granted automatically
    Consent, // sensitive — requires an explicit user grant
    Denied, // dangerous/ambient or unsupported — blocked by default
}

impl Policy {
    fn label(self) -> &'static str {
        match self {
            Policy::Auto => "auto-grant",
            Policy::Consent => "ask user",
            Policy::Denied => "DENIED",
        }
    }
}

/// Translate an Android permission string to an AfriSovera capability + policy.
fn translate(perm: &str) -> (&'static str, Policy) {
    // strip the android.permission. prefix if present
    let p = perm.rsplit('.').next().unwrap_or(perm);
    match p {
        "INTERNET" | "ACCESS_NETWORK_STATE" => ("net", Policy::Consent),
        "CAMERA" => ("camera", Policy::Consent),
        "RECORD_AUDIO" => ("microphone", Policy::Consent),
        "ACCESS_FINE_LOCATION" | "ACCESS_COARSE_LOCATION" => ("location", Policy::Consent),
        "READ_CONTACTS" | "WRITE_CONTACTS" => ("contacts", Policy::Consent),
        "READ_EXTERNAL_STORAGE" | "WRITE_EXTERNAL_STORAGE" => ("file (scoped)", Policy::Consent),
        "VIBRATE" | "WAKE_LOCK" | "SET_WALLPAPER" => ("device", Policy::Auto),
        // ambient / high-risk: no equivalent — blocked by default
        "SEND_SMS" | "READ_SMS" | "RECEIVE_SMS" => ("sms", Policy::Denied),
        "READ_PHONE_STATE" | "CALL_PHONE" => ("telephony", Policy::Denied),
        "SYSTEM_ALERT_WINDOW" | "WRITE_SETTINGS" => ("system", Policy::Denied),
        "REQUEST_INSTALL_PACKAGES" | "QUERY_ALL_PACKAGES" => ("package-mgmt", Policy::Denied),
        _ => ("unsupported", Policy::Denied),
    }
}

/// Return the value after the first "key:" on a line, trimmed, or None.
fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let line = line.trim();
    let rest = line.strip_prefix(key)?;
    let rest = rest.strip_prefix(':')?;
    Some(rest.trim())
}

/// Parse an `.apkmeta` descriptor and print the capability translation + verdict.
pub fn inspect(src: &[u8]) {
    let text = match core::str::from_utf8(src) {
        Ok(t) => t,
        Err(_) => {
            crate::kprintln!("daraja: not valid UTF-8");
            return;
        }
    };

    let mut package = "?";
    let mut label = "?";
    for line in text.lines() {
        if let Some(v) = field(line, "package") {
            package = v;
        } else if let Some(v) = field(line, "label") {
            label = v;
        }
    }

    crate::kprintln!("Daraja - Android app import (permission translation)");
    crate::kprintln!("  package: {}", package);
    crate::kprintln!("  label:   {}", label);
    crate::kprintln!("  {:<28} {:<16} {}", "android permission", "capability", "policy");
    crate::kprintln!("  {:-<28} {:-<16} {}", "", "", "------");

    let (mut total, mut auto, mut consent, mut denied) = (0u32, 0u32, 0u32, 0u32);
    for line in text.lines() {
        if let Some(perm) = field(line, "permission") {
            let (cap, pol) = translate(perm);
            let short = perm.rsplit('.').next().unwrap_or(perm);
            crate::kprintln!("  {:<28} {:<16} {}", short, cap, pol.label());
            total += 1;
            match pol {
                Policy::Auto => auto += 1,
                Policy::Consent => consent += 1,
                Policy::Denied => denied += 1,
            }
        }
    }

    crate::kprintln!(
        "  => {} permissions: {} auto, {} need consent, {} denied by default",
        total,
        auto,
        consent,
        denied
    );
    crate::kprintln!("  (imported Android apps run under AfriSovera capabilities - no ambient authority)");
    crate::kprintln!("  note: Daraja v0 translates permissions only; it does NOT run Android bytecode yet.");
}

/// A sample `.apkmeta` descriptor seeded into the VFS at boot.
pub fn sample() -> &'static [u8] {
    b"package: com.example.chat\n\
label: Example Chat\n\
permission: android.permission.INTERNET\n\
permission: android.permission.CAMERA\n\
permission: android.permission.RECORD_AUDIO\n\
permission: android.permission.READ_CONTACTS\n\
permission: android.permission.ACCESS_FINE_LOCATION\n\
permission: android.permission.VIBRATE\n\
permission: android.permission.SEND_SMS\n\
permission: android.permission.SYSTEM_ALERT_WINDOW\n"
}
