//! Address book: who did we connect to, when, and how do we get back there.
//!
//! TeamViewer keeps that list in its cloud account. This one lives in a plain
//! file next to the machine identity, so it works without any account at all -
//! and it can be synced later without changing the format.
//!
//! Saved passwords are encrypted with a key derived from this machine's
//! identity secret (`identity.txt`). That secret never leaves the machine and
//! is already the thing that makes this installation "us", so anybody able to
//! read it owns the installation anyway - but a stolen `partners.json` on its
//! own is worthless.

use std::time::{SystemTime, UNIX_EPOCH};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::crypto::random_bytes;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Partner {
    pub id: String,
    /// What the user called it. Empty = show the plain ID.
    #[serde(default)]
    pub name: String,
    /// Last successful connection, unix seconds.
    #[serde(default)]
    pub last: u64,
    /// How often we connected.
    #[serde(default)]
    pub count: u32,
    /// Total time connected, seconds.
    #[serde(default)]
    pub seconds: u64,
    /// Pinned entries stay on top.
    #[serde(default)]
    pub favorite: bool,
    /// Encrypted password (hex), only present if the user asked for it.
    #[serde(default)]
    pub secret: Option<String>,
    /// Ordner/Gruppe, in der das Geraet steht. Leer = "Alle".
    #[serde(default)]
    pub group: String,
    /// Freie Notiz zum Geraet.
    #[serde(default)]
    pub note: String,
    /// Wann dieser Eintrag zuletzt geaendert wurde (unix Sekunden).
    /// Der Abgleich mit dem Konto entscheidet damit, welche Fassung neuer ist.
    #[serde(default)]
    pub at: u64,
    /// Geloescht - bleibt als Grabstein liegen, damit ein auf einem PC
    /// entferntes Geraet nicht beim naechsten Abgleich wieder auftaucht.
    #[serde(default)]
    pub deleted: bool,
}

impl Partner {
    /// What the list shows.
    pub fn label(&self) -> String {
        if self.name.trim().is_empty() {
            pretty_id(&self.id)
        } else {
            self.name.clone()
        }
    }

    /// "vor 3 Minuten", "gestern", ...
    pub fn ago(&self) -> String {
        if self.last == 0 {
            return "noch nie".to_string();
        }
        let d = now().saturating_sub(self.last);
        match d {
            0..=59 => "gerade eben".to_string(),
            60..=3599 => format!("vor {} Min.", d / 60),
            3600..=86399 => format!("vor {} Std.", d / 3600),
            86400..=172799 => "gestern".to_string(),
            _ => format!("vor {} Tagen", d / 86400),
        }
    }

    pub fn total(&self) -> String {
        let s = self.seconds;
        if s < 60 {
            format!("{} s", s)
        } else if s < 3600 {
            format!("{} Min.", s / 60)
        } else {
            format!("{:.1} Std.", s as f64 / 3600.0)
        }
    }
}

/// Suche: Gross-/Kleinschreibung und Trennzeichen (Leerzeichen, Bindestrich,
/// Unterstrich, Punkt) sind egal - "flei one" findet "FLEI-ONE".
pub fn search_norm(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, ' ' | '-' | '_' | '.'))
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// 497628420 -> "497 628 420"
pub fn pretty_id(id: &str) -> String {
    match id.len() {
        9 => format!("{} {} {}", &id[0..3], &id[3..6], &id[6..9]),
        10 => format!(
            "{} {} {} {}",
            &id[0..1],
            &id[1..4],
            &id[4..7],
            &id[7..10]
        ),
        _ => id.to_string(),
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Serialize, Deserialize, Default)]
pub struct Book {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub entries: Vec<Partner>,
}

/// Never grow without bound - the list is a convenience, not an archive.
const MAX_ENTRIES: usize = 200;

impl Book {
    pub(crate) fn path() -> std::path::PathBuf {
        crate::ident::config_dir().join("partners.json")
    }

    /// Die Sicherheitskopie der letzten Fassung.
    fn backup_path() -> std::path::PathBuf {
        crate::ident::config_dir().join("partners.bak.json")
    }

    /// Wie viele echte (nicht geloeschte) Eintraege stehen gerade in der Datei?
    /// Wird gebraucht, um zu erkennen, dass ein Schreibvorgang eine gefuellte
    /// Liste durch eine leere ersetzen wuerde.
    fn on_disk() -> Option<Self> {
        let s = std::fs::read_to_string(Self::path()).ok()?;
        serde_json::from_str(&s).ok()
    }

