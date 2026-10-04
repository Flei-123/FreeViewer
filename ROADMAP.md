# FreeViewer -- Roadmap

The ONE roadmap of FreeViewer lives in the JARVIS project "FreeViewer" (tool `project_roadmap`, overview in the control room).
This file only points there; it is NOT maintained in parallel.

* Direction: remote desktop and meetings with the Fleitec account (address book, device list, relay).
* What exists: `README.md`, `CONTRIBUTING.md`, `api/ACTIONS` (command-line action manifest).
* Checks: `./ci.sh`; GitHub workflows in `.github/workflows/`.
* Fleitec account: `src/account.rs` follows the shared protocol (adapter test `tools/fleitec-id-adapter`).
