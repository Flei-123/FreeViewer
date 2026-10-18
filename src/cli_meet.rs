//! Split out of main.rs (r35): mechanical move, behaviour unchanged.
use super::*;

// DRESS REHEARSAL (before the release): the client does EVERYTHING at once -
// audio, camera, screen, remote-control release - through
// EXACTLY the class the window also uses (NativMeet). This measures
// what the app really does, not a side path next to it.
//   freeviewer --meetalles <raum> <passphrase> [name] [sekunden]
pub(crate) fn cli_meetalles(i: usize) -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    // Optional end-to-end key as the last argument - this
    // lets the native client be cross-checked against the browser.
    let e2e_arg = args
        .iter()
        .skip(i + 1)
        .find(|a| a.len() == 43 && crate::meete2e::Key::aus_text(a).is_some())
        .cloned();
    let room = args.get(i + 1).cloned().unwrap_or_default();
    let pass = args.get(i + 2).cloned().unwrap_or_default();
    let name = args
        .get(i + 3)
        .cloned()
        .unwrap_or_else(|| "NativAlles".to_string());
    let dur: u64 = args.get(i + 4).and_then(|s| s.parse().ok()).unwrap_or(35);
    let fvid = args
        .get(i + 5)
        .cloned()
        .unwrap_or_else(|| "497628420".to_string());
    let mut m = match meetui::NativeMeet::join_meeting(
        &meet::base(),
        &room,
        &pass,
        &name,
        &fvid,
        false,
        None,
        None,
        e2e_arg.clone(),
    ) {
        Ok(m) => m,
        Err(e) => {
            println!("ALLES FEHLER Beitritt: {}", e);
            return Ok(());
        }
    };
    println!("ALLES E2E {}", m.e2e_an);
    println!("ALLES START {}", m.notice);
    let start = std::time::Instant::now();
    let (mut camera_on, mut screen_on, mut free) = (false, false, false);
    while start.elapsed().as_secs() < dur {
        m.pumpe();
        // Switch on one after another so that it is visible in the browser
        // WHAT is being added.
        let s = start.elapsed().as_secs();
        if !camera_on && s >= 3 {
            camera_on = true;
            m.camera_toggle(true);
            println!("ALLES KAMERA {} ({})", m.camera_on, m.camera_notice);
        }
        if !screen_on && s >= 8 {
            screen_on = true;
            m.toggle_screen(true, 0);
            println!("ALLES SCHIRM {} ({})", m.screen_on, m.screen_notice);
        }
        if !free && s >= 12 {
            free = true;
            m.release_control(true);
            println!("ALLES STEUERUNG {}", m.control_free);
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let z = m.state();
    let numbers = m.numbers();
    println!(
        "ALLES ERGEBNIS leute={} ton_gesendet={} ton_empfangen={} kamera_gesendet={} schirm_gesendet={} bild_empfangen={} codec={}",
        z.people.len(),
        numbers.sent,
        numbers.receive,
        m.image_sent,
        m.screen_sent,
        m.image_counter,
        m.image_codec
    );
    println!(
        "ALLES ZUSTAND kamera={} schirm={} steuerung={} meldung={} kamera_meldung={} schirm_meldung={}",
        m.camera_on, m.screen_on, m.control_free, m.notice, m.camera_notice, m.screen_notice
    );
    m.leave();
    std::thread::sleep(std::time::Duration::from_millis(600));
    println!("ALLES FERTIG");
    return Ok(());
}

// Control test (stage 4): does the native client release remote control,
// and does it notice when someone else releases it? The browser gets
// its "Control" button from this - and we get the counter-check here.
//   freeviewer --meetsteuer <raum> <passphrase> [name] [sekunden] [fvid]
pub(crate) fn cli_meet_control(i: usize) -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let room = args.get(i + 1).cloned().unwrap_or_default();
    let pass = args.get(i + 2).cloned().unwrap_or_default();
    let name = args
        .get(i + 3)
        .cloned()
        .unwrap_or_else(|| "NativSteuer".to_string());
    let dur: u64 = args.get(i + 4).and_then(|s| s.parse().ok()).unwrap_or(25);
    let fvid = args
        .get(i + 5)
        .cloned()
        .unwrap_or_else(|| "497628420".to_string());
    // Deliberately join WITHOUT a number: this measures that the
    // button only appears for the other side through the release.
    let sig = match meetsig::join_meeting(&meet::base(), &room, &pass, &name, "", false) {
        Ok(s) => s,
        Err(e) => {
            println!("STEUERTEST FEHLER Signalisierung: {}", e);
            return Ok(());
        }
    };
    let start = std::time::Instant::now();
    let mut released = false;
    let mut go_back = false;
    let mut offers = 0u32;
    while start.elapsed().as_secs() < dur {
        for e in sig.poll_events() {
            match &e {
                meetsig::Event::RemoteControl { peer, fvid } => {
                    offers += 1;
                    println!(
                        "STEUERTEST ANGEBOT peer={} fvid={}",
                        peer,
                        if fvid.is_empty() { "-" } else { fvid.as_str() }
                    );
                }
                // Admit immediately in the test - otherwise the browser stands
                // at the door and nothing can be measured.
                meetsig::Event::WaitAlso { peer, .. } => {
                    sig.waiting_room("admit", Some(*peer))
                }
                meetsig::Event::Also(t) => println!("STEUERTEST DAZU {}", t.name),
                _ => {}
            }
        }
        if !released && sig.state().ich != 0 && start.elapsed().as_secs() >= 3 {
            released = true;
            sig.remote_control(true, &fvid);
            println!("STEUERTEST FREIGEGEBEN {}", fvid);
        }
        // Take it back in the middle of the run: only this shows
        // that the button also DISAPPEARS again for the other side - had
        // we left for that, it would be no proof, just the
        // usual cleanup.
        if released
            && !go_back
            && start.elapsed().as_secs() >= dur.saturating_sub(8).max(9)
        {
            go_back = true;
            sig.remote_control(false, "");
            println!("STEUERTEST ZURUECKGENOMMEN");
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    let z = sig.state();
    for t in z.people.iter() {
        if !t.fvid.is_empty() {
            println!("STEUERTEST STEUERBAR {} {}", t.name, t.fvid);
        }
    }
    println!(
        "STEUERTEST ZUSTAND ich={} leute={} angebote={}",
        z.ich,
        z.people.len(),
        offers
    );
    if !go_back {
        sig.remote_control(false, "");
        std::thread::sleep(std::time::Duration::from_millis(800));
    }
    sig.leave();
    std::thread::sleep(std::time::Duration::from_millis(500));
    println!("STEUERTEST FERTIG");
    return Ok(());
}

// Receive test (stage 3b): what ARRIVES at the native client - and is
// the camera cleanly separated from the screen?
//   freeviewer --meetempfang <raum> <passphrase> [name] [sekunden]
pub(crate) fn cli_meet_receive(i: usize) -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let room = args.get(i + 1).cloned().unwrap_or_default();
    let pass = args.get(i + 2).cloned().unwrap_or_default();
    let name = args
        .get(i + 3)
        .cloned()
        .unwrap_or_else(|| "NativAuge".to_string());
    let dur: u64 = args.get(i + 4).and_then(|s| s.parse().ok()).unwrap_or(30);
    let sig = match meetsig::join_meeting(&meet::base(), &room, &pass, &name, "", false) {
        Ok(s) => s,
        Err(e) => {
            println!("EMPFANGSTEST FEHLER Signalisierung: {}", e);
            return Ok(());
        }
    };
    let ton = match meetrtc::launch() {
        Ok(t) => t,
        Err(e) => {
            println!("EMPFANGSTEST FEHLER RTC: {}", e);
            return Ok(());
        }
    };
    let mut cameras = meetvideo::VideoDecoder::neu();
    let mut screens = meetvideo::VideoDecoder::neu();
    let mut offered = false;
    let (mut came_frame, mut screen_frame) = (0u64, 0u64);
    let start = std::time::Instant::now();
    while start.elapsed().as_secs() < dur {
        for e in sig.poll_events() {
            match &e {
                meetsig::Event::Welcome { .. } => {
                    if !offered {
                        offered = true;
                        sig.roh(serde_json::json!({"t":"offer","sdp":ton.offer}));
                    }
                }
                meetsig::Event::WaitAlso { peer, .. } => sig.waiting_room("admit", Some(*peer)),
                meetsig::Event::Spur {
                    mid,
                    peer,
                    display,
                    art,
                } => {
                    ton.spur_art(mid, *peer, *display);
                    println!("SPUR mid={} peer={} art={} bildschirm={}", mid, peer, art, display);
                }
                meetsig::Event::Sdp { art, sdp } if art == "answer" => {
                    ton.reply(sdp);
                    sig.roh(serde_json::json!({"t":"publish","mid":ton.mid,"screen":false}));
                    sig.roh(serde_json::json!({"t":"publish","mid":ton.vid,"screen":false}));
                    println!("EREIGNIS Antwort eingespielt");
                }
                meetsig::Event::Sdp { art, sdp } if art == "offer" => ton.server_offer(sdp),
                meetsig::Event::Separated(m) => println!("EREIGNIS Getrennt: {}", m),
                _ => {}
            }
        }
        if let Some(a) = ton.open_reply() {
            sig.roh(serde_json::json!({"t":"answer","sdp":a}));
        }
        for te in ton.poll_events() {
            if let meetrtc::ToneEvent::Image {
                origin,
                display,
                daten,
                ..
            } = te
            {
                if display {
                    screen_frame += 1;
                    screens.frame_box(origin, &daten);
                } else {
                    came_frame += 1;
                    cameras.frame_box(origin, &daten);
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let masse = |d: &meetvideo::VideoDecoder| -> String {
        d.images
            .iter()
            .map(|(peer, (w, h, _))| {
                format!("peer{}={}x{} hell={:.2}", peer, w, h, d.brightness(*peer))
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    println!(
        "ZAHLEN kamera_rahmen={} kamera_bilder={} [{}] schirm_rahmen={} schirm_bilder={} [{}] fehler={}",
        came_frame,
        cameras.counted,
        masse(&cameras),
        screen_frame,
        screens.counted,
        masse(&screens),
        if screens.last_error.is_empty() { cameras.last_error.clone() } else { screens.last_error.clone() }
    );
    ton.finish();
    sig.leave();
    std::thread::sleep(std::time::Duration::from_millis(300));
    println!("EMPFANGSTEST FERTIG");
    return Ok(());
}

// Screen test (stage 3): send the REAL screen as its own track into the
// meeting - without a browser.
//   freeviewer --meetschirm <raum> <passphrase> [name] [sekunden] [screen-nr]
pub(crate) fn cli_meet_screen(i: usize) -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let room = args.get(i + 1).cloned().unwrap_or_default();
    let pass = args.get(i + 2).cloned().unwrap_or_default();
    let name = args
        .get(i + 3)
        .cloned()
        .unwrap_or_else(|| "NativSchirm".to_string());
    let dur: u64 = args.get(i + 4).and_then(|s| s.parse().ok()).unwrap_or(25);
    let nr: usize = args.get(i + 5).and_then(|s| s.parse().ok()).unwrap_or(0);
    for (n, m) in meetschirm::liste().iter().enumerate() {
        println!(
            "SCHIRM {} name={} {}x{} primaer={}",
            n, m.name, m.width_px, m.height_px, m.is_primary
        );
    }
    let auf = match meetschirm::open(nr, 1920, 1080, 15) {
        Ok(a) => a,
        Err(e) => {
            println!("SCHIRMTEST FEHLER Aufnahme: {}", e);
            return Ok(());
        }
    };
    println!("SCHIRM OFFEN {} {}x{}", auf.name, auf.width_px, auf.height_px);
    let mut koder = match meetvideo::VideoEncoder::neu(auf.width_px, auf.height_px, 15, 3_500_000) {
        Ok(k) => k,
        Err(e) => {
            println!("SCHIRMTEST FEHLER Kodierer: {}", e);
            return Ok(());
        }
    };
    let sig = match meetsig::join_meeting(&meet::base(), &room, &pass, &name, "", false) {
        Ok(s) => s,
        Err(e) => {
            println!("SCHIRMTEST FEHLER Signalisierung: {}", e);
            return Ok(());
        }
    };
    let ton = match meetrtc::launch() {
        Ok(t) => t,
        Err(e) => {
            println!("SCHIRMTEST FEHLER RTC: {}", e);
            return Ok(());
        }
    };
    let mut offered = false;
    let mut packets: u64 = 0;
    let mut sent: u64 = 0;
    let mut bright_sum = 0f64;
    let mut motion = 0f64;
    let mut model: Vec<u8> = Vec::new();
    let start = std::time::Instant::now();
    let mut next = std::time::Instant::now();
    let mut last_keyframe = std::time::Instant::now();
    while start.elapsed().as_secs() < dur {
        for e in sig.poll_events() {
            match &e {
                meetsig::Event::Welcome { .. } => {
                    if !offered {
                        offered = true;
                        sig.roh(serde_json::json!({"t":"offer","sdp":ton.offer}));
                    }
                }
                meetsig::Event::WaitAlso { peer, .. } => sig.waiting_room("admit", Some(*peer)),
                meetsig::Event::Spur { mid, peer, .. } => ton.spur(mid, *peer),
                meetsig::Event::Sdp { art, sdp } if art == "answer" => {
                    ton.reply(sdp);
                    sig.roh(serde_json::json!({"t":"publish","mid":ton.mid,"screen":false}));
                    sig.roh(serde_json::json!({"t":"publish","mid":ton.vid,"screen":false}));
                    sig.roh(serde_json::json!({"t":"publish","mid":ton.vid2,"screen":true}));
                    sig.roh(serde_json::json!({"t":"screen","on":true}));
                    println!("EREIGNIS Antwort eingespielt, Bildschirm angemeldet");
                }
                meetsig::Event::Sdp { art, sdp } if art == "offer" => ton.server_offer(sdp),
                // Do NOT swallow errors - otherwise at the end you only see
                // "nothing sent" and guess what the cause was.
                meetsig::Event::Separated(m) => println!("EREIGNIS Getrennt: {}", m),
                meetsig::Event::FriendError { code, text } => {
                    println!("EREIGNIS Fehler {}: {}", code, text)
                }
                meetsig::Event::Refused(m) => println!("EREIGNIS Abgewiesen: {}", m),
                _ => {}
            }
        }
        if let Some(a) = ton.open_reply() {
            sig.roh(serde_json::json!({"t":"answer","sdp":a}));
        }
        for te in ton.poll_events() {
            match te {
                meetrtc::ToneEvent::PeerConnected => println!("EREIGNIS RTC verbunden"),
                meetrtc::ToneEvent::FriendError(f) => println!("EREIGNIS RTC-Fehler: {}", f),
                _ => {}
            }
        }
        if std::time::Instant::now() >= next {
            next += std::time::Duration::from_millis(66);
            if let Some(b) = auf.newest() {
                let y = &b.nv12[..(b.width_px * b.height_px) as usize];
                bright_sum += y.iter().map(|v| *v as f64).sum::<f64>() / y.len() as f64;
                if model.len() == y.len() {
                    let d: f64 = y
                        .iter()
                        .zip(model.iter())
                        .map(|(a, c)| (*a as i32 - *c as i32).unsigned_abs() as f64)
                        .sum::<f64>()
                        / y.len() as f64;
                    motion += d;
                }
                model = y.to_vec();
                if last_keyframe.elapsed() >= std::time::Duration::from_secs(4) {
                    last_keyframe = std::time::Instant::now();
                    koder.keyframe();
                }
                sent += 1;
                match koder.nv12_frame(&b.nv12) {
                    Ok(parts) => {
                        for t in parts {
                            packets += 1;
                            ton.screen_send(t.data);
                        }
                    }
                    Err(e) => println!("KODIER-FEHLER {}", e),
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let z = ton.numbers();
    println!(
        "ZAHLEN schirm={} groesse={}x{} aufgenommen={} verwendet={} kodiert={} pakete={} schirm_gesendet={} bild_empfangen={} bytes_raus={} helligkeit={:.1} bewegung={:.2} fehler={}",
        auf.name,
        auf.width_px,
        auf.height_px,
        auf.recorded(),
        sent,
        koder.images,
        packets,
        z.screen_sent,
        z.frame_received,
        z.bytes_out,
        if sent > 0 { bright_sum / sent as f64 } else { 0.0 },
        if sent > 1 { motion / (sent - 1) as f64 } else { 0.0 },
        auf.error()
    );
    auf.stop_capture();
    sig.roh(serde_json::json!({"t":"screen","on":false}));
    std::thread::sleep(std::time::Duration::from_millis(200));
    ton.finish();
    sig.leave();
    std::thread::sleep(std::time::Duration::from_millis(400));
    println!("SCHIRMTEST FERTIG");
    return Ok(());
}

// Kameratest (Stufe 2d): ECHTES Kamerabild als H.264 ins Meeting.
//   freeviewer --meetkamera <raum> <passphrase> [name] [sekunden] [kamera]
pub(crate) fn cli_meet_camera(i: usize) -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let room = args.get(i + 1).cloned().unwrap_or_default();
    let pass = args.get(i + 2).cloned().unwrap_or_default();
    let name = args
        .get(i + 3)
        .cloned()
        .unwrap_or_else(|| "NativKamera".to_string());
    let dur: u64 = args.get(i + 4).and_then(|s| s.parse().ok()).unwrap_or(25);
    let welche = args.get(i + 5).cloned().filter(|s| !s.is_empty());
    for (n, k) in meetcam::liste().iter().enumerate() {
        println!("KAMERA {} name={}", n, k.name);
    }
    let kam = match meetcam::open(welche, meetvideo::WIDTH, meetvideo::HEIGHT, 30) {
        Ok(k) => k,
        Err(e) => {
            println!("KAMERATEST FEHLER Kamera: {}", e);
            return Ok(());
        }
    };
    println!("KAMERA OFFEN name={} {}x{}", kam.name, kam.width_px, kam.height_px);
    let mut koder = match meetvideo::VideoEncoder::neu(kam.width_px, kam.height_px, 15, 1_500_000) {
        Ok(k) => k,
        Err(e) => {
            println!("KAMERATEST FEHLER Kodierer: {}", e);
            return Ok(());
        }
    };
    let sig = match meetsig::join_meeting(&meet::base(), &room, &pass, &name, "", false) {
        Ok(s) => s,
        Err(e) => {
            println!("KAMERATEST FEHLER Signalisierung: {}", e);
            return Ok(());
        }
    };
    let ton = match meetrtc::launch() {
        Ok(t) => t,
        Err(e) => {
            println!("KAMERATEST FEHLER Ton: {}", e);
            return Ok(());
        }
    };
    let mut offered = false;
    let mut packets: u64 = 0;
    let mut sent: u64 = 0;
    // Two measurements so that "a picture arrived" is provable: mean
    // brightness (not black) and difference to the previous image (moving).
    let mut bright_sum = 0f64;
    let mut motion = 0f64;
    let mut model: Vec<u8> = Vec::new();
    let start = std::time::Instant::now();
    let mut next = std::time::Instant::now();
    while start.elapsed().as_secs() < dur {
        for e in sig.poll_events() {
            match &e {
                meetsig::Event::Welcome { .. } => {
                    if !offered {
                        offered = true;
                        sig.roh(serde_json::json!({"t":"offer","sdp":ton.offer}));
                    }
                }
                meetsig::Event::WaitAlso { peer, .. } => sig.waiting_room("admit", Some(*peer)),
                meetsig::Event::Spur { mid, peer, .. } => ton.spur(mid, *peer),
                meetsig::Event::Sdp { art, sdp } if art == "answer" => {
                    ton.reply(sdp);
                    sig.roh(serde_json::json!({"t":"publish","mid":ton.mid,"screen":false}));
                    sig.roh(serde_json::json!({"t":"publish","mid":ton.vid,"screen":false}));
                    println!("EREIGNIS Antwort eingespielt");
                }
                meetsig::Event::Sdp { art, sdp } if art == "offer" => ton.server_offer(sdp),
                _ => {}
            }
        }
        if let Some(a) = ton.open_reply() {
            sig.roh(serde_json::json!({"t":"answer","sdp":a}));
        }
        for te in ton.poll_events() {
            if let meetrtc::ToneEvent::PeerConnected = te {
                println!("EREIGNIS Ton verbunden");
            }
        }
        // Send 15 frames per second - the newest one the camera has.
        if std::time::Instant::now() >= next {
            next += std::time::Duration::from_millis(66);
            if let Some(b) = kam.newest() {
                let y = &b.nv12[..(b.width_px * b.height_px) as usize];
                bright_sum += y.iter().map(|v| *v as f64).sum::<f64>() / y.len() as f64;
                if model.len() == y.len() {
                    let d: f64 = y
                        .iter()
                        .zip(model.iter())
                        .map(|(a, c)| (*a as i32 - *c as i32).unsigned_abs() as f64)
                        .sum::<f64>()
                        / y.len() as f64;
                    motion += d;
                }
                model = y.to_vec();
                sent += 1;
                match koder.nv12_frame(&b.nv12) {
                    Ok(parts) => {
                        for t in parts {
                            packets += 1;
                            ton.send_frame(t.data);
                        }
                    }
                    Err(e) => println!("KODIER-FEHLER {}", e),
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let z = ton.numbers();
    println!(
        "ZAHLEN kamera={} aufgenommen={} verwendet={} kodiert={} pakete={} bild_gesendet={} bild_empfangen={} bytes_raus={} helligkeit={:.1} bewegung={:.2} fehler={}",
        kam.name,
        kam.recorded(),
        sent,
        koder.images,
        packets,
        z.image_sent,
        z.frame_received,
        z.bytes_out,
        if sent > 0 { bright_sum / sent as f64 } else { 0.0 },
        if sent > 1 { motion / (sent - 1) as f64 } else { 0.0 },
        kam.error()
    );
    kam.stop_capture();
    ton.finish();
    sig.leave();
    std::thread::sleep(std::time::Duration::from_millis(400));
    println!("KAMERATEST FERTIG");
    return Ok(());
}

// Video test (stage 2b): sends a moving test pattern as H.264 into the
// meeting. This measures the whole chain without a camera.
//   freeviewer --meetbild <raum> <passphrase> [name] [sekunden]
pub(crate) fn cli_meet_image(i: usize) -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let room = args.get(i + 1).cloned().unwrap_or_default();
    let pass = args.get(i + 2).cloned().unwrap_or_default();
    let name = args.get(i + 3).cloned().unwrap_or_else(|| "NativBild".to_string());
    let dur: u64 = args.get(i + 4).and_then(|s| s.parse().ok()).unwrap_or(25);
    let mut koder = match meetvideo::VideoEncoder::neu(
        meetvideo::WIDTH,
        meetvideo::HEIGHT,
        15,
        1_500_000,
    ) {
        Ok(k) => k,
        Err(e) => {
            println!("BILDTEST FEHLER Kodierer: {}", e);
            return Ok(());
        }
    };
    let mut muster = meetvideo::Muster::neu(meetvideo::WIDTH, meetvideo::HEIGHT);
    let sig = match meetsig::join_meeting(&meet::base(), &room, &pass, &name, "", false) {
        Ok(s) => s,
        Err(e) => {
            println!("BILDTEST FEHLER Signalisierung: {}", e);
            return Ok(());
        }
    };
    let ton = match meetrtc::launch() {
        Ok(t) => t,
        Err(e) => {
            println!("BILDTEST FEHLER Ton: {}", e);
            return Ok(());
        }
    };
    let mut offered = false;
    let mut packets: u64 = 0;
    let start = std::time::Instant::now();
    let mut next = std::time::Instant::now();
    while start.elapsed().as_secs() < dur {
        for e in sig.poll_events() {
            match &e {
                meetsig::Event::Welcome { .. } => {
                    if !offered {
                        offered = true;
                        sig.roh(serde_json::json!({"t":"offer","sdp":ton.offer}));
                    }
                }
                meetsig::Event::WaitAlso { peer, .. } => sig.waiting_room("admit", Some(*peer)),
                meetsig::Event::Spur { mid, peer, .. } => ton.spur(mid, *peer),
                meetsig::Event::Sdp { art, sdp } if art == "answer" => {
                    ton.reply(sdp);
                    sig.roh(serde_json::json!({"t":"publish","mid":ton.mid,"screen":false}));
                    sig.roh(serde_json::json!({"t":"publish","mid":ton.vid,"screen":false}));
                    println!("EREIGNIS Antwort eingespielt");
                }
                meetsig::Event::Sdp { art, sdp } if art == "offer" => ton.server_offer(sdp),
                _ => println!("EREIGNIS {:?}", e),
            }
        }
        if let Some(a) = ton.open_reply() {
            sig.roh(serde_json::json!({"t":"answer","sdp":a}));
        }
        for te in ton.poll_events() {
            if let meetrtc::ToneEvent::PeerConnected = te {
                println!("EREIGNIS Ton verbunden");
            }
        }
        // 15 Bilder je Sekunde
        while std::time::Instant::now() >= next {
            next += std::time::Duration::from_millis(66);
            let rgb = muster.next();
            match koder.frame_box(&rgb) {
                Ok(parts) => {
                    for t in parts {
                        packets += 1;
                        ton.send_frame(t.data);
                    }
                }
                Err(e) => println!("KODIER-FEHLER {}", e),
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let z = ton.numbers();
    println!(
        "ZAHLEN verbunden={} bilder_kodiert={} pakete={} bild_gesendet={} bild_empfangen={} bytes_raus={}",
        z.connected, koder.images, packets, z.image_sent, z.frame_received, z.bytes_out
    );
    ton.finish();
    sig.leave();
    std::thread::sleep(std::time::Duration::from_millis(400));
    println!("BILDTEST FERTIG");
    return Ok(());
}

// Practical test with REAL devices (stage 1c):
//   freeviewer --meetmik <raum> <passphrase> [name] [sekunden]
// Takes the default microphone, plays the others through the speaker
// and switches the echo suppression in between.
pub(crate) fn cli_meet_mic(i: usize) -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let room = args.get(i + 1).cloned().unwrap_or_default();
    let pass = args.get(i + 2).cloned().unwrap_or_default();
    let name = args.get(i + 3).cloned().unwrap_or_else(|| "NativMikro".to_string());
    let dur: u64 = args.get(i + 4).and_then(|s| s.parse().ok()).unwrap_or(30);
    let (ein_liste, aus_liste) = meetaudio::devices_list();
    println!("MIKROFONE {:?}", ein_liste);
    println!("LAUTSPRECHER {:?}", aus_liste);
    let devices = match meetaudio::start_devices(None, None) {
        Ok(g) => g,
        Err(e) => {
            println!("MIKROTEST FEHLER Geraete: {}", e);
            return Ok(());
        }
    };
    println!("EINGANG {} / AUSGANG {}", devices.eingang, devices.exit);
    let sig = match meetsig::join_meeting(&meet::base(), &room, &pass, &name, "", false) {
        Ok(s) => s,
        Err(e) => {
            println!("MIKROTEST FEHLER Signalisierung: {}", e);
            return Ok(());
        }
    };
    let ton = match meetrtc::launch() {
        Ok(t) => t,
        Err(e) => {
            println!("MIKROTEST FEHLER Ton: {}", e);
            return Ok(());
        }
    };
    let mut offered = false;
    let start = std::time::Instant::now();
    let mut mic_frame: u64 = 0;
    let mut mic_level = 0.0f32;
    while start.elapsed().as_secs() < dur {
        for e in sig.poll_events() {
            match &e {
                meetsig::Event::Welcome { .. } => {
                    if !offered {
                        offered = true;
                        sig.roh(serde_json::json!({"t":"offer","sdp":ton.offer}));
                    }
                }
                meetsig::Event::WaitAlso { peer, .. } => sig.waiting_room("admit", Some(*peer)),
                meetsig::Event::Spur { mid, peer, .. } => ton.spur(mid, *peer),
                meetsig::Event::Sdp { art, sdp } if art == "answer" => {
                    ton.reply(sdp);
                    sig.roh(serde_json::json!({"t":"publish","mid":ton.mid,"screen":false}));
                }
                meetsig::Event::Sdp { art, sdp } if art == "offer" => ton.server_offer(sdp),
                _ => println!("EREIGNIS {:?}", e),
            }
        }
        if let Some(a) = ton.open_reply() {
            sig.roh(serde_json::json!({"t":"answer","sdp":a}));
        }
        // Mikrofon -> Netz
        while let Ok(frame_box) = devices.mic_name.try_recv() {
            let spitze = frame_box
                .iter()
                .map(|v| (*v as f32 / 32768.0).abs())
                .fold(0.0f32, f32::max);
            mic_level = mic_level.max(spitze);
            mic_frame += 1;
            ton.send_out(frame_box);
        }
        // Netz -> Lautsprecher
        for te in ton.poll_events() {
            match te {
                meetrtc::ToneEvent::FrameBox { origin, pcm } => {
                    if let Ok(mut m) = devices.speaker.lock() {
                        m.also(origin, &pcm);
                    }
                }
                meetrtc::ToneEvent::Image { .. } => {}
                meetrtc::ToneEvent::PeerConnected => println!("EREIGNIS Ton verbunden"),
                meetrtc::ToneEvent::FriendError(f) => println!("EREIGNIS Ton-Fehler {}", f),
                meetrtc::ToneEvent::End(f) => println!("EREIGNIS Ton-Ende {}", f),
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let z = ton.numbers();
    println!(
        "ZAHLEN verbunden={} gesendet={} empfangen={} mikro_rahmen={} mikro_pegel={:.3} pegel_rein={:.3}",
        z.connected, z.sent, z.receive, mic_frame, mic_level, z.level_pure
    );
    ton.finish();
    sig.leave();
    std::thread::sleep(std::time::Duration::from_millis(400));
    println!("MIKROTEST FERTIG");
    return Ok(());
}

// Native AUDIO test (stage 1b): join, build a real WebRTC connection to the
// media server and send a test tone or measure the audio of the
// others.
//   freeviewer --meetton <raum> <passphrase> [name] [sekunden]
pub(crate) fn cli_meet_tone(i: usize) -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let room = args.get(i + 1).cloned().unwrap_or_default();
    let pass = args.get(i + 2).cloned().unwrap_or_default();
    let name = args.get(i + 3).cloned().unwrap_or_else(|| "NativTon".to_string());
    let dur: u64 = args.get(i + 4).and_then(|s| s.parse().ok()).unwrap_or(20);
    let sig = match meetsig::join_meeting(&meet::base(), &room, &pass, &name, "", false) {
        Ok(s) => s,
        Err(e) => {
            println!("TONTEST FEHLER Signalisierung: {}", e);
            return Ok(());
        }
    };
    let ton = match meetrtc::launch() {
        Ok(t) => t,
        Err(e) => {
            println!("TONTEST FEHLER Ton: {}", e);
            return Ok(());
        }
    };
    println!("ANGEBOT {} Zeichen, mid {}", ton.offer.len(), ton.mid);
    let mut offered = false;
    let mut phase = 0.0f32;
    let start = std::time::Instant::now();
    let mut next = std::time::Instant::now();
    let mut level_max = 0.0f32;
    let burst: usize = std::env::var("FV_MEET_BURST")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let mut geschwallt = false;
    let mut backlog_max: u32 = 0;
    while start.elapsed().as_secs() < dur {
        for e in sig.poll_events() {
            match &e {
                meetsig::Event::Welcome { .. } => {
                    if !offered {
                        offered = true;
                        sig.roh(serde_json::json!({"t":"offer","sdp":ton.offer}));
                        println!("EREIGNIS Angebot verschickt");
                    }
                }
                meetsig::Event::WaitAlso { peer, .. } => sig.waiting_room("admit", Some(*peer)),
                meetsig::Event::Sdp { art, sdp } if art == "answer" => {
                    ton.reply(sdp);
                    sig.roh(serde_json::json!({"t":"publish","mid":ton.mid,"screen":false}));
                    println!("EREIGNIS Antwort eingespielt ({} Zeichen)", sdp.len());
                }
                meetsig::Event::Spur { mid, peer, .. } => {
                    ton.spur(mid, *peer);
                    println!("EREIGNIS Spur {} gehoert zu {}", mid, peer);
                }
                meetsig::Event::Sdp { art, sdp } if art == "offer" => {
                    ton.server_offer(sdp);
                    println!("EREIGNIS Server-Angebot bekommen");
                }
                _ => println!("EREIGNIS {:?}", e),
            }
        }
        if let Some(a) = ton.open_reply() {
            sig.roh(serde_json::json!({"t":"answer","sdp":a}));
            println!("EREIGNIS eigene Antwort verschickt");
        }
        for te in ton.poll_events() {
            match te {
                meetrtc::ToneEvent::PeerConnected => println!("EREIGNIS Ton verbunden"),
                meetrtc::ToneEvent::FriendError(f) => println!("EREIGNIS Ton-Fehler {}", f),
                meetrtc::ToneEvent::End(f) => println!("EREIGNIS Ton-Ende {}", f),
                meetrtc::ToneEvent::FrameBox { .. } => {}
                meetrtc::ToneEvent::Image {
                    origin,
                    display,
                    daten,
                    keyframe,
                    codec,
                } => {
                    println!(
                        "EREIGNIS {} von {} ({} Bytes, {}{})",
                        if display { "Bildschirm" } else { "Bild" },
                        origin,
                        daten.len(),
                        codec,
                        if keyframe { ", Schluesselbild" } else { "" }
                    );
                }
            }
        }
        // Deliberate FLOOD: reproduces a UI hang
        // (dragging the window, opening the camera). Previously the
        // resulting backlog stayed forever as a delay -
        // with FV_MEET_BURST=500 exactly that can be measured.
        if !geschwallt && start.elapsed().as_secs() >= 4 {
            geschwallt = true;
            if burst > 0 {
                for _ in 0..burst {
                    ton.send_out(meetrtc::testton(&mut phase, 440.0));
                }
                println!("SCHWALL {} Rahmen auf einmal eingespeist ({} ms Ton)", burst, burst * 20);
            }
        }
        // every 20 ms one frame of test tone (440 Hz)
        while std::time::Instant::now() >= next {
            next += std::time::Duration::from_millis(20);
            ton.send_out(meetrtc::testton(&mut phase, 440.0));
        }
        let z = ton.numbers();
        level_max = level_max.max(z.level_pure);
        backlog_max = backlog_max.max(z.tone_backlog);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let z = ton.numbers();
    println!(
        "ZAHLEN verbunden={} gesendet={} empfangen={} bild_gesendet={} bild_empfangen={} bytes_raus={} bytes_rein={} pegel_max={:.3} bandbreite={} ton_verworfen={} ton_stau={}",
        z.connected, z.sent, z.receive, z.image_sent, z.frame_received,
        z.bytes_out, z.bytes_rein, level_max, z.bandwidth, z.tone_dropped, z.tone_backlog
    );
    println!(
        "STAU hoechstens {} Rahmen = {} ms Verzoegerung im Sendepuffer",
        backlog_max,
        backlog_max * 20
    );
    ton.finish();
    sig.leave();
    std::thread::sleep(std::time::Duration::from_millis(500));
    println!("TONTEST FERTIG");
    return Ok(());
}

// Native meeting test (stage 1, without UI):
//   freeviewer --meettest <raum> <passphrase> [name] [sekunden]
// Joins, reports every event, sends a chat message and leaves again.
pub(crate) fn cli_meettest(i: usize) -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let room = args.get(i + 1).cloned().unwrap_or_default();
    let pass = args.get(i + 2).cloned().unwrap_or_default();
    let name = args.get(i + 3).cloned().unwrap_or_else(|| "Nativ".to_string());
    let dur: u64 = args.get(i + 4).and_then(|s| s.parse().ok()).unwrap_or(12);
    match meetsig::join_meeting(&meet::base(), &room, &pass, &name, "", false) {
        Ok(s) => {
            let start = std::time::Instant::now();
            let mut chat_sent = false;
            while start.elapsed().as_secs() < dur {
                for e in s.poll_events() {
                    println!("EREIGNIS {:?}", e);
                    // In the test we let waiting people in immediately so that the path
                    // browser -> waiting room -> native host
                    // can really be checked.
                    if let meetsig::Event::WaitAlso { peer, .. } = e {
                        s.waiting_room("admit", Some(peer));
                    }
                    // When someone comes in, we greet them - this way one
                    // can check that chat REALLY reaches the others
                    // (earlier messages are not re-delivered,
                    // that is intended by the server).
                    if let meetsig::Event::Also(_) = e {
                        s.chat("Willkommen im nativen Meeting");
                    }
                }
                if !chat_sent && s.state().ich != 0 {
                    s.chat("Hallo aus dem nativen Client");
                    s.hand(true);
                    chat_sent = true;
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            println!("ZUSTAND {:?}", s.state());
            s.leave();
            std::thread::sleep(std::time::Duration::from_millis(600));
            println!("MEETTEST FERTIG");
        }
        Err(e) => println!("MEETTEST FEHLER {}", e),
    }
    return Ok(());
}
