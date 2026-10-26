#!/usr/bin/env python3
"""The concept diagrams of Project Hoopeo, drawn in HTML and CSS with the site's tokens, and saved as PNG in a
light and a dark version.

  python3 diagrams.py OUT_DIR [name ...]

Each diagram is a function below. The colors are the site's tokens ($HOOPEO_PUBLIC/site/assets/css/tokens.css), so the
diagrams follow the brand in both themes: blue for protected values, teal for the operational path, orange for
the masked read path, violet for disclosed values, red for what is refused.
"""
import os, pathlib, sys
from playwright.sync_api import sync_playwright

HERE = pathlib.Path(__file__).resolve().parent
PUBLIC = pathlib.Path(__import__("os").environ.get("HOOPEO_PUBLIC", "public"))
FONTDIR = PUBLIC / "site/assets/fonts"
TOKENS = (PUBLIC / "site/assets/css/tokens.css").read_text()
FONTS = "".join(
    f"@font-face{{font-family:'{fam}';font-weight:{w};font-style:normal;src:url('{(FONTDIR / f).as_uri()}') format('woff2')}}"
    for fam, w, f in [
        ("IBM Plex Sans", 400, "ibm-plex-sans-latin-400-normal.woff2"), ("IBM Plex Sans", 500, "ibm-plex-sans-latin-500-normal.woff2"),
        ("IBM Plex Sans", 600, "ibm-plex-sans-latin-600-normal.woff2"), ("IBM Plex Sans Condensed", 500, "ibm-plex-sans-condensed-latin-500-normal.woff2"),
        ("IBM Plex Sans Condensed", 600, "ibm-plex-sans-condensed-latin-600-normal.woff2"),
        ("IBM Plex Mono", 400, "ibm-plex-mono-latin-400-normal.woff2"), ("IBM Plex Mono", 600, "ibm-plex-mono-latin-600-normal.woff2")])

CSS = """
*{box-sizing:border-box} body{margin:0;background:transparent;font-family:var(--f-body);color:var(--ink)}
.fig{width:1000px;padding:28px 30px 24px;background:var(--surface);border:1px solid var(--line);border-radius:14px}
.title{font:600 20px var(--f-body);margin:0 0 4px} .sub{color:var(--muted);font-size:14.5px;margin:0 0 20px;line-height:1.45}
.row{display:flex;gap:14px;align-items:stretch} .col{display:flex;flex-direction:column;gap:12px}
.box{border:1.5px solid var(--line);border-radius:10px;padding:12px 14px;background:var(--surface);font-size:14px;line-height:1.4}
.box h4{margin:0 0 4px;font:600 15px var(--f-body)} .box p{margin:0;color:var(--muted)}
.label{font:600 12px var(--f-label);letter-spacing:.06em;text-transform:uppercase;color:var(--muted);margin-bottom:6px}
code,.mono{font-family:var(--f-code);font-size:13px} .mono{white-space:pre}
.chip{display:inline-block;font:600 11.5px var(--f-label);letter-spacing:.04em;padding:2px 8px;border-radius:99px;vertical-align:1px}
.prot{border-color:var(--prot-line)} .prot-bg{background:var(--prot-soft)} .t-prot{color:var(--prot)}
.op{border-color:var(--op-line)} .op-bg{background:var(--op-soft)} .t-op{color:var(--op)}
.masked{border-color:var(--masked-line)} .masked-bg{background:var(--masked-soft)} .t-masked{color:var(--masked)}
.disc{border-color:var(--disc-line)} .disc-bg{background:var(--disc-soft)} .t-disc{color:var(--disc)}
.bad{border-color:var(--plain-line)} .bad-bg{background:var(--bad-soft)} .t-bad{color:var(--bad)}
.good{color:var(--good)} .plainbox{background:var(--surface-2)}
.chip.prot{background:var(--prot-soft);color:var(--prot)} .chip.op{background:var(--op-soft);color:var(--op)}
.chip.masked{background:var(--masked-soft);color:var(--masked)} .chip.disc{background:var(--disc-soft);color:var(--disc)}
.chip.bad{background:var(--bad-soft);color:var(--bad)} .chip.plain{background:var(--surface-2);color:var(--muted)}
.arrow{display:flex;align-items:center;justify-content:center;color:var(--muted);font-size:13px;min-width:44px;flex-direction:column;gap:2px}
.arrow svg{display:block} .down{min-height:34px}
.note{margin-top:16px;padding:10px 14px;border-left:3px solid var(--accent);background:var(--surface-2);border-radius:0 8px 8px 0;font-size:13.5px;color:var(--muted);line-height:1.45}
table.t{border-collapse:collapse;width:100%;font-size:13.5px} table.t th,table.t td{border-bottom:1px solid var(--line);padding:7px 10px;text-align:left;vertical-align:top}
table.t th{font:600 12px var(--f-label);letter-spacing:.05em;text-transform:uppercase;color:var(--muted)}
.yes{color:var(--good);font-weight:600} .no{color:var(--bad);font-weight:600}
.dash{border-style:dashed}
.fig.m{width:440px;padding:22px 18px 18px} .fig.m .row{flex-direction:column;align-items:stretch!important} .fig.m .arrows{display:none} .fig.m .title{font-size:18px} .fig.m table.t{font-size:12px} .fig.m table.t th,.fig.m table.t td{padding:6px 5px} .fig.m .box,.fig.m .col{width:auto!important;max-width:none!important} .fig.m .col{gap:10px!important} .fig.m .mono{white-space:pre-wrap;word-break:break-all}
"""

