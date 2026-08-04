@echo off
cd /d C:\FreeViewer
del build.log 2>nul
echo === FreeViewer build === > build.log
cargo build --release >> build.log 2>&1
if errorlevel 1 (echo FV_BUILD_FAIL >> build.log & exit /b 1)
copy /y target\release\freeviewer.exe freeviewer-0.26.1.exe >> build.log 2>&1
echo === X-Remote build === >> build.log
set FV_BRAND_NAME=X-Remote
set FV_BRAND_EXE=x-remote.exe
set FV_BRAND_DIR=X-Remote
set FV_BRAND_PUBLISHER=Xoffi
set FV_BRAND_SLUG=xoffi
set FV_BRAND_WEB=https://remote.fleitec.com
set FV_BRAND_FEED=https://remote.fleitec.com/fv/dist/version-xoffi.json
cargo build --release --features license >> build.log 2>&1
if errorlevel 1 (echo XR_BUILD_FAIL >> build.log & exit /b 1)
copy /y target\release\freeviewer.exe x-remote-0.26.1.exe >> build.log 2>&1
echo DONE >> build.log
