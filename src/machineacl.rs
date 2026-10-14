//! Folder ACL for the machine wide config folder (%ProgramData%\FreeViewer, r74).
//! Pure command builder, so it is unit tested on every platform; the
//! Windows service installer runs the commands (`service::prepare_machine_config`).

/// `icacls` argument lists that lock down the machine folder (r74). The
/// folder is shared by the SYSTEM service and the GUI of every local user,
/// so ordinary users keep read/write on the files the GUI edits
/// (`password.txt`, `passwords.json`, address book, flags), but
///  * they can no longer delete or replace files they do not own: the
///    folder itself gives them only list + create (RX, WD, AD); "modify"
///    is inheritable-only (IO) and applies to the files below,
///  * `protect` (the machine identity: relay credential and AES fallback
///    key) gets its own ACL without inheritance: SYSTEM + Administrators
///    full, ordinary users read only.
/// Pure function, so the exact commands are unit tested on every platform.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn machine_acl_commands(dir: &std::path::Path, protect: &[&str]) -> Vec<Vec<String>> {
    const SYSTEM: &str = "*S-1-5-18";
    const ADMINS: &str = "*S-1-5-32-544";
    const USERS: &str = "*S-1-5-32-545";
    let d = dir.display().to_string();
    let mut v: Vec<Vec<String>> = vec![
        vec![d.clone(), "/grant:r".into(), format!("{SYSTEM}:(OI)(CI)F"), format!("{ADMINS}:(OI)(CI)F")],
        // replaces the old explicit Users:(OI)(CI)M entry
        vec![d.clone(), "/grant:r".into(), format!("{USERS}:(OI)(CI)(IO)M")],
        vec![d.clone(), "/grant".into(), format!("{USERS}:(RX,WD,AD)")],
    ];
    for name in protect {
        let f = dir.join(name).display().to_string();
        v.push(vec![f.clone(), "/inheritance:r".into()]);
        v.push(vec![
            f,
            "/grant:r".into(),
            format!("{SYSTEM}:F"),
            format!("{ADMINS}:F"),
            format!("{USERS}:R"),
        ]);
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_acl_commands_lock_down_identity_and_delete_rights() {
        let dir = std::path::Path::new("C:\\ProgramData\\FreeViewer");
        let cmds = machine_acl_commands(dir, &["identity.txt"]);
        let flat: Vec<String> = cmds.iter().map(|c| c.join(" ")).collect();
        // old broad entry is replaced, never re-added without (IO)
        assert!(flat.iter().any(|c| c.contains("/grant:r *S-1-5-32-545:(OI)(CI)(IO)M")));
        assert!(!flat.iter().any(|c| c.contains("*S-1-5-32-545:(OI)(CI)M")));
        // the folder itself: list + create only for ordinary users
        assert!(flat.iter().any(|c| c.ends_with("/grant *S-1-5-32-545:(RX,WD,AD)")));
        // SYSTEM + Administrators full
        assert!(flat[0].contains("*S-1-5-18:(OI)(CI)F") && flat[0].contains("*S-1-5-32-544:(OI)(CI)F"));
        // identity: no inheritance first, then SYSTEM/Admins F and Users R
        let i = flat.iter().position(|c| c.contains("identity.txt") && c.contains("/inheritance:r")).unwrap();
        assert!(flat[i + 1].contains("identity.txt") && flat[i + 1].contains("*S-1-5-32-545:R"));
        assert!(!flat[i + 1].contains("*S-1-5-32-545:M"));
        // nothing protected = only the three folder commands
        assert_eq!(machine_acl_commands(dir, &[]).len(), 3);
    }

    /// Real Windows only (r74): applies the folder ACL to FV_ACL_DIR with
    /// identity.txt + password.txt inside. Run elevated by
    /// tools/win-verify/secstore-check.ps1 -Phase acl.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn win_acl_apply() {
        let dir = std::path::PathBuf::from(std::env::var("FV_ACL_DIR").expect("FV_ACL_DIR"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("identity.txt"), "0".repeat(64)).unwrap();
        std::fs::write(dir.join("password.txt"), "fvdp1:00").unwrap();
        for args in machine_acl_commands(&dir, &["identity.txt"]) {
            let o = std::process::Command::new("icacls").args(&args).output().unwrap();
            assert!(o.status.success(), "{:?}: {}", args, String::from_utf8_lossy(&o.stdout));
        }
    }
}
