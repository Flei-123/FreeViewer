//! Split out of main.rs (r35): mechanical move, behaviour unchanged.
use crate::lockutil::LockExt;
use super::*;

// Tausch mit Administrator-Rechten (vom Updater gestartet):
//   freeviewer --apply-update <frisch> <ziel> <pid>
pub(crate) fn cli_apply_update(i: usize) -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let src = args.get(i + 1).cloned().unwrap_or_default();
    let dst = args.get(i + 2).cloned().unwrap_or_default();
    let pid: u32 = args
        .get(i + 3)
        .and_then(|s| s.parse().ok())
        .unwrap_or_default();
    match update::apply_update(
        std::path::Path::new(&src),
        std::path::Path::new(&dst),
        pid,
    ) {
        Ok(()) => println!("Update eingespielt: {}", dst),
        Err(e) => {
            eprintln!("Update fehlgeschlagen: {}", e);
            std::thread::sleep(std::time::Duration::from_secs(4));
        }
    }
    return Ok(());
}

// Install / uninstall (comes from the button in the settings,
// which first gets administrator rights via UAC).
pub(crate) fn cli_install() -> eframe::Result<()> {
    let with_service = std::env::args().any(|a| a == "--with-service");
    match setup::install(with_service) {
        Ok(()) => {
            println!("installiert nach {}", setup::install_dir().display());
            // start the installed version as a normal user
            let _ = std::process::Command::new("explorer.exe")
                .arg(setup::installed_exe().as_os_str())
                .spawn();
            // the source (e.g. Downloads\FreeViewer-Einrichtung.exe) is
            // no longer needed - delete it shortly after exiting
            if let Ok(cur) = std::env::current_exe() {
                if cur != setup::installed_exe() {
                    let _ = std::process::Command::new("cmd")
                        .args([
                            "/c",
                            &format!(
                                "ping -n 6 127.0.0.1 >nul & del /f /q \"{}\"",
                                cur.display()
                            ),
                        ])
                        .spawn();
                }
            }
        }
        Err(e) => {
            eprintln!("Installation fehlgeschlagen: {}", e);
            std::thread::sleep(std::time::Duration::from_secs(5));
        }
    }
    return Ok(());
}

pub(crate) fn cli_uninstall() -> eframe::Result<()> {
    // --all also takes the configuration and identity backup along
    let all = std::env::args().any(|a| a == "--all");
    let r = if all {
        setup::uninstall_all()
    } else {
        setup::uninstall()
    };
    match r {
        Ok(()) => println!("deinstalliert"),
        Err(e) => {
            eprintln!("Deinstallation fehlgeschlagen: {}", e);
            std::thread::sleep(std::time::Duration::from_secs(5));
        }
    }
    return Ok(());
}

// print the version:  freeviewer --version
pub(crate) fn cli_version() -> eframe::Result<()> {
    println!("{} {}", crate::brand::NAME, update::VERSION);
    return Ok(());
}

// Which sound devices would a session use?  freeviewer --audiodev
pub(crate) fn cli_audiodev() -> eframe::Result<()> {
    print!("{}", audio::list_devices());
    return Ok(());
}

// Microphone -> codec -> speaker on this machine:  freeviewer --audioloop 5
pub(crate) fn cli_audioloop(i: usize) -> eframe::Result<()> {
    let secs = std::env::args()
        .nth(i + 1)
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(5);
    println!("{}", audio::loopback_test(secs));
    return Ok(());
}

