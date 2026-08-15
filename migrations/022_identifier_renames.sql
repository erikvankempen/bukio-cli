-- 022_identifier_renames.sql — Phase A: generic identifier columns (disruptive part).
--
-- Renames the NL-specific column names to generic ones (decisions §9.1.1/§9.1.4):
--   company.kvk            -> company.registration_id
--   company.btw_id         -> company.tax_id
--   accounts.rgs_code      -> accounts.taxonomy_code
--
-- Pure renames (SQLite >= 3.25 ALTER TABLE RENAME COLUMN — bundled 3.49.2) —
-- no rebuild needed, data preserved in place. This migration is DISRUPTIVE:
-- every code path reading the old column names breaks until the code churn
-- lands in the SAME commit (see the 022 code rename in src/ + test/). It is
-- deliberately separate from 021 (additive, gate stays green on its own) so
-- the rename + code churn is one atomic, reviewable change.
--
-- Note: contacts.kvk is NOT renamed — counterparty identifiers stay
-- per-country on the contacts table (Phase A keeps kvk there).

ALTER TABLE company RENAME COLUMN kvk TO registration_id;
ALTER TABLE company RENAME COLUMN btw_id TO tax_id;
ALTER TABLE accounts RENAME COLUMN rgs_code TO taxonomy_code;
