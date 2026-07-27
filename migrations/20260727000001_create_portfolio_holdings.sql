CREATE TABLE portfolio_holdings (
    ticker     TEXT PRIMARY KEY,
    shares     DOUBLE PRECISION NOT NULL,
    avg_cost   DOUBLE PRECISION NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
