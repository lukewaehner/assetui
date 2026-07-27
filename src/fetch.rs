//! Yahoo Finance API integration.

use tokio::try_join;
use tracing::{debug, instrument};
use yfinance_rs::{AnalysisBuilder, Candle, HistoryBuilder, Interval, Range, Ticker, YfClient};

use chrono::{DateTime, Utc};
use yfinance_rs::{CacheMode, OptionContract as YfOptionContract, OptionSide};

use crate::black_scholes::OptionKind;
use crate::models::{
    NewsItem, OptionChain, OptionContract, QuoteRecordAnalysis, QuoteRecordStatistics, StockNews,
};
use crate::sort::{SortMode, SortOrder};
use crate::{AppError, db, models::QuoteRecord};

#[instrument(skip(ticker), fields(symbol = %symbol))]
pub async fn fetch_quote(symbol: &str, ticker: &Ticker) -> Result<Option<QuoteRecord>, AppError> {
    debug!("requesting quote from yfinance");
    let quote = ticker.quote().await?;
    debug!(
        ticker = ?quote.name,
        has_price = quote.price.is_some(),
        "received quote payload"
    );
    let quote_record = QuoteRecord {
        id: None,
        ticker: Some(symbol.to_string()),
        name: quote.name.clone(),
        price: quote.price.map(|p| p.into_inner().as_f64()),
        previous_close: quote.previous_close.map(|p| p.into_inner().as_f64()),
        day_volume: quote
            .day_volume
            .map(|p| p.into_inner().as_decimal().as_f64()),
        as_of: quote.as_of,
    };
    Ok(Some(quote_record))
}

/// Fetches a quote for `symbol` and immediately persists it to the database.
#[instrument(skip(pool, client), fields(symbol = %symbol))]
pub async fn fetch_quote_and_store(
    pool: &sqlx::PgPool,
    client: &YfClient,
    symbol: &str,
) -> Result<Option<QuoteRecord>, AppError> {
    let ticker = Ticker::new(client, symbol);

    let Some(mut quote_record) = fetch_quote(symbol, &ticker).await? else {
        return Ok(None);
    };

    quote_record.id = Some(db::quotes::store_quote_to_db(&quote_record, pool).await?);
    Ok(Some(quote_record))
}

/// Pairs each ticker symbol with an initialised [`Ticker`] client ready for async fetching.
pub fn prepare_tickers(s: &[String], c: &YfClient) -> Vec<(String, Ticker)> {
    s.iter().map(|t| (t.clone(), Ticker::new(c, t))).collect()
}

/// Returns up to `limit` quotes sorted by `mode` in the given `order`.
pub async fn fetch_sorted(
    pool: &sqlx::PgPool,
    mode: SortMode,
    order: SortOrder,
    limit: i64,
) -> sqlx::Result<Vec<QuoteRecord>> {
    debug!(?mode, ?order, "fetching sorted quotes");
    let column = match mode {
        SortMode::ById => "id",
        SortMode::ByTicker => "ticker",
        SortMode::ByPrice => "price",
        SortMode::ByPrevClose => "previous_close",
        SortMode::ByVolume => "day_volume",
        SortMode::ByAsOf => "as_of",
        SortMode::ByName => "name",
    };
    let direction = match order {
        SortOrder::Ascending => "ASC",
        SortOrder::Descending => "DESC",
    };

    let query = format!(
        "SELECT id, ticker, name, price, previous_close, day_volume, as_of
              FROM quotes
              ORDER BY {column} {direction}, id DESC
              LIMIT $1"
    );

    sqlx::query_as::<_, QuoteRecord>(sqlx::AssertSqlSafe(query))
        .bind(limit)
        .fetch_all(pool)
        .await
}

/// Fetches analyst consensus and price targets for `symbol` concurrently.
#[instrument(skip(client), fields(symbol = %symbol))]
pub async fn fetch_analysis(
    client: &YfClient,
    symbol: &str,
) -> Result<QuoteRecordAnalysis, AppError> {
    let analysis_builder = AnalysisBuilder::new(client, symbol);

    let (rec, pt) = try_join!(
        analysis_builder.recommendations_summary(),
        analysis_builder.analyst_price_target(None),
    )?;

    Ok(QuoteRecordAnalysis {
        ticker: Some(symbol.to_string()),
        recommendation_summary: Some(rec),
        price_target: Some(pt),
    })
}

