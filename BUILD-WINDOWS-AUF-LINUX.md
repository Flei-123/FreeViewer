# Windows-EXE auf dem Linux-Server bauen (Cross-Compile)

Geht wirklich - gemessen am 09.08.2026, 16 MB freeviewer.exe aus dem
JARVIS-Server heraus. Das spart den Umweg ueber ein Windows-Geraet fuer
JEDEN Build; nur zum echten Ausprobieren braucht es noch Windows.

## Einmalig einrichten

    rustup target add x86_64-pc-windows-gnu
    apt-get install -y autoconf automake pkg-config    # mingw-w64 war schon da

Opus muss FUER WINDOWS gebaut werden. Die Kiste `audiopus_sys` baut es
sonst mit dem Linux-Compiler - dabei entsteht eine `libopus.so` (ELF), und
der Windows-Linker findet nichts:

    cp -r /root/.cargo/registry/src/*/audiopus_sys-0.1.8/opus /tmp/opus-src
    cd /tmp/opus-src && ./autogen.sh
    ./configure --host=x86_64-w64-mingw32 --prefix=/opt/opus-win \
        --disable-shared --enable-static --disable-doc --disable-extra-programs \
        CFLAGS="-O2 -U_FORTIFY_SOURCE -D_FORTIFY_SOURCE=0"
    make -j4 && make install

Das `-D_FORTIFY_SOURCE=0` ist NICHT kosmetisch: sonst ruft Opus
`__memcpy_chk`/`__memset_chk` auf, die Rust beim Linken (`-nodefaultlibs`)
nicht mitbringt - der Build scheitert erst ganz am Ende beim Linken.

## Bauen

    export PKG_CONFIG_ALLOW_CROSS=1
    export PKG_CONFIG_PATH=/opt/opus-win/lib/pkgconfig
    cargo build --release --target x86_64-pc-windows-gnu --bin freeviewer

Ergebnis: target/x86_64-pc-windows-gnu/release/freeviewer.exe

## Was dabei fehlt

- **Kein Programmsymbol.** build.rs braucht `rc.exe` aus dem Windows-SDK;
  ohne das bekommt die Datei kein Icon. Fuers Testen egal, fuer eine
  Auslieferung nicht - dafuer weiter auf Windows bauen.
- **Nicht signiert.**
- Getestet werden muss weiterhin auf Windows: Media Foundation (Kamera),
  D3D11 (Bildschirm) und die Fenster-Oberflaeche laufen nur dort wirklich.
