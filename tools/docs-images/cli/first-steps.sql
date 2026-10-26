\dt
-- A lookup by a protected value
SELECT full_name, national_id, health_fund FROM patients WHERE national_id = '123456782';
-- A partial match is refused
SELECT full_name FROM patients WHERE national_id LIKE '1%';
