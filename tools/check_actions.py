#!/usr/bin/env python3
"""CI check of the FreeViewer action manifest (api/ACTIONS): lints clean, and every flag it names is really
handled in src/main.rs (an action `foo_bar` is the flag --foo-bar; a bool argument or `view` is a flag too;
`state` and `file` are positional). Also: tools/manifest_lint.py is Firn's linter (a differing copy is a note).

    python3 tools/check_actions.py
"""
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, HERE)
import manifest_lint  # noqa: E402

POSITIONAL = {'state', 'file'}
bad = 0
text = open(os.path.join(ROOT, 'api', 'ACTIONS'), encoding='utf8').read()
r = manifest_lint.lint(text, 'api/ACTIONS')
for w in r.warnings:
    print('warning: ' + w)
for e in r.errors:
    print('FAIL ' + e)
    bad = 1
if not r.errors:
    print('ok   api/ACTIONS lints clean (%d actions)' % len(r.actions))

main = open(os.path.join(ROOT, 'src', 'main.rs'), encoding='utf8').read()
flags = set(re.findall(r'"(--[a-z0-9-]+)"', main))
missing = []
for a in r.actions:
    name = a['name'].split('.', 1)[1]
    need = ['--' + name.replace('_', '-')]
    for arg in a['args']:
        if arg['name'] in POSITIONAL:
            continue
        need.append('--' + arg['name'].replace('_', '-'))
    for f in need:
        if f not in flags:
            missing.append('%s needs %s' % (a['name'], f))
if missing:
    for m in missing:
        print('FAIL not handled in src/main.rs: ' + m)
    bad = 1
else:
    print('ok   every flag of the %d actions is handled in src/main.rs' % len(r.actions))

firn = os.environ.get('FIRN', '/root/jarvis/projects/u_DiS4in7esMF1/firn')
ref = os.path.join(firn, 'tools', 'manifest-lint', 'manifest_lint.py')
if os.path.exists(ref):
    same = open(ref, encoding='utf8').read() == open(os.path.join(HERE, 'manifest_lint.py'), encoding='utf8').read()
    print("ok   tools/manifest_lint.py is Firn's linter" if same else "note tools/manifest_lint.py differs from Firn's -- sync it")
sys.exit(bad)
