# -*- coding: utf-8 -*-
import io, sys

def patch(path, pairs):
    with io.open(path, "r", encoding="utf-8") as f:
        s = f.read()
    for old, new in pairs:
        n = s.count(old)
        if n == 0:
            print("MISSING in %s: %r" % (path, old[:60]))
            sys.exit(1)
        s = s.replace(old, new)
        print("OK  %dx  %s  <-  %r" % (n, path, old[:50]))
    with io.open(path, "w", encoding="utf-8", newline="") as f:
        f.write(s)

patch(r"C:\FreeViewer\src\main.rs", [
    ("let id_text = if my_id.len() == 9 {",
     "let id_text = if my_id.len() >= 9 {"),
    ("        if id.len() != 9 {\n            e.err = i18n::t(\"dev.add_bad_id\").to_string();",
     "        if !(9..=10).contains(&id.len()) {\n            e.err = i18n::t(\"dev.add_bad_id\").to_string();"),
    (".set_viewer_status(\"Bitte 9-stellige Partner-ID eingeben\");",
     ".set_viewer_status(i18n::t(\"start.bad_id\"));"),
])

patch(r"C:\FreeViewer\src\i18n.rs", [
    ("(\"dev.id9\", \"Erst eine 9-stellige ID eingeben\", \"Enter a 9 digit ID first\"),",
     "(\"dev.id9\", \"Erst eine ID eingeben (9–10 Ziffern)\", \"Enter an ID first (9-10 digits)\"),"),
    ("(\"dev.add_id_hint\", \"9-stellige ID, z. B. 497 628 420\", \"9 digit ID, e.g. 497 628 420\"),",
     "(\"dev.add_id_hint\", \"ID, z. B. 497 628 420 oder 1 298 814 267\", \"ID, e.g. 497 628 420 or 1 298 814 267\"),"),
    ("(\"dev.add_bad_id\", \"Die ID besteht aus 9 Ziffern\", \"The ID is 9 digits long\"),",
     "(\"dev.add_bad_id\", \"Die ID besteht aus 9–10 Ziffern\", \"The ID is 9-10 digits long\"),"),
    ("    (\"dev.newname\", \"Neuer Name\", \"New name\"),",
     "    (\"start.bad_id\", \"Bitte eine gültige Partner-ID eingeben (9–10 Ziffern)\", \"Please enter a valid partner ID (9-10 digits)\"),\n    (\"dev.newname\", \"Neuer Name\", \"New name\"),"),
])
print("ALL PATCHES APPLIED")