NARROW = os.environ.get("NARROW") == "1"   # stacked versions, 440 px wide, for the docs column and for phones

def arrow(label="", w=56, down=False):
    if NARROW and not down:
        down = True
    if down:
        return f'<div class="arrow down"><svg width="16" height="30" viewBox="0 0 16 30"><path d="M8 0v24" stroke="currentColor" stroke-width="2"/><path d="M2 20l6 8 6-8" fill="none" stroke="currentColor" stroke-width="2"/></svg>{label}</div>'
    return f'<div class="arrow" style="width:{w}px;flex:none"><span style="white-space:nowrap;text-align:center">{label}</span><svg width="{w}" height="14" viewBox="0 0 {w} 14"><path d="M0 7h{w-6}" stroke="currentColor" stroke-width="2"/><path d="M{w-12} 1l8 6-8 6" fill="none" stroke="currentColor" stroke-width="2"/></svg></div>'

# ---------------------------------------------------------------------------------------------------- diagrams

def products():
    return f"""
<p class="title">The products and how they connect</p>
<p class="sub">DEP Engine is the database server. Every client reaches it through an endpoint that fixes the access path. DEP CLI and Hoopeo Studio also include the engine, so they can open a database of their own with no server.</p>
<div class="row" style="align-items:center">
  <div class="col" style="width:250px">
    <div class="box"><h4>Hoopeo Studio</h4><p>The developer application</p></div>
    <div class="box"><h4>DEP CLI</h4><p><code>dep-cli -h host -p 5433</code></p></div>
    <div class="box"><h4>psql and PostgreSQL drivers</h4><p>JDBC, psycopg, node-postgres, ...</p></div>
    <div class="box"><h4>Applications over HTTP</h4><p><code>POST /v1/query</code></p></div>
  </div>
  <div class="col" style="gap:52px">{arrow("PostgreSQL protocol or HTTP API", 110) if NARROW else arrow("wire protocol", 110) + arrow("wire protocol", 110) + arrow("wire protocol", 110) + arrow("HTTP API", 110)}</div>
  <div class="box prot" style="width:290px;padding:16px">
    <div class="label">Server</div><h4 style="font-size:17px">DEP Engine</h4>
    <p style="margin-bottom:10px">Rust, Apache DataFusion, redb</p>
    <div class="box op op-bg" style="margin-bottom:8px;padding:8px 12px"><b class="t-op">operational</b> endpoint<br><span class="chip op">OPERATIONAL</span></div>
    <div class="box masked masked-bg" style="margin-bottom:8px;padding:8px 12px"><b class="t-masked">analytics</b> endpoint<br><span class="chip masked">MASKED_READ</span></div>
    <p><code>:5433</code> PostgreSQL protocol, TLS<br><code>:8433</code> HTTP API, <code>/metrics</code></p>
  </div>
  {arrow("", 44)}
  <div class="col" style="width:200px">
    <div class="box plainbox"><div class="label">Data directory</div><p><code>data.redb</code><br>rows, indexes, catalog</p><p style="margin-top:6px"><code>audit.redb</code><br>the audit log</p></div>
    <div class="box plainbox dash"><div class="label">Kept apart</div><p><code>master.key</code><br>with <code style="white-space:nowrap">--key-file</code></p></div>
  </div>
</div>
<div class="row" style="margin-top:20px">
  <div class="box" style="flex:1"><div class="label">Built in</div><h4>DEP CLI and Hoopeo Studio, with no server</h4><p><code>dep-cli --memory --init examples/clinic.sql</code>, or Session &gt; New Local Database in Studio: the same engine, in a folder or in memory, optionally served on a port.</p></div>
  <div class="box dash" style="flex:1"><div class="label">In the browser</div><h4>DEP.js and the playground</h4><p>A simulator of the same rules on SQLite (sql.js). Nothing to install and no server; it shows the rules and protects nothing.</p></div>
</div>"""

