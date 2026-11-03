#!/usr/bin/env python3
"""privacy_check: keeps a DEP database equal to the privacy decisions that were approved for it.

The decisions live in a privacy matrix (matrix.yaml): one row per protected column, one column per workflow, and in
each cell MATCH, LINK, DISCLOSE or nothing (NONE), with a reason. This tool does the checks that chapter 4 of the DEP
guide describes as a CI step, against a running DEP Engine:

  lint    every decision other than NONE has a reason, and every DISCLOSE or LINK names a declared function;
  drift   what the database grants equals what the matrix decides, read from SHOW PRIVACY MATRIX, SHOW FUNCTIONS
          and SHOW GRANTS: who may match each protected column, directly or through an undeclared function, which declared functions each role may call to
          disclose or link each match domain, who may connect through which path, who may declare functions, and
          whether every declared function is in the matrix and records its purpose;
  tests   the matrix compiled into statements, each run as the workflow's role on its path, with the expected
          outcome: a lookup for every MATCH cell, a refused lookup for every NONE cell, an anchored join and a
          refused unanchored join for every reference between protected columns, a call for every DISCLOSE and
          LINK cell and a refused call for every other role, and a refused connection through the other path.

  python3 privacy_check.py matrix.yaml --url http://127.0.0.1:8433 [--admin admin:admin] [--password demo] [--json out.json]

Exit status: 0 when the database equals the matrix and every test passes, 1 otherwise.
"""
import argparse, base64, json, sys, urllib.request
import yaml

DECISIONS = {"MATCH", "LINK", "DISCLOSE"}


def decisions(cell):
    d = cell.get("decision", [])
    return set(d if isinstance(d, list) else [d])


class Db:
    def __init__(self, url, admin, password):
        self.url, self.admin, self.password = url.rstrip("/"), admin, password

    def q(self, sql, role=None, endpoint="operational"):
        user, pw = (role, self.password) if role else self.admin.split(":", 1)
        req = urllib.request.Request(f"{self.url}/v1/query?endpoint={endpoint}", data=json.dumps({"sql": sql}).encode(), method="POST",
                                     headers={"Content-Type": "application/json", "Authorization": "Basic " + base64.b64encode(f"{user}:{pw}".encode()).decode()})
        try:
            with urllib.request.urlopen(req, timeout=60) as r:
                res = json.load(r)
        except urllib.error.HTTPError as e:
            body = e.read().decode()
            try:
                res = json.loads(body)
            except ValueError:
                return {"ok": False, "code": str(e.code), "message": body[:200], "rows": []}
        if "results" not in res:      # refused before any statement ran, such as a login through a path not granted
            err = res.get("error") or {}
            return {"ok": False, "code": err.get("sqlstate") or "ERROR", "name": err.get("code"), "message": err.get("message"), "rows": []}
        last = res["results"][-1]
        err = last.get("error")
        if err:
            return {"ok": False, "code": err.get("sqlstate") or "ERROR", "name": err.get("code"), "message": err.get("message"), "rows": []}
        return {"ok": True, "rows": last.get("rows") or [], "columns": last.get("columns") or []}

    def table(self, sql):
        r = self.q(sql)
        if not r["ok"]:
            raise SystemExit(f"{sql}: {r['message']}")
        return [dict(zip(r["columns"], row)) for row in r["rows"]]


def expected(m):
    """What the matrix decides, per role: the protected columns it may match, and per match domain the declared
    functions through which it may disclose or link."""
    match, disclose, link = {}, {}, {}
    for col, f in m["fields"].items():
        for wf, cell in (f.get("cells") or {}).items():
            role = m["workflows"][wf]["role"]
            ds = decisions(cell)
            if "MATCH" in ds:
                match.setdefault(role, set()).add(col)
            if "DISCLOSE" in ds:
                disclose.setdefault((role, f["domain"]), set()).add(cell["via"])
            if "LINK" in ds:
                link.setdefault((role, f["domain"]), set()).add(cell["via"])
    return match, disclose, link


def lint(m):
    problems = []
    for col, f in m["fields"].items():
        for wf, cell in (f.get("cells") or {}).items():
            if wf not in m["workflows"]:
                problems.append(f"{col} x {wf}: no such workflow")
            ds = decisions(cell)
            if not ds <= DECISIONS:
                problems.append(f"{col} x {wf}: unknown decision {sorted(ds - DECISIONS)}")
            if ds and not (cell.get("reason") or "").strip():
                problems.append(f"{col} x {wf}: {'+'.join(sorted(ds))} without a reason")
            if ds & {"LINK", "DISCLOSE"} and not cell.get("via"):
                problems.append(f"{col} x {wf}: {'+'.join(sorted(ds))} names no declared function")
    return problems


