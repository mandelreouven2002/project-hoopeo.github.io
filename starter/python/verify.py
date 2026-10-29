"""Integration checks against a running DEP Engine initialized with schema.sql.

Defaults are loopback-only tutorial identities. Override DEP_HOST and DEP_PORT as needed.
"""
import os
import psycopg

base = dict(host=os.getenv('DEP_HOST', '127.0.0.1'), port=os.getenv('DEP_PORT', '5543'), sslmode='disable', autocommit=True)

def refused(conn, query, expected):
    try:
        conn.execute(query)
    except psycopg.Error as error:
        assert error.sqlstate == expected, (error.sqlstate, expected)
    else:
        raise AssertionError('Query unexpectedly succeeded')

with psycopg.connect(**base, user='demo_app', password='demo-local', dbname='operational') as app:
    row = app.execute("SELECT full_name, national_id FROM patients WHERE national_id = '123456782'").fetchone()
    assert row == ('Dana Cohen', None), row
    refused(app, "SELECT full_name FROM patients WHERE national_id LIKE '123%'", 'ZD101')
    refused(app, 'SELECT p.full_name, r.diagnosis FROM patients p JOIN records r ON r.patient_national_id = p.national_id', 'ZD107')
    refused(app, 'SELECT * FROM patient_identifier(1)', '42501')

with psycopg.connect(**base, user='demo_analytics', password='demo-local', dbname='analytics') as analytics:
    refused(analytics, "SELECT full_name FROM patients WHERE national_id = '123456782'", 'ZD102')

with psycopg.connect(**base, user='export_job', password='export-local', dbname='operational') as export:
    assert export.execute('SELECT * FROM patient_identifier(1)').fetchone() == ('123456782',)
    refused(export, 'SELECT * FROM patient_identifier(1) ORDER BY national_id', 'ZD104')

print('PASS: masked output, partial-match refusal, anchored-join boundary, role grants, analytics boundary, approved disclosure and terminal rule.')