def access_paths():
    q = "SELECT full_name, national_id\nFROM patients\nWHERE national_id = '123456782';"
    return f"""
<p class="title">One statement, three outcomes</p>
<p class="sub">The endpoint a connection logs in through fixes its access path for the whole session. The same statement is refused on the masked read path, and answered on the operational path without returning the protected value.</p>
<div class="box plainbox mono" style="margin-bottom:6px">{q}</div>
<div class="row arrows" style="justify-content:space-around">{arrow("", 0, True)}{arrow("", 0, True)}{arrow("", 0, True)}</div>
<div class="row">
  <div class="box" style="flex:1"><div class="label">A regular database</div><h4>Any query, any value</h4>
    <div class="mono" style="margin:8px 0">full_name  | national_id\nDana Cohen | <span class="t-bad">123456782</span></div>
    <p>The ID number is stored, matched and returned as plain text.</p></div>
  <div class="box masked" style="flex:1"><div class="label t-masked">DEP, endpoint analytics</div><h4><span class="chip masked">MASKED_READ</span></h4>
    <div class="mono t-bad" style="margin:8px 0">ERROR: Matching PROTECTED values\nis unavailable on a MASKED_READ\nconnection.  ZD102</div>
    <p>Reads ordinary columns; protected columns return NULL; no matching, no writes.</p></div>
  <div class="box op" style="flex:1"><div class="label t-op">DEP, endpoint operational</div><h4><span class="chip op">OPERATIONAL</span></h4>
    <div class="mono" style="margin:8px 0">full_name  | national_id\nDana Cohen | <span class="t-prot">NULL</span></div>
    <p>Matches by full-value equality through the blind index; the value itself is still not returned.</p></div>
</div>
<table class="t" style="margin-top:18px">
<tr><th></th><th>Regular database</th><th class="t-masked">MASKED_READ</th><th class="t-op">OPERATIONAL</th></tr>
<tr><td>Read ordinary columns</td><td class="yes">yes</td><td class="yes">yes</td><td class="yes">yes</td></tr>
<tr><td>Return a protected value</td><td class="yes">yes</td><td>NULL</td><td>NULL, or disclosed by a <code>DISCLOSES</code> function</td></tr>
<tr><td>Match a protected value (<code>=</code>, <code>IN</code>)</td><td class="yes">yes</td><td class="no">no, ZD102</td><td class="yes">yes, through the blind index</td></tr>
<tr><td><code>LIKE</code>, ranges, sorting on a protected value</td><td class="yes">yes</td><td class="no">no</td><td class="no">no, ZD101</td></tr>
<tr><td>Join protected columns</td><td class="yes">yes</td><td>only in a <code>LINKS</code> function</td><td>anchored to one value, or in a <code>LINKS</code> function</td></tr>
<tr><td>Write</td><td class="yes">yes</td><td class="no">no</td><td class="yes">yes</td></tr>
</table>"""

