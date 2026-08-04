//! Schluessel-Werkzeug fuer X-Remote-Lizenzen. Liegt zwar im oeffentlichen
//! Repo, ist aber ohne den privaten Schluessel nutzlos - der bleibt beim
//! Verkaeufer. Bauen: cargo build --release --features license --bin keygen
//!
//! Aufruf:
//!   keygen new                              -> neues Schluesselpaar (einmalig!)
//!   keygen make <priv-hex> "Name" [tage]    -> Lizenzschluessel
//!     tage: Laufzeit ab heute, 0 oder weglassen = unbegrenzt
//!
//! Das Format (Nachricht || Ed25519-Signatur, Base32-Crockford) muss exakt
//! dem in src/license.rs entsprechen - dort sitzt die Pruefung im Client.

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

fn days_since_2026() -> u32 {
    const EPOCH_2026: u64 = 1_767_225_600;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    (now.saturating_sub(EPOCH_2026) / 86_400) as u32
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("new") => {
            use ed25519_dalek::SigningKey;
            let mut seed = [0u8; 32];
            rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut seed);
            let sk = SigningKey::from_bytes(&seed);
            println!("privat (GEHEIM halten, nie ins Repo!):");
            println!("  {}", hex::encode(sk.to_bytes()));
            println!("oeffentlich (gehoert in license.rs / FV_LICENSE_PUBKEY):");
            println!("  {}", hex::encode(sk.verifying_key().to_bytes()));
        }
        Some("make") => {
            use ed25519_dalek::{Signer, SigningKey};
            let (Some(priv_hex), Some(name)) = (args.get(2), args.get(3)) else {
                eprintln!("keygen make <priv-hex> \"Name\" [tage]");
                std::process::exit(2);
            };
            let days: u32 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(0);
            let expires = if days == 0 { 0 } else { days_since_2026() + days };
            let raw = hex::decode(priv_hex).expect("privater Schluessel: kein hex");
            assert!(raw.len() == 32, "privater Schluessel: 32 Bytes erwartet");
            let mut b = [0u8; 32];
            b.copy_from_slice(&raw);
            let sk = SigningKey::from_bytes(&b);
            let n = name.trim();
            assert!(!n.is_empty() && n.len() <= 60, "Name: 1-60 Zeichen");
            // Nachricht: "XRL1" || name-len u8 || name || ablauf u32 LE || ausgabe u8
            let mut msg = Vec::new();
            msg.extend_from_slice(b"XRL1");
            msg.push(n.len() as u8);
            msg.extend_from_slice(n.as_bytes());
            msg.extend_from_slice(&expires.to_le_bytes());
            msg.push(0u8); // Ausgabe 0 = Pro
            let sig = sk.sign(&msg);
            msg.extend_from_slice(&sig.to_bytes());
            println!("{}", b32_encode(&msg));
            if expires != 0 {
                eprintln!("(laeuft ab: Tag {} seit 1.1.2026)", expires);
            }
        }
        _ => {
            eprintln!("X-Remote Lizenz-Schluessel-Werkzeug");
            eprintln!("  keygen new");
            eprintln!("  keygen make <priv-hex> \"Name\" [tage]");
            std::process::exit(2);
        }
    }
}
