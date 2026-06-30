CREATE INDEX quotes_name_idx ON quotes (name);
CREATE INDEX quotes_price_idx ON quotes (price);
CREATE INDEX quotes_previous_close_idx ON quotes (previous_close);
CREATE INDEX quotes_day_volume_idx ON quotes (day_volume);
CREATE INDEX quotes_as_of_idx ON quotes (as_of DESC);
