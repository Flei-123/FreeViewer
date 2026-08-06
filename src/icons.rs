//! The icon set. Every symbol is an inline SVG (24x24, stroke based, white),
//! rasterized by egui_extras/resvg and tinted to the colour we need - the same
//! set of shapes FreeMeet uses on the web, so both programs look related.
//!
//! Add a symbol: one entry in ICONS, then `icons::show(ui, "name", 18.0, col)`.

/// name -> SVG source
pub const ICONS: &[(&str, &str)] = &[
    (
        "home",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M3 10.5 12 3l9 7.5"/><path d="M5.5 9.5V20h13V9.5"/><path d="M9.5 20v-6h5v6"/></svg>"##,
    ),
    (
        "devices",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="2.5" y="4" width="13" height="9.5" rx="1.6"/><path d="M6 17h6"/><rect x="17" y="9" width="4.5" height="11" rx="1.4"/></svg>"##,
    ),
    (
        "monitor",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="12" rx="1.8"/><path d="M9 20h6M12 16v4"/></svg>"##,
    ),
    (
        "laptop",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="4" y="5" width="16" height="10" rx="1.6"/><path d="M2 18.5h20"/></svg>"##,
    ),
    (
        "chat",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M20 12.5c0 3.6-3.6 6.5-8 6.5-1 0-2-.15-2.9-.42L5 20.5l1.2-3.1C4.85 16.2 4 14.45 4 12.5 4 8.9 7.6 6 12 6s8 2.9 8 6.5z"/></svg>"##,
    ),
    (
        "settings",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M22.12 10.76 L22.12 13.24 L19.32 14.03 L18.62 15.74 L20.03 18.28 L18.28 20.03 L15.74 18.62 L14.03 19.32 L13.24 22.12 L10.76 22.12 L9.97 19.32 L8.26 18.62 L5.72 20.03 L3.97 18.28 L5.38 15.74 L4.68 14.03 L1.88 13.24 L1.88 10.76 L4.68 9.97 L5.38 8.26 L3.97 5.72 L5.72 3.97 L8.26 5.38 L9.97 4.68 L10.76 1.88 L13.24 1.88 L14.03 4.68 L15.74 5.38 L18.28 3.97 L20.03 5.72 L18.62 8.26 L19.32 9.97 Z"/><circle cx="12" cy="12" r="3.4"/></svg>"##,
    ),
    (
        "palette",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3a9 9 0 1 0 0 18c1.2 0 1.8-.9 1.8-1.9 0-1.6 1.1-2.1 2.4-2.1H18a3 3 0 0 0 3-3 9 9 0 0 0-9-9z"/><circle cx="8" cy="10" r="1.1"/><circle cx="12" cy="7.5" r="1.1"/><circle cx="15.8" cy="10" r="1.1"/></svg>"##,
    ),
    (
        "search",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="6.5"/><path d="m20 20-4.2-4.2"/></svg>"##,
    ),
    (
        "mic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="2.5" width="6" height="11" rx="3"/><path d="M5.5 11.5a6.5 6.5 0 0 0 13 0"/><path d="M12 18v3.5M8.5 21.5h7"/></svg>"##,
    ),
    (
        "mic-off",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M15 5.5V5a3 3 0 0 0-6 0v5.5"/><path d="M5.5 11.5a6.5 6.5 0 0 0 10.2 5.3"/><path d="M12 18v3.5M8.5 21.5h7"/><path d="M3 3l18 18"/></svg>"##,
    ),
    (
        "sound",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4 9.5h3.5L12 5.5v13L7.5 14.5H4z"/><path d="M16 9.5a4 4 0 0 1 0 5"/><path d="M18.5 7a7 7 0 0 1 0 10"/></svg>"##,
    ),
    (
        "sound-off",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4 9.5h3.5L12 5.5v13L7.5 14.5H4z"/><path d="M16 10l5 4M21 10l-5 4"/></svg>"##,
    ),
    (
        "files",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M3.5 6.5A1.5 1.5 0 0 1 5 5h4l2 2.5h6.5A1.5 1.5 0 0 1 19 9v8.5A1.5 1.5 0 0 1 17.5 19H5a1.5 1.5 0 0 1-1.5-1.5z"/></svg>"##,
    ),
    (
        "keyboard",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="2.5" y="6" width="19" height="12" rx="2"/><path d="M6 10h.01M9.5 10h.01M13 10h.01M16.5 10h.01M7.5 14h9"/></svg>"##,
    ),
    (
        "gamepad",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M7.5 8h9a4.5 4.5 0 0 1 4.3 5.8l-.7 2.4A2.6 2.6 0 0 1 16 17.4L14.5 15.5h-5L8 17.4a2.6 2.6 0 0 1-4.1-1.2l-.7-2.4A4.5 4.5 0 0 1 7.5 8z"/><path d="M7 11.7v2M6 12.7h2M16.5 12h.01M15 13.5h.01"/></svg>"##,
    ),
    (
        "trash",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4 6.5h16"/><path d="M9.5 6.5V4.8A1.3 1.3 0 0 1 10.8 3.5h2.4a1.3 1.3 0 0 1 1.3 1.3v1.7"/><path d="M6.5 6.5 7.4 19a1.6 1.6 0 0 0 1.6 1.5h6a1.6 1.6 0 0 0 1.6-1.5l.9-12.5"/><path d="M10.5 10v6.5M13.5 10v6.5"/></svg>"##,
    ),
    (
        "meet",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="2.5" y="6" width="13" height="12" rx="2.2"/><path d="m15.5 11 5-3v8l-5-3z"/></svg>"##,
    ),
    (
        "plus",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M12 5v14M5 12h14"/></svg>"##,
    ),
    (
        "dots",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="#fff" stroke="none"><circle cx="12" cy="5.5" r="1.6"/><circle cx="12" cy="12" r="1.6"/><circle cx="12" cy="18.5" r="1.6"/></svg>"##,
    ),
    (
        "chevron-down",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6"/></svg>"##,
    ),
    (
        "chevron-right",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m9 6 6 6-6 6"/></svg>"##,
    ),
    (
        "copy",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="11" height="11" rx="1.6"/><path d="M15 6.5V5.6A1.6 1.6 0 0 0 13.4 4H5.6A1.6 1.6 0 0 0 4 5.6v7.8A1.6 1.6 0 0 0 5.6 15h.9"/></svg>"##,
    ),
    (
        "refresh",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M20 12a8 8 0 1 1-2.4-5.7"/><path d="M20 4v4.5h-4.5"/></svg>"##,
    ),
    (
        "connect",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4 12h9"/><path d="m10 8.5 3.5 3.5L10 15.5"/><path d="M15.5 5.5H18A2.5 2.5 0 0 1 20.5 8v8a2.5 2.5 0 0 1-2.5 2.5h-2.5"/></svg>"##,
    ),
    (
        "shield",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3l7 3v5.5c0 4.3-2.9 7.6-7 9.5-4.1-1.9-7-5.2-7-9.5V6z"/><path d="m9 12 2 2 4-4"/></svg>"##,
    ),
    (
        "star",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="m12 4 2.5 5.1 5.6.8-4 4 .9 5.6-5-2.7-5 2.7.9-5.6-4-4 5.6-.8z"/></svg>"##,
    ),
    (
        "user",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="8" r="3.6"/><path d="M4.5 20c0-3.6 3.4-5.6 7.5-5.6s7.5 2 7.5 5.6"/></svg>"##,
    ),
    (
        "pin",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M9 3.5h6l-1 5 3.5 3.5H6.5L10 8.5z"/><path d="M12 12v8.5"/></svg>"##,
    ),
    (
        "expand",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4 9V4h5"/><path d="M20 15v5h-5"/><path d="M15 4h5v5"/><path d="M9 20H4v-5"/></svg>"##,
    ),
    (
        "shrink",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M9 4v5H4"/><path d="M15 20v-5h5"/><path d="M20 9h-5V4"/><path d="M4 15h5v5"/></svg>"##,
    ),
    (
        "grip",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="#fff" stroke="none"><circle cx="9" cy="6" r="1.4"/><circle cx="15" cy="6" r="1.4"/><circle cx="9" cy="12" r="1.4"/><circle cx="15" cy="12" r="1.4"/><circle cx="9" cy="18" r="1.4"/><circle cx="15" cy="18" r="1.4"/></svg>"##,
    ),
    (
        "power",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3v9"/><path d="M6.8 6.8a7.5 7.5 0 1 0 10.4 0"/></svg>"##,
    ),
    (
        "eye-off",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M9.9 5.1A9.6 9.6 0 0 1 12 4.9c6 0 9.5 5.5 9.5 5.5a17 17 0 0 1-2.9 3.4"/><path d="M6.3 6.8A16.6 16.6 0 0 0 2.5 10.4S6 15.9 12 15.9c1.3 0 2.4-.25 3.4-.65"/><path d="M9.6 8.3a3 3 0 0 0 4.2 4.2"/><path d="M3.5 3.5l17 17"/></svg>"##,
    ),
    (
        "eye",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M2.5 12S6 6.5 12 6.5 21.5 12 21.5 12 18 17.5 12 17.5 2.5 12 2.5 12z"/><circle cx="12" cy="12" r="2.8"/></svg>"##,
    ),
    (
        "cam",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M3 8.2A2.2 2.2 0 0 1 5.2 6h7.6A2.2 2.2 0 0 1 15 8.2v7.6A2.2 2.2 0 0 1 12.8 18H5.2A2.2 2.2 0 0 1 3 15.8z"/><path d="M15 11.2l5-2.9v7.4l-5-2.9z"/></svg>"##,
    ),
    (
        "cam-off",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M9.4 6h3.4A2.2 2.2 0 0 1 15 8.2v3.2"/><path d="M15 14.6v1.2A2.2 2.2 0 0 1 12.8 18H5.2A2.2 2.2 0 0 1 3 15.8V8.2A2.2 2.2 0 0 1 5.2 6"/><path d="M15 11.2l5-2.9v7.4l-2.6-1.5"/><path d="M4 3.6l16 16.8"/></svg>"##,
    ),
    (
        "screen",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4.5" width="18" height="12.5" rx="2.2"/><path d="M9 20.5h6"/><path d="M12 17v3.5"/><path d="M12 13.5V8.6"/><path d="M9.7 10.9L12 8.6l2.3 2.3"/></svg>"##,
    ),
    (
        "screen-off",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M8.4 4.5h10.4A2.2 2.2 0 0 1 21 6.7v8.1a2.2 2.2 0 0 1-2.2 2.2h-1.4"/><path d="M13.6 17H5.2A2.2 2.2 0 0 1 3 14.8V6.7A2.2 2.2 0 0 1 5.2 4.5"/><path d="M9 20.5h6"/><path d="M12 17v3.5"/><path d="M4 3.6l16 16.8"/></svg>"##,
    ),
    (
        "hand",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M8.2 12.4V6.6a1.5 1.5 0 1 1 3 0V11"/><path d="M11.2 11V4.9a1.5 1.5 0 1 1 3 0V11"/><path d="M14.2 11V6.6a1.5 1.5 0 1 1 3 0v6.6c0 4.1-2.7 7.3-6.6 7.3s-6.4-3.2-6.4-7.3v-1.4a1.5 1.5 0 1 1 3 0v1.1"/></svg>"##,
    ),
    (
        "leave",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M3.4 9.6c5.2-3.7 12-3.7 17.2 0v3.1c0 1-.9 1.8-1.9 1.6l-2.4-.3a1.8 1.8 0 0 1-1.5-1.7l-.1-1.2a11.6 11.6 0 0 0-5.4 0l-.1 1.2a1.8 1.8 0 0 1-1.5 1.7l-2.4.3a1.8 1.8 0 0 1-1.9-1.6z"/><path d="M6.5 17.6l11-4"/></svg>"##,
    ),
    (
        "end",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="8.8"/><path d="M9 9l6 6"/><path d="M15 9l-6 6"/></svg>"##,
    ),
    (
        "crown",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M3.6 7.4l3.8 3.2L12 4.6l4.6 6 3.8-3.2-1.6 10.2H5.2z"/></svg>"##,
    ),
    (
        "signal",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4 20v-3.6"/><path d="M9.3 20v-7.2"/><path d="M14.7 20V8.4"/><path d="M20 20V4"/></svg>"##,
    ),
    (
        "pip",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="5" width="18" height="14" rx="2.4"/><rect x="12.2" y="11.4" width="7" height="6" rx="1.4"/></svg>"##,
    ),
    (
        "pip-off",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M8.6 5H18.6A2.4 2.4 0 0 1 21 7.4v9.2a2.4 2.4 0 0 1-2.4 2.4h-1"/><path d="M13.6 19H5.4A2.4 2.4 0 0 1 3 16.6V7.4A2.4 2.4 0 0 1 5.4 5"/><path d="M4 3.6l16 16.8"/></svg>"##,
    ),
    (
        "layout",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="5" width="11.5" height="14" rx="2"/><rect x="16.5" y="5" width="4.5" height="6.5" rx="1.4"/><rect x="16.5" y="12.5" width="4.5" height="6.5" rx="1.4"/></svg>"##,
    ),
    (
        "full",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M9 4H4v5"/><path d="M15 4h5v5"/><path d="M15 20h5v-5"/><path d="M9 20H4v-5"/></svg>"##,
    ),
    (
        "people",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><circle cx="9.2" cy="8.4" r="3.3"/><path d="M3.4 19.8a5.8 5.8 0 0 1 11.6 0"/><path d="M16.2 5.6a3.3 3.3 0 0 1 0 6.2"/><path d="M17.6 14.2a5.8 5.8 0 0 1 3 5"/></svg>"##,
    ),
    (
        "info",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="8.8"/><path d="M12 11.2v5.2"/><path d="M12 7.7h.01"/></svg>"##,
    ),
    (
        "send",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4.4 11.6L20 4.2l-7.4 15.6-2.4-6.4z"/><path d="M10.2 13.4L20 4.2"/></svg>"##,
    ),
    (
        "muteall",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M11 5.4L6.4 9.2H3.2v5.6h3.2L11 18.6z"/><path d="M15.4 9.6l4.4 4.8"/><path d="M19.8 9.6l-4.4 4.8"/></svg>"##,
    ),
    (
        "check",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4.8 12.8l4.4 4.4L19.2 6.8"/></svg>"##,
    ),
    (
        "back",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M19.2 12H4.8"/><path d="M10.4 6.4L4.8 12l5.6 5.6"/></svg>"##,
    ),
    (
        "x",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M6.2 6.2l11.6 11.6"/><path d="M17.8 6.2L6.2 17.8"/></svg>"##,
    ),
    (
        "link",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M10.2 13.8a4.2 4.2 0 0 0 5.9 0l2.4-2.4a4.2 4.2 0 0 0-5.9-5.9l-1.3 1.3"/><path d="M13.8 10.2a4.2 4.2 0 0 0-5.9 0l-2.4 2.4a4.2 4.2 0 0 0 5.9 5.9l1.3-1.3"/></svg>"##,
    ),
    (
        "enter",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M13.6 3.6h4.2A2.2 2.2 0 0 1 20 5.8v12.4a2.2 2.2 0 0 1-2.2 2.2h-4.2"/><path d="M4 12h9.6"/><path d="M10 8.4l3.6 3.6-3.6 3.6"/></svg>"##,
    ),
    (
        "bild",
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="3.4" y="4.6" width="17.2" height="14.8" rx="2.4"/><circle cx="9" cy="10" r="1.8"/><path d="M4 17.4l4.8-4.4 3.6 3.2 3-2.6 4.6 4"/></svg>"##,
    ),
];

