#!/usr/bin/env python3
"""Builds the website of Project Hoopeo into site/_site/.

  python3 site/build.py            build
  python3 site/build.py --check    build, then check every internal link and every fragment

Python 3 with the `markdown` package (pip install markdown). Nothing else: no framework, no other build step.

BASE_URL is the one place where the address of the site is written. It is read from the environment
if it is set, so a custom domain, or an organization named project-hoopeo, needs no change in the sources:
  BASE_URL=https://example.org/ python3 site/build.py
Every link the pages contain is relative to it; no page hard-codes it.

The pages come from Markdown in the repository: site/content/, docs/, the license files and the playground
README. A link from one Markdown file to another, written as a repository path, is rewritten to the page that
the file becomes, and a link to any other file of the repository becomes a link to GitHub.
"""
import html
import os
import re
import shutil
import subprocess
import sys
from urllib.parse import urlparse

import markdown

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import glossary_terms as GT

BASE_URL = os.environ.get('BASE_URL') or 'https://mandelreouven2002.github.io/project-hoopeo.github.io/'
if not BASE_URL.endswith('/'):
    BASE_URL += '/'
BASE = urlparse(BASE_URL).path            # '/project-hoopeo.github.io/'

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
OUT = os.path.join(HERE, '_site')
GITHUB = 'https://github.com/mandelreouven2002/project-hoopeo.github.io'
AUTHOR_SITE = 'https://lp.reuven.click'
SITE = 'Project Hoopeo'

# ---------------------------------------------------------------- the pages

# repository path -> (url path under the site, nav label)
START = [
    ('docs/guides/overview.md', 'DEP SQL in five minutes'),
    ('docs/guides/getting-started.md', 'Getting started'),
]
GUIDES = [
    ('docs/guides/concepts.md', 'Concepts'),
    ('docs/guides/dep-sql.md', 'DEP SQL'),
    ('docs/guides/transactions-and-constraints.md', 'Transactions and constraints'),
    ('docs/guides/administration.md', 'Administration'),
    ('docs/guides/applications.md', 'Applications'),
]
REFERENCE = [
    ('docs/reference/glossary.md', 'Glossary'),
    ('docs/reference/errors.md', 'Errors'),
    ('docs/reference/notes.md', 'DEP notes'),
    ('docs/reference/show.md', 'SHOW commands'),
    ('docs/reference/server-options.md', 'Server options'),
    ('docs/reference/http-api.md', 'HTTP API'),
    ('docs/reference/cli.md', 'DEP CLI'),
]
DESIGN = [
    ('docs/design/01-dep-concept.md', 'Design proposal'),
    ('docs/design/02-roadmap-and-release.md', 'Roadmap and release'),
]
PLAYGROUND_DOC = [('site/content/playground-guide.md', 'Using the playground')]
DOC_SECTIONS = [('Start here', START), ('Guides', GUIDES), ('Reference', REFERENCE), ('Design', DESIGN), ('Playground', PLAYGROUND_DOC)]

SPECIAL = {
    'docs/design/01-dep-concept.md': 'docs/design/proposal/',
    'docs/design/02-roadmap-and-release.md': 'docs/design/roadmap/',
    'site/content/playground-guide.md': 'playground/guide/',
    'playground/README.md': 'playground/guide/',
    'site/content/privacy-as-code.md': 'privacy-as-code/',
    'site/content/install.md': 'install/',
    'site/content/about.md': 'about/',
    'site/content/docs.md': 'docs/',
    'docs': 'docs/',
    'docs/playground': 'playground/',
    'playground': 'playground/',
    'site/content/legal/privacy.md': 'legal/privacy/',
    'site/content/legal/terms.md': 'legal/terms/',
    'LICENSING.md': 'legal/licensing/',
    'THIRD-PARTY.md': 'legal/licensing/#third-party-libraries',
    'BRAND.md': 'brand/',
    'SECURITY.md': 'security/',
    'CHANGELOG.md': 'changelog/',
    'CONTRIBUTING.md': 'contributing/',
}
# static files of the repository that are published with the site
STATIC = ['docs/design/diagrams/dep-architecture.html', 'docs/design/fig-architecture.png',
          'examples/clinic.sql']

