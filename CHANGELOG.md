# Changelog

## 0.30.0 (prepared 14.10.2026, NOT released/tagged)
Security release. **FreeMeet end-to-end format changes (v1 -> v2): old and new clients cannot talk E2E to each other.**
- FreeMeet E2E v2: per-sender salt + per-stream keys, unique nonces (fixes nonce reuse, r1). Existing links stay valid.
- FreeMeet E2E: replay guard for video (frame-level: same salt/counter in all NALs, increasing NAL index, 64-number window), fixed window edge at a jump of exactly 64 (r60).
- Join announces `caps.e2e_ver = 2` (server can refuse outdated E2E clients, r61).
- Update signature check (Ed25519, `mac_version` for macOS) - the client only installs signed updates (r2, r28).
- Host proves the password to the viewer (additive, r5); brute-force lock for password attempts (r4); viewer auto-reconnect (r25).
- Fixes: friendlier connect errors + retry button (r31), requester name marked unconfirmed (r14), password hidden by default (r10).
- Dependencies: `cargo audit` clean (rustls 0.23.45, xcb 1.7.1) (r9).
Rollout order: native release first, then the browser/SFU switch (rollout/03-freemeet-nonce/RELEASE-CHECKLIST.md).
