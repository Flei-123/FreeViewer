//! Lizenz-Pruefung fuer Marken-Builds (z. B. X-Remote). Dieses Modul wird
//! NUR mit `--features license` kompiliert - der FreeViewer-Build enthaelt
//! keinen Lizenzcode und bleibt komplett frei.
//!
//! Prinzip: Der Lizenzschluessel ist eine Nachricht (Name, Ablaufdatum,
//! Ausgabe) plus einer Ed25519-Signatur. Unterschreiben kann nur, wer den
//! privaten Schluessel hat (der bleibt beim Verkaeufer, NIEMALS im Repo).
//! Im Build steckt nur der oeffentliche Schluessel - gefaelsche Schluessel
//! fallen bei der Pruefung durch.
//!
//! Format des Schluessels, wie ihn der Kunde bekommt:
//!   Base32-Crockford der Bytes  nachricht || signatur(64)
//!   nachricht = "XRL1" || name-len u8 || name || ablauf u32 LE || ausgabe u8
//!   in Fuenfergruppen mit Bindestrichen.
//!
//! Ablauf: Tage seit dem 1.1.2026, 0 = unbegrenzt. Ausgabe: 0 = Pro.

use std::fs;
use std::path::PathBuf;

/// Oeffentlicher Pruef-Schluessel (Ed25519, 32 Bytes, hex). Darf per
/// FV_LICENSE_PUBKEY beim Bauen ueberschrieben werden; der eingebaute Wert
/// ist der Produktiv-Schluessel von X-Remote. Der zugehoerige private
/// Schluessel liegt ausserhalb des Repos.
pub const PUBKEY_HEX: &str = match option_env!("FV_LICENSE_PUBKEY") {
    Some(s) => s,
    None => "9c24c5d917aa3cab5f3ddc63f7e7026efd08bd68f82d5177089efc8fe5364230",
};

/// Eine gueltige Lizenz.
#[derive(Clone, Debug)]
pub struct License {
    /// Wem die Lizenz gehoert (Name oder E-Mail des Kunden).
    pub name: String,
    /// Ablauftag (Tage seit 1.1.2026) - 0 heisst unbegrenzt.
    pub expires: u32,
    /// 0 = Pro (spaeter weitere Stufen).
    pub edition: u8,
}

/// Was die Pruefung ergeben hat.
#[derive(Clone, Debug)]
pub enum Status {
    /// Gueltige Lizenz auf diesem Rechner gespeichert.
    Active(License),
    /// Noch kein Schluessel eingegeben.
    Missing,
    /// Es gibt einen Schluessel, aber er ist ungueltig oder abgelaufen.
    Invalid(String),
}

fn key_file() -> PathBuf {
    crate::ident::config_dir().join("license.key")
}

/// Heutige Tag-Nummer (Tage seit dem 1.1.2026).
fn today() -> u32 {
    // Ein Tag Unschaerfe bei Ablaufdaten ist egal - die Uhr des Rechners
    // ist ohnehin nur so genau wie der Benutzer sie stellt.
    const EPOCH_2026: u64 = 1_767_225_600;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    (now.saturating_sub(EPOCH_2026) / 86_400) as u32
}

impl License {
    /// Abgelaufen? Unbegrenzte Lizenzen (0) laufen nie ab.
    pub fn expired(&self) -> bool {
        self.expires != 0 && today() > self.expires
    }