    pub fn load() -> Self {
        // Kaputte Datei? Dann lieber die Sicherungskopie als eine leere Liste -
        // ein Adressbuch verschwindet nicht, weil ein Byte falsch steht.
        if let Some(b) = Self::on_disk() {
            if !b.entries.is_empty() {
                return b;
            }
        }
        if let Ok(s) = std::fs::read_to_string(Self::backup_path()) {
            if let Ok(b) = serde_json::from_str::<Self>(&s) {
                if !b.entries.is_empty() {
                    return b;
                }
            }
        }
        Self::on_disk().unwrap_or_default()
    }

    pub fn save(&self) {
        let dir = crate::ident::config_dir();
        let _ = std::fs::create_dir_all(&dir);

        // NIE eine gefuellte Liste durch eine leere ersetzen. Loeschen laeuft
        // ueber Grabsteine (deleted = true), die Eintraege bleiben also immer
        // stehen - eine leere Liste kann daher nur ein Fehler sein.
        if self.entries.is_empty() {
            if let Some(old) = Self::on_disk() {
                if !old.entries.is_empty() {
                    return;
                }
            }
        }

        // Vorherige Fassung als Sicherungskopie behalten.
        if std::fs::metadata(Self::path()).is_ok() {
            let _ = std::fs::copy(Self::path(), Self::backup_path());
        }

        if let Ok(s) = serde_json::to_string_pretty(self) {
            let tmp = dir.join("partners.json.tmp");
            if std::fs::write(&tmp, s).is_ok() {
                let _ = std::fs::rename(&tmp, Self::path());
            }
        }
    }

    /// Alle sichtbaren Eintraege (ohne Grabsteine).
    pub fn live(&self) -> impl Iterator<Item = &Partner> {
        self.entries.iter().filter(|p| !p.deleted)
    }

    /// Favourites first, then most recently used.
    pub fn sorted(&self) -> Vec<Partner> {
        let mut v: Vec<Partner> = self.live().cloned().collect();
        v.sort_by(|a, b| b.favorite.cmp(&a.favorite).then(b.last.cmp(&a.last)));
        v
    }

    pub fn get(&self, id: &str) -> Option<&Partner> {
        self.entries.iter().find(|p| p.id == id && !p.deleted)
    }

    fn entry(&mut self, id: &str) -> &mut Partner {
        if let Some(i) = self.entries.iter().position(|p| p.id == id) {
            let e = &mut self.entries[i];
            // wer einen Eintrag anfasst, holt ihn zurueck
            e.deleted = false;
            e.at = now();
            return &mut self.entries[i];
        }
        self.entries.push(Partner {
            id: id.to_string(),
            at: now(),
            ..Default::default()
        });
        let n = self.entries.len() - 1;
        &mut self.entries[n]
    }

    /// A session started. `remember` stores the password, `None` clears it.
    pub fn started(&mut self, id: &str, password: &str, remember: bool) {
        let secret = if remember && !password.is_empty() {
            protect(password)
        } else {
            None
        };
        {
            let e = self.entry(id);
            e.last = now();
            e.count += 1;
            if remember {
                if secret.is_some() {
                    e.secret = secret;
                }
            } else {
                e.secret = None;
            }
        }
        self.trim();
        self.save();
    }

    /// A session ended after `secs` seconds.
    pub fn ended(&mut self, id: &str, secs: u64) {
        {
            let e = self.entry(id);
            e.seconds += secs;
        }
        self.save();
    }

    pub fn rename(&mut self, id: &str, name: &str) {
        self.entry(id).name = name.trim().to_string();
        self.save();
    }

    pub fn toggle_favorite(&mut self, id: &str) {
        let e = self.entry(id);
        e.favorite = !e.favorite;
        self.save();
    }

    /// Entfernen heisst: als geloescht markieren. Der Grabstein wandert beim
    /// naechsten Abgleich zum Konto und raeumt das Geraet auch auf den anderen
    /// Rechnern weg. Nach 60 Tagen faellt er beim Aufraeumen selbst heraus.
    pub fn remove(&mut self, id: &str) {
        if let Some(e) = self.entries.iter_mut().find(|p| p.id == id) {
            e.deleted = true;
            e.at = now();
            e.secret = None;
            e.favorite = false;
        }
        self.save();
    }

    /// Ordner setzen (leer = kein Ordner).
    pub fn set_group(&mut self, id: &str, group: &str) {
        self.entry(id).group = group.trim().to_string();
        self.save();
    }

