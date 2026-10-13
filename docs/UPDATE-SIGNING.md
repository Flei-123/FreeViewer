# Signed auto-update (r2)

## Problem
The client trusted the SHA-256 from the same feed that serves the file. Whoever
controls the feed (relay compromise, TLS-terminating proxy, DNS) could ship any
exe; the Windows service runs as SYSTEM.

## Scheme
* Ed25519 signature over a fixed text, **not** over JSON (no canonicalisation issues):
  `freeviewer-update-v1\n<platform>\n<version>\n<sha256 hex>\n<size>`
  * `platform` = `win` (feed fields `url/sha256/size/sig`) or `mac` (`mac_url/mac_sha256/mac_size/mac_sig`)
  * The SHA-256 binds the package bytes, so the signature covers file + version.
  * Platform label: the Windows signature cannot unlock the Mac package.
* Public key compiled into the client: `update::UPDATE_PUBKEY`
  (override per brand with `FV_UPDATE_PUBKEY=<64 hex>` at build time).
* Verified twice: in `check()` (before offering) and in `download()` (before fetching).
* Private key: `/root/fvwork/keys/update-ed25519.key` (hex seed, `chmod 600`,
  dir `700`). **Never in git.** Back it up offline; losing it = no more
  signed updates until a new client with a new key is installed by hand.

## Tools
```
python3 tools/sign-release/sign_release.py keygen              # once
python3 tools/sign-release/sign_release.py pubkey              # print public key
python3 tools/sign-release/sign_release.py sign --feed version.json --win freeviewer.exe [--mac freeviewer-mac.zip]
python3 tools/sign-release/sign_release.py verify --feed version.json --pub <hex>
python3 tools/sign-release/sign_release.py sign-fields --platform win --version V --sha256 H --size N
```
`sign` fills `sha256/size` from the file (or checks them) and writes `sig` / `mac_sig`.
Needs the python `cryptography` package. Test vector: `update::tests::python_signature_verifies`.

## Migration plan (switch = `update::REQUIRE_SIGNATURE`, build env `FV_REQUIRE_SIG=1`)
| Phase | Client | Feed | Effect |
|---|---|---|---|
| 0 (now live) | old, no check | unsigned | works |
| 1 | new (this change), `REQUIRE_SIGNATURE=false` | unsigned or signed | unsigned still accepted; a feed WITH `sig` that does not verify is **always refused** |
| 2 | same client | always signed (publish via relay patch, see below) | all new clients verify |
| 3 | build with `FV_REQUIRE_SIG=1` | signed | unsigned feeds refused |

* Phase 1 alone does **not** stop an attacker who simply strips `sig` from the
  feed - it only protects once the feed is signed AND phase 3 is shipped. Phase 3
  is the real security gain; do it once most installs run a phase-1 client.
* Old clients (< phase 1) stay unprotected forever; they only update to the phase-1 build.
* Known limit: a signed old release can be replayed (a client only moves to a
  *newer* version, so this only matters between two published versions). Add a
  `valid_until` field to the signed text in v2 if that becomes a concern.
* Mac: sign with the same `version` as the feed (`version` is part of the text).
  If the Windows build is published alone, the preserved `mac_*` fields keep an
  older version's hash - publish Windows and Mac together, or sign the Mac file
  again after each Windows publish.

## Relay publish (patch in `relay/`, NOT deployed)
`relay/relay.js` already contains it; the diff against the imported live snapshot is
`relay/patches/0001-publish-hardening.patch`. Test: `node relay/test_publish.js`.
* `POST /fv/publish` only from the LAN (refuses requests carrying `X-Forwarded-For` / `X-Real-IP` / `Forwarded`, or from a public peer address).
  The publish script must call `http://192.168.1.60:7180/fv/publish` directly, not via the public proxy.
* Token only in `Authorization: Bearer ...` or `X-Upload-Token`; compared in constant time. `?token=` is refused
  (`FV_PUBLISH_QUERY_TOKEN=1` re-enables it temporarily, with a warning in the log).
* Signature in header `X-Fv-Sig` (hex). With `FV_UPDATE_PUBKEY=<hex>` the relay verifies it against the received
  bytes (refuses a bad one) and writes `sig` into the feed. `FV_REQUIRE_SIG=1` refuses unsigned uploads.

### Deploy checklist (needs Boss OK)
1. Backup CT110 (`pct snapshot 110 pre-publish-hardening`), compare live `relay.js` with `docs/RELAY-LIVE-IMPORT.md`.
2. Copy `relay/relay.js`; set `FV_UPDATE_PUBKEY` (value of `update::UPDATE_PUBKEY`) in the systemd unit; restart.
3. Change the publish script: LAN URL + `Authorization: Bearer` + `X-Fv-Sig` (from `sign-fields`; the sha256/size are those of the uploaded file).
4. Release a client with this change (phase 1), later one with `FV_REQUIRE_SIG=1` (phase 3).
