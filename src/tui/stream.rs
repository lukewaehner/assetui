//! Live quote streaming.

use tokio::sync::mpsc::Receiver;
use tracing::{debug, instrument};
use yfinance_rs::{QuoteUpdate, StreamBuilder, StreamHandle, StreamMethod, YfClient};

use crate::AppError;

pub type QuoteStream = (StreamHandle, Receiver<QuoteUpdate>);

/// Starts a live quote stream for `symbols`, returning its handle and update channel.
#[instrument(skip_all, fields(symbols = symbols.len()))]
pub async fn start_quote_stream(
    client: &YfClient,
    symbols: Vec<String>,
) -> Result<Option<QuoteStream>, AppError> {
    if symbols.is_empty() {
        return Ok(None);
    }

    debug!("starting quote stream");
    let (handle, receiver) = StreamBuilder::new(client)
        .method(StreamMethod::WebsocketWithFallback)
        .diff_only(true)
        .symbols(symbols)
        .start()
        .await?;

    Ok(Some((handle, receiver)))
}
