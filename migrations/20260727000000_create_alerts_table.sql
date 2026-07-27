CREATE TABLE alerts (
    id SERIAL PRIMARY KEY,
    ticker TEXT NOT NULL,
    direction TEXT NOT NULL,
    threshold DOUBLE PRECISION NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    triggered_at TIMESTAMPTZ
);
