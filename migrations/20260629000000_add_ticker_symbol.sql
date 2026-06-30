-- Drop table and recreate with new schema since lazy
TRUNCATE TABLE quotes;

-- 'ticker' currently stores company name - rename it
ALTER TABLE quotes RENAME COLUMN ticker TO name;

-- Drop the old constraint and index (they were on the mis-named ticker/company-name column)
ALTER TABLE quotes DROP CONSTRAINT quotes_ticker_as_of_key;
DROP INDEX quotes_ticker_as_of_idx;

-- Add the real ticker symbol column
ALTER TABLE quotes ADD COLUMN ticker TEXT;

-- Rebuild constraint and index on the actual symbol
ALTER TABLE quotes ADD CONSTRAINT quotes_ticker_as_of_key UNIQUE (ticker, as_of);
CREATE INDEX quotes_ticker_as_of_idx ON quotes (ticker, as_of DESC);
