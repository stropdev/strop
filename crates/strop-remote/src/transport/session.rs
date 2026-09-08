//! Per-job SFTP scripts for the pooled actors: resolve a location, read a
//! selection with strict captured-length semantics, route a path by its
//! attributes, or enumerate a directory. Each script runs inside
//! [`guarded`], which races the work against the job's cancellation token,
//! the endpoint's stop signal and the total deadline — so the actor always
//! regains control promptly and never resynchronizes an interrupted stream.

use super::error::{Fault, ReadFailureKind, ReadStage};
use super::wire::{Advertised, Attrs, ReadOnlySftp};
use crate::address::{RemoteFile, RemoteLocation};
use crate::client::{RemoteEntry, RemoteEntryKind};
use crate::pool::StopSignal;
use crate::selection::{utf8_boundary_range, ReadLimit, ReadSelection, RemoteSize, RemoteWindow};
use std::cell::Cell;
use std::future::{poll_fn, Future};
use std::pin::pin;
use std::task::Poll;
use std::time::Duration;
use strop_core::worker::CancelToken;

/// Total budget for one job's connection, transfer and close. Cleanup after
/// failure is separately bounded by the actor and never waits on this.
pub(crate) const DEADLINE: Duration = Duration::from_secs(60);

/// Cancellation and stop are polled alongside the work. The hooks have
/// already signalled the child by the time this notices, so its only job is
/// a prompt, clean stop instead of a broken-pipe diagnostic.
const CANCEL_POLL: Duration = Duration::from_millis(50);

/// Race one script against the job's cancellation, the endpoint's stop
/// signal and the total deadline. Whichever stops the work, the actor owns
/// killing and reaping the child; dropping the script future mid-operation
/// is the intended interruption path.
pub(crate) async fn guarded<T, F>(
    work: F,
    token: &CancelToken,
    stop: &StopSignal,
    stage: &Cell<ReadStage>,
) -> Result<T, Fault>
where
    F: Future<Output = Result<T, Fault>>,
{
    let mut work = pin!(work);
    let mut interrupt = pin!(interrupted(token, stop));
    let raced = poll_fn(|cx| {
        if let Poll::Ready(result) = work.as_mut().poll(cx) {
            return Poll::Ready(result);
        }
        if interrupt.as_mut().poll(cx).is_ready() {
            let stopped = stop.signalled();
            return Poll::Ready(Err(if stopped && !token.is_cancelled() {
                Fault::stopped(stage.get())
            } else {
                Fault::cancelled(stage.get())
            }));
        }
        Poll::Pending
    });
    match tokio::time::timeout(DEADLINE, raced).await {
        Ok(result) => result,
        Err(_elapsed) => Err(Fault::deadline(stage.get())),
    }
}

/// Never resolves while the token is live and the endpoint is running;
/// resolves once either fires.
async fn interrupted(token: &CancelToken, stop: &StopSignal) {
    loop {
        tokio::time::sleep(CANCEL_POLL).await;
        if token.is_cancelled() || stop.signalled() {
            return;
        }
    }
}

/// Resolve a location to its canonical file on an already connected
/// session. A home query is expanded only through a negotiated
/// `expand-path@openssh.com`; REALPATH alone is never guessed as home.
pub(crate) async fn resolve_location<W, R>(
    client: &mut ReadOnlySftp<W, R>,
    location: &RemoteLocation,
    advertised: &Advertised,
) -> Result<RemoteFile, Fault>
where
    W: tokio::io::AsyncWrite + Unpin,
    R: tokio::io::AsyncRead + Unpin,
{
    if let Some(file) = location.absolute_file() {
        return Ok(file.clone());
    }
    let endpoint = location.endpoint().clone();
    let relative = location.home_relative().ok_or_else(|| {
        Fault::new(
            ReadStage::Open,
            ReadFailureKind::Protocol,
            "remote location has neither an absolute path nor a home query",
        )
    })?;
    if !advertised.offers(b"expand-path@openssh.com") {
        return Err(Fault::new(
            ReadStage::Open,
            ReadFailureKind::HomeUnsupported,
            "the server does not advertise expand-path@openssh.com, and \
             REALPATH is not a portable substitute for home expansion",
        ));
    }
    let expanded = client.expand_path(relative).await?;
    RemoteFile::from_path(endpoint, expanded).map_err(|error| {
        Fault::new(
            ReadStage::Open,
            ReadFailureKind::Protocol,
            format!("expanded home path is not admissible: {error}"),
        )
    })
}

