-- Collapse exact-duplicate fetches: same ticker at the same market timestamp.
-- Keep the earliest-inserted row (lowest id) for each (name, as_of) pair.
DELETE FROM quotes a
USING quotes b
WHERE a.id > b.id
  AND a.name = b.name
  AND a.as_of = b.as_of;

-- Enforce uniqueness so re-fetching the same quote is a no-op (ON CONFLICT DO NOTHING).
-- Note: Postgres treats NULLs as distinct, so rows with a NULL name or NULL as_of
-- are not deduped by this constraint.
ALTER TABLE quotes ADD CONSTRAINT quotes_name_as_of_key UNIQUE (name, as_of);
