#!/usr/bin/env python3
"""Turn README.md into docs/tutorial.html, a single light-themed page with a
table of contents, and copy it to docs/index.html so the docs/ folder works as
a static site (Vercel serves index.html at the root; see vercel.json). Covers the Markdown this repository uses: headings, text,
bold, inline code, links, lists, tables, quotes and fenced code (wireframes).
Usage: python3 scripts/build-tutorial.py [--fragment OUT]  (fragment = no html/head/body)"""
import html, re, sys, pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent
md = (ROOT / 'README.md').read_text()

def slug(t):
    t = re.sub(r'<[^>]+>', '', t)
    return re.sub(r'[^a-z0-9]+', '-', t.lower()).strip('-')

def inline(t):
    t = html.escape(t, quote=False)
    t = re.sub(r'`([^`]+)`', r'<code>\1</code>', t)
    t = re.sub(r'\*\*([^*]+)\*\*', r'<strong>\1</strong>', t)
    t = re.sub(r'(?<![\w*])\*([^*]+)\*(?![\w*])', r'<em>\1</em>', t)
    t = re.sub(r'\[([^\]]+)\]\(([^)]+)\)', lambda m: '<a href="%s">%s</a>' % (m.group(2), m.group(1)), t)
    return t

out, toc = [], []
lines = md.splitlines()
i = 0
in_list = None
def close_list():
    global in_list
    if in_list:
        out.append('</%s>' % in_list); in_list = None
while i < len(lines):
    l = lines[i]
    if l.startswith('```'):
        close_list()
        lang = l[3:].strip(); buf = []; i += 1
        while i < len(lines) and not lines[i].startswith('```'):
            buf.append(lines[i]); i += 1
        cls = ' class="wire"' if not lang and any(ch in '┌─│└┐┘├┤▾►▰' for ch in ''.join(buf)) else ''
        out.append('<pre%s><code>%s</code></pre>' % (cls, html.escape('\n'.join(buf))))
        i += 1; continue
    m = re.match(r'^(#{1,4}) (.*)', l)
    if m:
        close_list()
        n = len(m.group(1)); text = inline(m.group(2)); s = slug(m.group(2))
        if n == 2: toc.append((s, re.sub(r'<[^>]+>', '', text)))
        out.append('<h%d id="%s">%s</h%d>' % (n, s, text, n)); i += 1; continue
    if l.startswith('|'):
        close_list()
        rows = []
        while i < len(lines) and lines[i].startswith('|'):
            rows.append([c.strip() for c in lines[i].strip().strip('|').split('|')]); i += 1
        head, body = rows[0], [r for r in rows[1:] if not all(re.match(r'^:?-+:?$', c) for c in r)]
        t = '<div class="tbl"><table><thead><tr>' + ''.join('<th>%s</th>' % inline(c) for c in head) + '</tr></thead><tbody>'
        for r in body:
            t += '<tr>' + ''.join('<td>%s</td>' % inline(c) for c in r) + '</tr>'
        out.append(t + '</tbody></table></div>'); continue
    if l.startswith('> '):
        close_list()
        buf = []
        while i < len(lines) and lines[i].startswith('> '):
            buf.append(lines[i][2:]); i += 1
        out.append('<blockquote>%s</blockquote>' % inline(' '.join(buf))); continue
    m = re.match(r'^(\d+)\. (.*)', l)
    if m or l.startswith('- '):
        kind = 'ol' if m else 'ul'
        if in_list != kind:
            close_list(); out.append('<%s>' % kind); in_list = kind
        out.append('<li>%s</li>' % inline(m.group(2) if m else l[2:])); i += 1; continue
    if not l.strip():
        close_list(); i += 1; continue
    close_list()
    buf = [l]; i += 1
    while i < len(lines) and lines[i].strip() and not re.match(r'^(#{1,4} |```|\||> |- |\d+\. )', lines[i]):
        buf.append(lines[i]); i += 1
    out.append('<p>%s</p>' % inline(' '.join(buf)))
close_list()

