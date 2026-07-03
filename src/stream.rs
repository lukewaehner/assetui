//! Live quote streaming.
//!
//! Thin adapter over the `yfinance-rs` streaming API. Like [`crate::fetch`],
//! this is the only place that touches the upstream `StreamBuilder` /
//! `StreamMethod` types, so the rest of the app depends only on a
//! [`Receiver`] of [`QuoteUpdate`]s and the [`StreamHandle`] that owns the
//! stream's lifecycle.

use tokio::sync::mpsc::Receiver;
use tracing::{debug, instrument};
use yfinance_rs::{QuoteUpdate, StreamBuilder, StreamHandle, StreamMethod, YfClient};

use crate::AppError;

/// A started quote stream: the lifecycle [`StreamHandle`] paired with the
/// channel of incoming [`QuoteUpdate`]s. The caller owns the handle and is
/// responsible for stopping or aborting the stream (see STREAM-3).
pub type QuoteStream = (StreamHandle, Receiver<QuoteUpdate>);

/// Starts a live quote stream for `symbols`, returning its handle and update
/// channel.
///
/// Returns `Ok(None)` when `symbols` is empty: no stream is started and no
/// resources are allocated, so callers can treat "nothing to watch" as a plain
/// no-op rather than a special case.
///
/// Uses the `WebsocketWithFallback` transport with `diff_only` enabled, so the
/// channel only carries the fields that actually changed on each tick.
#[instrument(skip_all, fields(symbols = symbols.len()))]
pub async fn start_quote_stream(symbols: Vec<String>) -> Result<Option<QuoteStream>, AppError> {
    if symbols.is_empty() {
        return Ok(None);
    }

    debug!("starting quote stream");
    let client = YfClient::default();
    let (handle, receiver) = StreamBuilder::new(&client)
        .method(StreamMethod::WebsocketWithFallback)
        .diff_only(true)
        .symbols(symbols)
        .start()
        .await?;

    Ok(Some((handle, receiver)))
}
