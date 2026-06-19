//! The [`Provider`] trait: the single seam the runner calls to stream a turn.
//!
//! A provider takes an [`LlmRequest`] and returns an ordered [`LlmStream`] of normalized
//! [`LlmEvent`]s. Setup failures that occur before any event can flow (the network kill-switch,
//! an absent credential, a failed connect, or a non-success upstream status) are reported as an
//! `Err(LlmError)` from [`Provider::stream`]; failures that surface mid-turn are normalized into
//! an [`LlmEvent::Error`] within the stream, which still terminates with a single
//! [`LlmEvent::Done`].

use std::pin::Pin;

use futures_util::Stream;
use futures_util::future::BoxFuture;

use crate::error::LlmError;
use crate::event::LlmEvent;
use crate::request::LlmRequest;

/// An ordered, owned stream of normalized events for a single turn.
pub type LlmStream = Pin<Box<dyn Stream<Item = LlmEvent> + Send>>;

/// A model provider that streams a normalized turn.
///
/// The trait is object-safe (`dyn Provider`) so the runner can hold a boxed provider chosen at
/// runtime. [`Provider::stream`] is asynchronous because establishing the upstream connection may
/// await I/O before the first event is available.
pub trait Provider: Send + Sync {
    /// Stream one turn for `request`.
    ///
    /// Returns the ordered [`LlmStream`] on success, or a normalized [`LlmError`] if the turn could
    /// not even begin (kill-switch engaged, missing credential, transport failure, or an upstream
    /// error status).
    fn stream<'a>(&'a self, request: LlmRequest) -> BoxFuture<'a, Result<LlmStream, LlmError>>;
}