def drift(m, db):
    admins = set(m.get("administrators", []))
    match, disclose, link = expected(m)
    roles_in_matrix = {w["role"] for w in m["workflows"].values()}
    problems = []
    fn = lambda s: {x.strip().split("(")[0] for x in (s or "").split(",") if x.strip()}
    seen = set()
    for r in db.table("SHOW PRIVACY MATRIX"):
        role, col, dom = r["role"], r["column"], r["match_domain"]
        if role in admins:
            continue
        if col not in m["fields"]:
            msg = f"{col} is PROTECTED in the database but has no row in the matrix"
            if msg not in problems:
                problems.append(msg)
            if (r["may_match"] == "yes" and "OPERATIONAL" in (r["paths"] or "")) or fn(r.get("matches_via")):
                problems.append(f"MATCH {col} for {role}: database grants it, the matrix has no row for {col}")
            continue
        # A privilege on a protected column lets a role match it only on the operational path. On the masked path it
        # only lets the column appear in the output, as NULL, so a masked-only role's privilege is not a MATCH.
        may = r["may_match"] == "yes" and "OPERATIONAL" in (r["paths"] or "")
        want = col in match.get(role, set())
        if may != want:
            problems.append(f"MATCH {col} for {role}: database {'grants' if may else 'does not grant'} it, matrix says {'MATCH' if want else 'NONE'}")
        # An undeclared function runs with its owner's privileges: a role that may execute one that reads a protected
        # column can match the column through it. That is a MATCH the matrix must contain.
        if not want:
            for f in sorted(fn(r.get("matches_via"))):
                problems.append(f"MATCH {col} for {role} through {f}(), an undeclared function: in the database, matrix says NONE")
        key = (role, dom, col)
        for kind, have, exp in [("DISCLOSE", fn(r["discloses_via"]), disclose.get((role, dom), set())),
                                ("LINK", fn(r["links_via"]), link.get((role, dom), set()))]:
            if (kind, role, dom) in seen:
                continue
            seen.add((kind, role, dom))
            for f in sorted(have - exp):
                problems.append(f"{kind} {dom} for {role} through {f}(): in the database, not in the matrix")
            for f in sorted(exp - have):
                problems.append(f"{kind} {dom} for {role} through {f}(): in the matrix, not in the database")
        if role not in roles_in_matrix and (may or fn(r["discloses_via"]) or fn(r["links_via"]) or fn(r.get("matches_via"))):
            problems.append(f"{role} has access to {col} but is no workflow's role in the matrix")
    vias = {c["via"] for f in m["fields"].values() for c in (f.get("cells") or {}).values() if c.get("via")}
    for f in db.table("SHOW FUNCTIONS"):
        name = f["function"].split("(")[0]
        if (f["discloses"] or f["links"]) and name not in vias:
            problems.append(f"{f['function']} is declared {'DISCLOSES ' + f['discloses'] if f['discloses'] else ''}{' ' if f['discloses'] and f['links'] else ''}{'LINKS ' + f['links'] if f['links'] else ''} but no matrix cell uses it")
        if (f["discloses"] or f["links"]) and not (f["comment"] or "").strip():
            problems.append(f"{f['function']} records no purpose (COMMENT ON FUNCTION)")
    paths = {}
    for w in m["workflows"].values():
        paths.setdefault(w["role"], set()).add(w["path"])
    allowed_declarers = m.get("declarers", {})
    for g in db.table("SHOW GRANTS"):
        who, priv = g["grantee"], g["privilege"]
        if priv.startswith("CONNECT ON ENDPOINT ") and who in paths:
            ep = priv.split()[-1]
            if ep not in paths[who]:
                problems.append(f"{who} may connect through {ep}, the matrix gives it {', '.join(sorted(paths[who]))}")
        if " ON MATCH DOMAIN " in priv:
            dom = priv.split()[-1]
            if who not in allowed_declarers.get(dom, []) and who not in admins:
                problems.append(f"{who} holds {priv}, the matrix allows it to {', '.join(allowed_declarers.get(dom, [])) or 'nobody'}")
    return problems


