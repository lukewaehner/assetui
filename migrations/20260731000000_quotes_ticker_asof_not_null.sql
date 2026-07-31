-- Quotes are deduplicated via UNIQUE (ticker, as_of), but Postgres treats
-- NULLs as distinct, so rows with a null ticker or as_of bypass ON CONFLICT
-- and duplicate. Enforce NOT NULL so the dedup constraint is actually honoured.
DELETE FROM quotes WHERE ticker IS NULL OR as_of IS NULL;
ALTER TABLE quotes ALTER COLUMN ticker SET NOT NULL;
ALTER TABLE quotes ALTER COLUMN as_of SET NOT NULL;