def blind_index():
    return f"""
<p class="title">How a protected value is stored and matched</p>
<p class="sub">The engine never stores the plain value. It keeps an index entry, a keyed hash used for equality matching, and a ciphertext. Both keys belong to the column's match domain.</p>
<div class="label">Writing: <code style="text-transform:none">INSERT ... national_id = '123456782'</code></div>
<div class="row" style="align-items:center">
  <div class="box prot prot-bg" style="width:180px"><h4 class="mono t-prot">'123456782'</h4><p>MATCH DOMAIN<br><b>person_id</b></p></div>
  <div class="col" style="flex:1">
    <div class="row" style="align-items:center">{arrow("HMAC-SHA256, index key", 250)}<div class="box prot" style="flex:1"><div class="label">index entry, stored and indexed</div><span class="mono">ie1:fed1451ad4eb22a68f74e52c2945fbd5</span></div></div>
    <div class="row" style="align-items:center">{arrow("XChaCha20-Poly1305, encryption key", 250)}<div class="box" style="flex:1"><div class="label">ciphertext, stored</div><span class="mono">ct1:Bc+atne32t78R2KCFF7a0WMYtiQjC...</span></div></div>
  </div>
</div>
<div class="label" style="margin-top:24px">Matching: <code style="text-transform:none">WHERE national_id = '123456782'</code></div>
<div class="row" style="align-items:center">
  <div class="box prot prot-bg" style="width:180px"><h4 class="mono t-prot">'123456782'</h4><p>the value in the query</p></div>
  {arrow("same HMAC", 100)}
  <div class="box prot" style="flex:1"><span class="mono">ie1:fed1451ad4eb22a68f74e52c2945fbd5</span></div>
  {arrow("index lookup", 100)}
  <div class="box op op-bg" style="width:170px"><h4 class="t-op">patient 1</h4><p>Dana Cohen</p></div>
</div>
<div class="note">Equal values in one domain give equal entries, so equality, <code>IN</code>, unique keys and references work. Entries say nothing about order or prefixes, so <code>LIKE</code>, ranges and sorting are refused. The ciphertext is decrypted only by <code>REVEAL</code>, inside a function declared <code>DISCLOSES</code> for the domain.</div>"""

def keys():
    return f"""
<p class="title">Keys</p>
<p class="sub">One master key per database. Every other key is derived from it, two for each match domain, so a domain's entries cannot be compared with another domain's.</p>
<div class="row" style="align-items:center">
  <div class="box prot prot-bg" style="width:220px"><div class="label">master.key</div><h4>Master key</h4><p>256 bits. Kept in the data directory, or apart with <code style="white-space:nowrap">--key-file</code>.</p></div>
  {arrow("HMAC-SHA256<br>(master, purpose, domain)", 200)}
  <div class="col" style="flex:1">
    <div class="row"><div class="box prot" style="flex:1"><div class="label">person_id</div><b>index key</b><p>index entries</p></div><div class="box prot" style="flex:1"><div class="label">person_id</div><b>encryption key</b><p>ciphertexts</p></div></div>
    <div class="row"><div class="box prot" style="flex:1"><div class="label">phone_number</div><b>index key</b><p>index entries</p></div><div class="box prot" style="flex:1"><div class="label">phone_number</div><b>encryption key</b><p>ciphertexts</p></div></div>
    <div class="box plainbox"><b>fingerprint</b><p>Checked at start: the engine refuses a key that is not the one its data was written with.</p></div>
  </div>
</div>
<div class="row" style="margin-top:18px">
  <div class="box" style="flex:1"><div class="label">Rotation</div><h4><code>ALTER SYSTEM ROTATE MASTER KEY</code></h4><p>A new master key: every protected value is decrypted and encrypted again, and every index entry recomputed. Crash-safe, recorded in the audit log.</p></div>
  <div class="box" style="flex:1"><div class="label">Keep them apart</div><h4>Data alone is ciphertext</h4><p>With the key on another volume, a copy of the data directory holds only index entries and ciphertexts. Back up both, separately.</p></div>
</div>"""

