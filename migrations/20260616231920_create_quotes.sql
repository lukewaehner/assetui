-- Add migration script here
CREATE TABLE quotes (
    id SERIAL PRIMARY KEY,
    name TEXT,
    price DOUBLE PRECISION,
    previous_close DOUBLE PRECISION,
    day_volume DOUBLE PRECISION,
    as_of TIMESTAMPTZ
);
