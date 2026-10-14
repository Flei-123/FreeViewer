# Password storage (secstore) - formats, threat model, verification

| Tag | Where | Key |
|---|---|---|
| `fvdp1:` | Windows | DPAPI, machine scope + own entropy |
| `fvmk1:` | macOS (r75) | AES-256-GCM, key = HKDF(random master key in the login Keychain) |
| `fvk1:` | Linux, fallback, legacy macOS | AES-256-GCM, key = HKDF(identity secret) |
| (none) | legacy | plain text, rewritten sealed on first load |

Callers rewrite a file on load when `secstore::needs_reseal` says so (plain text; on a Mac with a
usable Keychain also `fvk1:`). `pwlist::load` never rewrites while an entry could not be opened
(other machine, Keychain locked), so nothing is dropped by a failed migration.

## macOS Keychain (r75)
- Item: generic password, service `com.freeviewer.secstore`, account `host-password-master-key`, 32 random bytes.
- Never overwritten: read errors other than "not found" (locked, access denied, wrong size) -> `None`, sealing falls back to `fvk1:`.
- `FV_NO_KEYCHAIN=1` disables it. Risk: a differently signed update may trigger a Keychain access prompt; if the user denies it, `fvmk1:` values are unreadable (Keychain item stays intact, "Always allow" fixes it).
- Status: compiled for `aarch64-apple-darwin` only (cargo check); NOT run on a real Mac.

## ProgramData folder (r74, Windows service install)
- DPAPI machine scope means every local account can decrypt the files; the value of sealing is that a copied file / backup / disk image from another PC is useless.
- `service::machine_acl_commands`: SYSTEM + Administrators full; ordinary users: list + create in the folder, "modify" only inheritable on the files below (so they cannot delete or replace files they do not own); `identity.txt` has its own ACL without inheritance (users read only).
- The GUI of an ordinary user still reads/writes `password.txt`, `passwords.json`, address book: moving that behind IPC (service as the only reader, GUI asks over a local pipe) is the real fix and a protocol-free but larger change -> roadmap.
- Status: command lines unit tested; effect NOT yet verified on real Windows.

## Verification on a real Windows PC (r73)
`tools/win-verify/secstore-check.ps1` (header explains the three phases `unit`, `xacct`, `acl`). Test exe:
`cargo test --locked --target x86_64-pc-windows-gnu --no-run` (run via `heavy`).