def anchoring():
    def node(t, cls="prot"):
        return f'<div class="box {cls}" style="padding:7px 10px;text-align:center;max-width:100%"><span class="mono" style="font-size:11.5px">{t}</span></div>'
    return f"""
<p class="title">Anchored and unanchored matching</p>
<p class="sub">A join between protected columns is allowed when the query pins it to one value. Without an anchor it would link whole datasets, and that is allowed only inside a function declared <code>LINKS</code>.</p>
<div class="row">
  <div class="box op" style="flex:1 1 0;min-width:0;padding:16px"><div class="label t-op">Allowed: anchored</div>
    <div class="col" style="align-items:center;gap:0">
      <div class="box op op-bg" style="padding:6px 12px"><span class="mono">'123456782'</span> <span class="chip op">anchor</span></div>
      {arrow("=", 0, True)}{node("patients.national_id")}{arrow("JOIN ON =", 0, True)}{node("medical_records.patient_national_id")}
    </div>
    <div class="mono" style="margin-top:12px;font-size:11.5px">... JOIN medical_records m\n  ON m.patient_national_id\n   = p.national_id\nWHERE p.national_id = '123456782';</div>
    <p style="margin-top:8px;color:var(--muted)">One person's records: the match graph has an anchor.</p>
  </div>
  <div class="box bad" style="flex:1 1 0;min-width:0;padding:16px"><div class="label t-bad">Refused: unanchored</div>
    <div class="col" style="align-items:center;gap:0">
      <div class="box dash" style="padding:6px 12px;color:var(--muted)">no single value</div>
      {arrow("", 0, True)}{node("patients.national_id")}{arrow("JOIN ON =", 0, True)}{node("medical_records.patient_national_id")}
    </div>
    <div class="mono t-bad" style="margin-top:12px;font-size:11.5px">ERROR: Join on PROTECTED\ncolumns ... is not anchored\nto a single protected value.\nZD107</div>
    <p style="margin-top:8px;color:var(--muted)">Every patient linked to every record: refused before it runs.</p>
  </div>
  <div class="box disc" style="flex:1 1 0;min-width:0;padding:16px"><div class="label t-disc">Allowed: declared</div>
    <div class="box disc disc-bg" style="padding:10px 12px"><span class="mono">diagnoses_per_fund()</span><br><span class="chip disc">LINKS person_id</span></div>
    <p style="margin:10px 0;color:var(--muted)">The same join, inside a function its owner declared and was granted <code>LINK</code> for. It returns counts per fund and diagnosis, and no protected value.</p>
    <div class="mono" style="font-size:11.5px">fund   | diagnosis | patients\nClalit | Asthma    | 2</div>
  </div>
</div>"""

