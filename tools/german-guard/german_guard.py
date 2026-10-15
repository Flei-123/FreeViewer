#!/usr/bin/env python3
"""German-in-code guard (r37).

Counts German comment lines, identifiers and string literals in tracked
source files and compares them with baseline.json. Fails (exit 1) if any
count rises above the baseline.

Usage:
  german_guard.py [--root DIR] [--baseline FILE] [--json] [--update] [--list KIND[:PATH]]
  --update  write current counts as the new baseline (only allowed to lower
            unless --force is given)
Heuristic only: false positives are possible; the baseline absorbs them.
"""
import argparse, json, os, re, subprocess, sys

STOP = set("""der die das und nicht ist wird mit für fuer von zu den dem ein eine einen auf wenn oder
aber auch als bei nach nur wir sind kein keine noch dann wieder sonst damit weil wurde werden kann
können sich es im am an zum zur über ueber vor beim ohne durch dass diese dieser dieses jede jeder
alle schon immer hier gibt eigene eigenen neue neuen alte alten gleich""".split())
# words that are German and unlikely in English identifiers
IDWORDS = set("""
verbindung passwort fenster schirm konto freunde freund gesehen anzeige zustand einstellungen einstellung nachricht nachrichten teilnehmer kamera_an stumm lautstaerke datei ordner fehler fertig starten beenden trennen senden empfangen verbinden schliessen oeffnen dauer breite hoehe lesen schreiben suche suchen rechner nutzer benutzer kennwort sitzung steuerung zugriff erlauben verweigern anfrage bildschirm taste tastatur zwischenablage aktualisieren aktualisierung gruppe eintrag eintraege schluessel pruefen pruefung zaehler laenge anzahl sperren entsperren abbrechen zurueck wahr falsch meldung vorschau hintergrund vordergrund farbe linie kreis flaeche ausgabe eingabe
""".split())
UML = re.compile('[äöüÄÖÜß]')
WORD = re.compile(r"[a-zäöüß]+")
COMMENT_EXT = {'rs': ('//', '/*', '*'), 'js': ('//', '/*', '*'), 'mjs': ('//', '/*', '*'),
               'py': ('#',), 'sh': ('#',)}
STR_RE = re.compile(r'"((?:[^"\\]|\\.)*)"')
ID_RE = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")
EOL_RE = re.compile(r'^(?P<code>[^"\']*?\S)\s+//\s?(?P<c>.*)$')


def is_german_text(t, min_stop=2):
    ws = WORD.findall(t.lower())
    return bool(UML.search(t)) or sum(1 for w in ws if w in STOP) >= min_stop


def full_comment(line, ext):
    s = line.strip()
    if ext in ('py', 'sh'):
        return s.startswith('#') and not s.startswith('#!')
    return s.startswith(('//', '/*', '*'))


def id_german(name):
    if UML.search(name):
        return True
    parts = [p for p in re.split(r'_|(?<=[a-z])(?=[A-Z])', name) if p]
    return any(p.lower() in IDWORDS for p in parts)


def strip_strings(line):
    return STR_RE.sub('""', line)


def files(root):
    out = subprocess.check_output(['git', 'ls-files', 'src', 'relay', 'web-patch', 'tools'],
                                  cwd=root, text=True).split('\n')
    res = []
    for f in out:
        if not f or 'node_modules' in f or f.startswith('tools/german-guard/'):
            continue
        if f.rsplit('.', 1)[-1] in COMMENT_EXT:
            res.append(f)
    return res


def scan(root):
    counts = {'comments': 0, 'eol_comments': 0, 'identifiers': 0, 'strings': 0}
    per = {k: {} for k in counts}
    hits = []
    for f in files(root):
        ext = f.rsplit('.', 1)[1]
        try:
            lines = open(os.path.join(root, f), errors='replace').read().split('\n')
        except OSError:
            continue
        in_test = False
        for i, l in enumerate(lines, 1):
            def hit(kind, text):
                counts[kind] += 1
                per[kind][f] = per[kind].get(f, 0) + 1
                hits.append((kind, f, i, text.strip()[:100]))
            if full_comment(l, ext):
                if is_german_text(l):
                    hit('comments', l)
                continue
            if ext in ('rs', 'js', 'mjs'):
                m = EOL_RE.match(l)
                if m and is_german_text(m.group('c')):
                    hit('eol_comments', m.group('c'))
                    l = m.group('code')
            if ext in ('py', 'sh') or not l.strip():
                pass
            for sm in STR_RE.finditer(l):
                if is_german_text(sm.group(1)):
                    hit('strings', sm.group(1))
            code = strip_strings(l)
            for idm in set(ID_RE.findall(code)):
                if id_german(idm):
                    hit('identifiers', idm)
    return counts, per, hits


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--root', default=os.path.abspath(os.path.join(os.path.dirname(__file__), '..', '..')))
    ap.add_argument('--baseline', default=os.path.join(os.path.dirname(os.path.abspath(__file__)), 'baseline.json'))
    ap.add_argument('--json', action='store_true')
    ap.add_argument('--update', action='store_true')
    ap.add_argument('--force', action='store_true')
    ap.add_argument('--list', metavar='KIND[:PATH]')
    a = ap.parse_args()
    counts, per, hits = scan(a.root)
    if a.list:
        kind, _, path = a.list.partition(':')
        for k, f, i, t in hits:
            if k == kind and (not path or f.startswith(path)):
                print(f'{f}:{i}: {t}')
        return 0
    if a.json:
        print(json.dumps(counts))
        return 0
    base = json.load(open(a.baseline)) if os.path.exists(a.baseline) else None
    print('german-guard counts:', ', '.join(f'{k}={v}' for k, v in counts.items()))
    if a.update:
        if base and not a.force:
            for k, v in counts.items():
                if v > base['total'].get(k, 0):
                    print(f'refusing to raise baseline for {k} (use --force)')
                    return 1
        json.dump({'total': counts, 'per_file': per}, open(a.baseline, 'w'), indent=1, sort_keys=True)
        print('baseline written')
        return 0
    if base is None:
        print('no baseline; run with --update')
        return 1
    bad = 0
    for k, v in counts.items():
        b = base['total'].get(k, 0)
        if v > b:
            bad = 1
            print(f'FAIL {k}: {v} > baseline {b}')
            for f, n in sorted(per[k].items()):
                nb = base['per_file'].get(k, {}).get(f, 0)
                if n > nb:
                    print(f'  {f}: {n} (baseline {nb})')
        elif v < b:
            print(f'ok {k}: {v} < baseline {b} (lower the baseline with --update)')
    print('FAIL' if bad else 'OK')
    return bad


if __name__ == '__main__':
    sys.exit(main())