    /// Notiz setzen.
    pub fn set_note(&mut self, id: &str, note: &str) {
        self.entry(id).note = note.trim().to_string();
        self.save();
    }

    /// Passwort hinterlegen (None loescht es).
    pub fn set_password(&mut self, id: &str, password: Option<&str>) {
        let sealed = match password {
            Some(pw) if !pw.is_empty() => protect(pw),
            _ => None,
        };
        self.entry(id).secret = sealed;
        self.save();
    }

    /// Alle vorhandenen Ordner, alphabetisch.
    pub fn groups(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .live()
            .map(|p| p.group.trim().to_string())
            .filter(|g| !g.is_empty())
            .collect();
        v.sort();
        v.dedup();
        v
    }

    /// Decrypted password, if one was stored.
    pub fn password(&self, id: &str) -> Option<String> {
        self.get(id).and_then(|p| p.secret.as_ref()).and_then(|s| unprotect(s))
    }

    fn trim(&mut self) {
        // alte Grabsteine zuerst - die braucht nach zwei Monaten niemand mehr
        let cut = now().saturating_sub(60 * 24 * 3600);
        self.entries.retain(|p| !p.deleted || p.at > cut);
        if self.entries.len() <= MAX_ENTRIES {
            return;
        }
        let mut v = std::mem::take(&mut self.entries);
        v.sort_by(|a, b| {
            a.deleted
                .cmp(&b.deleted)
                .then(b.favorite.cmp(&a.favorite))
                .then(b.last.cmp(&a.last))
        });
        v.drain(MAX_ENTRIES..);
        self.entries = v;
    }

    // ------------------------------------------------------ Konto-Abgleich

    /// Was zum Konto hochgeht: alles ausser den gespeicherten Passwoertern.
    /// Die sind mit der Kennung DIESES Rechners verschluesselt und waeren
    /// woanders ohnehin nicht zu entschluesseln - also bleiben sie hier.
    pub fn to_sync(&self) -> Vec<SyncDevice> {
        self.entries
            .iter()
            .map(|p| SyncDevice {
                id: p.id.clone(),
                name: p.name.clone(),
                group: p.group.clone(),
                note: p.note.clone(),
                favorite: p.favorite,
                last: p.last,
                count: p.count,
                seconds: p.seconds,
                at: if p.at == 0 { p.last } else { p.at },
                deleted: p.deleted,
            })
            .collect()
    }

    /// Uebernimmt die Fassung des Kontos, wo sie neuer ist. Gibt zurueck, ob
    /// sich lokal etwas geaendert hat.
    pub fn merge_remote(&mut self, remote: &[SyncDevice]) -> bool {
        let mut changed = false;
        for r in remote {
            if r.id.is_empty() {
                continue;
            }
            match self.entries.iter_mut().find(|p| p.id == r.id) {
                Some(mine) => {
                    let mine_at = if mine.at == 0 { mine.last } else { mine.at };
                    if r.at <= mine_at {
                        continue;
                    }
                    mine.name = r.name.clone();
                    mine.group = r.group.clone();
                    mine.note = r.note.clone();
                    mine.favorite = r.favorite;
                    mine.last = mine.last.max(r.last);
                    mine.count = mine.count.max(r.count);
                    mine.seconds = mine.seconds.max(r.seconds);
                    mine.at = r.at;
                    if r.deleted != mine.deleted {
                        mine.deleted = r.deleted;
                        if r.deleted {
                            mine.secret = None;
                        }
                    }
                    changed = true;
                }
                None => {
                    self.entries.push(Partner {
                        id: r.id.clone(),
                        name: r.name.clone(),
                        group: r.group.clone(),
                        note: r.note.clone(),
                        favorite: r.favorite,
                        last: r.last,
                        count: r.count,
                        seconds: r.seconds,
                        at: r.at,
                        deleted: r.deleted,
                        secret: None,
                    });
                    changed = true;
                }
            }
        }
        if changed {
            self.trim();
            self.save();
        }
        changed
    }
}

/// Ein Geraet, wie es zwischen Rechner und Konto hin und her geht.
/// Bewusst ohne Passwortfeld - siehe `Book::to_sync`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct SyncDevice {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub group: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub favorite: bool,
    #[serde(default)]
    pub last: u64,
    #[serde(default)]
    pub count: u32,
    #[serde(default)]
    pub seconds: u64,
    #[serde(default)]
    pub at: u64,
    #[serde(default)]
    pub deleted: bool,
}

