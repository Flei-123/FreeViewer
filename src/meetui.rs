//! Natives Meeting - Stufe 1e: die Oberflaeche dazu.
//!
//! Haelt die drei Bausteine zusammen (Signalisierung, WebRTC-Ton, Geraete)
//! und zeichnet sie in das vorhandene Meeting-Fenster: Teilnehmerliste,
//! Stummschalten, Handzeichen, Chat, Warteraum. Video kommt in Stufe 2 -
//! deshalb steht der Browser-Weg noch daneben und wird erst geloescht,
//! wenn nativ ALLES kann.

use crate::meetaudio;
use crate::meetrtc;
use crate::meetsig;
use anyhow::Result;

pub struct NativMeet {
    sig: meetsig::Sitzung,
    ton: meetrtc::Ton,
    geraete: Option<meetaudio::Geraete>,
    /// Chatverlauf: (Absender, Text) - Absender 0 = System.
    pub chat: Vec<(u64, String)>,
    pub eingabe: String,
    angeboten: bool,
    pub hand: bool,
    pub stumm: bool,
    /// Ausschlag des eigenen Mikrofons (0..1), fuer die Anzeige.
    pub pegel: f32,
    /// Letzte Meldung (Fehler, Hinweise).
    pub meldung: String,
    pub raum: String,
    /// Wie viele Bildrahmen kamen schon an (Stufe 2 zeigt sie spaeter an).
    pub bild_zaehler: u64,
    /// Format der ankommenden Bilder ("H264"/"Vp8") - fuer die Anzeige.
    pub bild_codec: String,
    /// Dekodierte KAMERA-Bilder der anderen (je Teilnehmer das letzte).
    pub bilder: crate::meetvideo::Dekodierer,
    /// Dekodierte BILDSCHIRM-Bilder der anderen. Eigener Topf, sonst wuerde
    /// die Freigabe das Gesicht desselben Teilnehmers ueberschreiben.
    pub schirme: crate::meetvideo::Dekodierer,
    /// Eigene Kamera (Stufe 2d) - laeuft nur, wenn sie eingeschaltet ist.
    kamera: Option<crate::meetcam::Kamera>,
    koder: Option<crate::meetvideo::Kodierer>,
    pub kamera_an: bool,
    /// Welche Geraete gerade benutzt werden (leer = Standard des Systems).
    /// WARUM gemerkt: nur so laesst sich im laufenden Meeting umschalten,
    /// ohne den Raum zu verlassen - genau wie im Browser-Client.
    pub kamera_geraet: Option<String>,
    pub mikro_geraet: Option<String>,
    pub lautsprecher_geraet: Option<String>,
    pub kamera_meldung: String,
    /// Eigenes Bild fuer die Vorschau: (Breite, Hoehe, RGBA).
    pub eigen: Option<(u32, u32, Vec<u8>)>,
    /// Zaehlt hoch, wenn die Vorschau ein neues Bild hat (Textur-Nachladen).
    pub eigen_stand: u64,
    /// Wie viele eigene Bilder schon rausgingen.
    pub bild_gesendet: u64,
    /// Wann darf das naechste Bild raus (15 Bilder je Sekunde).
    naechstes_bild: std::time::Instant,
    /// Videospur ist beim Server angemeldet.
    video_gemeldet: bool,
    /// Bildschirmfreigabe (Stufe 3) - eigene Spur neben der Kamera.
    schirm: Option<crate::meetschirm::Aufnahme>,
    schirm_koder: Option<crate::meetvideo::Kodierer>,
    pub schirm_an: bool,
    pub schirm_meldung: String,
    pub schirm_gesendet: u64,
    /// Das eigene geteilte Bild als RGBA - damit man SELBST sieht, was man
    /// teilt. Der Browser-Client macht genau das ("sonst sieht man selbst
    /// nicht, WAS man gerade teilt"); nativ fehlte es, und ein Klick auf
    /// "Bildschirm" sah deshalb aus, als passiere gar nichts.
    pub eigen_schirm: Option<(u32, u32, Vec<u8>)>,
    pub eigen_schirm_stand: u64,
    naechstes_schirmbild: std::time::Instant,
    /// Wuensche der Zuschauer: wer will welchen Ausschnitt meines
    /// Bildschirms? (Teilnehmer -> Bereich, plus wann zuletzt gehoert)
    wunsch_von: std::collections::HashMap<u64, (crate::meetschirm::Bereich, std::time::Instant)>,
    /// Welchen Ausschnitt SENDE ich gerade?
    pub schirm_bereich: crate::meetschirm::Bereich,
    /// Welchen Ausschnitt senden die ANDEREN gerade (fuer die Zeichnung)?
    pub bereich_von: std::collections::HashMap<u64, crate::meetschirm::Bereich>,
    /// Wer kann ueberhaupt Ausschnitte liefern (neuer Client)? Wer sich
    /// einmal gemeldet hat, kann es - vorher waere "scharf" eine Luege.
    pub scharf_kann: std::collections::HashSet<u64>,
    /// Zuletzt gesendeter Wunsch und wann - Wiederholungen sind nutzlos.
    letzter_wunsch: Option<(u64, crate::meetschirm::Bereich)>,
    naechster_wunsch: std::time::Instant,
    /// Zeigerpositionen der anderen (0..1 auf IHREM geteilten Bildschirm).
    /// Damit kann ein Zuschauer dorthin zoomen, wohin der andere zeigt.
    pub zeiger_von: std::collections::HashMap<u64, (f32, f32)>,
    /// Wann darf der naechste eigene Zeigerstand raus (10-mal je Sekunde).
    naechster_zeiger: std::time::Instant,
    /// Zuletzt gesendeter Stand - unveraenderte Werte gehen nicht raus.
    letzter_zeiger: Option<(f32, f32)>,
    /// Wann zuletzt ein Schluesselbild angefordert wurde.
    letztes_schluesselbild: std::time::Instant,
    /// Eigene FreeViewer-Nummer - die geben wir bei der Freigabe bekannt.
    ich_fvid: String,
    /// Stufe 4: Fernsteuerung fuer die anderen freigegeben?
    pub steuer_frei: bool,
    /// Wen der Server gerade als Sprecher meldet.
    pub sprecher: Option<u64>,
    /// Wie laut jeder andere gerade ist (0..1). Daraus wird die gruene
    /// Umrandung der Kachel - der Server meldet den Sprecher nicht immer.
    pub pegel_von: std::collections::HashMap<u64, f32>,
    /// Wer gerade tippt (fuer "schreibt ..." im Chat).
    pub tippen: std::collections::HashSet<u64>,
    /// Protokoll fuer den Reiter "Info" - was im Raum passiert ist.
    pub protokoll: Vec<String>,
    /// Gemessener Durchsatz in kbit/s (aus den Byte-Zaehlern, je Sekunde).
    pub kbit_raus: u32,
    pub kbit_rein: u32,
    /// Letzte Messung fuer die Bandbreite.
    letzte_messung: std::time::Instant,
    letzte_bytes: (u64, u64),
}