NAV = [('Home', ''), ('Docs', 'docs/'), ('Playground', 'playground/'), ('Install', 'install/'),
       ('Privacy as Code', 'privacy-as-code/'), ('About', 'about/'), ('GitHub', GITHUB)]
GLOSSARY_SRC = 'docs/reference/glossary.md'
GLOSSARY_URL = 'docs/reference/glossary/'


def url_of(src):
    if src in SPECIAL:
        return SPECIAL[src]
    if src.startswith('docs/guides/') or src.startswith('docs/reference/'):
        return src[:-3] + '/'
    return None


def link(path):
    """A link to a page of the site, from its path under the site."""
    return path if path.startswith('http') else BASE + path


def read(rel):
    with open(os.path.join(ROOT, rel), encoding='utf-8') as f:
        return f.read()


def write(rel, text):
    p = os.path.join(OUT, rel)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    with open(p, 'w', encoding='utf-8') as f:
        f.write(text)


# ---------------------------------------------------------------- markdown

def resolve(href, src):
    """Rewrites a link found in the Markdown file `src`."""
    if re.match(r'^(https?:|mailto:|#|data:)', href) or href.startswith(BASE):
        return href
    path, _, frag = href.partition('#')
    if not path:
        return href
    target = os.path.normpath(os.path.join(os.path.dirname(src), path)).replace(os.sep, '/')
    u = url_of(target)
    if u:
        if '#' in u:
            return link(u)
        return link(u) + ('#' + frag if frag else '')
    if target in STATIC or target.startswith('docs/images/'):
        return link(target)
    if target.startswith('site/assets/'):
        return link(target[len('site/'):])
    full = os.path.join(ROOT, target)
    if os.path.isdir(full) or target.endswith('/'):
        return f'{GITHUB}/tree/main/{target.rstrip("/")}'
    if os.path.exists(full):
        return f'{GITHUB}/blob/main/{target}'
    return link(target)   # unknown: kept, so that --check reports it


def gh_slug(value, sep='-'):
    """The anchors GitHub gives to headings, so that links written for GitHub keep working."""
    value = re.sub(r'[^\w\s-]', '', re.sub(r'<[^>]+>', '', value).strip().lower())
    return value.replace(' ', '-')


_PICTURE = re.compile(r'<picture><source media="\(prefers-color-scheme: dark\)" srcset="([^"]+)"><img src="([^"]+)" alt="([^"]*)"(?: width="(\d+)")?></picture>')


def _narrow(path):
    """diagrams/NAME.png -> diagrams/NAME-narrow.png, and NAME-dark.png -> NAME-narrow-dark.png"""
    return re.sub(r'(-dark)?\.png$', lambda m: '-narrow' + (m.group(1) or '') + '.png', path)


def themed_images(body, docs):
    """An image with a dark version becomes one element per theme, which the stylesheet shows and hides with the theme
    (the site's theme button, not only the system's). The narrow version of a diagram is used in the docs column,
    and on phones elsewhere."""
    def sub(m):
        dark, light, alt, width = m.group(1), m.group(2), m.group(3), m.group(4)
        diagram = '/diagrams/' in light
        w = f' width="{width}"' if width else ''
        if diagram and docs:
            return (f'<img class="diagram for-light" src="{_narrow(light)}" alt="{alt}"{w}>'
                    f'<img class="diagram for-dark" src="{_narrow(dark)}" alt=""{w}>')
        if diagram:
            return (f'<picture class="for-light"><source media="(max-width: 640px)" srcset="{_narrow(light)}"><img class="diagram" src="{light}" alt="{alt}"{w}></picture>'
                    f'<picture class="for-dark"><source media="(max-width: 640px)" srcset="{_narrow(dark)}"><img class="diagram" src="{dark}" alt=""{w}></picture>')
        return f'<img class="for-light" src="{light}" alt="{alt}"{w}><img class="for-dark" src="{dark}" alt=""{w}>'
    return _PICTURE.sub(sub, body)


