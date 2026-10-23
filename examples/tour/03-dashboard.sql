\echo ===== 3. BI dashboard: statistics on the masked read path =====
\c analytics bi_dashboard

-- Ordinary columns are readable as usual. Expected: patients per health fund.
SELECT health_fund, count(*) AS patients
FROM patients
GROUP BY health_fund
ORDER BY health_fund;

-- A declared LINKS function: a linkage the clinic approved. Expected: a count per health fund and diagnosis.
SELECT * FROM diagnoses_per_fund();

-- No matching on the masked path, whatever the role holds. Expected: DEP_ENDPOINT_OPERATION.
SELECT health_fund
FROM patients
WHERE national_id = '123456782';

-- The dashboard was never granted the column. Expected: permission denied for column national_id.
SELECT national_id FROM patients;

-- Disclosing functions do not run on the masked path. Expected: DEP_ENDPOINT_OPERATION.
SELECT * FROM monthly_fund_report(DATE '2026-10-01');

-- Nothing is written on the masked path. Expected: DEP_ENDPOINT_OPERATION.
UPDATE patients SET health_fund = 'Maccabi' WHERE patient_id = 1;

-- The path is fixed when the connection opens. Expected: DEP_PATH_IMMUTABLE.
SET dep.path = 'OPERATIONAL';

-- The scheduling assistant holds no EXECUTE on the statistics. Expected: permission denied for function.
\c analytics scheduling_agent
SELECT * FROM diagnoses_per_fund();
