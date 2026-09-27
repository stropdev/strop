//! Session handshake and native capability advertisement.

use std::io::{Read, Write};

use strop_worker_protocol::codec::{self, Incoming};
use strop_worker_protocol::frame::{self, FrameDecoder};
use strop_worker_protocol::{
    Capabilities, ClientMessage, EndpointInfo, Limits, NamespaceIdentity, NotifyCoverage,
    ProtocolError, Session, ShutdownReason, WorkerMessage, PROTOCOL_VERSION,
};

#[cfg(unix)]
use super::cache_lease;
use super::{schedule, CacheGuard, ServeError};

fn mint(error_stage: &'static str) -> Result<u64, ServeError> {
    let mut bytes = [0_u8; 8];
    getrandom::fill(&mut bytes)
        .map_err(|error| ServeError::Random(format!("{error_stage}: {error}")))?;
    Ok(u64::from_le_bytes(bytes))
}

/// Stable observed boot and mount namespace on Linux. A missing source
/// fails recovery closed as "unattested"; a hostname is not authority.
fn namespace_identity() -> NamespaceIdentity {
    #[cfg(target_os = "linux")]
    let identity = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .and_then(|boot| {
            std::fs::read_link("/proc/self/ns/mnt")
                .map(|mount| format!("boot:{};mount:{}", boot.trim(), mount.display()))
        })
        .unwrap_or_else(|_| "unattested".into());
    #[cfg(not(target_os = "linux"))]
    let identity = "unattested".to_owned();
    #[cfg(unix)]
    // SAFETY: geteuid reads the process credential and accepts no pointers.
    let principal = Some(unsafe { libc::geteuid() });
    #[cfg(not(unix))]
    let principal = None;
    NamespaceIdentity {
        identity,
        principal,
    }
}

fn capabilities() -> Capabilities {
    let native_fs = cfg!(any(target_os = "linux", target_os = "macos"));
    Capabilities {
        observe: true,
        list: true,
        read: true,
        write: native_fs,
        trash: native_fs,
        notify: if cfg!(target_os = "linux") {
            NotifyCoverage::Native
        } else {
            NotifyCoverage::Unsupported
        },
        exec_finite: cfg!(unix),
        exec_service: cfg!(unix),
        // WK12 owns PTY serving on unix: spawn/feed/resize/exit ride the
        // exec family; elsewhere the refusal stays typed.
        pty: cfg!(unix),
    }
}

pub(super) fn limits() -> Limits {
    Limits {
        max_frame_bytes: strop_worker_protocol::MAX_BODY_BYTES,
        max_chunk_bytes: strop_worker_protocol::MAX_CHUNK_BYTES,
        max_pending_requests: 64,
        max_batch_steps: strop_fs::batch::STEP_LIMIT,
        max_listing_entries: 100_000,
        max_subscriptions: 64,
        max_streams: 64,
        max_exec_processes: 32,
        max_concurrent_reads: 16,
        control_reserve: 8,
        max_queued_data_chunks: schedule::DATA_LANE_CHUNKS,
    }
}

/// The handshake: exactly one `hello`, answered by `welcome` with the
/// fresh session authority. Anything else — or a version this build does
/// not speak — is a typed in-band failure and a clean close, never a
/// guessed downgrade.
pub(super) fn handshake(
    reader: &mut impl Read,
    decoder: &mut FrameDecoder,
    writer: &mut impl Write,
    limits: &Limits,
) -> Result<Option<(Session, NamespaceIdentity, CacheGuard)>, ServeError> {
    let first = frame::read_frame(reader, decoder)?;
    let Some(body) = first else {
        return Ok(None);
    };
    let hello = match codec::decode_body::<ClientMessage>(&body) {
        Ok(Incoming::Envelope(ClientMessage::Hello { protocol, .. })) => protocol,
        Ok(_) => {
            codec::write_envelope(
                &mut *writer,
                &WorkerMessage::Error {
                    id: None,
                    error: ProtocolError::Unexpected {
                        message: "the first message must be hello".into(),
                    },
                },
            )?;
            return Ok(None);
        }
        Err(error) => {
            codec::write_envelope(
                &mut *writer,
                &WorkerMessage::Error {
                    id: None,
                    error: ProtocolError::Decode {
                        message: error.to_string(),
                    },
                },
            )?;
            return Ok(None);
        }
    };
    if hello != PROTOCOL_VERSION {
        codec::write_envelope(
            &mut *writer,
            &WorkerMessage::Error {
                id: None,
                error: ProtocolError::Version {
                    supported: PROTOCOL_VERSION,
                    offered: hello,
                },
            },
        )?;
        codec::write_envelope(
            &mut *writer,
            &WorkerMessage::Bye {
                reason: ShutdownReason::ProtocolViolation,
            },
        )?;
        return Ok(None);
    }
    let session = Session {
        incarnation: mint("incarnation")?,
        lease: strop_worker_protocol::LeaseId(mint("lease")?),
    };
    #[cfg(unix)]
    let cache_lease = match cache_lease::CacheLeaseGuard::register(session) {
        Ok(lease) => lease,
        Err(error) => {
            codec::write_envelope(
                &mut *writer,
                &WorkerMessage::Error {
                    id: None,
                    error: ProtocolError::Unexpected {
                        message: format!("cache lease admission refused: {error}"),
                    },
                },
            )?;
            return Ok(None);
        }
    };
    #[cfg(not(unix))]
    let cache_lease = ();
    let namespace = namespace_identity();
    codec::write_envelope(
        writer,
        &WorkerMessage::Welcome {
            protocol: PROTOCOL_VERSION,
            worker: EndpointInfo {
                name: "strop".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                build: None,
                target: strop_worker_protocol::TARGET_TRIPLE.into(),
            },
            session,
            namespace: namespace.clone(),
            limits: *limits,
            capabilities: capabilities(),
        },
    )?;
    Ok(Some((session, namespace, cache_lease)))
}
