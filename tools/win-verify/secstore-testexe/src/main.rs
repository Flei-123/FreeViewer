//! Test harness: compiles the real `secstore.rs` / `machineacl.rs` of the
//! main crate with tiny stand-ins for `crypto` and `ident`.
#![allow(dead_code)]

#[path = "../../../../src/machineacl.rs"]
mod machineacl;
#[path = "../../../../src/secstore.rs"]
mod secstore;

mod crypto {
    use rand::RngCore;
    pub fn random_bytes(n: usize) -> Vec<u8> {
        let mut v = vec![0u8; n];
        rand::rngs::OsRng.fill_bytes(&mut v);
        v
    }
}

mod ident {
    use std::path::PathBuf;
    thread_local! {
        static DIR: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
    }
    pub fn set_test_config_dir(p: PathBuf) {
        let _ = std::fs::create_dir_all(&p);
        DIR.with(|d| *d.borrow_mut() = Some(p));
    }
    /// Stand-in: a fixed 64 hex secret per test dir (the real one reads identity.txt).
    pub fn load_or_create_secret() -> String {
        let d = DIR.with(|d| d.borrow().clone()).expect("test dir");
        let f = d.join("identity.txt");
        if let Ok(s) = std::fs::read_to_string(&f) {
            return s.trim().to_string();
        }
        let s = hex::encode(crate::crypto::random_bytes(32));
        let _ = std::fs::write(&f, &s);
        s
    }
}

fn main() {
    eprintln!("run with: fv-secstore-check.exe --test-threads=1 (cargo test exe), see tools/win-verify/secstore-check.ps1");
}