fn source(name: &str) -> &'static str {
    ICONS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, s)| *s)
        .unwrap_or(ICONS[0].1)
}

/// The icon as an egui image, already tinted.
pub fn image(name: &str, size: f32, color: egui::Color32) -> egui::Image<'static> {
    let uri = ICONS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(n, _)| *n)
        .unwrap_or("home");
    egui::Image::from_bytes(
        format!("bytes://fv-icon-{}.svg", uri),
        source(name).as_bytes(),
    )
    .fit_to_exact_size(egui::vec2(size, size))
    .tint(color)
}

/// Draw an icon inline (no interaction).
pub fn show(ui: &mut egui::Ui, name: &str, size: f32, color: egui::Color32) {
    ui.add(image(name, size, color));
}

/// A flat icon button, e.g. in a table row.
pub fn button(ui: &mut egui::Ui, name: &str, size: f32, color: egui::Color32) -> egui::Response {
    ui.add(
        egui::Button::image(image(name, size, color))
            .fill(egui::Color32::TRANSPARENT)
            .stroke(egui::Stroke::NONE)
            .min_size(egui::vec2(size + 10.0, size + 8.0)),
    )
}

/// Icon + text button used in the toolbars.
pub fn text_button(
    ui: &mut egui::Ui,
    name: &str,
    label: &str,
    on: bool,
) -> egui::Response {
    let col = if on {
        crate::theme::accent()
    } else {
        crate::theme::muted()
    };
    let r = ui.add(
        egui::Button::image_and_text(
            image(name, 16.0, col),
            egui::RichText::new(label).size(12.5).color(if on {
                crate::theme::text()
            } else {
                crate::theme::muted()
            }),
        )
        .fill(if on {
            crate::theme::accent().gamma_multiply(0.16)
        } else {
            egui::Color32::TRANSPARENT
        })
        .stroke(egui::Stroke::NONE),
    );
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_wellformed_svg() {
        for (n, s) in ICONS.iter() {
            assert!(s.starts_with("<svg"), "{} faengt nicht mit <svg an", n);
            assert!(s.ends_with("</svg>"), "{} endet nicht mit </svg>", n);
            assert!(s.contains("viewBox=\"0 0 24 24\""), "{} ohne viewBox", n);
        }
    }

    /// Ein Symbol darf nicht MITTEN im Strich aufhoeren.
    ///
    /// Genau das war beim Zahnrad der Fall: der Pfad endete mit
    /// "...c.66.28 1.09.92 1.09 1.65" und kehrte nie zum Anfang zurueck -
    /// die rechte Seite des Zahnrads fehlte einfach. Justin hat es gesehen,
    /// kein Test hatte es gemerkt. Ein GESCHLOSSENER Umriss endet auf "Z".
    #[test]
    fn geschlossene_umrisse_sind_wirklich_geschlossen() {
        // Diese Symbole sind Umrisse (keine offenen Striche) und muessen
        // deshalb zurueck zum Anfang laufen.
        for name in ["settings"] {
            let s = source(name);
            let d = s
                .split("d=\"")
                .nth(1)
                .and_then(|r| r.split('"').next())
                .unwrap_or("");
            assert!(!d.is_empty(), "{} hat keinen Pfad", name);
            let letzte = d.trim().chars().last().unwrap_or(' ');
            assert!(
                letzte == 'Z' || letzte == 'z',
                "{} endet auf '{}' statt geschlossen zu sein: ...{}",
                name,
                letzte,
                &d[d.len().saturating_sub(40)..]
            );
        }
    }

    /// Ein Zahnrad ist rund - also muss es links wie rechts gleich weit
    /// reichen. Fehlt eine Seite, faellt das hier auf.
    #[test]
    fn das_zahnrad_ist_symmetrisch() {
        let s = source("settings");
        let d = s.split("d=\"").nth(1).unwrap().split('"').next().unwrap();
        let zahlen: Vec<f32> = d
            .replace('M', " ")
            .replace('L', " ")
            .replace('Z', " ")
            .split_whitespace()
            .filter_map(|t| t.parse::<f32>().ok())
            .collect();
        assert!(zahlen.len() >= 16, "zu wenige Punkte: {}", zahlen.len());
        let xs: Vec<f32> = zahlen.iter().step_by(2).copied().collect();
        let links = 12.0 - xs.iter().cloned().fold(f32::MAX, f32::min);
        let rechts = xs.iter().cloned().fold(f32::MIN, f32::max) - 12.0;
        assert!(
            (links - rechts).abs() < 0.2,
            "Zahnrad haengt schief: links {:.2}, rechts {:.2}",
            links,
            rechts
        );
    }

    #[test]
    fn names_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for (n, _) in ICONS.iter() {
            assert!(seen.insert(*n), "Symbol {} doppelt", n);
        }
    }

    #[test]
    fn unknown_name_gives_a_shape_instead_of_panicking() {
        assert!(source("gibtsnicht").starts_with("<svg"));
    }
}
