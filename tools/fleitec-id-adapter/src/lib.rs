//! Test harness: compiles the real `src/account.rs` against three small stand-ins
//! for the modules it uses (`partners::SyncDevice`, `ident::user_config_dir`,
//! `brand::WEB`/`SLUG`). The stand-ins are copies of the signatures only.
#![allow(dead_code)]

pub mod partners {
    use serde::{Deserialize, Serialize};
    /// Same fields as src/partners.rs SyncDevice.
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
}

pub mod ident {
    pub fn user_config_dir() -> std::path::PathBuf {
        std::env::temp_dir().join("fleitec-id-adapter-unused")
    }
}

pub mod brand {
    pub const WEB: &str = "https://freeviewer.fleitec.com";
    pub const SLUG: &str = "freeviewer";
}

#[path = "../../../src/account.rs"]
pub mod account;
