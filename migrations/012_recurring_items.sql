-- v0.13.0: recurring invoice templates may reference catalog items.
-- Item specs ("1:2@140.00@21@-10%") are stored verbatim; prices are
-- snapshotted at each generation (recurring run).
ALTER TABLE recurring_templates ADD COLUMN invoice_items_json TEXT;