// Draws every page once without a window:  freeviewer --uitest
// egui can run headless, so a broken layout or a panic in the GUI shows
// up in a build step instead of in front of the user.
// Camera list (stage 2d): which cameras does this machine see?
//   freeviewer --kameraliste
// E2E TEST VECTORS: the same numbers the browser has to compute.
// Without this comparison you only notice in the meeting that browser and
// program see each other as nothing but visual noise.
//   freeviewer --e2evektor
pub(crate) fn cli_e2evektor() -> eframe::Result<()> {
    let roh: [u8; 32] = std::array::from_fn(|i| (i * 7 % 251) as u8);
    let k = meete2e::Key::aus_roh(roh);
    // Fixed sender salt, so the output is reproducible (format v2).
    let salt: meete2e::Salt = [0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8];
    println!("SCHLUESSEL {}", k.als_text());
    println!("SALT {}", hex::encode(salt));
    // H.264 vector: the same frame the browser has to compute.
    {
        let mut f: Vec<u8> = Vec::new();
        f.extend_from_slice(&[0, 0, 0, 1, 0x67, 0x42, 0x00, 0x1f, 0x96, 0x54]);
        f.extend_from_slice(&[0, 0, 0, 1, 0x68, 0xce, 0x3c, 0x80]);
        f.extend_from_slice(&[0, 0, 0, 1, 0x65]);
        f.extend((0..200u32).map(|i| (i * 13 % 256) as u8));
        f.extend_from_slice(&[0, 0, 1, 0x41]);
        f.extend((0..120u32).map(|i| (i * 29 % 256) as u8));
        let g = k.seal_h264_at(&salt, 42, &f);
        println!("H264 42 {} {}", hex::encode(&f), hex::encode(&g));
    }
    for (spur, name, ctr, key, cleartext) in [
        (meete2e::Spur::Ton, "ton", 0u32, false, "Hallo Ton"),
        (meete2e::Spur::Image, "bild", 1, true, "Ein Schluesselbild"),
        (meete2e::Spur::Image, "bild", 4294967295, false, "Zaehler am Anschlag"),
    ] {
        let g = k.seal_audio_at(spur, &salt, ctr, key, cleartext.as_bytes());
        println!(
            "VEKTOR {} {} {} {} {}",
            name,
            ctr,
            key,
            hex::encode(cleartext.as_bytes()),
            hex::encode(&g)
        );
    }
    return Ok(());
}

// SCREEN DIAGNOSTICS: capture an image EXACTLY as it goes into the meeting
// (including crop and painted mouse pointer) and save it as PNG.
// Only that proves whether menus and pointer really make it
// through - in a running meeting you only see it, you don't measure it.
//   freeviewer --schirmshot <file.png> [screen-nr] [wartesekunden] [x y b h]
pub(crate) fn cli_screen_shot(i: usize) -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let file = args
        .get(i + 1)
        .cloned()
        .unwrap_or_else(|| "schirm.png".to_string());
    let nr = args.get(i + 2).and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
    let warte = args.get(i + 3).and_then(|v| v.parse::<u64>().ok()).unwrap_or(3);
    let number = |n: usize, vor: f32| -> f32 {
        args.get(n).and_then(|v| v.parse::<f32>().ok()).unwrap_or(vor)
    };
    let part = (
        number(i + 4, 0.0),
        number(i + 5, 0.0),
        number(i + 6, 1.0),
        number(i + 7, 1.0),
    );
    let auf = match meetschirm::open(nr, 1920, 1080, 15) {
        Ok(a) => a,
        Err(e) => {
            println!("SCHIRMSHOT FEHLER {}", e);
            return Ok(());
        }
    };
    println!("SCHIRMSHOT quelle={} ausschnitt={:?}", auf.name, part);
    auf.region_set(part);
    // Run for a few seconds - that gives time to open a menu,
    // and the capturer has settled.
    let end = std::time::Instant::now() + std::time::Duration::from_secs(warte);
    let mut last = None;
    while std::time::Instant::now() < end {
        if let Some(b) = auf.newest() {
            last = Some(b);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    match last {
        Some(b) => {
            let mut rgba = Vec::new();
            if h264::nv12_to_rgba(&b.nv12, b.width_px, b.height_px, b.width_px as usize, b.height_px, &mut rgba)
            {
                match image::RgbaImage::from_raw(b.width_px, b.height_px, rgba) {
                    Some(img) => match img.save(&file) {
                        Ok(_) => println!("SCHIRMSHOT OK {} ({}x{})", file, b.width_px, b.height_px),
                        Err(e) => println!("SCHIRMSHOT SCHREIBFEHLER {}", e),
                    },
                    None => println!("SCHIRMSHOT PUFFER PASST NICHT"),
                }
            } else {
                println!("SCHIRMSHOT NV12 UNVOLLSTAENDIG");
            }
        }
        None => println!("SCHIRMSHOT KEIN BILD ({})", auf.error()),
    }
    auf.stop_capture();
    return Ok(());
}

// CAMERA DIAGNOSTICS: what does the device offer, which format do
// we really get, what do the raw and final images look like. Only that lets a
// color or crop error be PROVEN instead of guessed.
//   freeviewer --camtest [width_px height_px fps] [dir]
pub(crate) fn cli_camtest(i: usize) -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let number = |n: usize, vor: u32| -> u32 {
        args.get(n).and_then(|v| v.parse::<u32>().ok()).unwrap_or(vor)
    };
    let (w, h, fps) = (number(i + 1, 640), number(i + 2, 360), number(i + 3, 15));
    let dir = args
        .get(i + 4)
        .cloned()
        .unwrap_or_else(|| ".".to_string());
    println!("{}", meetcam::diagnose(None, w, h, fps, &dir));
    return Ok(());
}

