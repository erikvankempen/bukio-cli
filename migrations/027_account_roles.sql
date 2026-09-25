-- Special-meaning account flags: some chart accounts have a fixed role in the
-- books (accounts receivable, accounts payable, bank, revenue, VAT liability)
-- and the engine posts to them by default when no account is specified.
--
-- The flag lives on the account itself instead of in the jurisdiction profile:
-- a profile's account CODE assumes the profile's own default chart, and a book
-- whose chart came from an import numbers its accounts differently (the same
-- account under a different code, or the same code for a different account).
ALTER TABLE accounts ADD COLUMN role TEXT;

-- At most one account may carry each role.
CREATE UNIQUE INDEX accounts_role_unique ON accounts(role) WHERE role IS NOT NULL;
