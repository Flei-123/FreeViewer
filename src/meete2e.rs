//! Ende-zu-Ende-Verschluesselung fuer das Meeting.
//!
//! WARUM ueberhaupt: Ton und Bild sind heute nur auf dem WEG verschluesselt
//! (DTLS-SRTP). Der Server muss die Pakete verteilen und entschluesselt sie
//! dazu - er sieht also alles. Genau das steht ehrlich in der Marke oben im
//! Fenster ("E2E: aus - Server sieht Medien").
//!
//! Hier wird die NUTZLAST schon beim Absender verschluesselt, mit einem
//! Schluessel, den der Server nie bekommt. Er verteilt dann nur noch
//! Kauderwelsch.
//!
//! DAS PROBLEM DABEI - und warum es nicht reicht, einfach alles zu
//! verschluesseln: Der Server schaut heute IN die Bilddaten, um
//! Schluesselbilder zu finden (er braucht sie, um zwischen Qualitaetsstufen
//! umzuschalten). Waere alles verschluesselt, wuerde er blind raten und die
//! Bildqualitaet kaputt machen. Deshalb bekommt jeder Rahmen einen kleinen
//! KLARTEXT-KOPF, der nur zwei Dinge verraet: "ist das ein Schluesselbild"
//! und "welche laufende Nummer". Beides ist harmlos - daraus laesst sich
//! kein Bild und kein Wort rekonstruieren.
//!
//! Rahmenformat:
//!   Byte 0      Kennung 0xE2 (damit alte Staende es erkennen und in Ruhe lassen)
//!   Byte 1      Merker: Bit 0 = Schluesselbild
//!   Byte 2..6   Zaehler (32 Bit, gross-endisch) - zugleich der Zufallswert
//!   ab Byte 6   AES-256-GCM: Geheimtext + 16 Byte Pruefsumme
//!
//! Der Kopf geht als "zusaetzliche Daten" in die Pruefsumme ein. Wer am
//! Schluesselbild-Merker dreht, zerstoert damit die Pruefung - der Server
//! kann also mitlesen, was er zum Verteilen braucht, aber nichts faelschen.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};

/// Kennung im ersten Byte. Ein alter Empfaenger sieht damit sofort, dass er
/// den Rahmen nicht versteht, statt Bildsalat zu zeigen.
pub const KENNUNG: u8 = 0xE2;
/// Laenge des Klartext-Kopfs.
pub const KOPF: usize = 6;
/// Bit 0 im Merker-Byte.
const MERKER_SCHLUESSELBILD: u8 = 0x01;

/// Ein Raumschluessel. Nur wer den Link hat, hat ihn - der Server nie.
#[derive(Clone)]
pub struct Schluessel {
    /// Getrennte Schluessel je Spurart, damit ein Ton-Rahmen nie als
    /// Bild-Rahmen durchgeht (und umgekehrt).
    ton: Aes256Gcm,
    bild: Aes256Gcm,
    roh: [u8; 32],
}

/// Welche Art Spur - beide bekommen eigene Schluessel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Spur {
    Ton,
    Bild,
}

impl Schluessel {
    /// Aus 32 zufaelligen Bytes zwei Spur-Schluessel ableiten.
    pub fn aus_roh(roh: [u8; 32]) -> Schluessel {
        let ab = |zweck: &[u8]| -> Aes256Gcm {
            let hk = hkdf::Hkdf::<sha2::Sha256>::new(Some(b"freemeet-e2e-v1"), &roh);
            let mut k = [0u8; 32];
            // expand kann nur bei absurden Laengen scheitern - 32 Byte nie.
            hk.expand(zweck, &mut k).expect("32 Byte gehen immer");
            Aes256Gcm::new((&k).into())
        };
        Schluessel {
            ton: ab(b"ton"),
            bild: ab(b"bild"),
            roh,
        }
    }

