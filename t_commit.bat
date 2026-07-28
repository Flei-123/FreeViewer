@echo off
cd /d C:\Users\Admin\Projects\FreeViewer
(
git config --global --add safe.directory C:/Users/Admin/Projects/FreeViewer
git add -A
git commit -m "v0.4: DXGI capture, game mode with raw relative mouse, full keyboard grab, key combos and clipboard sync" -m "Capture: DXGI desktop duplication backend (9 ms/frame instead of 45 ms with xcap) with automatic fallback, dirty rects from the compositor, cursor sent separately and drawn by the viewer." -m "Input: host side injection moved to SendInput (real virtual keys, extended flags, 5 mouse buttons, wheel), new relative motion path, SAS/Ctrl+Alt+Del, task manager, Win, Alt+Tab, Win+L, release-all so nothing stays stuck." -m "Viewer: low level keyboard hook plus pointer lock for game mode (right Ctrl frees), mode switch between Fernwartung and Spiel profiles, remote cursor drawing." -m "Clipboard text sync in both directions with echo suppression. New --inputtest self test and tools/input_probe.ps1 to measure input on the host from outside."
git push origin main
git log --oneline -3
) > commit.log 2>&1
