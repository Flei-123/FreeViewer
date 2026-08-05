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