    /// Einen neuen Zufallsschluessel wuerfeln (beim Anlegen des Meetings).
    pub fn neu() -> Schluessel {
        use rand::RngCore;
        let mut roh = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut roh);
        Schluessel::aus_roh(roh)
    }

    /// Fuer den Link: 43 Zeichen URL-sicheres Base64 ohne Fuellzeichen.
    pub fn als_text(&self) -> String {
        base64_url(&self.roh)
    }

    /// Aus dem Link zurueck. None = kein gueltiger Schluessel.
    pub fn aus_text(t: &str) -> Option<Schluessel> {
        let roh = base64_url_zurueck(t.trim())?;
        if roh.len() != 32 {
            return None;
        }
        let mut a = [0u8; 32];
        a.copy_from_slice(&roh);
        Some(Schluessel::aus_roh(a))
    }

    fn fuer(&self, spur: Spur) -> &Aes256Gcm {
        match spur {
            Spur::Ton => &self.ton,
            Spur::Bild => &self.bild,
        }
    }

    /// Einen Rahmen verschluesseln. `zaehler` muss je Spur streng steigen.
    pub fn schuetzen(
        &self,
        spur: Spur,
        zaehler: u32,
        schluesselbild: bool,
        klar: &[u8],
    ) -> Vec<u8> {
        let mut kopf = [0u8; KOPF];
        kopf[0] = KENNUNG;
        kopf[1] = if schluesselbild { MERKER_SCHLUESSELBILD } else { 0 };
        kopf[2..6].copy_from_slice(&zaehler.to_be_bytes());
        // 96-Bit-Zufallswert: Spurart + Zaehler. Beides zusammen kommt je
        // Schluessel nur EINMAL vor - genau das verlangt AES-GCM.
        let mut nonce = [0u8; 12];
        nonce[0] = match spur {
            Spur::Ton => 1,
            Spur::Bild => 2,
        };
        nonce[8..12].copy_from_slice(&zaehler.to_be_bytes());
        let geheim = self
            .fuer(spur)
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: klar,
                    aad: &kopf,
                },
            )
            .unwrap_or_default();
        let mut aus = Vec::with_capacity(KOPF + geheim.len());
        aus.extend_from_slice(&kopf);
        aus.extend_from_slice(&geheim);
        aus
    }

    /// Einen Rahmen entschluesseln. None = nicht unser Format, falscher
    /// Schluessel oder verfaelscht - in jedem Fall NICHT anzeigen.
    pub fn oeffnen(&self, spur: Spur, rahmen: &[u8]) -> Option<Vec<u8>> {
        if !ist_geschuetzt(rahmen) {
            return None;
        }
        let kopf = &rahmen[..KOPF];
        let zaehler = u32::from_be_bytes([rahmen[2], rahmen[3], rahmen[4], rahmen[5]]);
        let mut nonce = [0u8; 12];
        nonce[0] = match spur {
            Spur::Ton => 1,
            Spur::Bild => 2,
        };
        nonce[8..12].copy_from_slice(&zaehler.to_be_bytes());
        self.fuer(spur)
            .decrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &rahmen[KOPF..],
                    aad: kopf,
                },
            )
            .ok()
    }
}

// ------------------------------------------------------- H.264 (Bild) ----
//
// WARUM Bild anders behandelt wird als Ton:
//
// Der Medienserver ist KEIN reiner Weiterleiter. Er bekommt entpackte Bilder
// und setzt sie neu zu RTP-Paketen zusammen - dafuer MUSS er die Struktur
// des Bildes verstehen (NAL-Einheiten, STAP-A). Gemessen am 07.08.2026:
// verschluesselt man den ganzen Rahmen, scheitert er mit
// "NaluTypeIsNotHandled" und beim Gegenueber kommt kein Bild an.
//
// Deshalb bleibt die STRUKTUR sichtbar und nur der INHALT wird verschluesselt:
//   * Startcodes und das Kopfbyte jeder NAL-Einheit bleiben im Klartext,
//   * SPS (7), PPS (8) und AUD (9) bleiben ganz unangetastet - darin steht
//     nur, wie gross das Bild ist, kein Bildinhalt; der Packer braucht sie,
//   * alles andere wird verschluesselt.
//
// Denselben Kompromiss geht SFrame bei H.264 ein: der Server sieht die
// Rahmenstruktur und die Groessen, aber KEINE Inhalte.
//
// Zusaetzlich noetig: der Geheimtext darf zufaellig "00 00 01" enthalten -
// das waere ein falscher Startcode und wuerde das Bild zerlegen. Deshalb
// wird derselbe Schutz eingesetzt, den H.264 selbst benutzt (eine 0x03
// dazwischen, "emulation prevention").