def compile_tests(m):
    """The matrix as statements with expected outcomes: (cell, role, endpoint, sql, expect) with expect 'OK' or
    'refused' (or a specific SQLSTATE)."""
    tests = []
    admins = set(m.get("administrators", []))
    wf = m["workflows"]
    for col, f in m["fields"].items():
        table, column = col.split(".")
        sample = f["sample"]
        cells = f.get("cells") or {}
        sql = f"SELECT 1 FROM {table} WHERE {column} = '{sample}'"
        by_role = {}
        for name, w in wf.items():
            by_role.setdefault((w["role"], w["path"]), set()).update(decisions(cells.get(name, {})))
        for (role, path), ds in sorted(by_role.items()):
            if role in admins:
                continue
            if "MATCH" in ds:
                tests.append((f"lookup on {col} by {role} (MATCH)", role, path, sql, "OK"))
            else:
                tests.append((f"lookup on {col} by {role} (no MATCH)", role, path, sql, "refused"))
        ref = f.get("references")
        if ref:
            rt, rc = ref.split(".")
            for name, w in wf.items():
                ds_here = decisions(cells.get(name, {}))
                ds_ref = decisions((m["fields"][ref].get("cells") or {}).get(name, {}))
                if "MATCH" in ds_here and "MATCH" in ds_ref:
                    join = f"SELECT 1 FROM {rt} AS a JOIN {table} AS b ON b.{column} = a.{rc}"
                    tests.append((f"{col} x {name}: anchored join", w["role"], w["path"], join + f" WHERE a.{rc} = '{sample}'", "OK"))
                    tests.append((f"{col} x {name}: unanchored join", w["role"], w["path"], join, "ZD107"))
    calls = {}
    for col, f in m["fields"].items():
        for name, cell in (f.get("cells") or {}).items():
            if cell.get("via"):
                calls.setdefault(cell["via"], set()).add(wf[name]["role"])
    roles = {}
    for name, w in wf.items():
        roles.setdefault(w["role"], w["path"])
    for fnname, who in sorted(calls.items()):
        call = f"SELECT * FROM {m['functions'][fnname]}"
        for role, path in sorted(roles.items()):
            if role in admins:
                continue
            tests.append((f"{fnname}() for {role}", role, path, call, "OK" if role in who else "refused"))
    for role, path in sorted(roles.items()):
        if role in admins:
            continue
        other = "analytics" if path == "operational" else "operational"
        if all(w["path"] != other for w in wf.values() if w["role"] == role):
            tests.append((f"{role} through {other}", role, other, "SELECT 1", "refused"))
    return tests


def run_tests(m, db):
    out = []
    for cell, role, ep, sql, exp in compile_tests(m):
        r = db.q(sql, role=role, endpoint=ep)
        got = "OK" if r["ok"] else r["code"]
        ok = (got == "OK") if exp == "OK" else (got != "OK" if exp == "refused" else got == exp)
        out.append({"cell": cell, "role": role, "endpoint": ep, "sql": sql, "expected": exp, "got": got, "pass": ok})
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("matrix"); ap.add_argument("--url", required=True)
    ap.add_argument("--admin", default="admin:admin"); ap.add_argument("--password", default="demo")
    ap.add_argument("--json"); ap.add_argument("--quiet", action="store_true")
    a = ap.parse_args()
    m = yaml.safe_load(open(a.matrix))
    db = Db(a.url, a.admin, a.password)
    report = {"lint": lint(m), "drift": drift(m, db), "tests": run_tests(m, db)}
    failed = [t for t in report["tests"] if not t["pass"]]
    report["summary"] = {"cells": sum(len(f.get("cells") or {}) for f in m["fields"].values()),
                         "decisions": sum(len(decisions(c)) for f in m["fields"].values() for c in (f.get("cells") or {}).values()),
                         "lint": len(report["lint"]), "drift": len(report["drift"]),
                         "tests": len(report["tests"]), "tests_failed": len(failed)}
    if a.json:
        json.dump(report, open(a.json, "w"), indent=1)
    if not a.quiet:
        for p in report["lint"]:
            print("lint:  ", p)
        for p in report["drift"]:
            print("drift: ", p)
        for t in failed:
            print(f"test:   {t['cell']}: {t['role']} on {t['endpoint']}: expected {t['expected']}, got {t['got']}")
        s = report["summary"]
        print(f"{s['decisions']} decisions in {s['cells']} cells; lint {s['lint']}, drift {s['drift']}, tests {s['tests'] - s['tests_failed']}/{s['tests']} passed")
    sys.exit(0 if not (report["lint"] or report["drift"] or failed) else 1)


if __name__ == "__main__":
    main()
