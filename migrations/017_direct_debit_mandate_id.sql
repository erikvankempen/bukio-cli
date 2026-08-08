-- 017_direct_debit_mandate_id.sql — track WHICH mandate a direct-debit line
-- was collected under, not just a text snapshot of its ref.
-- SEPA FRST/RCUR is per-mandate. With only mandate_ref snapshotted, a
-- mandate that is removed and re-added with the SAME ref would count the old
-- lines and emit RCUR instead of FRST. mandate_id distinguishes the mandate
-- instances: a re-created mandate is a new row with a new id, so its first
-- batch is FRST. ON DELETE SET NULL keeps the historical snapshot (ref/date/
-- scheme) intact when a mandate is removed.

ALTER TABLE payment_batch_lines ADD COLUMN mandate_id INTEGER REFERENCES sepa_mandates(id) ON DELETE SET NULL;

-- Backfill: link existing direct-debit lines to the mandate that matches
-- their snapshot (contact + ref). Lines whose mandate was already removed
-- stay NULL — they simply don't count toward any mandate's sequence.
UPDATE payment_batch_lines
SET mandate_id = (
  SELECT m.id FROM sepa_mandates m
  WHERE m.contact_id = payment_batch_lines.contact_id
    AND m.mandate_ref = payment_batch_lines.mandate_ref
)
WHERE mandate_ref IS NOT NULL;