#[instrument(skip(client), fields(symbol = %symbol))]
pub async fn fetch_key_statistics(
    client: &YfClient,
    symbol: &str,
) -> Result<QuoteRecordStatistics, AppError> {
    let stats = Ticker::new(client, symbol).key_statistics().await?;
    Ok(QuoteRecordStatistics {
        ticker: Some(symbol.to_string()),
        market_cap: stats.market_cap.map(|m| m.amount().as_f64()),
        pe_ratio: stats.pe_trailing_twelve_months.map(|d| d.as_f64()),
        eps: stats.eps_trailing_twelve_months.map(|p| p.amount().as_f64()),
        dividend_yield: stats.dividend_yield_trailing.map(|d| d.as_f64()),
        fifty_two_week_high: stats.fifty_two_week_high.map(|p| p.amount().as_f64()),
        fifty_two_week_low: stats.fifty_two_week_low.map(|p| p.amount().as_f64()),
        beta: stats.beta.map(|d| d.as_f64()),
    })
}

#[instrument(skip(client), fields(symbol = %symbol))]
pub async fn fetch_news(client: &YfClient, symbol: &str) -> Result<StockNews, AppError> {
    let articles = Ticker::new(client, symbol)
        .cache_mode(CacheMode::Use)
        .news()
        .await?;
    let items = articles
        .into_iter()
        .map(|a| NewsItem {
            title: a.title,
            publisher: a.publisher,
            published_at: Some(a.published_at),
            link: a.link,
        })
        .collect();
    Ok(StockNews {
        ticker: Some(symbol.to_string()),
        items,
    })
}

#[instrument(skip(client), fields(symbol = %symbol))]
pub async fn fetch_option_chain(client: &YfClient, symbol: &str) -> Result<OptionChain, AppError> {
    fetch_option_chain_inner(client, symbol, None).await
}

#[instrument(skip(client), fields(symbol = %symbol))]
pub async fn fetch_option_chain_for_expiry(
    client: &YfClient,
    symbol: &str,
    expiry: DateTime<Utc>,
) -> Result<OptionChain, AppError> {
    fetch_option_chain_inner(client, symbol, Some(expiry)).await
}

async fn fetch_option_chain_inner(
    client: &YfClient,
    symbol: &str,
    expiry: Option<DateTime<Utc>>,
) -> Result<OptionChain, AppError> {
    let ticker = Ticker::new(client, symbol).cache_mode(CacheMode::Use);
    let expirations = ticker
        .options()
        .await?
        .into_iter()
        .filter_map(|ts| DateTime::from_timestamp(ts, 0))
        .collect();
    let upstream = ticker.option_chain(expiry.map(|e| e.timestamp())).await?;
    let contracts = upstream.contracts.iter().map(map_option_contract).collect();
    Ok(OptionChain {
        underlying: symbol.to_string(),
        underlying_price: None,
        expirations,
        contracts,
    })
}

fn map_option_contract(c: &YfOptionContract) -> OptionContract {
    let kind = match c.key.side {
        OptionSide::Call => OptionKind::Call,
        OptionSide::Put => OptionKind::Put,
    };
    let expiry = c
        .expiration_at
        .unwrap_or_else(|| c.key.expiration_date.and_hms_opt(0, 0, 0).unwrap().and_utc());
    OptionContract {
        kind,
        expiry,
        strike: c.key.strike.amount().as_f64(),
        bid: c.bid.as_ref().map(|p| p.as_decimal().as_f64()),
        ask: c.ask.as_ref().map(|p| p.as_decimal().as_f64()),
        last: c.price.as_ref().map(|p| p.as_decimal().as_f64()),
        implied_volatility: c.implied_volatility.as_ref().map(|iv| iv.as_decimal().as_f64()),
        volume: c.volume,
        open_interest: c.open_interest,
    }
}

/// Fetches YTD ticker history at a one-day interval.
#[instrument(skip(client), fields(symbol = %symbol))]
pub async fn fetch_chart_data(client: &YfClient, symbol: &str) -> Result<Vec<Candle>, AppError> {
    let history = HistoryBuilder::new(client, symbol)
        .range(Range::Ytd)
        .interval(Interval::D1)
        .fetch()
        .await?;
    Ok(history)
}

