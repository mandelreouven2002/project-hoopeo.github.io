"""Run against the local tutorial server; connection settings come from DEP_DSN."""
import os
import psycopg

with psycopg.connect(os.environ['DEP_DSN'], autocommit=True) as conn:
    row = conn.execute(
        'SELECT full_name, national_id FROM patients WHERE national_id = %s',
        ('123456782',),
    ).fetchone()
    print(row)
    try:
        conn.execute('SELECT full_name FROM patients WHERE national_id LIKE %s', ('123%',))
    except psycopg.Error as error:
        print('Expected refusal:', error.sqlstate)
    else:
        raise AssertionError('Partial protected match should have been refused')
