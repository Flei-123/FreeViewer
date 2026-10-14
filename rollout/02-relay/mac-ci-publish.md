# Mac CI publish vs. LAN-only publish (found 14.10.2026, solved in code 14.10.2026 / r28)

**Old:** the last step of `.github/workflows/mac.yml` (X-Remote job) posted `X-Remote-macOS.zip` from a GitHub runner to the PUBLIC url
`https://freeviewer.fleitec.com/fv/publish?token=$FV_UPLOAD_TOKEN&...&noversion=1` (token in the URL). The normal `FreeViewer-macOS.zip`
build only left a CI artifact. Nothing ever wrote the `mac_*` fields of the feed (the relay only preserved them) - that is why the feed has had
`mac_version 0.26.10` while Windows moved on to 0.29.4 (roadmap r28).

**New (in the repo, nothing live yet):**
* `mac.yml`: no upload, no `FV_UPLOAD_TOKEN`; per brand a zip artifact (`freeviewer-macos-arm64` / `x-remote-macos-arm64`) + GitHub build attestation.
* Relay patch 0003: `platform=mac` publish writes the `mac_*` fields (own `mac_version`, signature platform `mac`); Windows publishes keep them valid.
* Client: the Mac client reads `mac_version` (`update::feed_version`).
* `rollout/02-relay/55-mac-pull.sh` (LAN, where the signing key is) pulls the artifact of the latest successful main run, checks it, signs it, publishes it.

| relay mode | effect |
|---|---|
| `compat` (30-apply default) | old CI step (if an old workflow run still posts) keeps working as today; new path works too |
| `strict` | old CI step would get **403** - there is none any more; 55-mac-pull talks to 127.0.0.1 on CT110 |

## Steps (Boss yes needed only for the live parts)
1. Deploy relay patch 0003 with block 2A (`30-apply.sh --mode compat`), release a client with `mac_version` support (version bump).
2. Delete the GitHub secret `FV_UPLOAD_TOKEN` and rotate the token on CT110: `openssl rand -hex 16 > /root/fvrelay/upload.token; chmod 600 ...` (no restart needed; the token also sat in CI logs/URLs).
3. Per Mac release: wait for the green `macOS Build` run on main, then
   `GH_TOKEN=<read-only Actions token> rollout/02-relay/55-mac-pull.sh --brand freeviewer` (dry-run) and again with `--apply`; same with `--brand x-remote`.
   Without a token: download the artifact on the Actions page, unzip it, `55-mac-pull.sh --zip <path>`.
4. The Windows publish script is **not in this repo** - find it (build machine / Dell) and move it to `50-publish-release.sh` or to
   `http://192.168.1.60:7180/fv/publish` with `Authorization: Bearer <token>` + `X-Fv-Sig`.
5. Only when 1-4 are done: `30-apply.sh --mode strict --only-dropin`, later `60-require-sig.sh` (needs a signed `mac_sig` in the feed first).

## Not proven
`mac.yml` was not run (no Mac runner here); `actions/attest-build-provenance@v2` and `gh attestation verify` are standard GitHub features but untested
for this repo (private repos may need a plan that supports attestations - then drop the step; 55-mac-pull only warns without `--require-attest`).
The GitHub API calls of 55-mac-pull (runs / artifacts) were written against the documented API and not run against GitHub (no `GH_TOKEN` here); the zip checks and signing path were run with a synthetic zip.