impl NativMeet {
    /// Beitreten. `mikro`/`lautsprecher` leer = Standardgeraet.
    pub fn beitreten(
        basis: &str,
        raum: &str,
        pass: &str,
        name: &str,
        fvid: &str,
        steuerung: bool,
        mikro: Option<String>,
        lautsprecher: Option<String>,
    ) -> Result<NativMeet> {
        // Die eigene Nummer merken wir uns IMMER (sonst liesse sich die
        // Freigabe spaeter nicht mehr einschalten) - bekanntgegeben wird sie
        // aber nur, wenn "Fernsteuerung anbieten" wirklich an ist.
        let sig = meetsig::beitreten(basis, raum, pass, name, if steuerung { fvid } else { "" })?;
        let ton = meetrtc::starten()?;
        // Ohne Soundkarte (Server, Testrechner) laeuft das Meeting trotzdem -
        // man hoert dann nur nichts. Ehrlich melden statt abbrechen.
        let (mikro_merk, lautsprecher_merk) = (mikro.clone(), lautsprecher.clone());
        let (geraete, meldung) = match meetaudio::geraete_starten(mikro, lautsprecher) {
            Ok(g) => {
                let m = format!("Mikrofon: {} / Lautsprecher: {}", g.eingang, g.ausgang);
                (Some(g), m)
            }
            Err(e) => (None, format!("Kein Ton-Geraet: {}", e)),
        };
        Ok(NativMeet {
            sig,
            ton,
            geraete,
            chat: Vec::new(),
            eingabe: String::new(),
            angeboten: false,
            hand: false,
            stumm: false,
            pegel: 0.0,
            meldung,
            raum: raum.to_string(),
            bild_zaehler: 0,
            bild_codec: String::new(),
            bilder: crate::meetvideo::Dekodierer::neu(),
            schirme: crate::meetvideo::Dekodierer::neu(),
            kamera: None,
            koder: None,
            kamera_an: false,
            kamera_geraet: None,
            mikro_geraet: mikro_merk,
            lautsprecher_geraet: lautsprecher_merk,
            kamera_meldung: String::new(),
            eigen: None,
            eigen_stand: 0,
            bild_gesendet: 0,
            naechstes_bild: std::time::Instant::now(),
            video_gemeldet: false,
            schirm: None,
            schirm_koder: None,
            schirm_an: false,
            schirm_meldung: String::new(),
            schirm_gesendet: 0,
            eigen_schirm: None,
            eigen_schirm_stand: 0,
            naechstes_schirmbild: std::time::Instant::now(),
            wunsch_von: std::collections::HashMap::new(),
            schirm_bereich: crate::meetschirm::GANZ,
            bereich_von: std::collections::HashMap::new(),
            scharf_kann: std::collections::HashSet::new(),
            letzter_wunsch: None,
            naechster_wunsch: std::time::Instant::now(),
            zeiger_von: std::collections::HashMap::new(),
            naechster_zeiger: std::time::Instant::now(),
            letzter_zeiger: None,
            letztes_schluesselbild: std::time::Instant::now(),
            ich_fvid: fvid.to_string(),
            steuer_frei: steuerung && fvid.chars().any(|c| c.is_ascii_digit()),
            sprecher: None,
            pegel_von: std::collections::HashMap::new(),
            tippen: std::collections::HashSet::new(),
            protokoll: vec![format!("Raum {} - Beitritt laeuft", raum)],
            kbit_raus: 0,
            kbit_rein: 0,
            letzte_messung: std::time::Instant::now(),
            letzte_bytes: (0, 0),
        })
    }

    pub fn zustand(&self) -> meetsig::Zustand {
        self.sig.zustand()
    }

    pub fn zahlen(&self) -> meetrtc::Zahlen {
        self.ton.zahlen()
    }