/// Wo faengt die naechste NAL-Einheit an? Liefert (Beginn des Startcodes,
/// Beginn der Daten).
fn naechster_startcode(daten: &[u8], ab: usize) -> Option<(usize, usize)> {
    let mut i = ab;
    while i + 3 <= daten.len() {
        if daten[i] == 0 && daten[i + 1] == 0 {
            if daten[i + 2] == 1 {
                return Some((i, i + 3));
            }
            if i + 4 <= daten.len() && daten[i + 2] == 0 && daten[i + 3] == 1 {
                return Some((i, i + 4));
            }
        }
        i += 1;
    }
    None
}

/// Einen Annex-B-Rahmen in seine NAL-Einheiten zerlegen: (Startcode-Beginn,
/// Daten-Beginn, Daten-Ende).
pub fn nal_einheiten(daten: &[u8]) -> Vec<(usize, usize, usize)> {
    let mut aus = Vec::new();
    let mut suche = 0usize;
    let mut offen: Option<(usize, usize)> = None;
    while let Some((sc, db)) = naechster_startcode(daten, suche) {
        if let Some((osc, odb)) = offen {
            aus.push((osc, odb, sc));
        }
        offen = Some((sc, db));
        suche = db;
    }
    if let Some((osc, odb)) = offen {
        aus.push((osc, odb, daten.len()));
    }
    aus
}

/// Muss diese NAL-Einheit im Klartext bleiben?
///
/// SPS/PPS beschreiben nur Groesse und Kodierparameter - kein Bildinhalt -
/// und der Packer im Server braucht sie. AUD ist ein reiner Trenner.
fn nal_bleibt_klar(typ: u8) -> bool {
    matches!(typ, 7 | 8 | 9)
}

/// Den Schutz gegen falsche Startcodes einfuegen (wie H.264 selbst).
fn epb_ein(daten: &[u8]) -> Vec<u8> {
    let mut aus = Vec::with_capacity(daten.len() + daten.len() / 64 + 4);
    let mut nullen = 0usize;
    for &b in daten {
        if nullen >= 2 && b <= 3 {
            aus.push(3);
            nullen = 0;
        }
        aus.push(b);
        if b == 0 {
            nullen += 1;
        } else {
            nullen = 0;
        }
    }
    aus
}

/// Den Schutz wieder herausnehmen.
fn epb_raus(daten: &[u8]) -> Vec<u8> {
    let mut aus = Vec::with_capacity(daten.len());
    let mut nullen = 0usize;
    let mut i = 0usize;
    while i < daten.len() {
        let b = daten[i];
        if nullen >= 2 && b == 3 && i + 1 < daten.len() && daten[i + 1] <= 3 {
            nullen = 0;
            i += 1;
            continue;
        }
        aus.push(b);
        if b == 0 {
            nullen += 1;
        } else {
            nullen = 0;
        }
        i += 1;
    }
    aus
}

impl Schluessel {
    fn bild_nonce(&self, zaehler: u32, nal_nr: u8) -> [u8; 12] {
        let mut n = [0u8; 12];
        n[0] = 2; // Bildspur
        n[7] = nal_nr; // je NAL eigener Wert - sonst waere der Zufallswert doppelt
        n[8..12].copy_from_slice(&zaehler.to_be_bytes());
        n
    }