pub(crate) fn cli_camera_list() -> eframe::Result<()> {
    let l = meetcam::liste();
    println!("KAMERAS {}", l.len());
    for (i, k) in l.iter().enumerate() {
        println!("KAMERA {} name={} id={}", i, k.name, k.id);
    }
    return Ok(());
}

pub(crate) fn cli_uitest(shared: Arc<Shared>) -> eframe::Result<()> {
    let tmp = std::env::temp_dir().join("fv-uitest");
    let _ = std::fs::create_dir_all(&tmp);
    std::env::set_var("FV_CONFIG", &tmp);
    let ctx = egui::Context::default();
    install_theme(&ctx);
    let mut app = App::new(shared.clone(), false, None);
    app.book.started("123456789", "geheim", true);
    app.book.rename("123456789", "Test-PC");
    app.book.started("987654321", "", false);
    app.selected = Some("123456789".to_string());
    app.partner_id = "123456789".to_string();
    *shared.my_id.lock_ok() = "497628420".to_string();
    *shared.knock.lock_ok() = Some(shared::Knock {
        from: "Pait Laptop".to_string(),
        code: "1234".to_string(),
        at: std::time::Instant::now(),
        unverified: true,
        friend_like: false,
    });
    let mut ok = true;
    for (name, view, stab) in [
        ("Start", View::Start, SettingsTab::General),
        ("Geraete", View::Devices, SettingsTab::General),
        ("Einstellungen", View::Settings, SettingsTab::General),
        ("Konto", View::Settings, SettingsTab::Account),
        ("Meet", View::Meet, SettingsTab::General),
        ("Freunde", View::Friends, SettingsTab::General),
    ] {
        app.view = view;
        app.stab = stab;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(1100.0, 720.0),
            )),
            ..Default::default()
        };
        let out = ctx.run(input, |ctx| {
            app.knock_ui(ctx);
            egui::CentralPanel::default().show(ctx, |ui| app.home_ui(ui));
        });
        let shapes = out.shapes.len();
        println!("{}: {} Formen gezeichnet", name, shapes);
        if shapes < 10 {
            ok = false;
        }
    }
    // The session view needs an image and both bars once:
    // in the window and floating in fullscreen.
    shared.connected.store(true, Ordering::Relaxed);
    *shared.remote_size.lock_ok() = (1920, 1080);
    for (name, full) in [("Sitzung", false), ("Sitzung Vollbild", true)] {
        app.full = full;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(1100.0, 720.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(input.clone(), |ctx| app.session_ui(ctx));
        let out = ctx.run(input, |ctx| app.session_ui(ctx));
        let shapes = out.shapes.len();
        println!("{}: {} Formen gezeichnet", name, shapes);
        if shapes < 10 {
            ok = false;
        }
    }
    shared.connected.store(false, Ordering::Relaxed);

    // ---- The meeting window. Join screen once via the real
    // path (with device query), the running meeting via an assembled
    // `MeetView` - this shows full stage, chat,
    // waiting room and screen share without server and camera.
    app.headless = true;
    app.meet_win_open(meet::Meeting {
        id: "482-913-770".to_string(),
        heading: "Wochenbesprechung".to_string(),
        passphrase: "test1234".to_string(),
        termin_text: String::new(),
        e2e: String::new(),
    });
    let meetschirm = |b: f32, h: f32| egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(b, h),
        )),
        ..Default::default()
    };
    {
        // Draw twice: the first frame creates textures and ids.
        let _ = ctx.run(meetschirm(920.0, 640.0), |ctx| app.meet_win_ui(ctx));
        let out = ctx.run(meetschirm(920.0, 640.0), |ctx| app.meet_win_ui(ctx));
        let shapes = out.shapes.len();
        println!("Meet Beitritt: {} Formen gezeichnet", shapes);
        if shapes < 30 {
            ok = false;
        }
    }
    // Waiting room and join screen also belong in the test.
    {
        let _ = ctx.run(meetschirm(920.0, 640.0), |ctx| {
            meetwindow::waiting_screen_ui(ctx, "Wochenbesprechung", "Gleich gehts los.");
        });
        let out = ctx.run(meetschirm(920.0, 640.0), |ctx| {
            meetwindow::waiting_screen_ui(ctx, "Wochenbesprechung", "Gleich gehts los.");
        });
        println!("Meet Warteschirm: {} Formen gezeichnet", out.shapes.len());
        if out.shapes.len() < 12 {
            ok = false;
        }
    }
    // Render all example states - the same ones --meetdemo
    // later shows as a real image.
    let images = meetwindow::test_images(&ctx);
    for i in 0..meetwindow::EXAMPLES {
        let (name, view_state, mut z) = meetwindow::example(i);
        let (b, h) = if name.starts_with("Info") { (560.0, 430.0) } else { (1100.0, 720.0) };
        // Twice: the first frame creates ids and textures.
        let _ = ctx.run(meetschirm(b, h), |ctx| {
            meetwindow::meeting_ui(ctx, &view_state, &mut z, &images);
        });
        let out = ctx.run(meetschirm(b, h), |ctx| {
            meetwindow::meeting_ui(ctx, &view_state, &mut z, &images);
        });
        let shapes = out.shapes.len();
        println!("Meet {}: {} Formen gezeichnet", name, shapes);
        if shapes < 40 {
            ok = false;
        }
    }
    // The small window (picture-in-picture) has its own buttons - the test
    // has to draw them too, otherwise an error in there is only noticed by
    // the user, because it never appears in the main window.
    let mut pipzahl = std::collections::HashMap::new();
    for (nr, myself) in [(2usize, true), (2, false), (0, true), (0, false)] {
        let (_, view_state, _) = meetwindow::example(nr);
        let mut mich = myself;
        let mal = |ctx: &egui::Context, mich: &mut bool| {
            egui::CentralPanel::default().show(ctx, |ui| {
                meetwindow::pip_content(ui, &view_state, &images, mich);
            });
        };
        let _ = ctx.run(meetschirm(260.0, 330.0), |ctx| mal(ctx, &mut mich));
        let out = ctx.run(meetschirm(260.0, 330.0), |ctx| mal(ctx, &mut mich));
        let n = out.shapes.len();
        println!(
            "Meet Bild-im-Bild (Beispiel {}, eigen {}): {} Formen gezeichnet",
            nr, myself, n
        );
        // Four round buttons and a background - something is missing underneath.
        if n < 12 {
            ok = false;
        }
        pipzahl.insert((nr, myself), n);
    }
    // The own tile MUST make a difference - otherwise the
    // switch would have no effect and nobody would notice.
    for nr in [0usize, 2] {
        let (an, aus) = (pipzahl[&(nr, true)], pipzahl[&(nr, false)]);
        if an <= aus {
            println!("ERROR: own tile missing in picture-in-picture (example {})", nr);
            ok = false;
        }
    }
    app.meet_win.offen = false;

    let _ = std::fs::remove_dir_all(&tmp);
    println!("{}", if ok { "UITEST OK" } else { "UITEST FAIL" });
    return Ok(());
}

