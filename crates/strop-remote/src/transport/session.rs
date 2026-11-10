//! Per-job SFTP scripts for the pooled actors: resolve a location, read a
//! whole text file with its stamp, route a path by its attributes, or
//! enumerate a directory. Each script runs inside
//! [`guarded`], which races the work against the job's cancellation token,
//! the endpoint's stop signal and the total deadline — so the actor always
//! regains control promptly and never resynchronizes an interrupted stream.

use super::error::{Fault, RemoteFailureKind, RemoteStage};
use super::save::{stamp, Stamp};
use super::wire::Sftp;
use crate::client::{RemoteEntry, RemoteEntryKind};
use crate::pool::StopSignal;
use std::cell::Cell;
use std::future::{poll_fn, Future};
use std::pin::pin;
use std::task::Poll;
use std::time::Duration;
use strop_core::worker::CancelToken;
use strop_workspace::{RemoteFile, RemoteLocation};

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
    stage: &Cell<RemoteStage>,
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
    client: &mut Sftp<W, R>,
    location: &RemoteLocation,
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
            RemoteStage::Open,
            RemoteFailureKind::Protocol,
            "remote location has neither an absolute path nor a home query",
        )
    })?;
    if !client.offers(b"expand-path@openssh.com") {
        return Err(Fault::new(
            RemoteStage::Open,
            RemoteFailureKind::HomeUnsupported,
            "the server does not advertise expand-path@openssh.com, and \
             REALPATH is not a portable substitute for home expansion",
        ));
    }
    let expanded = client.expand_path(relative).await?;
    RemoteFile::from_path(endpoint, expanded).map_err(|error| {
        Fault::new(
            RemoteStage::Open,
            RemoteFailureKind::Protocol,
            format!("expanded home path is not admissible: {error}"),
        )
    })
}

/// Read one whole text file and the stamp a later save compares
/// against. The bytes must be UTF-8: a buffer is text.
pub(crate) async fn read_text<W, R>(
    client: &mut Sftp<W, R>,
    file: &RemoteFile,
    max: u64,
    stage: &Cell<RemoteStage>,
) -> Result<(String, Stamp), Fault>
where
    W: tokio::io::AsyncWrite + Unpin,
    R: tokio::io::AsyncRead + Unpin,
{
    stage.set(RemoteStage::Transfer);
    let (bytes, attrs) = client.read_whole(file.path(), max).await?;
    let stamp = stamp(&attrs)?;
    stage.set(RemoteStage::Validate);
    match String::from_utf8(bytes) {
        Ok(text) => Ok((text, stamp)),
        Err(error) => Err(Fault::new(
            RemoteStage::Validate,
            RemoteFailureKind::InvalidUtf8,
            format!("invalid UTF-8 at byte {}", error.utf8_error().valid_up_to()),
        )),
    }
}

/// Route a path by its attributes without opening anything: how an explicit
/// open decides between the file and directory resources.
pub(crate) async fn path_kind<W, R>(
    client: &mut Sftp<W, R>,
    file: &RemoteFile,
    stage: &Cell<RemoteStage>,
) -> Result<RemoteEntryKind, Fault>
where
    W: tokio::io::AsyncWrite + Unpin,
    R: tokio::io::AsyncRead + Unpin,
{
    stage.set(RemoteStage::Inspect);
    let attrs = client.stat(file.path()).await?;
    Ok(kind_of(attrs.permissions))
}

/// Enumerate one directory: pages are packet-bounded, the accumulated
/// listing is capped, `.` and `..` are not entries the user can open, and
/// the handle is closed before the result is returned.
pub(crate) async fn list_entries<W, R>(
    client: &mut Sftp<W, R>,
    directory: &RemoteFile,
    stage: &Cell<RemoteStage>,
) -> Result<Vec<RemoteEntry>, Fault>
where
    W: tokio::io::AsyncWrite + Unpin,
    R: tokio::io::AsyncRead + Unpin,
{
    stage.set(RemoteStage::Open);
    let handle = client.opendir(directory.path()).await?;
    let listing = enumerate(client, directory, stage, &handle).await;
    finish_handle(client, handle, listing).await
}

async fn enumerate<W, R>(
    client: &mut Sftp<W, R>,
    directory: &RemoteFile,
    stage: &Cell<RemoteStage>,
    handle: &super::wire::FileHandle,
) -> Result<Vec<RemoteEntry>, Fault>
where
    W: tokio::io::AsyncWrite + Unpin,
    R: tokio::io::AsyncRead + Unpin,
{
    stage.set(RemoteStage::Transfer);
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
                                RemoteStage::Transfer,
                                RemoteFailureKind::Protocol,
                                format!("entry name is not admissible: {error}"),
                            )
                        })?;
                    entries.push(RemoteEntry {
                        file,
                        kind: kind_of(raw.permissions),
                        permissions: raw.permissions.map(crate::RemotePermissions::from_mode),
                        size: raw.size,
                    });
                    if entries.len() > super::wire::MAX_ENTRIES {
                        return Err(Fault::new(
                            RemoteStage::Transfer,
                            RemoteFailureKind::TooManyEntries,
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
    client: &mut Sftp<W, R>,
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
                RemoteFailureKind::Protocol
                    | RemoteFailureKind::Io
                    | RemoteFailureKind::Cancelled
                    | RemoteFailureKind::Deadline
                    | RemoteFailureKind::Stopped
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
/// is honestly `Unknown` — never guessed from the human longname.
pub(crate) fn kind_of(permissions: Option<u32>) -> RemoteEntryKind {
    match permissions {
        Some(mode) => match mode & 0xf000 {
            0x4000 => RemoteEntryKind::Directory,
            0x8000 => RemoteEntryKind::File,
            0xa000 => RemoteEntryKind::SymbolicLink,
            0x1000 => RemoteEntryKind::Fifo,
            0xc000 => RemoteEntryKind::Socket,
            0x6000 => RemoteEntryKind::BlockDevice,
            0x2000 => RemoteEntryKind::CharacterDevice,
            _ => RemoteEntryKind::Unknown,
        },
        None => RemoteEntryKind::Unknown,
    }
}
