# Mac CI publish vs. LAN-only publish (found 14.10.2026)

`.github/workflows/mac.yml` (last step) uploads `X-Remote-macOS.zip` from a GitHub runner:

```
curl -fsS --data-binary @target/X-Remote-macOS.zip \
  "https://freeviewer.fleitec.com/fv/publish?token=$FV_UPLOAD_TOKEN&version=$VERSION&name=X-Remote-macOS.zip&noversion=1&notes=X-Remote-macOS"
```

That is a **public URL with the token in the query**: exactly what the r2 patch forbids in
`strict` mode (LAN-only, header token). So:

| relay mode | effect on the Mac CI upload |
|---|---|
| `compat` (30-apply.sh default) | keeps working as today (LAN check off, `?token=` still accepted, warning in the log) |
| `strict` | answered with **403**, the CI step fails (the artifact of the job is still uploaded earlier in the workflow) |

## What to do before `strict`

1. Remove the last step of `mac.yml` (or make it `continue-on-error: true`) and delete the GitHub secret `FV_UPLOAD_TOKEN`
   (the token also sat in CI logs/URLs - **rotate it**: `openssl rand -hex 16 > /root/fvrelay/upload.token` on CT110, mode 600, no restart needed).
2. Mac packages are published from the LAN instead: download the CI artifact (`gh run download <id> -n X-Remote-macOS`, or from the Actions page)
   and run `02-relay/50-publish-release.sh --platform mac --file <zip> --version X.Y.Z --brand x-remote`.
3. The Windows publish script is **not in this repo** - find it (build machine / Dell) and move it to
   `http://192.168.1.60:7180/fv/publish` with `Authorization: Bearer <token>` or let it call `50-publish-release.sh`.

Until step 1-3 are done, stay in `compat` mode. Stage A (compat) already gives the signature check;
stage B (strict) closes the "token in URL / publish from the internet" hole.
