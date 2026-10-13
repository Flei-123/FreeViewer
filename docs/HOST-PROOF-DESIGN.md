# Host counter-proof (r5) - design, not implemented

## Gap
Only the viewer proves the password (`0x03 || HMAC(Argon2id(pw,salt), "fv-auth"||client_pub||host_pub||salt)`).
The host answers a bare `0x04`. A man in the middle (a relay operator, or anyone who
can sit between viewer and host) can answer `0x02`, pick the salt, and collect a proof
that is an **offline brute-force oracle** for the password (Argon2id slows it down, does
not stop it). It cannot read the session afterwards without the password, but the
viewer is also told "ok" by a host that never knew the password.

## Proposal (additive, no PAKE library)
Keep the existing flow, add a second HMAC in the opposite direction:

```
host_proof = HMAC-SHA256(pw_key, "fv-host" || client_pub || host_pub || salt || session_key)
host -> viewer : 0x04 || host_proof(32)        (instead of bare 0x04)
```
* Binding `session_key` ties the proof to this very X25519 exchange (a MITM with two
  different sessions to both sides cannot relay it).
* Viewer verifies in constant time (`crypto::proof_matches`) before it shows "Verbunden".
* This does **not** remove the offline oracle for a fake *host* (it still sees the viewer proof).
  Real protection against that needs a PAKE (OPAQUE / SPAKE2+) -> separate, larger step.
  The counter-proof closes the "viewer trusts a host that does not know the password" hole
  and makes a fake host detectable at once.

## Compatibility (why additive and vector-safe)
* Host: `hostside.rs` accepts hello frames `>= 33` bytes -> the viewer can append a capability
  byte (`0x01` = "send host proof") to `0x01 || client_pub`. Old hosts ignore it and send the bare `0x04`.
* New host answers `0x04 || host_proof` **only** if the capability byte was present. Old viewers
  (and the OrientOS/Firn viewer, and `tests/orientos-vectors.json`, which fix the bare `0x04`) never
  send it -> their frames stay byte-identical, vector test stays green.
* New viewer: bare `0x04` = legacy host -> accept in phase 1 (show a "host not verified" hint),
  refuse in a later phase (same switch idea as `update::REQUIRE_SIGNATURE`). A wrong proof is always refused.
* Ask mode (no password): there is no shared secret, so no host proof - the session code
  (`session_code`) compared by the humans stays the only check.

## Work
`crypto.rs` (+`host_proof()`), `hostside.rs` (capability + append), `viewer.rs` (send capability + verify),
vector file: add NEW vectors for the capability frames, do not edit the old ones; OrientOS `lib/fv/` follows later.
Not done now because it touches `hostside.rs`/`viewer.rs`/`crypto.rs` (outside this worker's files) and needs a
Windows run of a real host+viewer pair.