    /// Muss in jedem Bild aufgerufen werden: verteilt Nachrichten und Ton.
    pub fn pumpe(&mut self) {
        for e in self.sig.abholen() {
            match e {
                meetsig::Ereignis::Willkommen { .. } => {
                    if !self.angeboten {
                        self.angeboten = true;
                        self.sig
                            .roh(serde_json::json!({"t":"offer","sdp":self.ton.angebot}));
                    }
                }
                meetsig::Ereignis::Sdp { art, sdp } => {
                    if art == "answer" {
                        self.ton.antwort(&sdp);
                        self.sig.roh(
                            serde_json::json!({"t":"publish","mid":self.ton.mid,"screen":false}),
                        );
                        // Auch die Bildspur anmelden - sonst weiss der Server
                        // spaeter nicht, wohin mit unserem Kamerabild. Bis die
                        // Kamera an ist, gilt sie als abgeschaltet (die anderen
                        // sehen dann einen Platzhalter statt schwarz).
                        self.sig.roh(
                            serde_json::json!({"t":"publish","mid":self.ton.vid,"screen":false}),
                        );
                        // Die Bildschirmspur GLEICH als solche anmelden -
                        // sonst haelt die Gegenseite sie fuer eine zweite
                        // Kamera, sobald spaeter Daten fliessen. Eine Kachel
                        // entsteht erst, wenn wirklich Bild kommt.
                        self.sig.roh(
                            serde_json::json!({"t":"publish","mid":self.ton.vid2,"screen":true}),
                        );
                        self.video_gemeldet = true;
                        self.sig.stumm("video", !self.kamera_an);
                    } else {
                        self.ton.server_angebot(&sdp);
                    }
                }
                meetsig::Ereignis::Spur {
                    mid,
                    peer,
                    bildschirm,
                    ..
                } => self.ton.spur_art(&mid, peer, bildschirm),
                meetsig::Ereignis::Chat { von, text, .. } => self.chat.push((von, text)),
                meetsig::Ereignis::Dazu(t) => {
                    self.chat.push((0, format!("{} ist dazugekommen", t.name)))
                }
                meetsig::Ereignis::Weg(id) => {
                    self.bilder.vergessen(id);
                    self.schirme.vergessen(id);
                    let name = self
                        .sig
                        .zustand()
                        .leute
                        .iter()
                        .find(|x| x.id == id)
                        .map(|x| x.name.clone())
                        .unwrap_or_else(|| format!("#{}", id));
                    self.chat.push((0, format!("{} ist gegangen", name)));
                }
                meetsig::Ereignis::ZwangStumm { art } => {
                    if art == "audio" {
                        self.stumm = true;
                        if let Some(g) = &self.geraete {
                            g.stumm.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                        self.ton.stumm(true);
                        self.chat
                            .push((0, "Der Gastgeber hat dich stummgeschaltet".into()));
                    }
                }
                meetsig::Ereignis::Rausgeworfen(m)
                | meetsig::Ereignis::Beendet(m)
                | meetsig::Ereignis::Abgewiesen(m) => {
                    self.meldung = m;
                }
                meetsig::Ereignis::Fernsteuerung { peer, fvid } => {
                    // Nicht ueber die eigene Freigabe selbst berichten.
                    let ich = self.sig.zustand().ich;
                    // Nur das FREIGEBEN ist eine Nachricht wert. Das
                    // Zuruecknehmen stand frueher auch im Chat - das ist
                    // Rauschen, der Knopf verschwindet ohnehin sichtbar.
                    if peer != ich && !fvid.is_empty() {
                        let wer = self.name_von(peer);
                        self.chat
                            .push((0, format!("{} erlaubt Fernsteuerung ({})", wer, fvid)));
                    }
                }
                meetsig::Ereignis::Zeiger { peer, x, y } => {
                    self.zeiger_von.insert(peer, (x, y));
                }
                meetsig::Ereignis::Bereichswunsch { peer, x, y, w, h } => {
                    // Ein Zuschauer sagt, welchen Teil MEINES Bildschirms er
                    // gerade ansieht. Merken - angewendet wird es gebuendelt,
                    // damit mehrere Zuschauer sich nicht gegenseitig jagen.
                    self.wunsch_von
                        .insert(peer, ((x, y, w, h), std::time::Instant::now()));
                }
                meetsig::Ereignis::Bereich { peer, x, y, w, h } => {
                    self.scharf_kann.insert(peer);
                    self.bereich_von.insert(peer, (x, y, w, h));
                }
                meetsig::Ereignis::Fehler { code, text } => {
                    self.meldung = format!("{}: {}", code, text);
                }
                meetsig::Ereignis::Sprecher(p) => self.sprecher = p,
                meetsig::Ereignis::Tippt { peer, an } => {
                    if an {
                        self.tippen.insert(peer);
                    } else {
                        self.tippen.remove(&peer);
                    }
                }
                meetsig::Ereignis::Willkommen { raum, server, .. } => {
                    self.protokoll
                        .push(format!("Im Raum {} - Server {}", raum, server));
                }
                meetsig::Ereignis::Getrennt(m) => {
                    self.meldung = if m.is_empty() {
                        "Verbindung beendet".into()
                    } else {
                        format!("Verbindung weg: {}", m)
                    };
                }
                _ => {}
            }
        }
        if let Some(a) = self.ton.offene_antwort() {
            self.sig.roh(serde_json::json!({"t":"answer","sdp":a}));
        }

        // Mikrofon -> Netz
        if let Some(g) = &self.geraete {
            while let Ok(rahmen) = g.mikro.try_recv() {
                let spitze = rahmen
                    .iter()
                    .map(|v| (*v as f32 / 32768.0).abs())
                    .fold(0.0f32, f32::max);
                self.pegel = self.pegel * 0.7 + spitze * 0.3;
                self.ton.senden(rahmen);
            }
        }
        // Netz -> Lautsprecher
        for te in self.ton.abholen() {
            match te {
                meetrtc::TonEreignis::Rahmen { quelle, pcm } => {
                    // Lautstaerke je Teilnehmer merken: daraus wird die
                    // gruene Umrandung "spricht gerade". Der Server meldet
                    // den Sprecher nur bei Wechseln, das hier ist sofort da.
                    let spitze = pcm
                        .iter()
                        .map(|v| (*v as f32 / 32768.0).abs())
                        .fold(0.0f32, f32::max);
                    let e = self.pegel_von.entry(quelle).or_insert(0.0);
                    *e = e.max(spitze);
                    if let Some(g) = &self.geraete {
                        if let Ok(mut m) = g.lautsprecher.lock() {
                            m.dazu(quelle, &pcm);
                        }
                    }
                }
                meetrtc::TonEreignis::Bild {
                    quelle,
                    bildschirm,
                    daten,
                    schluesselbild,
                    codec,
                } => {
                    self.bild_codec = codec;
                    // Stufe 2: das Dekodieren macht spaeter die Plattform.
                    // Hier wird vorerst nur gezaehlt, damit man sieht, dass
                    // wirklich Bild ankommt.
                    self.bild_zaehler += 1;
                    let _ = schluesselbild;
                    if bildschirm {
                        self.schirme.rahmen(quelle, &daten);
                    } else {
                        self.bilder.rahmen(quelle, &daten);
                    }
                }
                meetrtc::TonEreignis::Fehler(f) => self.meldung = f,
                meetrtc::TonEreignis::Ende(f) => {
                    if !f.is_empty() {
                        self.meldung = f;
                    }
                }
                meetrtc::TonEreignis::Verbunden => {
                    self.meldung = "Ton verbunden".into();
                }
            }
        }
        self.kamera_pumpe();
        self.schirm_pumpe();
        self.zeiger_pumpe();
        self.bereich_pumpe();
        self.messen();
    }

    /// Einmal je Sekunde: Durchsatz ausrechnen, Pegel abklingen lassen.
    /// WARUM gemessen und nicht geschaetzt: die Marke oben soll eine Zahl
    /// zeigen, die wirklich stimmt - geraten waere sie wertlos.
    fn messen(&mut self) {
        for v in self.pegel_von.values_mut() {
            *v *= 0.90;
        }
        let jetzt = std::time::Instant::now();
        let dt = jetzt.duration_since(self.letzte_messung).as_secs_f32();
        if dt < 1.0 {
            return;
        }
        let z = self.ton.zahlen();
        let (alt_raus, alt_rein) = self.letzte_bytes;
        self.kbit_raus = ((z.bytes_raus.saturating_sub(alt_raus) as f32 * 8.0) / dt / 1000.0) as u32;
        self.kbit_rein = ((z.bytes_rein.saturating_sub(alt_rein) as f32 * 8.0) / dt / 1000.0) as u32;
        self.letzte_bytes = (z.bytes_raus, z.bytes_rein);
        self.letzte_messung = jetzt;
    }

    /// Spricht dieser Teilnehmer gerade?
    pub fn spricht(&self, id: u64) -> bool {
        self.sprecher == Some(id) || self.pegel_von.get(&id).copied().unwrap_or(0.0) > 0.06
    }

    /// Gastgeber-Befehle: "mute-all" | "mute" | "kick" | "end".
    pub fn gastgeber_aktion(&self, aktion: &str, peer: Option<u64>) {
        self.sig.gastgeber_aktion(aktion, peer);
    }

    /// "schreibt gerade" an die anderen melden.
    pub fn tippt(&self, an: bool) {
        self.sig.tippt(an);
    }

    /// Bildschirm -> H.264 -> Meeting (eigene Spur). 15 Bilder je Sekunde,
    /// immer das neueste Bild; alle 4 Sekunden ein Schluesselbild, damit
    /// spaet Dazugekommene nicht vor einer leeren Flaeche sitzen.
    /// Eigene Zeigerposition melden, solange der Bildschirm geteilt wird.
    ///
    /// 10-mal je Sekunde und nur bei ECHTER Aenderung: das sind ein paar
    /// Dutzend Byte, die dem Zuschauer aber erlauben, genau dorthin zu
    /// zoomen, wohin gezeigt wird.
    fn zeiger_pumpe(&mut self) {
        if !self.schirm_an {
            return;
        }
        let jetzt = std::time::Instant::now();
        if jetzt < self.naechster_zeiger {
            return;
        }
        self.naechster_zeiger = jetzt + std::time::Duration::from_millis(100);
        let index = match self.schirm.as_ref() {
            Some(a) => a.index,
            None => return,
        };
        if let Some((x, y)) = crate::meetschirm::zeiger_anteil(index) {
            let neu = (x, y);
            let anders = match self.letzter_zeiger {
                Some((ax, ay)) => (ax - x).abs() > 0.002 || (ay - y).abs() > 0.002,
                None => true,
            };
            if anders {
                self.letzter_zeiger = Some(neu);
                self.sig.zeiger(x, y);
            }
        }
    }

    fn schirm_pumpe(&mut self) {
        let jetzt = std::time::Instant::now();
        if jetzt < self.naechstes_schirmbild {
            return;
        }
        let (auf, kod) = match (self.schirm.as_ref(), self.schirm_koder.as_mut()) {
            (Some(a), Some(k)) => (a, k),
            _ => return,
        };
        let bild = match auf.neuestes() {
            Some(b) => b,
            None => return,
        };
        // 50 ms = 20 Bilder je Sekunde. Mehr als Text und Fenster brauchen,
        // aber deutlich fluessiger als die frueheren 15 - und ein geteilter
        // Bildschirm kostet je Bild weniger als eine Kamera (viel bleibt gleich).
        self.naechstes_schirmbild = jetzt + std::time::Duration::from_millis(50);
        if jetzt.duration_since(self.letztes_schluesselbild) >= std::time::Duration::from_secs(4) {
            self.letztes_schluesselbild = jetzt;
            kod.schluesselbild();
        }
        match kod.nv12_rahmen(&bild.nv12) {
            Ok(teile) => {
                for t in teile {
                    self.ton.schirm_senden(t.data);
                }
                self.schirm_gesendet += 1;
            }
            Err(e) => self.schirm_meldung = format!("Kodierer: {}", e),
        }
        // Eigene Vorschau: nur jedes ZWEITE Bild (10-mal je Sekunde). Ein
        // Bildschirm ist gross, das Umrechnen nach RGBA kostet mehr als bei
        // der Kamera - und zum Kontrollieren reicht das dicke.
        if self.schirm_gesendet % 2 == 0 {
            let mut rgba = Vec::new();
            if crate::h264::nv12_to_rgba(
                &bild.nv12,
                bild.breite,
                bild.hoehe,
                bild.breite as usize,
                bild.hoehe,
                &mut rgba,
            ) {
                self.eigen_schirm = Some((bild.breite, bild.hoehe, rgba));
                self.eigen_schirm_stand += 1;
            }
        }
        let f = auf.fehler();
        if !f.is_empty() {
            self.schirm_meldung = f;
        }
    }

    /// Wie viele Bildschirme gibt es hier?
    pub fn monitore() -> Vec<crate::meetschirm::Schirm> {
        crate::meetschirm::liste()
    }

    /// Bildschirmfreigabe an/aus. `index` waehlt den Bildschirm.
    /// Bildschirmfreigabe schalten. Liefert eine MELDUNG zurueck, wenn es
    /// nicht geklappt hat - die gehoert dem Nutzer vor die Nase, nicht nur
    /// ins Protokoll. Genau daran lag es, dass ein Fehlschlag aussah wie
    /// "es passiert nichts".
    pub fn schirm_schalten_melden(&mut self, an: bool, index: usize) -> Option<String> {
        self.schirm_schalten(an, index);
        if an && !self.schirm_an {
            let m = self.schirm_meldung.clone();
            return Some(if m.is_empty() {
                "Bildschirm laesst sich nicht teilen".to_string()
            } else {
                m
            });
        }
        None
    }

    pub fn schirm_schalten(&mut self, an: bool, index: usize) {
        if an {
            if self.schirm.is_some() {
                return;
            }
            match crate::meetschirm::oeffnen(index, 1920, 1080, 15) {
                Ok(a) => match crate::meetvideo::Kodierer::neu(a.breite, a.hoehe, 20, 4_000_000) {
                    Ok(k) => {
                        self.schirm_meldung = format!("Teile {}", a.name);
                        self.schirm = Some(a);
                        self.schirm_koder = Some(k);
                        self.schirm_an = true;
                        self.schirm_gesendet = 0;
                        self.letztes_schluesselbild =
                            std::time::Instant::now() - std::time::Duration::from_secs(9);
                        // Erst die Spur als Bildschirm melden, dann die
                        // Freigabe ankuendigen - in der Reihenfolge, sonst
                        // legt die Gegenseite die Kachel falsch an.
                        self.sig.roh(
                            serde_json::json!({"t":"publish","mid":self.ton.vid2,"screen":true}),
                        );
                        self.sig.roh(serde_json::json!({"t":"screen","on":true}));
                    }
                    Err(e) => {
                        a.stoppen();
                        self.schirm_meldung = format!("Kein Kodierer: {}", e);
                        self.schirm_an = false;
                    }
                },
                Err(e) => {
                    self.schirm_meldung = format!("Kein Bildschirm: {}", e);
                    self.schirm_an = false;
                }
            }
        } else {
            if let Some(a) = self.schirm.take() {
                a.stoppen();
            }
            self.schirm_koder = None;
            self.schirm_an = false;
            self.eigen_schirm = None;
            self.eigen_schirm_stand += 1;
            self.schirm_meldung = String::new();
            self.sig.roh(serde_json::json!({"t":"screen","on":false}));
        }
    }

    /// Name der laufenden Bildschirmfreigabe (leer = aus).
    pub fn schirm_name(&self) -> String {
        self.schirm.as_ref().map(|a| a.name.clone()).unwrap_or_default()
    }

    /// Kamera -> H.264 -> Meeting. Laeuft mit 15 Bildern je Sekunde; das
    /// Bild wird beim Abholen genommen, nicht gestaut.
    fn kamera_pumpe(&mut self) {
        let jetzt = std::time::Instant::now();
        if jetzt < self.naechstes_bild {
            return;
        }
        let (kam, kod) = match (self.kamera.as_ref(), self.koder.as_mut()) {
            (Some(k), Some(c)) => (k, c),
            _ => return,
        };
        let bild = match kam.neuestes() {
            Some(b) => b,
            None => return,
        };
        // 33 ms = 30 Bilder je Sekunde. Vorher waren es 66 ms (15/s) - das
        // war zusammen mit dem langsamen Neuzeichnen der Grund fuer das
        // ruckelige Bild. Die Kamera liefert ohnehin 30.
        self.naechstes_bild = jetzt + std::time::Duration::from_millis(33);
        match kod.nv12_rahmen(&bild.nv12) {
            Ok(teile) => {
                for t in teile {
                    self.ton.bild_senden(t.data);
                }
                self.bild_gesendet += 1;
            }
            Err(e) => self.kamera_meldung = format!("Kodierer: {}", e),
        }
        // Eigene Vorschau. Frueher nur jedes DRITTE Bild - bei 15 Bildern
        // je Sekunde also 5/s, und die eigene Kachel ruckelte sichtbar
        // staerker als die der anderen. Das Umrechnen nach RGBA kostet bei
        // 640x360 kaum etwas; jetzt jedes Bild.
        {
            let mut rgba = Vec::new();
            if crate::h264::nv12_to_rgba(
                &bild.nv12,
                bild.breite,
                bild.hoehe,
                bild.breite as usize,
                bild.hoehe,
                &mut rgba,
            ) {
                self.eigen = Some((bild.breite, bild.hoehe, rgba));
                self.eigen_stand += 1;
            }
        }
        let f = kam.fehler();
        if !f.is_empty() {
            self.kamera_meldung = f;
        }
    }

    /// Kamera an- oder ausschalten.
    pub fn kamera_schalten(&mut self, an: bool) {
        if an {
            if self.kamera.is_some() {
                return;
            }
            match crate::meetcam::oeffnen(
                self.kamera_geraet.clone(),
                crate::meetvideo::BREITE,
                crate::meetvideo::HOEHE,
                30,
            ) {
                Ok(k) => {
                    match crate::meetvideo::Kodierer::neu(k.breite, k.hoehe, 30, 2_000_000) {
                        Ok(c) => {
                            self.kamera_meldung = format!("Kamera: {}", k.name);
                            self.kamera = Some(k);
                            self.koder = Some(c);
                            self.kamera_an = true;
                        }
                        Err(e) => {
                            self.kamera_meldung = format!("Kein Kodierer: {}", e);
                            self.kamera_an = false;
                        }
                    }
                }
                Err(e) => {
                    self.kamera_meldung = format!("Keine Kamera: {}", e);
                    self.kamera_an = false;
                }
            }
        } else {
            if let Some(k) = self.kamera.take() {
                k.stoppen();
            }
            self.koder = None;
            self.eigen = None;
            self.eigen_stand += 1;
            self.kamera_an = false;
        }
        if self.video_gemeldet {
            self.sig.stumm("video", !self.kamera_an);
        }
    }

    /// Name der laufenden Kamera (leer = aus).
    pub fn kamera_name(&self) -> String {
        self.kamera.as_ref().map(|k| k.name.clone()).unwrap_or_default()
    }

    pub fn stumm_schalten(&mut self, an: bool) {
        self.stumm = an;
        self.ton.stumm(an);
        if let Some(g) = &self.geraete {
            g.stumm.store(an, std::sync::atomic::Ordering::Relaxed);
        }
        self.sig.stumm("audio", an);
    }

    /// Kamera im LAUFENDEN Meeting wechseln. `geraet` = None ist das
    /// Standardgeraet. Laeuft die Kamera gerade, wird sie neu geoeffnet -
    /// sonst merkt sich das Meeting die Wahl fuer das naechste Einschalten.
    pub fn kamera_waehlen(&mut self, geraet: Option<String>) {
        if self.kamera_geraet == geraet {
            return;
        }
        self.kamera_geraet = geraet;
        if self.kamera_an {
            // Aus und wieder an: der Aufnahmefaden haengt fest am Geraet,
            // ein Wechsel im Betrieb ist nicht vorgesehen.
            self.kamera_schalten(false);
            self.kamera_schalten(true);
        }
    }

    /// Mikrofon und/oder Lautsprecher im laufenden Meeting wechseln.
    ///
    /// WARUM beides zusammen: Aufnahme und Wiedergabe haengen an EINEM
    /// Baustein (gemeinsame Echoausloeschung). Einzeln tauschen ginge nur,
    /// indem man die Echoschaetzung wegwirft - dann pfeift es.
    pub fn ton_waehlen(&mut self, mikro: Option<String>, lautsprecher: Option<String>) {
        if self.mikro_geraet == mikro && self.lautsprecher_geraet == lautsprecher {
            return;
        }
        self.mikro_geraet = mikro.clone();
        self.lautsprecher_geraet = lautsprecher.clone();
        // Erst das alte loslassen (Drop stoppt die Faeden), dann das neue.
        self.geraete = None;
        match meetaudio::geraete_starten(mikro, lautsprecher) {
            Ok(g) => {
                g.stumm
                    .store(self.stumm, std::sync::atomic::Ordering::Relaxed);
                self.meldung = format!("Mikrofon: {} / Lautsprecher: {}", g.eingang, g.ausgang);
                self.protokoll.push(self.meldung.clone());
                self.geraete = Some(g);
            }
            Err(e) => {
                self.meldung = format!("Kein Ton-Geraet: {}", e);
                self.protokoll.push(self.meldung.clone());
            }
        }
    }

    /// Namen der gerade benutzten Tongeraete (fuer die Anzeige).
    pub fn ton_namen(&self) -> (String, String) {
        match &self.geraete {
            Some(g) => (g.eingang.clone(), g.ausgang.clone()),
            None => (String::new(), String::new()),
        }
    }

    /// Als ZUSCHAUER: sagen, welchen Teil des fremden Bildschirms ich
    /// ansehe. Der Sender schneidet dann genau das aus seiner nativen
    /// Aufnahme - gleiche Kodiergroesse, also gleiche Bandbreite, aber
    /// echte Bildpunkte statt hochgerechneter.
    ///
    /// `scharf = false` heisst: bitte weiter alles schicken (dann bleibt es
    /// beim reinen Vergroessern).
    pub fn zoom_wunsch(&mut self, peer: u64, bereich: crate::meetschirm::Bereich, scharf: bool) {
        let will = if scharf {
            bereich
        } else {
            crate::meetschirm::GANZ
        };
        let jetzt = std::time::Instant::now();
        let neu = match self.letzter_wunsch {
            Some((p, alt)) => p != peer || crate::meetschirm::bereich_lohnt_wechsel(alt, will),
            None => true,
        };
        // Auch ohne Aenderung alle 3 s wiederholen: der Sender vergisst
        // Wuensche, von denen er nichts mehr hoert (sonst bliebe er beim
        // Ausschnitt eines laengst gegangenen Zuschauers stehen).
        if !neu && jetzt < self.naechster_wunsch {
            return;
        }
        self.naechster_wunsch = jetzt + std::time::Duration::from_secs(3);
        self.letzter_wunsch = Some((peer, will));
        self.sig.bereichswunsch(peer, will.0, will.1, will.2, will.3);
    }

    /// Als SENDER: aus allen Wuenschen den Ausschnitt bilden und anwenden.
    ///
    /// Laeuft in jedem Durchlauf, aendert aber nur bei einem lohnenden
    /// Unterschied etwas - jeder Wechsel kostet ein Schluesselbild und ist
    /// als kleiner Sprung sichtbar.
    fn bereich_pumpe(&mut self) {
        if !self.schirm_an {
            if self.schirm_bereich != crate::meetschirm::GANZ {
                self.schirm_bereich = crate::meetschirm::GANZ;
            }
            return;
        }
        // Wuensche, von denen wir seit 8 s nichts gehoert haben, zaehlen
        // nicht mehr - der Zuschauer ist weg oder wieder herausgezoomt.
        let jetzt = std::time::Instant::now();
        self.wunsch_von
            .retain(|_, (_, wann)| jetzt.duration_since(*wann) < std::time::Duration::from_secs(8));
        let wuensche: Vec<crate::meetschirm::Bereich> =
            self.wunsch_von.values().map(|(b, _)| *b).collect();
        let neu = crate::meetschirm::bereich_vereinen(&wuensche);
        if neu == self.schirm_bereich {
            return;
        }
        if self.schirm_bereich != crate::meetschirm::GANZ
            && neu != crate::meetschirm::GANZ
            && !crate::meetschirm::bereich_lohnt_wechsel(self.schirm_bereich, neu)
        {
            return;
        }
        self.schirm_bereich = neu;
        if let Some(a) = self.schirm.as_ref() {
            a.bereich_setzen(neu);
        }
        // Der Bildinhalt springt - ohne frisches Schluesselbild saehen die
        // anderen einen Moment lang Bildsalat.
        if let Some(k) = self.schirm_koder.as_mut() {
            k.schluesselbild();
        }
        self.letztes_schluesselbild = jetzt;
        // Und den anderen sagen, WAS jetzt kommt. Ohne diese Meldung wuerden
        // sie in das schon zugeschnittene Bild noch einmal hineinzoomen.
        self.sig.bereich(neu.0, neu.1, neu.2, neu.3);
    }

    /// Zeiger dessen, der gerade teilt (0..1). None = kommt (noch) nicht an.
    pub fn zeiger_des_teilers(&self, peer: u64) -> Option<(f32, f32)> {
        self.zeiger_von.get(&peer).copied()
    }

    /// Der EIGENE Mauszeiger auf dem gerade geteilten Bildschirm.
    ///
    /// WARUM extra: der Server schickt Zeigerpositionen nur an die ANDEREN -
    /// den eigenen bekommt man nie zurueck. Wer seine eigene Freigabe ansieht,
    /// las deshalb ewig "Maus des Teilers (kommt nicht an)", obwohl die
    /// Angabe direkt vor der Nase lag.
    pub fn eigener_zeiger(&self) -> Option<(f32, f32)> {
        let a = self.schirm.as_ref()?;
        crate::meetschirm::zeiger_anteil(a.index)
    }

    /// Welchen Ausschnitt sendet dieser Teilnehmer gerade?
    pub fn bereich_des_teilers(&self, peer: u64) -> Option<crate::meetschirm::Bereich> {
        self.bereich_von.get(&peer).copied()
    }

    /// Kann dieser Teilnehmer ueberhaupt Ausschnitte liefern?
    pub fn kann_scharf(&self, peer: u64) -> bool {
        self.scharf_kann.contains(&peer)
    }

    pub fn hand_heben(&mut self, an: bool) {
        self.hand = an;
        self.sig.hand(an);
    }

    pub fn senden(&mut self) {
        let t = self.eingabe.trim().to_string();
        if t.is_empty() {
            return;
        }
        self.sig.chat(&t);
        let ich = self.sig.zustand().ich;
        self.chat.push((ich, t));
        self.eingabe.clear();
    }

    pub fn einlassen(&self, peer: u64) {
        self.sig.warteraum("admit", Some(peer));
    }
    pub fn abweisen(&self, peer: u64) {
        self.sig.warteraum("deny", Some(peer));
    }
    pub fn alle_einlassen(&self) {
        self.sig.warteraum("admit-all", None);
    }
    pub fn warteraum(&self, an: bool) {
        self.sig.warteraum(if an { "on" } else { "off" }, None);
    }

    pub fn verlassen(&self) {
        if let Some(k) = self.kamera.as_ref() {
            k.stoppen();
        }
        if let Some(a) = self.schirm.as_ref() {
            a.stoppen();
            self.sig.roh(serde_json::json!({"t":"screen","on":false}));
        }
        self.sig.verlassen();
        self.ton.beenden();
    }

    /// Name eines Teilnehmers (fuer den Chat).
    pub fn name_von(&self, id: u64) -> String {
        let z = self.sig.zustand();
        if id == z.ich {
            return "Du".into();
        }
        z.leute
            .iter()
            .find(|x| x.id == id)
            .map(|x| x.name.clone())
            .unwrap_or_else(|| format!("#{}", id))
    }

    /// Stufe 4: eigene Fernsteuerung freigeben oder zuruecknehmen. Die
    /// anderen bekommen dadurch einen Knopf, der eine echte FreeViewer-
    /// Sitzung zu uns aufbaut - zugelassen wird sie trotzdem erst hier im
    /// Programm, also nie hinter dem Ruecken des Besitzers.
    pub fn steuerung_freigeben(&mut self, an: bool) {
        // Ohne eigene Nummer waere die Freigabe wertlos - dann lieber melden.
        if an && self.ich_fvid.chars().filter(|c| c.is_ascii_digit()).count() == 0 {
            self.meldung = "Keine FreeViewer-Nummer - Steuerung nicht freigebbar".into();
            return;
        }
        self.steuer_frei = an;
        self.sig.fernsteuerung(an, &self.ich_fvid);
        if an {
            self.chat
                .push((0, "Du erlaubst jetzt Fernsteuerung".to_string()));
        }
    }

    /// Wer im Raum laesst sich fernsteuern? (Teilnehmer, Name, FreeViewer-Nr.)
    pub fn steuerbare(&self) -> Vec<(u64, String, String)> {
        let z = self.sig.zustand();
        z.leute
            .iter()
            .filter(|t| t.id != z.ich && !t.fvid.is_empty())
            .map(|t| (t.id, t.name.clone(), t.fvid.clone()))
            .collect()
    }

    /// Eigene FreeViewer-Nummer (fuer die Anzeige).
    pub fn meine_fvid(&self) -> &str {
        &self.ich_fvid
    }

    pub fn bin_gastgeber(&self) -> bool {
        let z = self.sig.zustand();
        z.gastgeber != 0 && z.gastgeber == z.ich
    }
}

/// Die Vorschau VOR dem Beitritt: eigenes Kamerabild und Mikrofonpegel,
/// ohne dass schon ein Meeting laeuft.
///
/// WARUM ein eigener Baustein: der Browser zeigt auf seinem Beitritts-Schirm
/// ein lebendes Selbstbild - ohne das wirkt die Seite wie ein Formular, und
/// genau das hat Justin bemaengelt. NativMeet kann das nicht liefern, weil es
/// erst nach dem Beitritt existiert. Beim Beitreten werden die Geraete hier
/// wieder losgelassen, sonst haelt die Vorschau die Kamera fest und das
/// Meeting bekaeme sie nicht.
pub struct Vorprobe {
    kamera: Option<crate::meetcam::Kamera>,
    ton: Option<meetaudio::Geraete>,
    /// Eigenes Bild fuer die Anzeige: (Breite, Hoehe, RGBA).
    pub eigen: Option<(u32, u32, Vec<u8>)>,
    /// Zaehlt hoch, sobald ein neues Bild da ist (Textur nachladen).
    pub stand: u64,
    pub pegel: f32,
    pub meldung: String,
    naechstes: std::time::Instant,
    /// Was der Nutzer WILL - getrennt vom Ist-Zustand. Ohne diese Trennung
    /// wuerde ein Geraetefehler in jedem Bild einen neuen Versuch ausloesen.
    pub wunsch_kamera: bool,
    pub wunsch_mikro: bool,
    pub mikro_geraet: Option<String>,
}

impl Default for Vorprobe {
    fn default() -> Self {
        Vorprobe {
            kamera: None,
            ton: None,
            eigen: None,
            stand: 0,
            pegel: 0.0,
            meldung: String::new(),
            naechstes: std::time::Instant::now(),
            wunsch_kamera: false,
            wunsch_mikro: false,
            mikro_geraet: None,
        }
    }
}

impl Vorprobe {
    pub fn kamera_an(&self) -> bool {
        self.kamera.is_some()
    }

