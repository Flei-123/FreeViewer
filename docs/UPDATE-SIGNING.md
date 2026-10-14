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
* Mac (r28): the Mac package has its OWN version, `mac_version`, and `mac_sig` signs
  `freeviewer-update-v1\nmac\n<mac_version>\n<mac_sha256>\n<mac_size>`. The Mac client takes its version from
  `mac_version` (`update::feed_version`; legacy feeds without it fall back to `version`). So a Windows publish
  (which only moves `version`/`sha256`/`size`/`sig`) can never invalidate `mac_sig`, and a Mac publish never touches the
  Windows fields. Before this the Mac client compared against `version` (0.29.4) but fetched an old zip (0.26.10):
  it was offered an "update" to nothing and the Mac package was never refreshed because no code path wrote `mac_*`.

## Relay publish (patch in `relay/`, NOT deployed)
`relay/relay.js` already contains it; the diff against the imported live snapshot is
`relay/patches/0001-publish-hardening.patch`. Test: `node relay/test_publish.js`.
* `POST /fv/publish` only from the LAN (refuses requests carrying `X-Forwarded-For` / `X-Real-IP` / `Forwarded`, or from a public peer address).
  The publish script must call `http://192.168.1.60:7180/fv/publish` directly, not via the public proxy.
* Token only in `Authorization: Bearer ...` or `X-Upload-Token`; compared in constant time. `?token=` is refused
  (`FV_PUBLISH_QUERY_TOKEN=1` re-enables it temporarily, with a warning in the log).
* Signature in header `X-Fv-Sig` (hex). With `FV_UPDATE_PUBKEY=<hex>` the relay verifies it against the received
  bytes (refuses a bad one) and writes `sig` into the feed. `FV_REQUIRE_SIG=1` refuses unsigned uploads.

## Mac publish (relay patch 0003, NOT deployed)
`POST /fv/publish?platform=mac&version=X.Y.Z&name=FreeViewer-macOS.zip` (or `X-Remote-macOS.zip`, which goes to
`version-xoffi.json`). Same auth as above (LAN-only in strict mode, header token). The relay:
* only accepts those two names, the body must be a zip (`PK\x03\x04`), never a Windows exe;
* verifies `X-Fv-Sig` as platform **mac** over `X.Y.Z` (a `win` signature does not unlock a Mac package) and writes
  `mac_version/mac_url/mac_sha256/mac_size/mac_sig/mac_published` into the feed - atomically, win fields untouched;
* an unsigned upload deletes the old `mac_sig` (no stale signature of an older zip); `FV_REQUIRE_SIG=1` refuses unsigned;
* refuses a Mac version older than `mac_version` (HTTP 409); the same version again is a re-upload.
`noversion=1` (old CI upload: file only) still works in compat mode; `platform=mac` wins if both are given.
Tests: `node relay/test_publish.js` (mac section).

### Mac CI -> relay without opening the LAN-only publish
The GitHub runner neither gets a relay token nor the signing key (the old `mac.yml` posted to the PUBLIC url with the
token in the query - exactly what strict mode refuses). Now:
1. `mac.yml` builds the zip with `ditto`, creates a GitHub **build attestation** and uploads the **artifact** only.
2. On the LAN machine that holds `/root/fvwork/keys/update-ed25519.key`: `GH_TOKEN=... rollout/02-relay/55-mac-pull.sh --brand freeviewer|x-remote --apply`
   (or `--zip <hand-downloaded artifact>`). It takes the latest successful **push run of main**, downloads the artifact, reads the
   version from the zip's `Info.plist`, verifies the attestation (`gh attestation verify`, if `gh` is installed;
   `--require-attest` makes it mandatory) and hands over to `50-publish-release.sh --platform mac`, which re-checks zip /
   version / executable bit, **signs** (platform `mac`) and uploads via 127.0.0.1 on CT110.
3. Trust: the signature means "the key holder approved exactly these bytes". A hacked runner can only produce an artifact, which is
   inspected and confirmed before it is signed; r2 is not weakened (no new public write path, no token in CI).
Delete the GitHub secret `FV_UPLOAD_TOKEN` and rotate the token (`rollout/02-relay/mac-ci-publish.md`).

### Deploy checklist (needs Boss OK)
1. Backup CT110 (`pct snapshot 110 pre-publish-hardening`), compare live `relay.js` with `docs/RELAY-LIVE-IMPORT.md`.
2. Copy `relay/relay.js` (= live + patches 0001 + 0002 + 0003) and `clientip.js`; set `FV_UPDATE_PUBKEY` (value of `update::UPDATE_PUBKEY`) in the systemd unit; restart. (`rollout/02-relay/30-apply.sh`)
3. Change the publish script: LAN URL + `Authorization: Bearer` + `X-Fv-Sig` (from `sign-fields`; the sha256/size are those of the uploaded file) - or use `50-publish-release.sh`.
4. Release a client with this change (phase 1; it also brings `mac_version` support), later one with `FV_REQUIRE_SIG=1` (phase 3).
5. Publish a fresh, signed Mac build (`55-mac-pull.sh`) so `mac_version` catches up with Windows. Until then old Mac clients (0.26.10, no `mac_version` support)
   keep seeing `version` 0.29.4 and re-download the old zip - the real fix for them is the first new Mac zip under `mac_url`.