_SKIP = {'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'a', 'code', 'pre', 'script', 'style', 'button', 'summary', 'title', 'nav'}
_GLOSSARY = None


def glossary():
    """{heading: definition} and the compiled patterns, read once."""
    global _GLOSSARY
    if _GLOSSARY is None:
        defs = GT.definitions(read(GLOSSARY_SRC))
        pats = []
        for name, variants in GT.TERMS:
            parts = [('(?-i:%s)' % v) if (re.search(r'[A-Z]{3}', v) and v == v.upper()) else ('(?i:%s)' % v) for v in variants]
            pats.append((name, re.compile(r'\b(?:%s)\b' % '|'.join(parts))))
        _GLOSSARY = (defs, pats)
    return _GLOSSARY


def link_terms(body):
    """Links the first occurrence of each glossary term in the text of the page (not in headings, code or links)."""
    defs, pats = glossary()
    todo = list(pats)
    out, depth = [], 0
    for tok in re.split(r'(<[^>]+>)', body):
        if tok.startswith('<'):
            m = re.match(r'<(/?)([a-zA-Z][a-zA-Z0-9]*)', tok)
            if m and m.group(2).lower() in _SKIP and not tok.endswith('/>'):
                depth += -1 if m.group(1) else 1
                depth = max(depth, 0)
            out.append(tok)
            continue
        if depth or not todo or not tok.strip():
            out.append(tok)
            continue
        pos, buf = 0, []
        while todo:
            best = None
            for name, pat in todo:
                m = pat.search(tok, pos)
                if m and (best is None or m.start() < best[1].start() or (m.start() == best[1].start() and m.end() > best[1].end())):
                    best = (name, m)
            if not best:
                break
            name, m = best
            todo = [t for t in todo if t[0] != name]
            d = html.escape(defs.get(name, ''), quote=True)
            buf.append(tok[pos:m.start()])
            buf.append(f'<a class="term" href="{BASE}{GLOSSARY_URL}#{GT.slug(name)}" data-term="{html.escape(name)}" data-def="{d}">{m.group(0)}</a>')
            pos = m.end()
        buf.append(tok[pos:])
        out.append(''.join(buf))
    return ''.join(out)


def zoom_images(body):
    """A screenshot or a terminal session is wider than the text column: a click opens it at full size."""
    out, depth = [], 0
    for tok in re.split(r'(<[^>]+>)', body):
        m = re.match(r'<(/?)([a-zA-Z][a-zA-Z0-9]*)', tok) if tok.startswith('<') else None
        if m and m.group(2).lower() == 'a':
            depth += -1 if m.group(1) else 1
            depth = max(depth, 0)
        if m and m.group(2).lower() == 'img' and not depth and 'class="diagram' not in tok:
            src = re.search(r'\bsrc="([^"]+)"', tok)
            if src:
                tok = f'<a class="zoom" href="{src.group(1)}" title="Open at full size">{tok}</a>'
        out.append(tok)
    return ''.join(out)


def render_md(text, src, docs=False, terms=True):
    md = markdown.Markdown(extensions=['tables', 'fenced_code', 'toc', 'attr_list', 'sane_lists', 'md_in_html'],
                           extension_configs={'toc': {'toc_depth': '2-3', 'slugify': gh_slug}})
    body = themed_images(md.convert(text), docs)
    toc = md.toc_tokens

    def sub(m):
        return f'{m.group(1)}="{resolve(html.unescape(m.group(2)), src)}"'
    body = re.sub(r'\b(href|src|srcset)="([^"]*)"', sub, body)
    body = re.sub(r'<table>', '<div class="table-scroll"><table>', body)
    body = body.replace('</table>', '</table></div>')
    body = re.sub(r'<img (?!loading)', '<img loading="lazy" ', body)
    body = zoom_images(body)
    if terms and src != GLOSSARY_SRC:
        body = link_terms(body)
    return body, toc


def title_of(text, fallback):
    m = re.search(r'^# (.+)$', text, re.M)
    return re.sub(r'[`*_]', '', m.group(1)).strip() if m else fallback


def toc_html(tokens):
    def li(t):
        kids = ''.join(li(c) for c in t.get('children', []))
        return f'<li><a href="#{t["id"]}">{html.escape(re.sub(r"<[^>]+>", "", t["name"]))}</a>' + (f'<ul>{kids}</ul>' if kids else '') + '</li>'
    return '<ul>' + ''.join(li(t) for t in tokens[0]['children']) + '</ul>' if tokens and len(tokens) == 1 and tokens[0].get('children') else \
           '<ul>' + ''.join(li(t) for t in tokens) + '</ul>'


# ---------------------------------------------------------------- layout

def head_meta(title, desc, path, og):
    """The tags every page carries, besides charset, viewport, title and the stylesheets."""
    full_title = page_title(title)
    img = BASE_URL + f'assets/brand/{og}.png'
    return f'''<meta name="description" content="{html.escape(desc)}">
<meta name="theme-color" content="#011A3E">
<link rel="canonical" href="{BASE_URL}{path}">
<link rel="icon" href="{BASE}assets/brand/favicon.ico" sizes="48x48">
<link rel="icon" type="image/png" sizes="192x192" href="{BASE}assets/brand/icon-192.png">
<link rel="apple-touch-icon" href="{BASE}assets/brand/apple-touch-icon.png">
<link rel="manifest" href="{BASE}assets/brand/site.webmanifest">
<meta property="og:type" content="website">
<meta property="og:site_name" content="{SITE}">
<meta property="og:title" content="{html.escape(full_title)}">
<meta property="og:description" content="{html.escape(desc)}">
<meta property="og:url" content="{BASE_URL}{path}">
<meta property="og:image" content="{img}">
<meta name="twitter:card" content="summary_large_image">
<meta name="twitter:title" content="{html.escape(full_title)}">
<meta name="twitter:description" content="{html.escape(desc)}">
<meta name="twitter:image" content="{img}">'''


def page_title(title):
    return f'{title} | {SITE}' if title != SITE else f'{SITE}: Database-Enforced Privacy'


def head(title, desc, path, og='og-default'):
    return f'''<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{html.escape(page_title(title))}</title>
<meta name="color-scheme" content="light dark">
{head_meta(title, desc, path, og)}
<link rel="preload" href="{BASE}assets/fonts/ibm-plex-sans-latin-400-normal.woff2" as="font" type="font/woff2" crossorigin>
<link rel="preload" href="{BASE}assets/fonts/ibm-plex-sans-latin-600-normal.woff2" as="font" type="font/woff2" crossorigin>
<link rel="stylesheet" href="{BASE}assets/css/fonts.css">
<link rel="stylesheet" href="{BASE}assets/css/tokens.css">
<link rel="stylesheet" href="{BASE}assets/css/site.css">'''


def site_header(current):
    items = []
    for label, path in NAV:
        cur = ' aria-current="page"' if label == current else ''
        ext = ' rel="noopener"' if path.startswith('http') else ''
        items.append(f'<a href="{link(path)}"{cur}{ext}>{label}</a>')
    return f'''<a class="skip" href="#main">Skip to the content</a>
<header class="site-header"><div class="wrap">
  <a class="brand" href="{BASE}"><img src="{BASE}assets/brand/mascot-40.webp" srcset="{BASE}assets/brand/mascot-40.webp 1x, {BASE}assets/brand/mascot-80.webp 2x" width="34" height="41" alt=""><span>{SITE}</span></a>
  <nav class="nav" aria-label="Main">{''.join(items)}<button class="theme-btn" id="theme-btn" type="button" aria-pressed="false" aria-label="Switch to dark theme" title="Switch to dark theme"><svg class="i-moon" viewBox="0 0 24 24" width="18" height="18" aria-hidden="true"><path fill="currentColor" d="M20.5 14.6A8.5 8.5 0 0 1 9.4 3.5a.7.7 0 0 0-.9-.9A10 10 0 1 0 21.4 15.5a.7.7 0 0 0-.9-.9z"/></svg><svg class="i-sun" viewBox="0 0 24 24" width="18" height="18" aria-hidden="true"><circle cx="12" cy="12" r="4.2" fill="currentColor"/><g stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M12 2.5v2.4M12 19.1v2.4M2.5 12h2.4M19.1 12h2.4M5.3 5.3L7 7M17 17l1.7 1.7M18.7 5.3L17 7M7 17l-1.7 1.7"/></g></svg></button></nav>
</div></header>'''


def site_footer():
    return f'''<footer class="site-footer"><div class="wrap">
  <div class="footer-grid">
    <div><h2>{SITE}</h2><ul>
      <li><a href="{BASE}about/">About</a></li>
      <li><a href="{BASE}changelog/">Changelog</a></li>
      <li><a href="{BASE}security/">Security</a></li>
      <li><a href="{GITHUB}" rel="noopener">GitHub</a></li></ul></div>
    <div><h2>Use it</h2><ul>
      <li><a href="{BASE}playground/">Playground</a></li>
      <li><a href="{BASE}install/">Install</a></li>
      <li><a href="{BASE}docs/">Docs</a></li>
      <li><a href="{BASE}docs/reference/glossary/">Glossary</a></li>
      <li><a href="{BASE}privacy-as-code/">Privacy as Code</a></li></ul></div>
    <div><h2>Legal</h2><ul>
      <li><a href="{BASE}legal/privacy/">Privacy</a></li>
      <li><a href="{BASE}legal/terms/">Terms</a></li>
      <li><a href="{BASE}legal/licensing/">Licensing</a></li>
      <li><a href="{BASE}brand/">Names and logos</a></li></ul></div>
    <div><h2>Author</h2><ul>
      <li><a href="{AUTHOR_SITE}" rel="noopener">lp.reuven.click</a></li></ul></div>
  </div>
  <p class="footer-note"><span class="badge">Experimental</span> Project Hoopeo is an experimental, conceptual project. See the <a href="{BASE}legal/terms/">terms of use</a>. &copy; 2026 Reuven Mandelovich.</p>
</div></footer>
<script src="{BASE}assets/js/site.js" defer></script>'''


def site_footer_compact():
    return f'''<footer class="pg-foot"><span><strong>Experimental.</strong> An experimental, conceptual project: see the <a href="{BASE}legal/terms/">terms of use</a>.</span>
  <a href="{BASE}legal/privacy/">Privacy</a><a href="{BASE}legal/terms/">Terms</a><a href="{BASE}legal/licensing/">Licensing</a>
  <a href="{BASE}">Project Hoopeo</a><a href="{AUTHOR_SITE}" rel="noopener">lp.reuven.click</a></footer>
<script src="{BASE}assets/js/site.js" defer></script>'''


def page_html(title, desc, path, body, current='', og='og-default', cls=''):
    return f'''<!doctype html>
<html lang="en">
<head>
{head(title, desc, path, og)}
</head>
<body{(' class="' + cls + '"') if cls else ''}>
{site_header(current)}
<main id="main">
{body}
</main>
{site_footer()}
</body>
</html>
'''


EXPERIMENTAL = f'<p class="muted"><span class="badge">Experimental</span> Project Hoopeo is an experimental, conceptual project. See the <a href="{BASE}legal/terms/">terms of use</a>.</p>'


def doc_sidebar(current_src):
    out = []
    for name, items in DOC_SECTIONS:
        li = ''.join(f'<li><a href="{link(url_of(src))}"{" aria-current=\"page\"" if src == current_src else ""}>{label}</a></li>' for src, label in items)
        out.append(f'<h2>{name}</h2><ul>{li}</ul>')
    return ''.join(out)


def docs_page(src, label, text, order):
    body, toc = render_md(text, src, docs=True)
    u = url_of(src)
    title = title_of(text, label)
    desc = first_paragraph(text)
    idx = order.index(src)
    prev = order[idx - 1] if idx > 0 else None
    nxt = order[idx + 1] if idx + 1 < len(order) else None
    names = {s: l for _, items in DOC_SECTIONS for s, l in items}
    pn = ''
    if prev:
        pn += f'<a class="prev" href="{link(url_of(prev))}"><small>Previous</small>{html.escape(names[prev])}</a>'
    if nxt:
        pn += f'<a class="next" href="{link(url_of(nxt))}"><small>Next</small>{html.escape(names[nxt])}</a>'
    side = doc_sidebar(src)
    toc_block = ''
    if toc:
        flat = toc[0]['children'] if len(toc) == 1 and toc[0]['level'] == 1 else toc
        if flat:
            toc_block = f'<aside class="docs-toc" aria-label="On this page"><h2>On this page</h2>{toc_html(flat)}</aside>'
    article = f'''<div class="wrap docs">
  <nav class="docs-nav" aria-label="Docs">{side}</nav>
  <details class="docs-menu"><summary>Docs menu</summary><nav aria-label="Docs menu">{side}</nav></details>
  <article class="docs-article"><div class="prose">{body}</div><div class="prevnext">{pn}</div></article>
  {toc_block}
</div>'''
    og = 'og-docs'
    write(u + 'index.html', page_html(title, desc, u, article, 'Docs', og))
    return u, title


def first_paragraph(text):
    for para in re.split(r'\n\s*\n', text):
        p = para.strip()
        if p and not p.startswith(('#', '>', '|', '```', '-', '<', '!', '*')) and not re.match(r'^\w+:', p.split('\n')[0]):
            p = re.sub(r'\[([^\]]+)\]\([^)]*\)', r'\1', p)
            p = re.sub(r'[`*_]', '', p).replace('\n', ' ')
            return p if len(p) < 200 else p[:197].rsplit(' ', 1)[0] + '...'
    return SITE


def simple_page(src, current='', og='og-default', desc=None, lead=''):
    text = read(src)
    body, _ = render_md(text, src)
    u = url_of(src)
    title = title_of(text, SITE)
    d = desc or first_paragraph(text)
    inner = f'<div class="wrap section"><div class="prose">{lead}{body}</div></div>'
    write(u + 'index.html', page_html(title, d, u, inner, current, og))
    return u, title


# ---------------------------------------------------------------- build

def build_playground():
    pg = os.path.join(ROOT, 'playground')
    if not os.path.isdir(os.path.join(pg, 'node_modules', 'sql.js')):
        subprocess.check_call(['npm', 'ci'], cwd=pg)
    sys.path.insert(0, pg)
    import importlib
    pb = importlib.import_module('build')
    os.chdir(pg)
    pb.main()                                   # the standalone files, in playground/site and playground/
    page = pb.page(False, pb.mascot_uri())      # the page, with the placeholders of the site still in it
    desc = 'DEP SQL Playground: a browser simulator of Database-Enforced Privacy, running the DEP rules in your browser.'
    page = (page.replace('<!--__SITE_HEAD__-->', head_meta('DEP SQL Playground', desc, 'playground/', 'og-playground'))
            .replace('<!--__SITE_HEADER__-->', site_header('Playground'))
            .replace('<!--__SITE_FOOTER__-->', site_footer_compact())
            .replace('__BODY_CLASS__', 'in-site').replace('__GUIDE_URL__', link('playground/guide/'))
            .replace('__GLOSSARY_URL__', link(GLOSSARY_URL)))
    dest = os.path.join(OUT, 'playground')
    os.makedirs(dest, exist_ok=True)
    with open(os.path.join(dest, 'index.html'), 'w', encoding='utf-8') as f:
        f.write(page)
    for name in ('depsql.js', 'sql-wasm.js', 'sql-wasm.wasm'):
        shutil.copy(os.path.join(pg, 'site', name), os.path.join(dest, name))
    shutil.copytree(os.path.join(pg, 'site', 'fonts'), os.path.join(dest, 'fonts'), dirs_exist_ok=True)
    shutil.copy(os.path.join(pg, 'dep-sql-playground.html'), os.path.join(dest, 'dep-sql-playground.html'))
    os.chdir(ROOT)


def build():
    if os.path.isdir(OUT):
        shutil.rmtree(OUT)
    os.makedirs(OUT)
    shutil.copytree(os.path.join(HERE, 'assets'), os.path.join(OUT, 'assets'))
    urls = ['']

    # docs
    order = [s for _, items in DOC_SECTIONS for s, _ in items]
    for sec, items in DOC_SECTIONS:
        for src, label in items:
            if os.path.exists(os.path.join(ROOT, src)):
                urls.append(docs_page(src, label, read(src), order)[0])
    # the Docs index, and the pages
    pages = [('site/content/docs.md', 'Docs', 'og-docs'), ('site/content/privacy-as-code.md', 'Privacy as Code', 'og-privacy-as-code'),
             ('site/content/install.md', 'Install', 'og-install'), ('site/content/about.md', 'About', 'og-default'),
             ('site/content/legal/privacy.md', '', 'og-default'), ('site/content/legal/terms.md', '', 'og-default'),
             ('BRAND.md', '', 'og-default'), ('SECURITY.md', '', 'og-default'), ('CHANGELOG.md', '', 'og-default'),
             ('CONTRIBUTING.md', '', 'og-default')]
    for src, cur, og in pages:
        if os.path.exists(os.path.join(ROOT, src)):
            lead = EXPERIMENTAL if src.startswith('site/content/legal') else ''
            urls.append(simple_page(src, cur, og, lead=lead)[0])
    # licensing: LICENSING.md with the third-party libraries
    lic = read('LICENSING.md').rstrip() + '\n\n' + re.sub(r'^# Third-party software', '## Third-party libraries in detail {#third-party-libraries}', read('THIRD-PARTY.md'), count=1)
    body, _ = render_md(lic, 'LICENSING.md')
    write('legal/licensing/index.html', page_html('Licensing', first_paragraph(read('LICENSING.md')), 'legal/licensing/',
          f'<div class="wrap section"><div class="prose">{EXPERIMENTAL}{body}</div></div>', '', 'og-default'))
    urls.append('legal/licensing/')

    # home and 404
    home = link_terms(read('site/content/home.html').replace('{{BASE}}', BASE))
    write('index.html', page_html(SITE, 'Project Hoopeo implements Database-Enforced Privacy (DEP): privacy rules that the database engine enforces on every query, for every client.', '', home, 'Home'))
    nf = f'''<div class="wrap notfound"><img src="{BASE}assets/brand/mascot-240.webp" width="200" height="240" alt="The Project Hoopeo hoopoe">
<h1>This page was not found</h1><p class="lede">The page may have moved, or the address may be mistyped.</p>
<p><a class="btn" href="{BASE}">Home</a> <a class="btn secondary" href="{BASE}docs/">Docs</a> <a class="btn secondary" href="{BASE}playground/">Playground</a></p></div>'''
    write('404.html', page_html('Page not found', 'The page was not found.', '404.html', nf))

    build_playground()
    urls.append('playground/')

    # the documentation images, and the one-line installer
    shutil.copytree(os.path.join(ROOT, 'docs', 'images'), os.path.join(OUT, 'docs', 'images'))
    shutil.copy(os.path.join(HERE, 'static', 'install.sh'), os.path.join(OUT, 'install.sh'))

    # static files
    for rel in STATIC:
        if os.path.exists(os.path.join(ROOT, rel)):
            os.makedirs(os.path.join(OUT, os.path.dirname(rel)), exist_ok=True)
            shutil.copy(os.path.join(ROOT, rel), os.path.join(OUT, rel))

    # sitemap and robots
    sm = '<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n' + ''.join(
        f'  <url><loc>{BASE_URL}{u}</loc></url>\n' for u in sorted(set(urls)) if u != '404.html') + '</urlset>\n'
    write('sitemap.xml', sm)
    write('robots.txt', f'User-agent: *\nAllow: /\n\nSitemap: {BASE_URL}sitemap.xml\n')
    write('.nojekyll', '')
    print(f'built {len(set(urls)) + 1} pages into {os.path.relpath(OUT, ROOT)} (base {BASE_URL})')


# ---------------------------------------------------------------- check

def check():
    ids = {}
    files = []
    for d, _, fs in os.walk(OUT):
        for f in fs:
            if f.endswith('.html'):
                files.append(os.path.join(d, f))
    for f in files:
        ids[f] = set(re.findall(r'\bid="([^"]+)"', open(f, encoding='utf-8').read()))
    bad = 0
    for f in files:
        text = open(f, encoding='utf-8').read()
        refs = []
        for m in re.finditer(r'\b(href|src|srcset)="([^"]*)"', text):
            if m.group(1) == 'srcset':
                refs += [x.strip().split(' ')[0] for x in html.unescape(m.group(2)).split(',') if x.strip()]
            else:
                refs.append(m.group(2))
        for ref in refs:
            ref = html.unescape(ref)
            if re.match(r'^(https?:|mailto:|data:|javascript:)', ref) or ref == '':
                continue
            path, _, frag = ref.partition('#')
            if path == '':
                target = f
            elif path.startswith(BASE):
                target = os.path.join(OUT, path[len(BASE):])
            else:
                target = os.path.normpath(os.path.join(os.path.dirname(f), path))
            if os.path.isdir(target):
                target = os.path.join(target, 'index.html')
            if not os.path.exists(target):
                print(f'BROKEN  {os.path.relpath(f, OUT)}: {ref}')
                bad += 1
            elif frag and target.endswith('.html') and frag not in ids.get(target, set()):
                print(f'NOFRAG  {os.path.relpath(f, OUT)}: {ref}')
                bad += 1
    print(f'checked {len(files)} pages: {bad} problem(s)')
    return bad


if __name__ == '__main__':
    build()
    if '--check' in sys.argv:
        sys.exit(1 if check() else 0)
