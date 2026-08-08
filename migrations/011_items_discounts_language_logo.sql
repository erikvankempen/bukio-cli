-- v0.13.0: items catalog, invoice discounts + language, company logo,
-- fractional quantities (quantity column migrates to milli-units, 1500 = 1.5)
CREATE TABLE items (
  id               INTEGER PRIMARY KEY AUTOINCREMENT,
  name             TEXT NOT NULL,
  description      TEXT,
  unit             TEXT NOT NULL DEFAULT 'unit',
  unit_price_cents INTEGER NOT NULL CHECK (unit_price_cents > 0),
  vat_code         TEXT,
  gl_account       TEXT,
  active           INTEGER NOT NULL DEFAULT 1,
  created_by       TEXT NOT NULL,
  created_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_by       TEXT,
  updated_at       TEXT
);

CREATE INDEX idx_items_active ON items(active);

ALTER TABLE company ADD COLUMN logo BLOB;
ALTER TABLE company ADD COLUMN logo_mime TEXT;

ALTER TABLE invoices ADD COLUMN discount_type TEXT CHECK (discount_type IN ('pct','amount') OR discount_type IS NULL);
ALTER TABLE invoices ADD COLUMN discount_value INTEGER;
ALTER TABLE invoices ADD COLUMN language TEXT NOT NULL DEFAULT 'nl' CHECK (language IN ('nl','en'));

ALTER TABLE invoice_lines ADD COLUMN item_id INTEGER REFERENCES items(id);
ALTER TABLE invoice_lines ADD COLUMN unit TEXT;
ALTER TABLE invoice_lines ADD COLUMN gl_account TEXT;
ALTER TABLE invoice_lines ADD COLUMN discount_type TEXT CHECK (discount_type IN ('pct','amount') OR discount_type IS NULL);
ALTER TABLE invoice_lines ADD COLUMN discount_value INTEGER;

-- quantity becomes milli-units (1.5 = 1500); backfill existing rows
UPDATE invoice_lines SET quantity = quantity * 1000;