/// Read one selection of a canonical file. The length is captured at
/// inspection; fewer bytes than captured is a short read, never a partial
/// success. Partial UTF-8 sequences at the window edges are trimmed and the
/// window metadata reports exactly the bytes held.
pub(crate) async fn read_selection<W, R>(
    client: &mut ReadOnlySftp<W, R>,
    file: &RemoteFile,
    selection: &ReadSelection,
    stage: &Cell<ReadStage>,
) -> Result<(String, RemoteWindow), Fault>
where
    W: tokio::io::AsyncWrite + Unpin,
    R: tokio::io::AsyncRead + Unpin,
{
    stage.set(ReadStage::Open);
    let handle = client.open(file.path()).await?;
    let outcome = transfer(client, selection, stage, &handle).await;
    finish_handle(client, handle, outcome).await
}

async fn transfer<W, R>(
    client: &mut ReadOnlySftp<W, R>,
    selection: &ReadSelection,
    stage: &Cell<ReadStage>,
    handle: &super::wire::FileHandle,
) -> Result<(String, RemoteWindow), Fault>
where
    W: tokio::io::AsyncWrite + Unpin,
    R: tokio::io::AsyncRead + Unpin,
{
    stage.set(ReadStage::Inspect);
    let attrs = client.fstat(handle).await?;
    let size = prove_regular_sized(&attrs)?;
    let window = RemoteWindow::resolve(selection, size);
    if window.length().get() > ReadLimit::MAX {
        return Err(Fault::new(
            ReadStage::Inspect,
            ReadFailureKind::TooLarge,
            format!(
                "{} bytes exceeds the {} byte snapshot cap; use a bounded tail or range",
                window.length().get(),
                ReadLimit::MAX
            ),
        ));
    }
    stage.set(ReadStage::Transfer);
    let bytes = client
        .read_at(handle, window.start().get(), window.length().get())
        .await?;
    if bytes.len() as u64 != window.length().get() {
        return Err(Fault::new(
            ReadStage::Transfer,
            ReadFailureKind::ShortRead,
            format!(
                "captured {} bytes but {} arrived before the captured end",
                window.length().get(),
                bytes.len()
            ),
        ));
    }
    stage.set(ReadStage::Validate);
    let (front, back) = utf8_boundary_range(&bytes);
    let front = if window.start().get() == 0 { 0 } else { front };
    let back = if window.reaches_eof() {
        bytes.len()
    } else {
        back
    };
    let window = window.narrowed(front as u64, (back - front) as u64);
    let mut bytes = bytes;
    bytes.truncate(back);
    if front != 0 {
        bytes.drain(..front);
    }
    let text = match String::from_utf8(bytes) {
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
    // A remote snapshot is text: the buffer holds exactly the window.
    Ok((text, window))
}

/// The opened handle must be proven a regular file of known, bounded size —
/// the same proof the single-use read path enforced.
fn prove_regular_sized(attrs: &Attrs) -> Result<RemoteSize, Fault> {
    if !attrs
        .permissions
        .is_some_and(|mode| mode & 0xf000 == 0x8000)
    {
        return Err(Fault::new(
            ReadStage::Inspect,
            ReadFailureKind::NotRegularFile,
            "opened handle is not a proven regular file",
        ));
    }
    let size = attrs.size.ok_or_else(|| {
        Fault::new(
            ReadStage::Inspect,
            ReadFailureKind::UnknownLength,
            "server reported no file length",
        )
    })?;
    Ok(RemoteSize::new(size))
}

/// Route a path by its attributes without opening anything: how an explicit
/// open decides between the file and directory resources.
pub(crate) async fn path_kind<W, R>(
    client: &mut ReadOnlySftp<W, R>,
    file: &RemoteFile,
    stage: &Cell<ReadStage>,
) -> Result<RemoteEntryKind, Fault>
where
    W: tokio::io::AsyncWrite + Unpin,
    R: tokio::io::AsyncRead + Unpin,
{
    stage.set(ReadStage::Inspect);
    let attrs = client.stat(file.path()).await?;
    Ok(kind_of(attrs.permissions))
}

/// Enumerate one directory: pages are packet-bounded, the accumulated
/// listing is capped, `.` and `..` are not entries the user can open, and
/// the handle is closed before the result is returned.
pub(crate) async fn list_entries<W, R>(
    client: &mut ReadOnlySftp<W, R>,
    directory: &RemoteFile,
    stage: &Cell<ReadStage>,
) -> Result<Vec<RemoteEntry>, Fault>
where
    W: tokio::io::AsyncWrite + Unpin,
    R: tokio::io::AsyncRead + Unpin,
{
    stage.set(ReadStage::Open);
    let handle = client.opendir(directory.path()).await?;
    let listing = enumerate(client, directory, stage, &handle).await;
    finish_handle(client, handle, listing).await
}

async fn enumerate<W, R>(
    client: &mut ReadOnlySftp<W, R>,
    directory: &RemoteFile,
    stage: &Cell<ReadStage>,
    handle: &super::wire::FileHandle,
) -> Result<Vec<RemoteEntry>, Fault>
where
    W: tokio::io::AsyncWrite + Unpin,
    R: tokio::io::AsyncRead + Unpin,
{
    stage.set(ReadStage::Transfer);
    let mut entries = Vec::new();
    loop {
        match client.readdir(handle).await? {
            super::wire::Page::End => break,
            super::wire::Page::Entries(page) => {
                for raw in page {
                    if raw.name.as_os_str() == "." || raw.name.as_os_str() == ".." {
                        continue;
                    }
                    let file = directory
                        .with_path(directory.path().join(&raw.name))
                        .map_err(|error| {
                            Fault::new(
                                ReadStage::Transfer,
                                ReadFailureKind::Protocol,
                                format!("entry name is not admissible: {error}"),
                            )
                        })?;
                    entries.push(RemoteEntry {
                        file,
                        kind: kind_of(raw.permissions),
                    });
                    if entries.len() > super::wire::MAX_ENTRIES {
                        return Err(Fault::new(
                            ReadStage::Transfer,
                            ReadFailureKind::TooManyEntries,
                            format!(
                                "directory listing exceeds the {} entry cap",
                                super::wire::MAX_ENTRIES
                            ),
                        ));
                    }
                }
            }
        }
    }
    Ok(entries)
}

/// Only a sound stream can close a handle; never try to resynchronize broken
/// framing with a second request. Every handle is closed at most once.
async fn finish_handle<W, R, T>(
    client: &mut ReadOnlySftp<W, R>,
    handle: super::wire::FileHandle,
    outcome: Result<T, Fault>,
) -> Result<T, Fault>
where
    W: tokio::io::AsyncWrite + Unpin,
    R: tokio::io::AsyncRead + Unpin,
{
    if let Err(fault) = &outcome {
        let (kind, poison) = fault.disposition();
        if poison
            || matches!(
                kind,
                ReadFailureKind::Protocol
                    | ReadFailureKind::Io
                    | ReadFailureKind::Cancelled
                    | ReadFailureKind::Deadline
                    | ReadFailureKind::Stopped
            )
        {
            return outcome;
        }
    }
    match (outcome, client.close(handle).await) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(fault), Ok(())) => Err(fault),
        (Ok(_), Err(fault)) => Err(fault.poisoned()),
        (Err(fault), Err(cleanup)) => Err(fault.with_cleanup(cleanup)),
    }
}

/// Map a permissions word to the entry kind; without permissions the kind
/// is honestly `Other` — never guessed from the human longname.
fn kind_of(permissions: Option<u32>) -> RemoteEntryKind {
    match permissions {
        Some(mode) => match mode & 0xf000 {
            0x4000 => RemoteEntryKind::Directory,
            0x8000 => RemoteEntryKind::File,
            _ => RemoteEntryKind::Other,
        },
        None => RemoteEntryKind::Other,
    }
}