def declared():
    return f"""
<p class="title">Declared functions: disclosing and linking</p>
<p class="sub">Two things ordinary queries cannot do are allowed only inside functions that declare them. A role needs a grant on the match domain to write such a function, and other roles only <code>EXECUTE</code> it. Every call is in the audit log.</p>
<div class="row">
  <div class="box disc" style="flex:1;padding:16px"><div class="label t-disc">DISCLOSES person_id</div><h4><code>monthly_fund_report(DATE)</code></h4>
    <div class="mono" style="font-size:12px;margin:8px 0">SELECT <b class="t-disc">REVEAL(p.national_id)</b>, ...\nFROM appointments a JOIN patients p ...</div>
    <p style="color:var(--muted)"><code>REVEAL</code> decrypts a value, only here. The result column is <b class="t-disc">disclosed</b>.</p></div>
  <div class="box disc" style="flex:1;padding:16px"><div class="label t-disc">LINKS person_id</div><h4><code>diagnoses_per_fund()</code></h4>
    <div class="mono" style="font-size:12px;margin:8px 0">SELECT p.health_fund, m.diagnosis, count(*)\nFROM patients p JOIN medical_records m\n  ON m.patient_national_id = p.national_id</div>
    <p style="color:var(--muted)">Joins protected values without an anchor, only here.</p></div>
</div>
<div class="row" style="margin-top:14px;align-items:center">
  <div class="box disc disc-bg" style="width:240px"><h4 class="t-disc">A disclosed value</h4><p>is terminal: it can be returned to the caller, and nothing else.</p></div>
  {arrow("", 40)}
  <div class="col" style="flex:1;gap:8px">
    <div class="box" style="padding:8px 12px"><span class="yes">returned</span>: <code>SELECT * FROM monthly_fund_report(current_date)</code></div>
    <div class="box bad" style="padding:8px 12px"><span class="no">refused, ZD104</span>: filtered, joined, grouped or matched again</div>
  </div>
</div>
<table class="t" style="margin-top:16px">
<tr><th>To write the function</th><th>To call it</th><th>When it runs</th></tr>
<tr><td><code>GRANT DISCLOSE</code> or <code>GRANT LINK ON MATCH DOMAIN person_id</code> to its owner</td><td><code>GRANT EXECUTE</code> to a role, never to <code>PUBLIC</code></td><td>With its owner's privileges, as <code>LANGUAGE sql</code>; each call is audited</td></tr>
</table>"""

def statement():
    steps = [("Endpoint", "the role and the access path, fixed at login", "op"),
             ("Parser", "DEP SQL: PROTECTED, MATCH DOMAIN, DISCLOSES, LINKS, REVEAL", ""),
             ("DEP analyzer", "kinds of values, the match graph and its anchors, privileges, match limits", "prot"),
             ("Rewrite", "blind-index comparisons, NULL for protected outputs, REVEAL inside disclosing functions", "prot"),
             ("Apache DataFusion", "plans and executes; index lookups where it can", ""),
             ("Storage", "redb: rows with index entries and ciphertexts", "")]
    col = "".join(f'<div class="box {c}" style="padding:9px 14px"><b>{t}</b> <span style="color:var(--muted)">{d}</span></div>' + (arrow("", 0, True) if i < len(steps) - 1 else "") for i, (t, d, c) in enumerate(steps))
    col = col.replace('class="arrow down"', 'class="arrow down" style="min-height:22px"').replace('height="30" viewBox="0 0 16 30"', 'height="20" viewBox="0 0 16 30" preserveAspectRatio="none"')
    return f"""
<p class="title">The path of a statement</p>
<p class="sub">Every statement goes through the DEP analyzer before it is planned. A statement that breaks a rule is refused there, and never reaches the data.</p>
<div class="row">
  <div class="col" style="flex:1.45 1 0;gap:0">{col}</div>
  <div class="col" style="flex:1 1 0;justify-content:center">
    <div class="box bad"><div class="label t-bad">Refused at the analyzer</div><p>An error with a SQLSTATE, a DEP code (ZD101 to ZD109) and a hint. Messages never contain a protected value or the values of parameters.</p></div>
    <div class="box op"><div class="label t-op">Returned</div><p>Rows or a command tag, with DEP notes that say how each protected column was matched, masked or disclosed.</p></div>
    <div class="box"><div class="label">Audit log</div><p>Each protected match, disclosure, link, refusal, rate limit and key rotation, without values: <code>SHOW AUDIT</code>.</p></div>
  </div>
</div>"""


