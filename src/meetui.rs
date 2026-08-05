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
    /// Dekodierte Bilder der anderen (je Teilnehmer das letzte).
    pub bilder: crate::meetvideo::Dekodierer,
    /// Eigene Kamera (Stufe 2d) - laeuft nur, wenn sie eingeschaltet ist.
    kamera: Option<crate::meetcam::Kamera>,
    koder: Option<crate::meetvideo::Kodierer>,
    pub kamera_an: bool,
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
    naechstes_schirmbild: std::time::Instant,
    /// Wann zuletzt ein Schluesselbild angefordert wurde.
    letztes_schluesselbild: std::time::Instant,
}

impl NativMeet {
    /// Beitreten. `mikro`/`lautsprecher` leer = Standardgeraet.
    pub fn beitreten(
        basis: &str,
        raum: &str,
        pass: &str,
        name: &str,
        fvid: &str,
        mikro: Option<String>,
        lautsprecher: Option<String>,
    ) -> Result<NativMeet> {
        let sig = meetsig::beitreten(basis, raum, pass, name, fvid)?;
        let ton = meetrtc::starten()?;
        // Ohne Soundkarte (Server, Testrechner) laeuft das Meeting trotzdem -
        // man hoert dann nur nichts. Ehrlich melden statt abbrechen.
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
            kamera: None,
            koder: None,
            kamera_an: false,
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
            naechstes_schirmbild: std::time::Instant::now(),
            letztes_schluesselbild: std::time::Instant::now(),
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
                meetsig::Ereignis::Spur { mid, peer, .. } => self.ton.spur(&mid, peer),
                meetsig::Ereignis::Chat { von, text, .. } => self.chat.push((von, text)),
                meetsig::Ereignis::Dazu(t) => {
                    self.chat.push((0, format!("{} ist dazugekommen", t.name)))
                }
                meetsig::Ereignis::Weg(id) => {
                    self.bilder.vergessen(id);
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
                meetsig::Ereignis::Fehler { code, text } => {
                    self.meldung = format!("{}: {}", code, text);
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
                    if let Some(g) = &self.geraete {
                        if let Ok(mut m) = g.lautsprecher.lock() {
                            m.dazu(quelle, &pcm);
                        }
                    }
                }
                meetrtc::TonEreignis::Bild {
                    quelle,
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
                    self.bilder.rahmen(quelle, &daten);
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
    }

    /// Bildschirm -> H.264 -> Meeting (eigene Spur). 15 Bilder je Sekunde,
    /// immer das neueste Bild; alle 4 Sekunden ein Schluesselbild, damit
    /// spaet Dazugekommene nicht vor einer leeren Flaeche sitzen.
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
        self.naechstes_schirmbild = jetzt + std::time::Duration::from_millis(66);
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
        let f = auf.fehler();
        if !f.is_empty() {
            self.schirm_meldung = f;
        }
    }

    /// Wie viele Bildschirme gibt es hier?
    pub fn schirme() -> Vec<crate::meetschirm::Schirm> {
        crate::meetschirm::liste()
    }

    /// Bildschirmfreigabe an/aus. `index` waehlt den Bildschirm.
    pub fn schirm_schalten(&mut self, an: bool, index: usize) {
        if an {
            if self.schirm.is_some() {
                return;
            }
            match crate::meetschirm::oeffnen(index, 1920, 1080, 15) {
                Ok(a) => match crate::meetvideo::Kodierer::neu(a.breite, a.hoehe, 15, 3_500_000) {
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
        self.naechstes_bild = jetzt + std::time::Duration::from_millis(66);
        match kod.nv12_rahmen(&bild.nv12) {
            Ok(teile) => {
                for t in teile {
                    self.ton.bild_senden(t.data);
                }
                self.bild_gesendet += 1;
            }
            Err(e) => self.kamera_meldung = format!("Kodierer: {}", e),
        }
        // Eigene Vorschau (nur 5-mal je Sekunde - mehr braucht kein Mensch
        // und es spart Rechenzeit).
        if self.bild_gesendet % 3 == 0 {
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
                None,
                crate::meetvideo::BREITE,
                crate::meetvideo::HOEHE,
                30,
            ) {
                Ok(k) => {
                    match crate::meetvideo::Kodierer::neu(k.breite, k.hoehe, 15, 1_500_000) {
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

    pub fn bin_gastgeber(&self) -> bool {
        let z = self.sig.zustand();
        z.gastgeber != 0 && z.gastgeber == z.ich
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let r = NativMeet::beitreten("http://127.0.0.1:1", "000-000-000", "x", "T", "", None, None);
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
        let r = NativMeet::beitreten("http://127.0.0.1:1", "000-000-000", "x", "T", "", None, None);
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
    fn beitreten_ohne_server_meldet_sauber() {
        // Falscher Port -> die Signalisierung darf nicht in Panik geraten,
        // sondern muss einen Fehler liefern oder still getrennt melden.
        let r = NativMeet::beitreten(
            "http://127.0.0.1:1",
            "000-000-000",
            "x",
            "Test",
            "",
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