    /// Lesbares Ablaufdatum oder "unbegrenzt".
    pub fn expiry_text(&self) -> String {
        if self.expires == 0 {
            return "unbegrenzt".to_string();
        }
        // Tage seit 1.1.2026 zurueckrechnen (Schaltjahre beachten).
        let mut days = self.expires as i64;
        let mut year = 2026i64;
        loop {
            let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
            let yd = if leap { 366 } else { 365 };
            if days < yd {
                break;
            }
            days -= yd;
            year += 1;
        }
        let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
        const MLEN: [i64; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        let mut month = 0usize;
        while month < 12 {
            let m = if month == 1 && leap { 29 } else { MLEN[month] };
            if days < m {
                break;
            }
            days -= m;
            month += 1;
        }
        format!("{:02}.{:02}.{}", days + 1, month + 1, year)
    }
}

// ---------------------------------------------------------------- base32 ---
// Crockford: ohne I, L, O, U - keine Verwechslungen beim Abtippen.

const B32: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

fn b32_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() * 8 / 5 + data.len() / 5);
    let (mut acc, mut bits) = (0u32, 0u32);
    let mut n = 0usize;
    for &b in data {
        acc = (acc << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(B32[((acc >> bits) & 31) as usize] as char);
            n += 1;
            if n % 5 == 0 {
                out.push('-');
            }
        }
    }
    if bits > 0 {
        out.push(B32[((acc << (5 - bits)) & 31) as usize] as char);
    }
    out.trim_end_matches('-').to_string()
}