def dep_sql_adds():
    def col(n, title, code, meaning, cls):
        return f"""<div class="box {cls}" style="flex:1 1 0;min-width:0;padding:16px"><div class="label t-{cls}">{n}</div><h4>{title}</h4>
<div class="box plainbox mono" style="font-size:12px;margin:8px 0 10px;padding:10px 12px">{code}</div><p>{meaning}</p></div>"""
    a = col("1. In the schema", "Protected columns", "national_id TEXT\n  <b class='t-prot'>PROTECTED MATCH DOMAIN</b> person_id",
            "The database can use the value to find, check and connect records, by equality. No query gets the value back: it is returned as NULL.", "prot")
    b = col("2. At login", "Endpoints", "GRANT CONNECT ON <b class='t-op'>ENDPOINT</b>\n  analytics TO bi_dashboard;",
            "Each connection enters through an endpoint that fixes what it may do: <b class='t-op'>OPERATIONAL</b> may match one value at a time; <b class='t-masked'>MASKED_READ</b> may not match protected values.", "op")
    c = col("3. The exceptions", "Declared functions", "CREATE FUNCTION\n  monthly_fund_report(...)\n  <b class='t-disc'>DISCLOSES</b> person_id AS $$\n  SELECT <b class='t-disc'>REVEAL</b>(national_id), ...",
            "Returning a value (<b>DISCLOSES</b>) or linking whole tables (<b>LINKS</b>) happens only in named functions that a privacy owner wrote, with a grant, and every call is audited.", "disc")
    return f"""
<p class="title">What DEP SQL adds to SQL</p>
<p class="sub">Everything else is ordinary SQL, over the PostgreSQL protocol, so existing tools and drivers work. The privacy rules are written in the database, and the engine enforces them on every query, for every client.</p>
<div class="row">{a}{b}{c}</div>"""

def roles():
    def card(who, path, cls, can, cannot):
        return f"""<div class="box {cls}" style="flex:1 1 0;min-width:0;padding:14px"><h4>{who}</h4><span class="chip {cls}">{path}</span>
<p style="margin:10px 0 6px"><span class="yes">can</span> {can}</p><p><span class="no">cannot</span> {cannot}</p></div>"""
    return f"""
<p class="title">One clinic, four users, one rule set</p>
<p class="sub">The demo clinic protects ID numbers and phone numbers. Each application gets exactly what its job needs, and the database says no to the rest.</p>
<div class="row">
{card("Reception", "OPERATIONAL", "op", "find Dana by the ID number she presents, and update her record. The ID number comes back as NULL.", "read any ID number back, or search by part of one (<code>LIKE</code>).")}
{card("Doctors", "OPERATIONAL", "op", "read the records of the patient in front of them: a join anchored to one ID number.", "join every patient to every diagnosis (refused, ZD107).")}
{card("BI dashboard", "MASKED_READ", "masked", "count patients per fund, and call <code>diagnoses_per_fund()</code>, declared LINKS.", "look anyone up by ID number (refused, ZD102).")}
{card("Billing", "OPERATIONAL", "op", "send the funds their monthly report, with the ID numbers, through a DISCLOSES function.", "use the disclosed numbers for anything but returning them (ZD104).")}
</div>"""

DIAGRAMS = {"dep-sql-adds": dep_sql_adds, "roles": roles, "products": products, "access-paths": access_paths, "blind-index": blind_index, "keys": keys,
            "anchoring": anchoring, "declared-functions": declared, "statement-path": statement}

def main():
    out = pathlib.Path(sys.argv[1]); out.mkdir(parents=True, exist_ok=True)
    names = sys.argv[2:] or list(DIAGRAMS)
    mobile = NARROW
    with sync_playwright() as p:
        b = p.chromium.launch(**({"executable_path": __import__("os").environ["CHROMIUM"]} if "CHROMIUM" in __import__("os").environ else {}), args=["--no-sandbox"])
        for scheme in ("light", "dark"):
            pg = b.new_page(viewport={"width": 480 if mobile else 1100, "height": 900}, device_scale_factor=2, color_scheme=scheme)
            for n in names:
                pg.set_content(f"<!doctype html><meta charset=utf-8><style>{FONTS}{TOKENS}{CSS}</style><div class='fig{" m" if mobile else ""}'>{DIAGRAMS[n]()}</div>")
                pg.evaluate("document.fonts.ready"); pg.wait_for_timeout(150)
                sfx = ("-narrow" if mobile else "") + ("-dark" if scheme == "dark" else "")
                path = out / f"{n}{sfx}.png"
                pg.locator(".fig").screenshot(path=str(path), omit_background=True)
                print(path)
        b.close()

if __name__ == "__main__":
    main()
