//! The Intel firmware file reader (kernel/src/pc/iwm_fw.rs, Wi-Fi W5), compiled here unchanged by `#[path]`: its
//! unit tests run on the host (files built in the test from the layout: good ones, every truncation, wrong lengths,
//! unknown records, section limits and overlaps).
#[path = "../../kernel/src/pc/iwm_fw.rs"]
#[allow(dead_code)]
mod iwm_fw;

/// The real Intel files, when a run fetched them (IWM_FW_DIR, set by the cloud job that pins them; skipped
/// otherwise): each must read whole, with a regular and an init image, and its summary is printed for the evidence.
#[test]
fn real_intel_files_read_whole() {
    let Ok(dir) = std::env::var("IWM_FW_DIR") else {
        println!("IWM_FW_DIR not set: no real firmware file to read here");
        return;
    };
    let mut n = 0;
    let mut refused = 0;
    for e in std::fs::read_dir(&dir).expect("IWM_FW_DIR") {
        let p = e.unwrap().path();
        if p.extension().and_then(|x| x.to_str()) != Some("ucode") {
            continue;
        }
        let f = std::fs::read(&p).unwrap();
        // every record type in the file, by a plain walk here, so a refusal can be read
        let mut types = Vec::new();
        let mut at = 88usize;
        while f.len() >= at + 8 {
            let t = u32::from_le_bytes(f[at..at + 4].try_into().unwrap());
            let l = u32::from_le_bytes(f[at + 4..at + 8].try_into().unwrap()) as usize;
            types.push((t, l));
            at += 8 + ((l + 3) & !3);
        }
        println!("IWMFW {}: records (type, length): {:?}", p.file_name().unwrap().to_string_lossy(), types);
        let fw = match iwm_fw::parse(&f) {
            Ok(fw) => fw,
            Err(w) => {
                println!("IWMFW {}: REFUSED: {:?}", p.file_name().unwrap().to_string_lossy(), w);
                refused += 1;
                continue;
            }
        };
        let img = |i| fw.image(i);
        println!(
            "IWMFW {}: {} bytes, name '{}', ver {:#010x} build {}, {} records; regular {} sections ({} bytes, paging {}), init {}, wowlan {}; phy_sku {:?}, cpus {:?}, scan channels {}, fw_version {:?}, api {:08x?}, capa {:08x?}",
            p.file_name().unwrap().to_string_lossy(),
            f.len(),
            String::from_utf8_lossy(fw.name()),
            fw.ver,
            fw.build,
            fw.records,
            img(iwm_fw::Image::Regular).n,
            img(iwm_fw::Image::Regular).iter().map(|s| s.len).sum::<usize>(),
            img(iwm_fw::Image::Regular).paging,
            img(iwm_fw::Image::Init).n,
            img(iwm_fw::Image::Wow).n,
            fw.phy_sku,
            fw.num_cpu,
            fw.n_scan_channels,
            fw.fw_version,
            fw.api,
            fw.capa
        );
        for (k, i) in [("regular", iwm_fw::Image::Regular), ("init", iwm_fw::Image::Init)] {
            for s in img(i).iter() {
                println!("IWMFW   {k} section at {:#010x}, {} bytes{}", s.devoff, s.len, if s.is_separator() { " (separator)" } else { "" });
            }
        }
        n += 1;
    }
    assert!(n + refused > 0, "no .ucode file in {dir}");
    assert_eq!(refused, 0, "a real Intel file was refused (see the IWMFW lines)");
}
