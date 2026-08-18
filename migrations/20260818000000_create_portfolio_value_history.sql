CREATE TABLE portfolio_value_history (
    day          DATE PRIMARY KEY,
    market_value DOUBLE PRECISION NOT NULL,
    cost_basis   DOUBLE PRECISION NOT NULL,
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
