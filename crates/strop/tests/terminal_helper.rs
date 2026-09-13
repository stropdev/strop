#![cfg(unix)]
use std::{
    ffi::OsStr,
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::{ffi::OsStrExt, net::UnixStream, process::CommandExt},
    },
    process::{Child, Command, Stdio},
    time::Duration,
};

struct Helper {
    channel: UnixStream,
    child: Child,
}
impl Helper {
    fn new() -> Self {
        let (channel, inherited) = UnixStream::pair().unwrap();
        channel
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        channel
            .set_write_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let fd = inherited.as_raw_fd();
        let mut command = Command::new(env!("CARGO_BIN_EXE_strop"));
        command
            .args(["--terminal-helper", &fd.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        // SAFETY: inherited remains live through spawn. This changes only the
        // child's descriptor flags, never the parent test runner's inheritance.
        unsafe {
            command.pre_exec(move || {
                let flags = libc::fcntl(fd, libc::F_GETFD);
                if flags == -1 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn().unwrap();
        drop(inherited);
        Self { channel, child }
    }
    fn send(&mut self, kind: u8, bytes: &[u8]) {
        self.channel
            .write_all(&(bytes.len() as u32).to_le_bytes())
            .unwrap();
        self.channel.write_all(&[kind]).unwrap();
        self.channel.write_all(bytes).unwrap();
    }
    fn receive(&mut self) -> (u8, Vec<u8>) {
        let mut header = [0; 5];
        self.channel.read_exact(&mut header).unwrap();
        let length = u32::from_le_bytes(header[..4].try_into().unwrap()) as usize;
        assert!(length <= 1024 * 1024 + 128);
        let mut body = vec![0; length];
        self.channel.read_exact(&mut body).unwrap();
        (header[4], body)
    }
    fn launch(&mut self, directory: &std::path::Path, script: &str) {
        let bytes = |text: &OsStr| text.as_bytes().to_vec();
        let launch = serde_json::json!({"version":1,"program":b"/bin/sh","arguments":[b"-c".as_slice(),script.as_bytes()],
            "directory":bytes(directory.as_os_str()),"environment":[(b"PATH".as_slice(),b"/usr/bin:/bin".as_slice()),(b"HOME".as_slice(),directory.as_os_str().as_bytes()),(b"TERM".as_slice(),b"xterm-256color".as_slice())],
            "geometry":{"columns":80,"rows":24,"revision":1}});
        self.send(1, &serde_json::to_vec(&launch).unwrap());
        assert_eq!(self.receive(), (16, 1u32.to_le_bytes().to_vec()));
    }
    fn finish(&mut self) -> (Vec<u8>, serde_json::Value, Vec<u64>) {
        let mut output = Vec::new();
        let mut acknowledgments = Vec::new();
        let phase = loop {
            let (kind, bytes) = self.receive();
            match kind {
                17 => output.extend(bytes),
                18 => acknowledgments.push(u64::from_le_bytes(bytes.try_into().unwrap())),
                19 => break serde_json::from_slice(&bytes).unwrap(),
                20 => panic!("helper failed: {}", String::from_utf8_lossy(&bytes)),
                _ => panic!("unexpected helper record {kind}"),
            }
        };
        let status = self.child.wait().unwrap();
        assert!(
            status.success(),
            "helper terminated after final publication: {status}"
        );
        (output, phase, acknowledgments)
    }
}
impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.channel.shutdown(std::net::Shutdown::Both);
        let _ = self.child.wait();
    }
}

#[test]
fn native_resize_input_final_output_and_background_cleanup_share_one_lifetime() {
    let directory = tempfile::tempdir().unwrap();
    let mut helper = Helper::new();
    // The FIFO proves the background child installed its ignored dispositions
    // before the shell exits; scheduler luck cannot make the escalation pass.
    helper.launch(directory.path(), "[ -t 0 ] && [ -t 1 ] || exit 90; read value; stty size; printf 'RESULT:%s\\n' \"$value\"; mkfifo ready; (trap '' HUP TERM; printf 'ready\\n' > ready; exec sleep 600) & child=$!; read ready < ready; printf 'BG:%s\\n' \"$child\"; exit 7");
    let mut resize = 1u64.to_le_bytes().to_vec();
    resize.extend(
        serde_json::to_vec(&serde_json::json!({"columns":37,"rows":9,"revision":2})).unwrap(),
    );
    helper.send(3, &resize);
    let mut input = 2u64.to_le_bytes().to_vec();
    input.extend_from_slice(b"hello\n");
    helper.send(2, &input);
    let (output, phase, acknowledgments) = helper.finish();
    let output = String::from_utf8(output).unwrap();
    assert!(
        output.contains("9 37") && output.contains("RESULT:hello"),
        "{output:?}"
    );
    assert_eq!(acknowledgments, [1, 2]);
    assert_eq!(
        phase,
        serde_json::json!({"Exited":{"code":7,"signal":null}})
    );
    #[cfg(target_os = "linux")]
    {
        let child: u32 = output
            .split("BG:")
            .nth(1)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(
            !std::path::Path::new(&format!("/proc/{child}")).exists(),
            "owned background child was not reaped"
        );
    }
}

#[test]
fn lease_revocation_before_launch_does_not_run_a_program() {
    let mut helper = Helper::new();
    helper.channel.shutdown(std::net::Shutdown::Write).unwrap();
    let (output, phase, acknowledgments) = helper.finish();
    assert!(output.is_empty() && acknowledgments.is_empty());
    assert_eq!(
        phase,
        serde_json::json!({"Exited":{"code":null,"signal":null}})
    );
}

#[test]
fn partial_unadmitted_command_is_revoked_by_lease_eof() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("must-not-execute");
    let mut helper = Helper::new();
    helper.launch(directory.path(), "read value; touch must-not-execute");
    helper.channel.write_all(&100u32.to_le_bytes()).unwrap();
    helper.channel.write_all(&[2]).unwrap();
    helper.channel.write_all(&1u64.to_le_bytes()).unwrap();
    helper.channel.write_all(b"incomplete").unwrap();
    helper.channel.shutdown(std::net::Shutdown::Write).unwrap();
    let (_, phase, acknowledgments) = helper.finish();
    assert!(!marker.exists());
    assert!(acknowledgments.is_empty());
    assert!(phase.get("Exited").is_some());
}