fn b32_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 5 / 8);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.trim().chars() {
        if c == '-' || c == ' ' {
            continue;
        }
        let c = c.to_ascii_uppercase();
        // Nachsicht beim Abtippen: O -> 0, I/L -> 1
        let c = match c {
            'O' => '0',
            'I' | 'L' => '1',
            c => c,
        };
        let v = B32.iter().position(|&b| b as char == c)? as u32;
        acc = (acc << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    Some(out)
}

// ------------------------------------------------------------- signatur ---

/// Nachricht bauen: "XRL1" || name-len u8 || name || ablauf u32 LE || ausgabe u8.
fn message(name: &str, expires: u32, edition: u8) -> Option<Vec<u8>> {
    let n = name.trim();
    if n.is_empty() || n.len() > 60 {
        return None;
    }
    let mut m = Vec::with_capacity(4 + 1 + n.len() + 4 + 1);
    m.extend_from_slice(b"XRL1");
    m.push(n.len() as u8);
    m.extend_from_slice(n.as_bytes());
    m.extend_from_slice(&expires.to_le_bytes());
    m.push(edition);
    Some(m)
}

fn verify(msg: &[u8], sig: &[u8]) -> bool {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    let Ok(pk) = hex::decode(PUBKEY_HEX) else {
        return false;
    };
    if pk.len() != 32 {
        return false;
    }
    let mut pkb = [0u8; 32];
    pkb.copy_from_slice(&pk);
    let Ok(key) = VerifyingKey::from_bytes(&pkb) else {
        return false;
    };
    let Ok(signature) = Signature::try_from(sig) else {
        return false;
    };
    key.verify(msg, &signature).is_ok()
}

/// Schluessel-Text (wie ihn der Kunde eintippt) pruefen und auswerten.
pub fn parse(text: &str) -> Result<License, String> {
    let raw = b32_decode(text).ok_or_else(|| "kein gueltiges Format".to_string())?;
    if raw.len() < 64 + 11 {
        return Err("Schluessel zu kurz".to_string());
    }
    let (msg, sig) = raw.split_at(raw.len() - 64);
    if msg.len() < 11 || &msg[..4] != b"XRL1" {
        return Err("falsche Marke/Version".to_string());
    }
    let nlen = msg[4] as usize;
    if msg.len() != 5 + nlen + 5 {
        return Err("beschaedigter Schluessel".to_string());
    }
    let name = String::from_utf8_lossy(&msg[5..5 + nlen]).to_string();
    let o = 5 + nlen;
    let expires = u32::from_le_bytes([msg[o], msg[o + 1], msg[o + 2], msg[o + 3]]);
    let edition = msg[o + 4];
    if !verify(msg, sig) {
        return Err("Signatur ungueltig".to_string());
    }
    let lic = License {
        name,
        expires,
        edition,
    };
    if lic.expired() {
        return Err(format!("abgelaufen am {}", lic.expiry_text()));
    }
    Ok(lic)
}

/// Gespeicherten Schluessel laden und pruefen.
pub fn status() -> Status {
    let Ok(text) = fs::read_to_string(key_file()) else {
        return Status::Missing;
    };
    match parse(&text) {
        Ok(lic) => Status::Active(lic),
        Err(e) => Status::Invalid(e),
    }
}

/// Schluessel aktivieren: pruefen, dann auf diesem Rechner ablegen.
pub fn activate(text: &str) -> Result<License, String> {
    let lic = parse(text)?;
    let dir = crate::ident::config_dir();
    let _ = fs::create_dir_all(&dir);
    fs::write(key_file(), text.trim()).map_err(|e| format!("speichern: {}", e))?;
    Ok(lic)
}

/// Lizenz entfernen (z. B. bei Geraetewechsel).
pub fn revoke() {
    let _ = fs::remove_file(key_file());
}

/// Einen Schluessel erzeugen - geht nur mit dem privaten Schluessel.
/// Wird vom Schluessel-Werkzeug (bin keygen) benutzt, nie vom Client.
pub fn make(priv_hex: &str, name: &str, expires: u32, edition: u8) -> Result<String, String> {
    use ed25519_dalek::{Signer, SigningKey};
    let raw = hex::decode(priv_hex).map_err(|_| "privater Schluessel: kein hex")?;
    if raw.len() != 32 {
        return Err("privater Schluessel: 32 Bytes erwartet".to_string());
    }
    let mut b = [0u8; 32];
    b.copy_from_slice(&raw);
    let sk = SigningKey::from_bytes(&b);
    let msg = message(name, expires, edition).ok_or("Name: 1-60 Zeichen")?;
    let sig = sk.sign(&msg);
    let mut full = msg;
    full.extend_from_slice(&sig.to_bytes());
    Ok(b32_encode(&full))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base32_roundtrip() {
        let data = b"XRL1\x05justin\x00\x00\x00\x00\x00";
        let enc = b32_encode(data);
        assert_eq!(b32_decode(&enc).unwrap(), data);
        // Bindestriche, Kleinschreibung und O/I-Verwechslungen verzeihen
        assert_eq!(b32_decode(&enc.to_lowercase().replace('0', "o")).unwrap(), data);
    }

    #[test]
    fn make_and_parse_roundtrip() {
        use ed25519_dalek::SigningKey;
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let key = make(&hex::encode(sk.to_bytes()), "Max Mustermann", 0, 0).unwrap();
        let raw = b32_decode(&key).unwrap();
        let (msg, sig) = raw.split_at(raw.len() - 64);
        // Signatur gegen den frischen Schluessel pruefen (statt dem eingebauten)
        use ed25519_dalek::{Signature, Verifier, VerifyingKey};
        let vk = VerifyingKey::from_bytes(&sk.verifying_key().to_bytes()).unwrap();
        assert!(vk.verify(msg, &Signature::try_from(sig).unwrap()).is_ok());
        // Nachricht korrekt zerlegt?
        assert_eq!(&msg[..4], b"XRL1");
        assert_eq!(msg[4], 14);
        assert_eq!(&msg[5..19], b"Max Mustermann");
    }

    /// End-to-End: einen echten, mit keygen erzeugten Schluessel gegen den
    /// eingebauten oeffentlichen Schluessel pruefen. Laueft nur, wenn die
    /// Umgebungsvariable XR_TEST_KEY gesetzt ist (CI/Handtest).
    #[test]
    fn real_key_parses() {
        let Ok(k) = std::env::var("XR_TEST_KEY") else { return };
        let lic = parse(&k).expect("echter Schluessel muss gelten");
        assert!(!lic.name.is_empty());
    }

    #[test]
    fn expiry_text_works() {
        let lic = License { name: "x".into(), expires: 0, edition: 0 };
        assert_eq!(lic.expiry_text(), "unbegrenzt");
        let lic = License { name: "x".into(), expires: 365 + 31, edition: 0 };
        assert_eq!(lic.expiry_text(), "01.02.2027");
    }
}
