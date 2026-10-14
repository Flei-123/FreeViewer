# Block 4 - SFU live merge (r65)

Status 14.10.2026: `/root/projects/freemeet-sfu/rollout/` (worker W19) does NOT exist yet.
When it exists, use it for the merge/build and keep only `10-precheck.sh`, `40-swap-binary.sh`
and `90-rollback.sh` from here (swap + rollback are generic). Merge recipe until then:
`/root/projects/freemeet-sfu/docs/LIVE-MERGE.md` (format-patch + `git am -3` on CT111, conflicts
only in signal.rs, then `cargo build --release && cargo test --release`; never proven to compile).

**Combine with block 3:** the e2e.js lives inside the SFU binary, so ONE rebuild + ONE restart can
ship the SFU fixes and the E2E v2 browser file (do block 3 stage `file`, then this merge, then one swap).
Order: 03 gate passed -> 04 merge+build on a branch -> 40-swap-binary.sh (restart) -> 03 50-verify.

Scripts: `10-precheck.sh` (read-only), `40-swap-binary.sh` (stop/copy/start, auto-undo), `90-rollback.sh`.
Take `03-freemeet-nonce/30-backup.sh` as the backup step (binary + snapshot).
