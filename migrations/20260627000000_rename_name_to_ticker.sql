ALTER TABLE quotes RENAME COLUMN name TO ticker;
ALTER INDEX quotes_name_as_of_idx RENAME TO quotes_ticker_as_of_idx;
ALTER TABLE quotes RENAME CONSTRAINT quotes_name_as_of_key TO quotes_ticker_as_of_key;