// Sync with the account once by hand:  freeviewer --synctest [token]
// Without a token the one in account.json is used. Shows exactly
// what went up and what came back - for troubleshooting without a window.
pub(crate) fn cli_synctest(shared: Arc<Shared>) -> eframe::Result<()> {
    let arg = std::env::args()
        .skip_while(|a| a != "--synctest")
        .nth(1)
        .unwrap_or_default();
    let token = if arg.is_empty() || arg.starts_with("--") {
        account::load().map(|s| s.token).unwrap_or_default()
    } else {
        arg
    };
    if token.is_empty() {
        println!("SYNCTEST: kein Token - erst anmelden");
        return Ok(());
    }
    let book = partners::Book::load();
    let mine = book.to_sync();
    println!("SYNCTEST: {} Geraete gehen hoch", mine.len());
    match account::sync(&shared.relay_url, &token, mine) {
        Ok(r) => {
            println!(
                "SYNCTEST OK: Konto {}, Stand {}, {} Geraete zurueck",
                r.username,
                r.rev,
                r.devices.len()
            );
            for d in r.devices.iter().take(20) {
                println!(
                    "  {} {} {}{}",
                    d.id,
                    if d.name.is_empty() { "-" } else { &d.name },
                    d.at,
                    if d.deleted { " (entfernt)" } else { "" }
                );
            }
        }
        Err(e) => println!("SYNCTEST FEHLER: {}", e),
    }
    return Ok(());
}

