//! One SFTP read inside the worker's private Tokio runtime: negotiate the
//! session, open the file, prove the handle, transfer exactly the captured
//! length, validate UTF-8 and close — all under one total deadline, with
//! cancellation observed alongside. Nothing here outlives the call.

use super::error::{Fault, ReadFailureKind, ReadStage};
use super::wire::ReadOnlySftp;
use std::cell::Cell;
use std::future::{poll_fn, Future};
use std::path::Path;
use std::pin::pin;
use std::task::Poll;
use std::time::Duration;
use strop_core::worker::CancelToken;
use tokio::io::{AsyncRead, AsyncWrite};

/// Total budget for connection, transfer and close. Cleanup after failure
/// is separately bounded by the orchestrator and never waits on this.
pub(super) const DEADLINE: Duration = Duration::from_secs(60);

/// Cancellation is polled alongside the transfer. The cancellation hook
/// has already signalled the child by the time this notices, so its only
/// job is a prompt, clean stop instead of a broken-pipe diagnostic.
const CANCEL_POLL: Duration = Duration::from_millis(50);

/// Fetch one remote file as validated UTF-8 text.
///
/// The work future is raced against explicit cancellation observation and
/// the total deadline; whichever stops the read, the orchestrator owns
/// killing and reaping the child. Dropping the future mid-operation is the
/// intended failure exit — an unbounded graceful close is never attempted
/// after one.
pub(super) async fn run(
    stdin: impl AsyncWrite + Unpin,
    stdout: impl AsyncRead + Unpin,
    path: &Path,
    token: &CancelToken,
) -> Result<String, Fault> {
    let stage = Cell::new(ReadStage::Connect);
    let mut work = pin!(transfer(stdin, stdout, path, token, &stage));
    let mut cancel = pin!(cancellation(token));
    let raced = poll_fn(|cx| {
        if let Poll::Ready(result) = work.as_mut().poll(cx) {
            return Poll::Ready(result);
        }
        if cancel.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(Fault::cancelled(stage.get())));
        }
        Poll::Pending
    });
    match tokio::time::timeout(DEADLINE, raced).await {
        Ok(result) => result,
        Err(_elapsed) => Err(Fault::deadline(stage.get())),
    }
}

/// Never resolves while the token is live; resolves once cancelled.
async fn cancellation(token: &CancelToken) {
    loop {
        tokio::time::sleep(CANCEL_POLL).await;
        if token.is_cancelled() {
            return;
        }
    }
}

async fn transfer(
    stdin: impl AsyncWrite + Unpin,
    stdout: impl AsyncRead + Unpin,
    path: &Path,
    token: &CancelToken,
    stage: &Cell<ReadStage>,
) -> Result<String, Fault> {
    stage.set(ReadStage::Connect);
    let mut client = ReadOnlySftp::connect(stdin, stdout).await?;
    stage.set(ReadStage::Open);
    let handle = client.open(path).await?;
    stage.set(ReadStage::Inspect);
    let length = client.inspect(&handle).await?;
    stage.set(ReadStage::Transfer);
    let snapshot = client.read(&handle, length).await?;
    stage.set(ReadStage::Validate);
    let text = match String::from_utf8(snapshot) {
        Ok(text) => text,
        Err(error) => {
            let at = error.utf8_error().valid_up_to();
            return Err(Fault::new(
                ReadStage::Validate,
                ReadFailureKind::InvalidUtf8,
                format!("invalid UTF-8 at byte {at}"),
            ));
        }
    };
    stage.set(ReadStage::Teardown);
    client.close(handle).await?;
    drop(client); // EOF ends the SFTP subsystem; ssh must then exit cleanly.
                  // Cancellation is observed before success is published, so a stale
                  // result can never win against its owner's cancel.
    if token.is_cancelled() {
        return Err(Fault::cancelled(ReadStage::Teardown));
    }
    Ok(text)
}