    /// Einen H.264-Rahmen schuetzen: Struktur bleibt, Inhalt wird geheim.
    pub fn schuetzen_h264(&self, zaehler: u32, rahmen: &[u8]) -> Vec<u8> {
        let teile = nal_einheiten(rahmen);
        if teile.is_empty() {
            return rahmen.to_vec(); // kein Annex-B: unveraendert lassen
        }
        let mut aus = Vec::with_capacity(rahmen.len() + teile.len() * 24);
        for (nr, (sc, db, ende)) in teile.iter().enumerate() {
            let kopf_byte = rahmen[*db];
            let typ = kopf_byte & 0x1f;
            // Startcode und Kopfbyte immer im Klartext.
            aus.extend_from_slice(&rahmen[*sc..*db + 1]);
            let inhalt = &rahmen[*db + 1..*ende];
            if nal_bleibt_klar(typ) || inhalt.is_empty() {
                aus.extend_from_slice(inhalt);
                continue;
            }
            let nr8 = (nr & 0xff) as u8;
            // Zaehler und NAL-Nummer gehen in die Pruefsumme ein - wer sie
            // vertauscht, macht den Rahmen ungueltig.
            let aad = [kopf_byte, nr8, (zaehler >> 24) as u8, (zaehler >> 16) as u8,
                       (zaehler >> 8) as u8, zaehler as u8];
            let geheim = self
                .bild
                .encrypt(
                    Nonce::from_slice(&self.bild_nonce(zaehler, nr8)),
                    Payload { msg: inhalt, aad: &aad },
                )
                .unwrap_or_default();
            let mut roh = Vec::with_capacity(5 + geheim.len());
            roh.push(nr8);
            roh.extend_from_slice(&zaehler.to_be_bytes());
            roh.extend_from_slice(&geheim);
            aus.extend_from_slice(&epb_ein(&roh));
        }
        aus
    }

    /// Einen geschuetzten H.264-Rahmen wieder oeffnen.
    pub fn oeffnen_h264(&self, rahmen: &[u8]) -> Option<Vec<u8>> {
        let teile = nal_einheiten(rahmen);
        if teile.is_empty() {
            return None;
        }
        let mut aus = Vec::with_capacity(rahmen.len());
        for (sc, db, ende) in teile {
            let kopf_byte = rahmen[db];
            let typ = kopf_byte & 0x1f;
            aus.extend_from_slice(&rahmen[sc..db + 1]);
            let inhalt = &rahmen[db + 1..ende];
            if nal_bleibt_klar(typ) || inhalt.is_empty() {
                aus.extend_from_slice(inhalt);
                continue;
            }
            let roh = epb_raus(inhalt);
            if roh.len() < 5 + 16 {
                return None;
            }
            let nr8 = roh[0];
            let zaehler = u32::from_be_bytes([roh[1], roh[2], roh[3], roh[4]]);
            let aad = [kopf_byte, nr8, roh[1], roh[2], roh[3], roh[4]];
            let klar = self
                .bild
                .decrypt(
                    Nonce::from_slice(&self.bild_nonce(zaehler, nr8)),
                    Payload { msg: &roh[5..], aad: &aad },
                )
                .ok()?;
            aus.extend_from_slice(&klar);
        }
        Some(aus)
    }
}

/// Sieht das nach einem geschuetzten Rahmen aus? (Auch der Server fragt das.)
pub fn ist_geschuetzt(rahmen: &[u8]) -> bool {
    rahmen.len() > KOPF + 16 && rahmen[0] == KENNUNG
}

/// Steckt ein Schluesselbild darin? Das darf der Server lesen - er braucht
/// es, um Qualitaetsstufen umzuschalten.
pub fn ist_schluesselbild(rahmen: &[u8]) -> Option<bool> {
    if !ist_geschuetzt(rahmen) {
        return None;
    }
    Some(rahmen[1] & MERKER_SCHLUESSELBILD != 0)
}