    /// Kamera fuer die Vorschau an- oder ausschalten.
    pub fn kamera(&mut self, an: bool) {
        self.wunsch_kamera = an;
        if an == self.kamera.is_some() {
            return;
        }
        if !an {
            if let Some(k) = self.kamera.take() {
                k.stoppen();
            }
            self.eigen = None;
            self.stand += 1;
            return;
        }
        match crate::meetcam::oeffnen(None, crate::meetvideo::BREITE, crate::meetvideo::HOEHE, 30) {
            Ok(k) => {
                self.meldung = format!("Kamera: {}", k.name);
                self.kamera = Some(k);
            }
            Err(e) => {
                self.meldung = format!("Keine Kamera: {}", e);
            }
        }
    }

    /// Mikrofon fuer den Pegelbalken an- oder ausschalten.
    pub fn mikro(&mut self, an: bool, geraet: Option<String>) {
        let wechsel = geraet != self.mikro_geraet;
        self.wunsch_mikro = an;
        self.mikro_geraet = geraet.clone();
        if !an {
            self.ton = None;
            self.pegel = 0.0;
            return;
        }
        if self.ton.is_some() && !wechsel {
            return;
        }
        self.ton = None;
        match meetaudio::geraete_starten(geraet, None) {
            Ok(g) => self.ton = Some(g),
            Err(e) => self.meldung = format!("Kein Mikrofon: {}", e),
        }
    }

