//! Admitted finite exec, service and PTY client requests. Streaming
//! attachments belong to the exact worker incarnation that minted them.

use std::collections::HashMap;
use std::sync::mpsc::channel;
use std::sync::Arc;

use strop_core::worker::CancelToken;
use strop_worker_protocol::{
    ClientMessage, ExecId, ProtocolError, PtyGeometry, Request, ResultOutcome,
};

use crate::connection::{self, Conn, Reply};
use crate::error::ClientError;
use crate::payload::{CancelGuard, ReadPayload};
use crate::session::{ExecHandle, PtySession};
use crate::{missing_stream, unexpected, CallResult, Worker};

impl Worker {
    /// Spawn one admitted finite command or leased service. The exit
    /// arrives on the handle's receiver even if the process settles
    /// before this call returns — the reader thread registers the waiter
    /// before delivering `ExecStarted`.
    pub fn exec(
        &self,
        token: &CancelToken,
        spec: strop_worker_protocol::ExecSpec,
    ) -> Result<ExecHandle, ClientError> {
        let CallResult {
            outcome,
            attachments,
            conn,
            ..
        } = self.call_streaming(token, Request::Exec { spec })?;
        let cancel = CancelGuard(token.clone());
        let (exec, stdin, stdout, stderr) = match outcome {
            ResultOutcome::ExecStarted {
                exec,
                stdin,
                stdout,
                stderr,
            } => (exec, stdin, stdout, stderr),
            other => return Err(unexpected(other)),
        };
        let stdout_stream = stdout;
        let stderr_stream = stderr;
        let mut streams: HashMap<_, _> = attachments.streams.into_iter().collect();
        let stdout = streams
            .remove(&stdout_stream)
            .ok_or_else(|| missing_stream("exec stdout"))?;
        let stderr = streams
            .remove(&stderr_stream)
            .ok_or_else(|| missing_stream("exec stderr"))?;
        let Some((_, exit)) = attachments.exits.into_iter().next() else {
            return Err(ClientError::Protocol(ProtocolError::Stream {
                message: "exec exit waiter was not attached".into(),
            }));
        };
        let stdout = ReadPayload::exec(stdout, token.clone(), Arc::clone(&conn), stdout_stream);
        let stderr = ReadPayload::exec(stderr, token.clone(), Arc::clone(&conn), stderr_stream);
        Ok(ExecHandle {
            id: exec,
            conn,
            stdin,
            stdin_sequence: 0,
            stdout,
            stderr,
            exit,
            cancel,
        })
    }

    pub(crate) fn exec_cancel_on(
        &self,
        token: &CancelToken,
        exec: ExecId,
        conn: Arc<Conn>,
    ) -> Result<(), ClientError> {
        match self
            .call_inner_on(token, Request::ExecCancel { exec }, false, conn)?
            .outcome
        {
            ResultOutcome::Done => Ok(()),
            other => Err(unexpected(other)),
        }
    }

    /// Resize one admitted PTY exec (0058 WK12). The reply means the
    /// worker applied `TIOCSWINSZ` in the exec's input order; the
    /// ordered [`StreamEvent::Resized`] boundary rides the exec's
    /// output stream in wire order.
    pub(crate) fn exec_resize_on(
        &self,
        token: &CancelToken,
        exec: ExecId,
        geometry: PtyGeometry,
        conn: Arc<Conn>,
    ) -> Result<(), ClientError> {
        if token.is_cancelled() {
            return Err(ClientError::Cancelled);
        }
        let id = conn.alloc_request();
        let (tx, rx) = channel();
        conn.admit(id, connection::Pending::pty_resize(tx, exec))?;
        let registration = token.register_cancel_resource({
            let conn = Arc::clone(&conn);
            move || {
                conn.write_quiet(&ClientMessage::Cancel {
                    session: conn.session(),
                    id,
                });
                Ok(())
            }
        });
        if let Err(failure) = registration {
            conn.retract(id);
            return Err(ClientError::Protocol(ProtocolError::Unexpected {
                message: failure.message,
            }));
        }
        let sent = conn.write(&ClientMessage::Request {
            session: conn.session(),
            id,
            body: Box::new(Request::ExecResize { exec, geometry }),
        });
        if let Err(error) = sent {
            conn.retract(id);
            token.clear_cancel_resource();
            return Err(error);
        }
        let reply = rx.recv();
        token.clear_cancel_resource();
        match reply {
            Ok(Reply::Outcome { outcome, .. }) => match outcome {
                ResultOutcome::Done => Ok(()),
                other => Err(unexpected(other)),
            },
            Ok(Reply::Protocol(error)) => Err(ClientError::Protocol(error)),
            Ok(Reply::Lost(reason)) => Err(ClientError::WorkerLost(reason)),
            Err(_) => Err(ClientError::WorkerLost(conn.death_detail())),
        }
    }

    /// Spawn one admitted PTY (0058 WK12): the terminal's input,
    /// bounded merged output, ordered resize and truthful exit as one
    /// session. The spec must carry `pty` geometry; the worker's
    /// admission decides the rest — a refusal is typed, never a silent
    /// pipe.
    pub fn exec_pty(
        &self,
        token: &CancelToken,
        spec: strop_worker_protocol::ExecSpec,
    ) -> Result<PtySession, ClientError> {
        if spec.pty.is_none() {
            return Err(ClientError::Protocol(ProtocolError::Unexpected {
                message: "exec_pty requires a PTY geometry in the spec".into(),
            }));
        }
        let CallResult {
            outcome,
            attachments,
            conn,
            ..
        } = self.call_streaming(token, Request::Exec { spec })?;
        let spawn_cancel = CancelGuard(token.clone());
        let (exec, stdin, stdout, _stderr) = match outcome {
            ResultOutcome::ExecStarted {
                exec,
                stdin,
                stdout,
                stderr,
            } => (exec, stdin, stdout, stderr),
            other => return Err(unexpected(other)),
        };
        let Some(stdin) = stdin else {
            return Err(ClientError::Protocol(ProtocolError::Stream {
                message: "a PTY exec admitted no input stream".into(),
            }));
        };
        let mut streams: HashMap<_, _> = attachments.streams.into_iter().collect();
        let output = streams
            .remove(&stdout)
            .ok_or_else(|| missing_stream("pty output"))?;
        let Some((_, events)) = attachments.exits.into_iter().next() else {
            return Err(ClientError::Protocol(ProtocolError::Stream {
                message: "exec exit waiter was not attached".into(),
            }));
        };
        // The session's own control token for resize/terminate: the
        // spawn token's cancel hook stays registered for its streams,
        // so control calls must not share it.
        let (control, lease) = CancelToken::standalone();
        Ok(PtySession {
            worker: self.clone(),
            conn,
            id: exec,
            stdin,
            stdin_sequence: 0,
            output_stream: stdout,
            output,
            events,
            control,
            _lease: lease,
            _spawn_cancel: spawn_cancel,
        })
    }
}