body = '\n'.join(out)
nav = '<nav class="toc" aria-label="Contents"><div class="toc-h">Contents</div>' + ''.join('<a href="#%s">%s</a>' % (s, t) for s, t in toc) + '</nav>'
style = '''<style>
/* Layout: a fixed contents rail on wide screens, one column on phones; wireframes are monospace blocks that scroll sideways. */
:root{--bg:#fbfbf9;--panel:#ffffff;--ink:#1b1b20;--muted:#6b6b75;--line:#e5e5e0;--accent:#2b5cff;--accent-soft:#eef2ff;--code-bg:#f3f3ef;--wire-bg:#1f2230;--wire-ink:#e8e8f0;--sans:"Inter",-apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif;--mono:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace}
@media (prefers-color-scheme:dark){:root:not([data-theme="light"]){--bg:#141518;--panel:#1b1c21;--ink:#ececf0;--muted:#9b9ba6;--line:#2b2c33;--accent:#8aa2ff;--accent-soft:#1e2540;--code-bg:#22242b;--wire-bg:#0f1017;--wire-ink:#e8e8f0;color-scheme:dark}}
:root[data-theme="dark"]{--bg:#141518;--panel:#1b1c21;--ink:#ececf0;--muted:#9b9ba6;--line:#2b2c33;--accent:#8aa2ff;--accent-soft:#1e2540;--code-bg:#22242b;--wire-bg:#0f1017;--wire-ink:#e8e8f0;color-scheme:dark}
body{margin:0;background:var(--bg);color:var(--ink);font:16px/1.6 var(--sans);padding-left:max(16px,env(safe-area-inset-left,0px));padding-right:max(16px,env(safe-area-inset-right,0px));padding-block:0}
.wrap{display:grid;grid-template-columns:240px minmax(0,1fr);gap:40px;max-width:1180px;margin:0 auto}
.toc{position:sticky;top:0;align-self:start;max-height:100vh;overflow:auto;padding:28px 0;font-size:13.5px;display:flex;flex-direction:column;gap:4px}
.toc-h{font-size:11px;letter-spacing:.08em;text-transform:uppercase;color:var(--muted);margin-bottom:6px}
.toc a{color:var(--ink);text-decoration:none;padding:4px 8px;border-radius:7px}.toc a:hover{background:var(--accent-soft)}
main{min-width:0;padding:28px 0 80px;max-width:78ch}
h1{font-size:40px;line-height:1.1;margin:8px 0 10px;letter-spacing:-.02em;text-wrap:balance}
h2{font-size:26px;margin:52px 0 10px;letter-spacing:-.01em;text-wrap:balance;padding-top:18px;border-top:1px solid var(--line)}
h3{font-size:19px;margin:30px 0 8px}h4{font-size:16px;margin:22px 0 6px}
p{margin:0 0 14px}a{color:var(--accent)}
blockquote{margin:0 0 16px;padding:10px 14px;border-left:3px solid var(--accent);background:var(--accent-soft);border-radius:0 10px 10px 0;color:var(--ink)}
code{font-family:var(--mono);font-size:.9em;background:var(--code-bg);padding:1px 5px;border-radius:5px}
pre{margin:0 0 16px;background:var(--code-bg);border:1px solid var(--line);border-radius:10px;padding:12px 14px;overflow-x:auto;font-size:13px;line-height:1.5}pre code{background:transparent;padding:0;font-size:inherit}
pre.wire{background:var(--wire-bg);color:var(--wire-ink);border-color:transparent;font-size:12.5px;line-height:1.35}
ul,ol{margin:0 0 14px;padding-left:22px}li{margin:4px 0}
.tbl{overflow-x:auto;margin:0 0 18px;border:1px solid var(--line);border-radius:10px;background:var(--panel)}
table{border-collapse:collapse;width:100%;font-size:14px}th,td{text-align:left;padding:8px 10px;border-bottom:1px solid var(--line);vertical-align:top}th{color:var(--muted);font-weight:600;font-size:12px;letter-spacing:.04em;text-transform:uppercase}tr:last-child td{border-bottom:0}
strong{font-weight:650}
@media (max-width:860px){.wrap{grid-template-columns:1fr;gap:0}.toc{position:static;max-height:none;padding:18px 0 0;flex-direction:row;flex-wrap:wrap}.toc-h{width:100%}h1{font-size:30px}h2{font-size:22px}}
@media (prefers-reduced-motion:no-preference){html{scroll-behavior:smooth}}
</style>'''
fonts = '<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;650&display=swap">'
inner = '<title>Lorefile Tutorial</title>\n<meta name="claude:viewport-fit-sides" content="cover">\n' + fonts + '\n' + style + '\n<div class="wrap">' + nav + '<main>' + body + '</main></div>'
full = '<!doctype html>\n<html lang="en">\n<head>\n<meta charset="utf-8">\n<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">\n' + inner.replace('<meta name="claude:viewport-fit-sides" content="cover">\n', '') + '\n</body>\n</html>\n'
# the full document needs <body> open after </style>; rebuild cleanly
full = ('<!doctype html>\n<html lang="en">\n<head>\n<meta charset="utf-8">\n<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">\n<title>Lorefile Tutorial</title>\n'
        + fonts + '\n' + style + '\n</head>\n<body>\n<div class="wrap">' + nav + '<main>' + body + '</main></div>\n</body>\n</html>\n')
(ROOT / 'docs' / 'tutorial.html').write_text(full)
# The same page at the site root: docs/ is published as a static site (vercel.json),
# and a static host serves index.html for "/". A copy, not a redirect, so the root
# URL shows the tutorial straight away. tutorial.html stays for existing links.
(ROOT / 'docs' / 'index.html').write_text(full)
if '--fragment' in sys.argv:
    pathlib.Path(sys.argv[sys.argv.index('--fragment') + 1]).write_text(inner)
print('wrote docs/tutorial.html and docs/index.html', len(full), 'bytes each;', len(toc), 'sections')