    /// In jedem Bild aufrufen: Bild abholen, Pegel messen.
    pub fn pumpe(&mut self) {
        if let Some(g) = &self.ton {
            let mut spitze = 0.0f32;
            while let Ok(rahmen) = g.mikro.try_recv() {
                spitze = spitze.max(
                    rahmen
                        .iter()
                        .map(|v| (*v as f32 / 32768.0).abs())
                        .fold(0.0f32, f32::max),
                );
            }
            self.pegel = self.pegel * 0.7 + spitze * 0.3;
        }
        let jetzt = std::time::Instant::now();
        if jetzt < self.naechstes {
            return;
        }
        let kam = match self.kamera.as_ref() {
            Some(k) => k,
            None => return,
        };
        let bild = match kam.neuestes() {
            Some(b) => b,
            None => return,
        };
        // 12 Bilder je Sekunde reichen fuer eine Vorschau vollkommen.
        self.naechstes = jetzt + std::time::Duration::from_millis(80);
        let mut rgba = Vec::new();
        if crate::h264::nv12_to_rgba(
            &bild.nv12,
            bild.breite,
            bild.hoehe,
            bild.breite as usize,
            bild.hoehe,
            &mut rgba,
        ) {
            self.eigen = Some((bild.breite, bild.hoehe, rgba));
            self.stand += 1;
        }
        let f = kam.fehler();
        if !f.is_empty() {
            self.meldung = f;
        }
    }