#[cfg(test)]
mod tests {
    use super::{map_option_contract, prepare_tickers};
    use crate::black_scholes::OptionKind;
    use chrono::NaiveDate;
    use std::str::FromStr;
    use yfinance_rs::{
        AssetKind, Currency, Decimal, Instrument, NonNegativeDecimal, OptionContract,
        OptionContractKey, OptionSide, Price, PriceAmount, YfClient,
    };

    fn upstream_call_contract() -> OptionContract {
        let usd = Currency::try_from_str("USD").unwrap();
        let underlying = Instrument::from_symbol("AAPL", AssetKind::Equity).unwrap();
        let strike = Price::from_canonical_str("150.00", usd.clone()).unwrap();
        let expiry = NaiveDate::from_ymd_opt(2026, 1, 16).unwrap();
        let key = OptionContractKey::new(underlying, OptionSide::Call, strike, expiry);

        let mut c = OptionContract::new(key, usd.clone());
        c.price = Some(PriceAmount::new(Decimal::from_str("5.25").unwrap()));
        c.bid = Some(PriceAmount::new(Decimal::from_str("5.20").unwrap()));
        c.ask = Some(PriceAmount::new(Decimal::from_str("5.30").unwrap()));
        c.implied_volatility = Some(NonNegativeDecimal::new(Decimal::from_str("0.25").unwrap()).unwrap());
        c.volume = Some(1_000);
        c.open_interest = Some(500);
        c
    }

    #[test]
    fn map_option_contract_translates_all_fields() {
        // arrange
        let upstream = upstream_call_contract();

        // act
        let mapped = map_option_contract(&upstream);

        // assert
        assert_eq!(mapped.kind, OptionKind::Call);
        assert_eq!(mapped.strike, 150.00);
        assert_eq!(mapped.last, Some(5.25));
        assert_eq!(mapped.bid, Some(5.20));
        assert_eq!(mapped.ask, Some(5.30));
        assert_eq!(mapped.implied_volatility, Some(0.25));
        assert_eq!(mapped.volume, Some(1_000));
        assert_eq!(mapped.open_interest, Some(500));
        assert_eq!(mapped.expiry.date_naive(), NaiveDate::from_ymd_opt(2026, 1, 16).unwrap());
    }

    #[test]
    fn map_option_contract_maps_put_side_and_absent_quotes() {
        // arrange
        let usd = Currency::try_from_str("USD").unwrap();
        let underlying = Instrument::from_symbol("AAPL", AssetKind::Equity).unwrap();
        let strike = Price::from_canonical_str("140.00", usd.clone()).unwrap();
        let expiry = NaiveDate::from_ymd_opt(2026, 1, 16).unwrap();
        let key = OptionContractKey::new(underlying, OptionSide::Put, strike, expiry);
        let upstream = OptionContract::new(key, usd);

        // act
        let mapped = map_option_contract(&upstream);

        // assert
        assert_eq!(mapped.kind, OptionKind::Put);
        assert_eq!(mapped.strike, 140.00);
        assert_eq!(mapped.bid, None);
        assert_eq!(mapped.ask, None);
        assert_eq!(mapped.last, None);
        assert_eq!(mapped.implied_volatility, None);
        assert_eq!(mapped.volume, None);
        assert_eq!(mapped.open_interest, None);
    }

    #[test]
    fn test_prepare_tickers_empty() {
        let client = YfClient::default();
        let result = prepare_tickers(&[], &client);
        assert!(result.is_empty());
    }

    #[test]
    fn test_prepare_tickers_count() {
        let client = YfClient::default();
        let symbols = vec!["AAPL".to_string(), "MSFT".to_string()];
        let result = prepare_tickers(&symbols, &client);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_prepare_tickers_symbols_preserved() {
        let client = YfClient::default();
        let symbols = vec!["AAPL".to_string(), "MSFT".to_string()];
        let result = prepare_tickers(&symbols, &client);
        assert_eq!(result[0].0, "AAPL");
        assert_eq!(result[1].0, "MSFT");
    }
}
