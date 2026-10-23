\echo ===== 5. Privacy administrator: declaring a new disclosure =====
\c operational privacy_admin

-- REVEAL outside a disclosing function. Expected: DEP_REVEAL_OUTSIDE_DISCLOSURE.
SELECT full_name, REVEAL(national_id) FROM patients;

-- A function that discloses the phone of one patient, found by ID number.
CREATE OR REPLACE FUNCTION patient_contact(id TEXT)
RETURNS TABLE (full_name TEXT, phone TEXT)
LANGUAGE sql
DISCLOSES phone_number
AS $$
    SELECT p.full_name, REVEAL(p.phone)
    FROM patients AS p
    WHERE p.national_id = id
$$;

-- Disclosing the wrong match domain. Expected: DEP_REVEAL_OUTSIDE_DISCLOSURE.
CREATE OR REPLACE FUNCTION patient_id_card(id TEXT)
RETURNS TABLE (national_id TEXT)
LANGUAGE sql
DISCLOSES phone_number
AS $$
    SELECT REVEAL(p.national_id) FROM patients AS p WHERE p.national_id = id
$$;

-- A declared function is never executable by PUBLIC. Expected: DEP_PUBLIC_EXECUTE.
GRANT EXECUTE ON FUNCTION patient_contact(TEXT) TO PUBLIC;

-- Grant it to reception only. Expected: GRANT.
GRANT EXECUTE ON FUNCTION patient_contact(TEXT) TO reception_app;

-- Reception can now get a phone number, one patient at a time. Expected: Dana's phone.
\c operational reception_app
SELECT * FROM patient_contact('123456782');

-- A developer may write code, but not declare disclosures. Expected: DEP_DECLARATION_PRIVILEGE.
\c operational app_developer
CREATE FUNCTION all_ids()
RETURNS TABLE (national_id TEXT)
LANGUAGE sql
DISCLOSES person_id
AS $$
    SELECT REVEAL(national_id) FROM patients
$$;