    /// Alles wieder loslassen (vor dem Beitritt und beim Schliessen).
    pub fn aus(&mut self) {
        if let Some(k) = self.kamera.take() {
            k.stoppen();
        }
        self.wunsch_kamera = false;
        self.wunsch_mikro = false;
        self.mikro_geraet = None;
        self.ton = None;
        self.eigen = None;
        self.pegel = 0.0;
        self.stand += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vorprobe_ohne_kamera_und_mikrofon_meldet_sauber() {
        // Auf einem Server ohne Kamera/Soundkarte darf die Vorschau nicht
        // knallen - sie muss nur ehrlich melden, dass nichts da ist.
        let mut v = Vorprobe::default();
        v.kamera(true);
        v.mikro(true, None);
        v.pumpe();
        #[cfg(not(windows))]
        assert!(!v.kamera_an(), "ohne Kamera darf keine laufen");
        assert!(v.eigen.is_none() || v.kamera_an());
        v.aus();
        assert!(!v.kamera_an());
        assert_eq!(v.pegel, 0.0);
    }

    #[test]
    fn geraeteliste_stuerzt_nicht_ab() {
        // Auf einem Server ohne Soundkarte muss das eine leere Liste geben
        // und nicht knallen.
        let (ein, aus) = meetaudio::geraete_liste();
        println!("Mikrofone {:?} Lautsprecher {:?}", ein.len(), aus.len());
    }

    #[test]
    fn kamera_schalten_ohne_kamera_meldet_sauber() {
        // Server ohne Kamera: der Schalter muss eine Meldung setzen und
        // aus bleiben - kein Absturz, kein haengender Faden.
        let r = NativMeet::beitreten("http://127.0.0.1:1", "000-000-000", "x", "T", "", false, None, None);
        if let Ok(mut m) = r {
            m.kamera_schalten(true);
            #[cfg(not(windows))]
            {
                assert!(!m.kamera_an, "ohne Kamera darf sie nicht an sein");
                assert!(!m.kamera_meldung.is_empty(), "keine Meldung");
            }
            m.kamera_schalten(false);
            assert!(!m.kamera_an);
            m.verlassen();
        }
    }

    #[test]
    fn schirm_schalten_ohne_bildschirm_meldet_sauber() {
        // Server ohne Bildschirm: sauber melden, nicht abstuerzen.
        let r = NativMeet::beitreten("http://127.0.0.1:1", "000-000-000", "x", "T", "", false, None, None);
        if let Ok(mut m) = r {
            m.schirm_schalten(true, 0);
            if !m.schirm_an {
                assert!(!m.schirm_meldung.is_empty(), "keine Meldung");
            }
            m.schirm_schalten(false, 0);
            assert!(!m.schirm_an);
            m.verlassen();
        }
    }

    #[test]
    fn steuerung_ohne_nummer_wird_nicht_freigegeben() {
        // Ohne eigene FreeViewer-Nummer waere die Freigabe eine Luege:
        // die anderen bekaemen einen Knopf, der ins Leere fuehrt.
        let r = NativMeet::beitreten(
            "http://127.0.0.1:1",
            "000-000-000",
            "x",
            "T",
            "",
            true,
            None,
            None,
        );
        if let Ok(mut m) = r {
            assert!(!m.steuer_frei, "ohne Nummer darf nichts freigegeben sein");
            m.steuerung_freigeben(true);
            assert!(!m.steuer_frei, "ohne Nummer darf die Freigabe nicht greifen");
            assert!(!m.meldung.is_empty(), "keine Meldung");
            m.verlassen();
        }
    }

    #[test]
    fn steuerung_mit_nummer_schaltet_um() {
        let r = NativMeet::beitreten(
            "http://127.0.0.1:1",
            "000-000-000",
            "x",
            "T",
            "497628420",
            false,
            None,
            None,
        );
        if let Ok(mut m) = r {
            assert!(!m.steuer_frei, "beim Beitreten war die Freigabe aus");
            m.steuerung_freigeben(true);
            assert!(m.steuer_frei, "Freigabe hat nicht gegriffen");
            m.steuerung_freigeben(false);
            assert!(!m.steuer_frei, "Zuruecknehmen hat nicht gegriffen");
            assert_eq!(m.meine_fvid(), "497628420");
            // Ohne Verbindung kennt der Raum niemanden - trotzdem darf die
            // Liste nur laufen, nicht knallen.
            assert!(m.steuerbare().is_empty());
            m.verlassen();
        }
    }

    #[test]
    fn beitreten_ohne_server_meldet_sauber() {
        // Falscher Port -> die Signalisierung darf nicht in Panik geraten,
        // sondern muss einen Fehler liefern oder still getrennt melden.
        let r = NativMeet::beitreten(
            "http://127.0.0.1:1",
            "000-000-000",
            "x",
            "Test",
            "",
            false,
            None,
            None,
        );
        match r {
            Ok(m) => {
                // Verbindung laeuft im Hintergrund - nach kurzer Zeit muss
                // eine Meldung dastehen, kein Absturz.
                std::thread::sleep(std::time::Duration::from_millis(300));
                let mut m = m;
                m.pumpe();
                assert!(!m.zustand().verbunden || m.zustand().letzter_fehler.is_empty());
                m.verlassen();
            }
            Err(e) => {
                assert!(!e.to_string().is_empty());
            }
        }
    }
}
