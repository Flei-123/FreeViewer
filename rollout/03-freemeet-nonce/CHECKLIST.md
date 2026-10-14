# Verify checklist - FreeMeet E2E v2 (do after 50-verify.sh is green)

Material: 2 computers/phones with browsers (e.g. Chrome + Safari/iPhone), 1 native client
(Windows or Mac, version >= the release with v2), a NEW E2E meeting (new link, new `#k=` key).

## Before
- [ ] Every participant reloads the page once (Ctrl+F5 / pull to refresh). The file is served with `no-cache`, a normal reload is enough.
- [ ] Native clients are on the v2 release (Settings/About shows the version).

## In one NEW E2E room (browser A creates it, B and the native join)
1. [ ] Browser A: the tag shows **"E2E: on"** (German UI: "E2E: an").
2. [ ] Browser B joins: A and B see and hear each other (camera + microphone).
3. [ ] Native joins: native sees A and B, A and B see the native tile. No message "A participant uses an outdated end-to-end format".
4. [ ] **Screen share** from browser A **while its camera is on** (this was the camera+screen same-nonce case): both streams arrive at B and the native, picture not frozen / not green.
5. [ ] **Rejoin**: B closes the tab and joins again (counter restart case): picture and sound come back within a few seconds.
6. [ ] Mute/unmute and camera on/off in each client at least twice.
7. [ ] 5 minutes of talking: no growing delay, no crackling.

## Negative test (proves the old format is really refused, expected behaviour)
- [ ] Optional: an OLD native client (before v2) joins this E2E room: it must see NO garbage picture; the new clients show the "outdated format" hint once. It must NOT decode anything.

## Normal (non-E2E) room
- [ ] One quick room without the lock: still works for browser and native.

## Result
- All green -> keep. After 3 days remove the LXC snapshots: `01-backups/80-cleanup-snapshots.sh --apply`.
- Anything red -> `90-rollback.sh --apply` (browser back to v1) and report which step failed.
- Rotate the link of important E2E rooms: everything recorded under v1 stays exposed to the nonce reuse; a new link only limits the future.
