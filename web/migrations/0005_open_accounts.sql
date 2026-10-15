-- hushush web app: one kind of account for everyone.
--
-- Accounts no longer have a fixed type (employer | worker). Anyone can sign
-- up; the same account can pay people (by owning a company) and get paid (by
-- joining streams). What an account may do is decided by payroll itself
-- (company managers, stream recipients), never by a label.
--
-- `intent` is only the answer to the onboarding question "How will you use
-- hushush?" and picks the screen an account starts on. It can be changed at
-- any time. NULL = not answered yet.

ALTER TABLE accounts ADD COLUMN intent TEXT CHECK (intent IN ('pay', 'get_paid'));
UPDATE accounts SET intent = CASE kind WHEN 'employer' THEN 'pay' ELSE 'get_paid' END;
ALTER TABLE accounts DROP COLUMN kind;