// ------------------------------------------------------------- encryption --

/// AES-256-GCM with a key derived from the machine identity. Format:
/// hex(12 byte nonce || ciphertext).
fn book_key() -> [u8; 32] {
    let secret = crate::ident::load_or_create_secret();
    let hk = Hkdf::<Sha256>::new(None, secret.as_bytes());
    let mut key = [0u8; 32];
    hk.expand(b"freeviewer-v1 partners", &mut key)
        .expect("hkdf expand");
    key
}

fn protect(plain: &str) -> Option<String> {
    let aead = Aes256Gcm::new_from_slice(&book_key()).ok()?;
    let nonce_bytes = random_bytes(12);
    let ct = aead
        .encrypt(Nonce::from_slice(&nonce_bytes), plain.as_bytes())
        .ok()?;
    let mut out = nonce_bytes;
    out.extend_from_slice(&ct);
    Some(hex::encode(out))
}

fn unprotect(stored: &str) -> Option<String> {
    let raw = hex::decode(stored).ok()?;
    if raw.len() < 13 {
        return None;
    }
    let aead = Aes256Gcm::new_from_slice(&book_key()).ok()?;
    let pt = aead.decrypt(Nonce::from_slice(&raw[..12]), &raw[12..]).ok()?;
    String::from_utf8(pt).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Eigener Ordner nur fuer diesen Test.
    fn eigener_ordner(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("fv-book-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&d);
        crate::ident::set_test_config_dir(d.clone());
        d
    }

    /// Der Ausloeser des Datenverlusts vom 31.07.2026: ein Testlauf hat ueber
    /// merge_remote() -> save() das echte Adressbuch ueberschrieben.
    #[test]
    fn ein_test_schreibt_nie_in_die_echte_konfiguration() {
        eigener_ordner("isoliert");
        let mut b = Book::default();
        b.merge_remote(&[dev("111111111", "Neu", 200)]);
        assert!(Book::path().starts_with(std::env::temp_dir()));
        if let Some(m) = crate::ident::machine_config_dir() {
            assert!(!Book::path().starts_with(m));
        }
        assert!(!Book::path().starts_with(crate::ident::user_config_dir()));
    }

    /// Eine gefuellte Liste darf nie durch eine leere ersetzt werden.
    #[test]
    fn eine_leere_liste_ueberschreibt_keine_gefuellte() {
        eigener_ordner("nichtleeren");
        let mut voll = Book::default();
        voll.entries = vec![Partner {
            id: "123456789".into(),
            name: "Buero-PC".into(),
            at: 100,
            ..Default::default()
        }];
        voll.save();
        Book::default().save();
        let wieder = Book::load();
        assert_eq!(wieder.sorted().len(), 1);
        assert_eq!(wieder.sorted()[0].name, "Buero-PC");
    }

    /// Kaputte Datei -> die Sicherungskopie rettet die Liste.
    #[test]
    fn eine_kaputte_datei_faellt_auf_die_sicherung_zurueck() {
        eigener_ordner("sicherung");
        let mut b = Book::default();
        b.entries = vec![Partner {
            id: "222222222".into(),
            name: "Laptop".into(),
            at: 1,
            ..Default::default()
        }];
        b.save();
        // zweiter Schreibvorgang legt die Sicherungskopie an
        b.rename("222222222", "Laptop 2");
        std::fs::write(Book::path(), b"{ kaputt").unwrap();
        let wieder = Book::load();
        assert_eq!(wieder.sorted().len(), 1);
        assert_eq!(wieder.sorted()[0].id, "222222222");
    }

    fn dev(id: &str, name: &str, at: u64) -> SyncDevice {
        SyncDevice {
            id: id.into(),
            name: name.into(),
            at,
            ..Default::default()
        }
    }

    #[test]
    fn der_neuere_eintrag_gewinnt() {
        let mut b = Book::default();
        b.entries = vec![Partner {
            id: "111111111".into(),
            name: "Alt".into(),
            at: 100,
            ..Default::default()
        }];
        // aelter als unserer: bleibt liegen
        assert!(!b.merge_remote(&[dev("111111111", "Noch aelter", 50)]));
        assert_eq!(b.get("111111111").unwrap().name, "Alt");
        // neuer: wird uebernommen
        assert!(b.merge_remote(&[dev("111111111", "Neu", 200)]));
        assert_eq!(b.get("111111111").unwrap().name, "Neu");
    }

    #[test]
    fn ein_unbekanntes_geraet_kommt_dazu() {
        let mut b = Book::default();
        assert!(b.merge_remote(&[dev("222222222", "Laptop", 10)]));
        assert_eq!(b.sorted().len(), 1);
        assert_eq!(b.sorted()[0].name, "Laptop");
    }

    #[test]
    fn ein_grabstein_raeumt_auch_hier_auf() {
        let mut b = Book::default();
        b.entries = vec![Partner {
            id: "333333333".into(),
            name: "Weg damit".into(),
            at: now().saturating_sub(60),
            ..Default::default()
        }];
        // frischer Grabstein - ein uralter faellt beim Aufraeumen heraus
        let mut tot = dev("333333333", "", now());
        tot.deleted = true;
        assert!(b.merge_remote(&[tot]));
        assert!(b.get("333333333").is_none());
        assert_eq!(b.sorted().len(), 0);
        // der Grabstein selbst bleibt, sonst kaeme das Geraet zurueck
        assert_eq!(b.to_sync().len(), 1);
        assert!(b.to_sync()[0].deleted);
    }

    #[test]
    fn entfernen_erzeugt_einen_grabstein_statt_einer_luecke() {
        let mut b = Book::default();
        b.entries = vec![Partner {
            id: "444444444".into(),
            name: "Tschuess".into(),
            at: 1,
            ..Default::default()
        }];
        b.remove("444444444");
        assert!(b.get("444444444").is_none());
        let s = b.to_sync();
        assert_eq!(s.len(), 1);
        assert!(s[0].deleted);
        assert!(s[0].at > 1);
    }

    #[test]
    fn passwoerter_gehen_nie_zum_konto() {
        let mut b = Book::default();
        b.started("555555555", "geheim", true);
        assert!(b.password("555555555").is_some());
        let json = serde_json::to_string(&b.to_sync()).unwrap();
        assert!(!json.contains("secret"));
        assert!(!json.contains("geheim"));
    }

    #[test]
    fn ids_are_grouped_for_reading() {
        assert_eq!(pretty_id("497628420"), "497 628 420");
        assert_eq!(pretty_id("1298814267"), "1 298 814 267");
        assert_eq!(pretty_id("12345"), "12345");
    }

    #[test]
    fn favourites_come_first_then_most_recent() {
        let mut b = Book::default();
        b.entries = vec![
            Partner {
                id: "111111111".into(),
                last: 100,
                ..Default::default()
            },
            Partner {
                id: "222222222".into(),
                last: 500,
                ..Default::default()
            },
            Partner {
                id: "333333333".into(),
                last: 10,
                favorite: true,
                ..Default::default()
            },
        ];
        let order: Vec<String> = b.sorted().into_iter().map(|p| p.id).collect();
        assert_eq!(order, vec!["333333333", "222222222", "111111111"]);
    }

    #[test]
    fn stored_passwords_are_not_readable_in_the_file() {
        let pw = "FleiTec2026";
        let blob = protect(pw).expect("protect");
        assert!(!blob.contains("FleiTec"));
        assert!(!blob.contains(pw));
        assert_eq!(unprotect(&blob).as_deref(), Some(pw));
        // a damaged blob must not panic and must not return garbage
        let mut bad = blob.clone();
        bad.replace_range(20..21, if &bad[20..21] == "a" { "b" } else { "a" });
        assert!(unprotect(&bad).is_none() || unprotect(&bad).as_deref() != Some(pw));
        assert!(unprotect("zzzz").is_none());
        assert!(unprotect("").is_none());
    }

    #[test]
    fn labels_fall_back_to_the_id() {
        let p = Partner {
            id: "497628420".into(),
            ..Default::default()
        };
        assert_eq!(p.label(), "497 628 420");
        let p2 = Partner {
            id: "497628420".into(),
            name: "Werkstatt-PC".into(),
            ..Default::default()
        };
        assert_eq!(p2.label(), "Werkstatt-PC");
        assert_eq!(p.ago(), "noch nie");
    }

    #[test]
    fn suche_ignoriert_trennzeichen_und_grossbuchstaben() {
        assert_eq!(search_norm("flei one"), "fleione");
        assert_eq!(search_norm("FLEI-ONE"), "fleione");
        assert_eq!(search_norm(" 497 628 420 "), "497628420");
        assert!(search_norm("FLEI-ONE").contains(&search_norm("flei one")));
    }
}