// ---- the Windows service -------------------------------------------
// "freeviewer --service" is what the service control manager starts.
pub(crate) fn cli_service() -> eframe::Result<()> {
    // The service installs updates itself - it runs as SYSTEM and
    // needs no administrator prompt for that.
    #[cfg(windows)]
    update::service_watcher();
    if let Err(e) = service::run() {
        service::log(&format!("Dienst konnte nicht starten: {}", e));
    }
    return Ok(());
}

// state of service and autostart:  freeviewer --status
pub(crate) fn cli_status() -> eframe::Result<()> {
    println!("Version:    {}", update::VERSION);
    println!("Config:     {}", ident::config_dir().display());
    println!("Autostart:  {}", match autostart::current() {
        Some(c) => c,
        None => "aus".to_string(),
    });
    println!("Dienst:     installiert={} laeuft={}", service::installed(), service::running());
    match service::published() {
        Some(p) => println!("Agent:      ID {} Passwort {} (vor {} s, Desktop {})",
            p.id, p.password,
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs()).unwrap_or(0).saturating_sub(p.at),
            p.desktop),
        None => println!("Agent:      nichts veroeffentlicht"),
    }
    return Ok(());
}

// switch autostart from a script:  freeviewer --autostart on|off
pub(crate) fn cli_autostart(pos: usize) -> eframe::Result<()> {
    let arg: Vec<String> = std::env::args().collect();
    match arg.get(pos + 1).map(|s| s.as_str()) {
        Some("on") | Some("off") => {
            let on = arg[pos + 1] == "on";
            match autostart::set(on) {
                Ok(()) => println!("Autostart {}", if on { "an" } else { "aus" }),
                Err(e) => println!("FAIL: {}", e),
            }
        }
        _ => {}
    }
    println!("Autostart jetzt: {:?} (zeigt hierher: {})", autostart::current(), autostart::points_here());
    return Ok(());
}