/// Laufende Nummer eines Rahmens.
pub fn zaehler_von(rahmen: &[u8]) -> Option<u32> {
    if !ist_geschuetzt(rahmen) {
        return None;
    }
    Some(u32::from_be_bytes([rahmen[2], rahmen[3], rahmen[4], rahmen[5]]))
}

/// Merkt sich, welche Nummern schon da waren - gegen Wiedereinspielen.
///
/// Ein Angreifer mit Zugriff auf den Server koennte sonst alte Rahmen erneut
/// einspeisen. Die Pruefsumme waere gueltig, das Bild aber gelogen.
#[derive(Default)]
pub struct Wache {
    hoechste: u32,
    /// Bitfeld der letzten 64 Nummern unter `hoechste` (Netz darf umsortieren).
    fenster: u64,
    gesehen: bool,
}

impl Wache {
    /// True = neu und in Ordnung. False = Wiederholung oder zu alt.
    pub fn pruefen(&mut self, zaehler: u32) -> bool {
        if !self.gesehen {
            self.gesehen = true;
            self.hoechste = zaehler;
            return true;
        }
        if zaehler > self.hoechste {
            let schritt = zaehler - self.hoechste;
            self.fenster = if schritt >= 64 {
                0
            } else {
                (self.fenster << schritt) | (1u64 << (schritt - 1))
            };
            self.hoechste = zaehler;
            return true;
        }
        let zurueck = self.hoechste - zaehler;
        if zurueck == 0 || zurueck > 64 {
            return false; // genau die hoechste, oder laengst aus dem Fenster
        }
        let bit = 1u64 << (zurueck - 1);
        if self.fenster & bit != 0 {
            return false;
        }
        self.fenster |= bit;
        true
    }
}

// ------------------------------------------------------------- Base64 -----
// URL-sicher, ohne Fuellzeichen. Selbst gerechnet, damit keine weitere
// Abhaengigkeit noetig ist.

const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

