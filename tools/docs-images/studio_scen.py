#!/usr/bin/env python3
"""Prints the HOOPEO_STUDIO_SELFTEST script of one screenshot scenario of Hoopeo Studio (see make-images.sh)."""
import json, sys
import os
clinic = open(os.environ.get('CLINIC', 'examples/clinic.sql')).read()
mem = {"connect": {"name": "Clinic (in memory)", "kind": "memory", "user": "admin", "endpoint": "operational"}}
setup = [mem, {"sql": clinic}, {"menu": "query.execute"}, {"wait": 1500}, {"menu": "query.clearOutput"}, {"menu": "view.refresh"}, {"wait": 300}]
LOOKUP = """-- A lookup by a protected value: matched through the blind index
SELECT full_name, national_id, health_fund
FROM patients
WHERE national_id = '123456782';

-- An anchored join between two protected columns
SELECT p.full_name, m.diagnosis, m.recorded_at
FROM patients AS p
JOIN medical_records AS m ON m.patient_national_id = p.national_id
WHERE p.national_id = '123456782';
"""
ERR = """-- A join on protected columns with no anchor: refused before it runs
SELECT p.full_name, m.diagnosis
FROM patients AS p
JOIN medical_records AS m ON m.patient_national_id = p.national_id;

-- A partial match on a protected value: refused
SELECT full_name FROM patients WHERE national_id LIKE '1%';
"""
FN = "-- A declared function: aggregates by fund, with no protected value in the output\nSELECT * FROM monthly_fund_report(current_date);\n"
EXPL = "SELECT full_name, national_id\nFROM patients\nWHERE national_id = '123456782';\n"
s = sys.argv[1]
if s == "lookup":  steps = setup + [{"open": "t:patients"}, {"open": "t:medical_records"}, {"sql": LOOKUP}, {"menu": "query.execute"}, {"wait": 600}, {"select": "c:patients.national_id"}, {"pane": "output"}]
elif s == "refused": steps = setup + [{"open": "t:patients"}, {"sql": ERR}, {"menu": "query.execute"}, {"wait": 600}, {"pane": "output"}]
elif s == "function": steps = setup + [{"open": "functions"}, {"sql": FN}, {"menu": "query.execute"}, {"wait": 600}, {"select": "f:monthly_fund_report(DATE)"}]
elif s == "explain": steps = setup + [{"sql": EXPL, "cursor": 10}, {"menu": "query.explain"}, {"wait": 600}]
elif s == "matrix": steps = setup + [{"sql": "-- Tools > Privacy Matrix: who can match, and disclose, each protected column\nSHOW PRIVACY MATRIX;\n"}, {"menu": "tools.matrix"}, {"wait": 600}]
elif s == "audit": steps = setup + [{"sql": LOOKUP}, {"menu": "query.execute"}, {"wait": 500}, {"menu": "tools.audit"}, {"wait": 600}]
elif s == "newlocal": steps = [{"menu": "session.newLocal"}, {"wait": 500}]
elif s == "connect": steps = [{"menu": "session.connect"}, {"wait": 500}]
print(json.dumps({"steps": steps}))