// update check/installation from the command line:  freeviewer --update
pub(crate) fn cli_update() -> eframe::Result<()> {
    update::cleanup();
    println!("lokal: {}", update::VERSION);
    match update::check() {
        Ok(rel) => {
            println!(
                "relay: {} ({} Bytes, {}...)",
                rel.version,
                rel.size,
                &rel.sha256[..12]
            );
            if update::newer(&rel.version, update::VERSION) {
                println!("installiere...");
                match update::download(&rel) {
                    Ok(tmp) => {
                        println!("geladen + Pruefsumme ok: {}", tmp.display());
                        if std::env::args().any(|a| a == "--dry") {
                            let _ = std::fs::remove_file(&tmp);
                            println!("DRY: nicht ersetzt");
                            return Ok(());
                        }
                        match update::swap(&tmp) {
                            Ok(p) => println!("UPDATE OK -> {}", p.display()),
                            Err(e) => println!("FAIL: {}", e),
                        }
                    }
                    Err(e) => println!("FAIL: {}", e),
                }
            } else {
                println!("schon aktuell");
            }
        }
        Err(e) => println!("FAIL: {}", e),
    }
    return Ok(());
}

// which screens can this machine share?   freeviewer --monitors
pub(crate) fn cli_monitors() -> eframe::Result<()> {
    for (i, m) in hostside::monitor_list(true).iter().enumerate() {
        println!(
            "{}: {} {}x{}{}",
            i,
            m.name,
            m.w,
            m.h,
            if m.primary { "  (primaer)" } else { "" }
        );
    }
    return Ok(());
}

// capture self test:  freeviewer --captest   (writes <config>/captest.txt)
pub(crate) fn cli_captest() -> eframe::Result<()> {
    let report = hostside::capture_selftest(20);
    let path = ident::config_dir().join("captest.txt");
    let _ = std::fs::write(&path, &report);
    println!("{}", report);
    return Ok(());
}

// GPU vs CPU scaling:  freeviewer --gputest [rounds]
pub(crate) fn cli_gputest() -> eframe::Result<()> {
    let rounds: u32 = std::env::args()
        .skip_while(|a| a != "--gputest")
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);
    let report = hostside::gpu_selftest(rounds);
    let path = ident::config_dir().join("gputest.txt");
    let _ = std::fs::write(&path, &report);
    println!("{}", report);
    return Ok(());
}

// H.264 codec self test:  freeviewer --h264test [rounds]
pub(crate) fn cli_h264test() -> eframe::Result<()> {
    let rounds: u32 = std::env::args()
        .skip_while(|a| a != "--h264test")
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);
    let report = h264::selftest(rounds);
    let path = ident::config_dir().join("h264test.txt");
    let _ = std::fs::write(&path, &report);
    println!("{}", report);
    return Ok(());
}

// live H.264 vs JPEG on the real screen:  freeviewer --videotest [rounds]
pub(crate) fn cli_videotest() -> eframe::Result<()> {
    let rounds: u32 = std::env::args()
        .skip_while(|a| a != "--videotest")
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(120);
    let report = hostside::video_selftest(rounds);
    let path = ident::config_dir().join("videotest.txt");
    let _ = std::fs::write(&path, &report);
    println!("{}", report);
    return Ok(());
}

// can we punch a hole?  freeviewer --p2ptest
pub(crate) fn cli_p2ptest() -> eframe::Result<()> {
    let report = p2p::selftest();
    let path = ident::config_dir().join("p2ptest.txt");
    let _ = std::fs::write(&path, &report);
    println!("{}", report);
    return Ok(());
}

// delta/full-frame benchmark:  freeviewer --deltatest [rounds]
pub(crate) fn cli_deltatest() -> eframe::Result<()> {
    let rounds: u32 = std::env::args()
        .skip_while(|a| a != "--deltatest")
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);
    let report = hostside::delta_selftest(rounds);
    let path = ident::config_dir().join("deltatest.txt");
    let _ = std::fs::write(&path, &report);
    println!("{}", report);
    return Ok(());
}
