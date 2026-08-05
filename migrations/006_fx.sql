-- 006_fx.sql — FX rates + auditable FX conversion (Phase 5, FR5.X)
-- rate_x10000: 1 EUR = rate units of foreign currency, scaled by 10000
-- (e.g. 1.0875 USD per EUR -> 10875). Conversion is integer math:
--   eur_cents = round(fx_cents * 10000 / rate_x10000)

CREATE TABLE IF NOT EXISTS fx_rates (
  currency     TEXT NOT NULL,          -- ISO 4217 (USD, GBP, ...)
  date         TEXT NOT NULL,          -- rate as of this date (ISO)
  rate_x10000  INTEGER NOT NULL CHECK (rate_x10000 > 0),
  source       TEXT NOT NULL DEFAULT 'manual',
  created_by   TEXT NOT NULL DEFAULT 'human',
  created_at   TEXT NOT NULL DEFAULT (datetime('now')),
  PRIMARY KEY (currency, date)
);

-- the conversion is recorded per posting so the ledger shows both the EUR
-- amount (amount_cents, the books) and the original foreign amount
ALTER TABLE postings ADD COLUMN fx_currency TEXT;
ALTER TABLE postings ADD COLUMN fx_amount_cents INTEGER;
