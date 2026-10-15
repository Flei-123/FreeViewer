# Native release 0.30.0 - checklist (r70). Nothing here was run; do NOT tag/publish without the boss's OK.

Why a minor bump: FreeMeet E2E wire format v2 is not compatible with v1 (see docs/FREEMEET-E2E-NONCE.md). The updater compares
numerically (`update::newer`), 0.30.0 > 0.29.4. `Cargo.toml` + `Cargo.lock` are already at 0.30.0 on main; CHANGELOG.md is written.

## Before the build
- [ ] main is green: `cargo test --locked` (heavy), `node --test web-patch/`, `python3 tools/freemeet-e2e/ref.py --check`, `bash rollout/tests/run-dryrun-tests.sh`.
- [ ] CI on GitHub for the release commit: `ci.yml` (cargo test --locked, audit, relay, freemeet-e2e job), `windows.yml` and `mac.yml` (both run on push to main; Mac zip is the `ditto` artifact, never re-zip).
- [ ] r59: CI has never been seen running on GitHub - look at the first run before trusting it.

## Build + publish (Boss-Ja)
1. [ ] Windows exe: artifact of `windows.yml` or `tools/release/build-win.sh fv|xr` (via heavy). The Windows publish script is NOT in the repo (r2/r56) - find it, then use `rollout/02-relay/50-publish-release.sh --platform win --file ... --version 0.30.0 --apply` (signs with the update key). Needs relay block 2A first.
2. [ ] Mac: artifact of `mac.yml` -> `rollout/02-relay/55-mac-pull.sh --brand freeviewer --apply` (+ `--brand x-remote`). Untested parts: attestation, GitHub API.
3. [ ] `release.yml` (tag `v*`) only builds Linux/macOS/Windows archives for GitHub releases; it is NOT the update channel. Tag only when wanted: `git tag v0.30.0`.
4. [ ] Wait until `20-gate-native.sh --native-min-version 0.30.0` is green (enough clients updated), then block 3 (browser/SFU switch).

## After
- [ ] Auto-update on one Windows and one Mac test machine: old version -> 0.30.0 (signature accepted).
- [ ] Optional r61: native announces `e2e_ver: 2`; once the browser patch is live, set `FM_E2E_MIN_VER=2` on the SFU.
- [ ] Rotate `FV_UPLOAD_TOKEN` (r69) before relay strict.
