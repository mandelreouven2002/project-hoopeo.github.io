#!/usr/bin/env python3
"""Renders a terminal transcript (text file) as a PNG of a terminal window, in the Project Hoopeo style.

  python3 render_term.py transcript.txt out.png "Window title"

Colors: shell and DEP prompts, commands, DEP notes, errors, hints. The fonts are the site's IBM Plex woff2 files,
read from $HOOPEO_PUBLIC/site/assets/fonts (the public repository).
"""
import html, re, sys, pathlib
from playwright.sync_api import sync_playwright

HERE = pathlib.Path(__file__).resolve().parent
PUBLIC = pathlib.Path(__import__("os").environ.get("HOOPEO_PUBLIC", "public"))
FONTDIR = PUBLIC / "site/assets/fonts"
src, out, title = sys.argv[1], sys.argv[2], sys.argv[3] if len(sys.argv) > 3 else "Terminal"
lines = pathlib.Path(src).read_text().rstrip("\n").split("\n")

PROMPT = re.compile(r"^([a-z_][a-z0-9_]*@[a-z_][a-z0-9_]*=[*!]?> )(.*)$")
def fmt(line):
    e = html.escape
    if line.startswith("$ ") or (line.startswith("    ") and prev_shell[0]):
        prev_shell[0] = line.rstrip().endswith("\\")
        if line.startswith("$ "):
            return f'<span class="sh">$</span> <span class="cmd">{e(line[2:])}</span>'
        return f'<span class="cmd">{e(line)}</span>'
    prev_shell[0] = False
    m = PROMPT.match(line)
    if m:
        p = m.group(1)
        cls = "pr-fail" if "=!>" in p else "pr-txn" if "=*>" in p else "pr"
        return f'<span class="{cls}">{e(p)}</span><span class="cmd">{e(m.group(2))}</span>'
    for key, cls in (("DEP:", "dep"), ("NOTICE:  DEP:", "dep"), ("ERROR:", "err"), ("HINT:", "hint"), ("DETAIL:", "hint"), ("SQLSTATE:", "hint")):
        if line.startswith(key):
            return f'<span class="{cls}">{e(line)}</span>'
    if line.startswith("--"):
        return f'<span class="com">{e(line)}</span>'
    if line.startswith("  dep-cli ") or line.startswith("DEP Engine ") or line.startswith("  data directory") or line.startswith("  TLS") or line.startswith("  query memory"):
        return f'<span class="log">{e(line)}</span>'
    if line.startswith("You are now connected"):
        return f'<span class="ok">{e(line)}</span>'
    return e(line)
prev_shell = [False]
body = "\n".join(fmt(l) for l in lines)

fonts = "".join(
    f"@font-face{{font-family:'{fam}';font-weight:{w};src:url('{(FONTDIR/f).as_uri()}') format('woff2')}}"
    for fam, w, f in [("IBM Plex Mono", 400, "ibm-plex-mono-latin-400-normal.woff2"), ("IBM Plex Mono", 600, "ibm-plex-mono-latin-600-normal.woff2"),
                      ("IBM Plex Sans", 500, "ibm-plex-sans-latin-500-normal.woff2")])
page = f"""<!doctype html><meta charset="utf-8"><style>{fonts}
body{{margin:0;padding:24px;background:transparent}}
.win{{display:inline-block;background:#07101D;border:1px solid #22344D;border-radius:10px;box-shadow:0 10px 30px rgba(1,26,62,.25);overflow:hidden;min-width:760px}}
.bar{{height:34px;background:#0D1A2D;border-bottom:1px solid #22344D;display:flex;align-items:center;padding:0 12px;gap:8px;font:500 13px 'IBM Plex Sans',sans-serif;color:#A3B3C9}}
.dot{{width:12px;height:12px;border-radius:50%;background:#22344D}} .bar span.t{{margin-left:10px}}
pre{{margin:0;padding:16px 20px 18px;font:400 13.5px/1.55 'IBM Plex Mono',monospace;color:#D7E2F0;white-space:pre}}
.sh{{color:#FC910A;font-weight:600}} .cmd{{color:#FFFFFF;font-weight:600}}
.pr{{color:#02D0D1;font-weight:600}} .pr-txn{{color:#6DB3FF;font-weight:600}} .pr-fail{{color:#FF8F85;font-weight:600}}
.dep{{color:#FC910A}} .err{{color:#FF8F85;font-weight:600}} .hint{{color:#A3B3C9}} .com{{color:#6F84A3;font-style:italic}}
.log{{color:#A3B3C9}} .ok{{color:#5FD39A}}
</style><div class="win"><div class="bar"><i class="dot"></i><i class="dot"></i><i class="dot"></i><span class="t">{html.escape(title)}</span></div><pre>{body}</pre></div>"""
with sync_playwright() as p:
    b = p.chromium.launch(**({"executable_path": __import__("os").environ["CHROMIUM"]} if "CHROMIUM" in __import__("os").environ else {}), args=["--no-sandbox"])
    pg = b.new_page(viewport={"width": 2000, "height": 800}, device_scale_factor=2)
    pg.set_content(page); pg.evaluate("document.fonts.ready")
    pg.wait_for_timeout(200)
    pg.locator(".win").screenshot(path=out, omit_background=True)
    b.close()
print(out)