pub fn base64_url(daten: &[u8]) -> String {
    let mut aus = String::new();
    for stueck in daten.chunks(3) {
        let b = [
            stueck[0],
            *stueck.get(1).unwrap_or(&0),
            *stueck.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        let zeichen = stueck.len() + 1;
        for i in 0..zeichen {
            aus.push(ABC[((n >> (18 - i * 6)) & 0x3f) as usize] as char);
        }
    }
    aus
}

pub fn base64_url_zurueck(text: &str) -> Option<Vec<u8>> {
    let wert = |c: u8| -> Option<u32> { ABC.iter().position(|x| *x == c).map(|p| p as u32) };
    let roh: Vec<u8> = text.bytes().filter(|c| *c != b'=').collect();
    let mut aus = Vec::new();
    for stueck in roh.chunks(4) {
        if stueck.len() < 2 {
            return None;
        }
        let mut n = 0u32;
        for (i, c) in stueck.iter().enumerate() {
            n |= wert(*c)? << (18 - i * 6);
        }
        for i in 0..stueck.len() - 1 {
            aus.push(((n >> (16 - i * 8)) & 0xff) as u8);
        }
    }
    Some(aus)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ein Rahmen wie ihn der Kodierer liefert: SPS, PPS, dann ein Bild.
    fn beispielrahmen() -> Vec<u8> {
        let mut f = Vec::new();
        f.extend_from_slice(&[0, 0, 0, 1, 0x67, 0x42, 0x00, 0x1f, 0x96, 0x54]); // SPS (7)
        f.extend_from_slice(&[0, 0, 0, 1, 0x68, 0xce, 0x3c, 0x80]); // PPS (8)
        f.extend_from_slice(&[0, 0, 0, 1, 0x65]); // IDR (5)
        f.extend((0..400u32).map(|i| (i * 13 % 256) as u8));
        f.extend_from_slice(&[0, 0, 1, 0x41]); // Nicht-IDR (1), 3-Byte-Startcode
        f.extend((0..250u32).map(|i| (i * 29 % 256) as u8));
        f
    }

    #[test]
    fn h264_hin_und_zurueck() {
        let k = Schluessel::neu();
        let f = beispielrahmen();
        let g = k.schuetzen_h264(11, &f);
        assert_ne!(g, f, "nichts verschluesselt");
        assert_eq!(k.oeffnen_h264(&g).as_deref(), Some(&f[..]), "kommt nicht heil zurueck");
    }

    /// DER Punkt, an dem es beim ersten Anlauf gescheitert ist: der Server
    /// muss die Struktur weiter lesen koennen.
    #[test]
    fn die_struktur_bleibt_lesbar() {
        let k = Schluessel::neu();
        let f = beispielrahmen();
        let g = k.schuetzen_h264(3, &f);
        let vorher: Vec<u8> = nal_einheiten(&f).iter().map(|(_, d, _)| f[*d] & 0x1f).collect();
        let nachher: Vec<u8> = nal_einheiten(&g).iter().map(|(_, d, _)| g[*d] & 0x1f).collect();
        assert_eq!(vorher, nachher, "NAL-Typen haben sich geaendert");
        assert_eq!(vorher, vec![7, 8, 5, 1]);
    }

    /// SPS und PPS muessen WORTGLEICH bleiben - der Packer im Server liest sie.
    #[test]
    fn sps_und_pps_bleiben_unangetastet() {
        let k = Schluessel::neu();
        let f = beispielrahmen();
        let g = k.schuetzen_h264(4, &f);
        for (sc, db, ende) in nal_einheiten(&f) {
            let typ = f[db] & 0x1f;
            if typ == 7 || typ == 8 {
                let stueck = &f[sc..ende];
                assert!(
                    g.windows(stueck.len()).any(|w| w == stueck),
                    "NAL-Typ {} wurde veraendert",
                    typ
                );
            }
        }
    }

    /// Der Bildinhalt darf NICHT mehr im Rahmen stehen.
    #[test]
    fn h264_inhalt_ist_wirklich_weg() {
        let k = Schluessel::neu();
        let mut f = vec![0, 0, 0, 1, 0x65];
        let geheim: Vec<u8> = b"GEHEIMERBILDINHALT".repeat(6);
        f.extend_from_slice(&geheim);
        let g = k.schuetzen_h264(1, &f);
        assert!(
            !g.windows(geheim.len()).any(|w| w == &geheim[..]),
            "der Bildinhalt liegt offen im Rahmen"
        );
    }

    /// Falsche Startcodes wuerden das Bild zerlegen. Das darf NIE passieren -
    /// auch nicht, wenn der Geheimtext zufaellig 00 00 01 enthaelt.
    #[test]
    fn es_entstehen_keine_falschen_startcodes() {
        let k = Schluessel::neu();
        for runde in 0..300u32 {
            let mut f = vec![0, 0, 0, 1, 0x41];
            f.extend((0..64u32).map(|i| ((i * runde + 7) % 256) as u8));
            let g = k.schuetzen_h264(runde, &f);
            // Genau EIN Startcode - naemlich der am Anfang.
            assert_eq!(
                nal_einheiten(&g).len(),
                1,
                "Runde {}: es sind falsche Startcodes entstanden",
                runde
            );
            assert_eq!(k.oeffnen_h264(&g).as_deref(), Some(&f[..]), "Runde {}", runde);
        }
    }

    #[test]
    fn h264_fremder_schluessel_oeffnet_nichts() {
        let a = Schluessel::neu();
        let b = Schluessel::neu();
        let g = a.schuetzen_h264(2, &beispielrahmen());
        assert!(b.oeffnen_h264(&g).is_none());
    }

    #[test]
    fn h264_verfaelschung_faellt_auf() {
        let k = Schluessel::neu();
        let g = k.schuetzen_h264(6, &beispielrahmen());
        // Eine Stelle mitten im verschluesselten Teil umdrehen.
        let mut kaputt = g.clone();
        let mitte = g.len() - 40;
        kaputt[mitte] ^= 0x01;
        assert!(k.oeffnen_h264(&kaputt).is_none(), "Verfaelschung blieb unbemerkt");
    }

    /// Das Schluesselbild bleibt am NAL-Typ 5 erkennbar - der Server findet
    /// es also wieder ohne unser Zutun.
    #[test]
    fn schluesselbild_bleibt_erkennbar() {
        let k = Schluessel::neu();
        let g = k.schuetzen_h264(8, &beispielrahmen());
        let hat_idr = nal_einheiten(&g).iter().any(|(_, d, _)| g[*d] & 0x1f == 5);
        assert!(hat_idr, "der Server kann das Schluesselbild nicht mehr finden");
    }

    #[test]
    fn schutz_vor_falschen_startcodes_ist_umkehrbar() {
        for muster in [
            vec![0u8, 0, 1, 2, 3],
            vec![0, 0, 0, 0, 0, 0],
            vec![0, 0, 3, 0, 0, 1],
            vec![1, 2, 3, 4, 5],
            vec![0, 0, 2],
        ] {
            let ein = epb_ein(&muster);
            assert_eq!(epb_raus(&ein), muster, "Muster {:?}", muster);
        }
    }

    #[test]
    fn hin_und_zurueck() {
        let k = Schluessel::neu();
        let klar = b"Guten Tag, das ist ein Bildrahmen.";
        let g = k.schuetzen(Spur::Bild, 7, true, klar);
        assert!(ist_geschuetzt(&g));
        assert_eq!(ist_schluesselbild(&g), Some(true));
        assert_eq!(zaehler_von(&g), Some(7));
        assert_eq!(k.oeffnen(Spur::Bild, &g).as_deref(), Some(&klar[..]));
    }

    /// Der Kern der Sache: der Klartext darf NICHT im Rahmen stehen.
    #[test]
    fn der_klartext_steht_nicht_drin() {
        let k = Schluessel::neu();
        let klar = b"GEHEIMESWORT";
        let g = k.schuetzen(Spur::Ton, 1, false, klar);
        assert!(
            g.windows(klar.len()).all(|f| f != klar),
            "der Klartext liegt offen im Rahmen"
        );
    }

    /// Ein anderer Schluessel darf NICHTS oeffnen - das ist der ganze Sinn.
    #[test]
    fn fremder_schluessel_oeffnet_nichts() {
        let a = Schluessel::neu();
        let b = Schluessel::neu();
        let g = a.schuetzen(Spur::Bild, 3, false, b"Inhalt");
        assert!(b.oeffnen(Spur::Bild, &g).is_none());
    }

    /// Ton- und Bildschluessel sind getrennt - ein Tonrahmen darf nie als
    /// Bildrahmen durchgehen.
    #[test]
    fn spuren_sind_getrennt() {
        let k = Schluessel::neu();
        let g = k.schuetzen(Spur::Ton, 5, false, b"Ton");
        assert!(k.oeffnen(Spur::Bild, &g).is_none());
        assert!(k.oeffnen(Spur::Ton, &g).is_some());
    }

    /// Jede Aenderung am Rahmen muss auffallen - auch am KLARTEXT-Kopf.
    /// Sonst koennte der Server den Schluesselbild-Merker faelschen.
    #[test]
    fn jede_verfaelschung_faellt_auf() {
        let k = Schluessel::neu();
        let g = k.schuetzen(Spur::Bild, 9, true, b"Ein laengerer Inhalt zum Pruefen");
        for stelle in [1usize, 2, 5, KOPF, KOPF + 3, g.len() - 1] {
            let mut kaputt = g.clone();
            kaputt[stelle] ^= 0x01;
            assert!(
                k.oeffnen(Spur::Bild, &kaputt).is_none(),
                "Aenderung an Stelle {} blieb unbemerkt",
                stelle
            );
        }
    }

    #[test]
    fn alte_rahmen_werden_nicht_angefasst() {
        let k = Schluessel::neu();
        // Ein unverschluesselter H.264-Rahmen faengt mit 00 00 00 01 an.
        let alt = [0u8, 0, 0, 1, 0x65, 0x88, 0x84, 0x21, 0x00, 0x11, 0x22, 0x33,
                   0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee];
        assert!(!ist_geschuetzt(&alt));
        assert_eq!(ist_schluesselbild(&alt), None);
        assert!(k.oeffnen(Spur::Bild, &alt).is_none());
    }

    #[test]
    fn schluessel_ueberlebt_den_link() {
        let k = Schluessel::neu();
        let t = k.als_text();
        assert_eq!(t.len(), 43, "43 Zeichen erwartet, sind {}", t.len());
        assert!(!t.contains('+') && !t.contains('/') && !t.contains('='), "nicht linksicher: {}", t);
        let zurueck = Schluessel::aus_text(&t).expect("muss zurueckkommen");
        let g = k.schuetzen(Spur::Ton, 2, false, b"Probe");
        assert_eq!(zurueck.oeffnen(Spur::Ton, &g).as_deref(), Some(&b"Probe"[..]));
    }

    #[test]
    fn unsinniger_schluesseltext_wird_abgelehnt() {
        assert!(Schluessel::aus_text("").is_none());
        assert!(Schluessel::aus_text("zu-kurz").is_none());
        assert!(Schluessel::aus_text(&"A".repeat(43)).is_some());
        assert!(Schluessel::aus_text("!!!!").is_none());
    }

    #[test]
    fn base64_haelt_beliebige_daten_aus() {
        for laenge in [1usize, 2, 3, 4, 31, 32, 33, 100] {
            let daten: Vec<u8> = (0..laenge).map(|i| (i * 7 % 256) as u8).collect();
            let t = base64_url(&daten);
            assert_eq!(base64_url_zurueck(&t).as_deref(), Some(&daten[..]), "Laenge {}", laenge);
        }
    }

    #[test]
    fn wache_laesst_neues_durch_und_wiederholungen_nicht() {
        let mut w = Wache::default();
        assert!(w.pruefen(10));
        assert!(w.pruefen(11));
        assert!(!w.pruefen(11), "Wiederholung durchgelassen");
        assert!(!w.pruefen(10), "Wiederholung durchgelassen");
        // Das Netz darf umsortieren: eine luecke spaeter nachreichen ist ok.
        assert!(w.pruefen(13));
        assert!(w.pruefen(12), "verspaetetes Paket faelschlich verworfen");
        assert!(!w.pruefen(12));
        // 1 liegt bei Hoechststand 13 noch INNERHALB des 64er-Fensters und
        // ist damit ein legitim verspaetetes Paket - es MUSS durch.
        assert!(w.pruefen(1), "verspaetetes Paket im Fenster faelschlich verworfen");
        assert!(!w.pruefen(1), "dasselbe zweimal durchgelassen");
        // Erst weit ausserhalb des Fensters wird abgewiesen.
        assert!(w.pruefen(200));
        assert!(!w.pruefen(13), "uralter Rahmen (187 zurueck) durchgelassen");
    }

    #[test]
    fn wache_kommt_mit_grossen_spruengen_klar() {
        let mut w = Wache::default();
        assert!(w.pruefen(1));
        assert!(w.pruefen(1_000_000));
        assert!(!w.pruefen(500), "uralter Rahmen durchgelassen");
        assert!(w.pruefen(1_000_001));
    }

    /// Der Kopf ist genau 6 Byte - der Zuwachs je Rahmen sind 6 + 16 Byte
    /// Pruefsumme. Bei 30 Bildern je Sekunde also rund 660 Byte/s. Das darf
    /// nicht unbemerkt wachsen.
    #[test]
    fn der_aufschlag_bleibt_klein() {
        let k = Schluessel::neu();
        let klar = vec![0u8; 5000];
        let g = k.schuetzen(Spur::Bild, 1, false, &klar);
        assert_eq!(g.len(), klar.len() + KOPF + 16, "Aufschlag hat sich geaendert");
    }
}
