#!/usr/bin/env python3
"""Self-test scenarios for Hoopeo Studio: prints the HOOPEO_STUDIO_SELFTEST script of one scenario.

  python3 studio/dev/selftest.py output|grid|matrix|explain|newlocal|connect|about
"""
import json, sys
which = sys.argv[1]
demo = {"connect": {"name": "Demo clinic (admin)", "kind": "demo", "user": "admin", "endpoint": "operational"}}
if which == "output":
    sql = """-- Plain SQL: no DEP notes
SELECT full_name, health_fund FROM patients ORDER BY full_name;

-- A lookup by a protected value
SELECT full_name, national_id, health_fund
FROM patients
WHERE national_id = '123456782';

-- An anchored join
SELECT p.full_name, m.diagnosis
FROM patients AS p JOIN medical_records AS m ON m.patient_national_id = p.national_id
WHERE p.national_id = '123456782';

SELECT full_name FROM patients WHERE national_id LIKE '1%';
"""
    steps = [demo, {"open": "t:patients"}, {"sql": sql}, {"menu": "query.execute"}, {"wait": 300}, {"select": "c:patients.national_id"}, {"pane": "output"}, {"log": 1}]
elif which == "grid":
    sql = "SELECT * FROM monthly_fund_report(DATE '2026-10-01');\n"
    steps = [demo, {"open": "functions"}, {"sql": sql}, {"menu": "query.execute"}, {"wait": 300}, {"select": "f:monthly_fund_report(DATE)"}, {"log": 1}]
elif which == "matrix":
    steps = [demo, {"menu": "tools.matrix"}, {"wait": 300}]
elif which == "explain":
    sql = "SELECT full_name, national_id\nFROM patients\nWHERE national_id = '123456782';\n"
    steps = [demo, {"sql": sql, "cursor": 10}, {"menu": "query.explain"}, {"wait": 300}]
elif which == "newlocal":
    steps = [{"menu": "session.newLocal"}, {"wait": 400}]
elif which == "connect":
    steps = [{"menu": "session.connect"}, {"wait": 400}]
elif which == "about":
    steps = [{"menu": "help.about"}, {"wait": 400}]
print(json.dumps({"steps": steps}))
