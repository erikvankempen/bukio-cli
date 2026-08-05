-- 005_recurring_invoices.sql — recurring invoice templates (Phase 3, FR3A.6)
-- Extends recurring_templates so a template can generate DRAFT sales invoices
-- (abonnementen) instead of journal entries. Finalizing stays a separate
-- audited action — the engine never finalizes on its own.

ALTER TABLE recurring_templates ADD COLUMN kind TEXT NOT NULL DEFAULT 'entry'
  CHECK (kind IN ('entry', 'invoice'));
ALTER TABLE recurring_templates ADD COLUMN contact_id INTEGER REFERENCES contacts(id);
ALTER TABLE recurring_templates ADD COLUMN invoice_lines_json TEXT;
ALTER TABLE recurring_templates ADD COLUMN due_days INTEGER;
