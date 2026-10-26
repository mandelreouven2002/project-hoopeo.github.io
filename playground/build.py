"""Builds the playground in two forms:
   site/                     index.html + sql-wasm.js + sql-wasm.wasm + depsql.js + fonts/ (the two compilers as separate files)
   dep-sql-playground.html   one self-contained file that also works offline (the fonts are inlined)

The colors and fonts are the ones of the website: ../site/assets/css/tokens.css and ../site/assets/fonts/.
site/build.py calls page() to put the playground inside the website, with the site header and footer."""
import base64, os, shutil
HERE = os.path.dirname(os.path.abspath(__file__))
ASSETS = os.path.join(HERE, '..', 'site', 'assets')
R = lambda p: open(os.path.join(HERE, p), encoding='utf-8').read()
FONTS = [('IBM Plex Sans', 'ibm-plex-sans-latin', (400, 500, 600)),
         ('IBM Plex Sans Condensed', 'ibm-plex-sans-condensed-latin', (400, 500, 600)),
         ('IBM Plex Mono', 'ibm-plex-mono-latin', (400, 500, 600))]


def font_files():
    return [f'{stem}-{w}-normal.woff2' for _, stem, ws in FONTS for w in ws]


def font_css(inline):
    css = ''
    for fam, stem, ws in FONTS:
        for w in ws:
            f = f'{stem}-{w}-normal.woff2'
            if inline:
                data = base64.b64encode(open(os.path.join(ASSETS, 'fonts', f), 'rb').read()).decode()
                src = f"url(data:font/woff2;base64,{data})"
            else:
                src = f"url(fonts/{f})"
            css += f"@font-face {{ font-family: '{fam}'; font-style: normal; font-weight: {w}; font-display: swap; src: {src} format('woff2'); }}\n"
    return css


def page(inline_fonts, mascot_src):
    """The page with every placeholder filled except the ones of the site (<!--__SITE_*__-->, __GUIDE_URL__, __BODY_CLASS__)."""
    page = R('src/page.html')
    lic = R('node_modules/sql.js/LICENSE').replace('--', '- -')
    tokens = open(os.path.join(ASSETS, 'css', 'tokens.css'), encoding='utf-8').read()
    page = page.replace('__SQLJS_LICENSE__', lic).replace('/*__CLINIC__*/', R('src/clinic.js')).replace('/*__UI__*/', R('src/ui.js'))
    page = page.replace('/*__FONTS__*/', font_css(inline_fonts)).replace('/*__TOKENS__*/', tokens)
    return page


def standalone(page_html, mascot_data_uri, guide_url, glossary_url):
    head = ('<header class="site-header"><div class="wrap"><span class="brand"><img src="%s" alt="">'
            '<span>Project Hoopeo</span></span></div></header>' % mascot_data_uri)
    return (page_html.replace('<!--__SITE_HEAD__-->', '').replace('<!--__SITE_HEADER__-->', head)
            .replace('<!--__SITE_FOOTER__-->', '').replace('__BODY_CLASS__', 'standalone').replace('__GUIDE_URL__', guide_url).replace('__GLOSSARY_URL__', glossary_url))


def mascot_uri():
    p = os.path.join(ASSETS, 'brand', 'mascot-80.webp')
    return 'data:image/webp;base64,' + base64.b64encode(open(p, 'rb').read()).decode()


def main():
    guide = 'https://github.com/mandelreouven2002/project-hoopeo.github.io/blob/main/playground/README.md'
    glossary = 'https://github.com/mandelreouven2002/project-hoopeo.github.io/blob/main/docs/reference/glossary.md'
    uri = mascot_uri()
    os.makedirs('site/fonts', exist_ok=True)
    for f in font_files():
        shutil.copy(os.path.join(ASSETS, 'fonts', f), 'site/fonts/' + f)
    open('site/index.html', 'w', encoding='utf-8').write(standalone(page(False, uri), uri, guide, glossary))
    shutil.copy('depsql.js', 'site/depsql.js')
    shutil.copy('node_modules/sql.js/dist/sql-wasm.js', 'site/sql-wasm.js')
    shutil.copy('node_modules/sql.js/dist/sql-wasm.wasm', 'site/sql-wasm.wasm')
    # single file
    wasm = base64.b64encode(open('node_modules/sql.js/dist/sql-wasm.wasm', 'rb').read()).decode()
    dep = R('depsql.js'); sqljs = R('node_modules/sql.js/dist/sql-wasm.js')
    for n, c in (('depsql', dep), ('sqljs', sqljs)): assert '</script' not in c.lower(), n
    one = standalone(page(True, uri), uri, guide, glossary)
    one = one.replace('<script src="sql-wasm.js"></script>', '<script>\n' + sqljs + '\n</script>\n<script>window.__SQLJS_WASM__ = "' + wasm + '";</script>')
    one = one.replace('<script src="depsql.js"></script>', '<script>\n' + dep + '\n</script>')
    open('dep-sql-playground.html', 'w', encoding='utf-8').write(one)
    for f in ('site/index.html', 'site/depsql.js', 'dep-sql-playground.html'):
        t = R(f).replace(wasm, ''); print(f, len(R(f)) // 1024, 'KB', 'emdash:', chr(0x2014) in t)


if __name__ == '__main__':
    main()
