#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Minimal process boundary for an externally installed core worker.
//!
//! This is not an OS sandbox. The caller must authenticate the executable and its
//! parent directories against trusted installation metadata before spawning it.
//! Worker messages are bounded by the shared framing protocol.

use crate::worker_messages::{validate_worker_message, WorkerPayloadError};
use emulation_core::module_worker_protocol::{
    read_message, write_message, WorkerMessage, WorkerMessageKind, WorkerProtocolError,
};
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

/// Errors from worker executable validation, startup, or IPC.
#[derive(Debug)]
pub enum WorkerProcessError {
    /// The requested executable path is not absolute or does not name a file.
    InvalidExecutablePath,
    /// Process startup or lifecycle operation failed.
    Process(io::Error),
    /// Worker standard input/output pipes were not available.
    MissingPipe,
    /// The executable's SHA-256 did not match trusted installation metadata.
    ExecutableHashMismatch,
    /// No complete worker message arrived before the requested timeout.
    ReceiveTimeout,
    /// Framed IPC failed.
    Protocol(WorkerProtocolError),
    /// Message payload did not match the strict schema for its operation.
    Payload(WorkerPayloadError),
}

impl std::fmt::Display for WorkerProcessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidExecutablePath => {
                f.write_str("worker executable must be an absolute file path")
            }
            Self::Process(error) => write!(f, "worker process operation failed: {error}"),
            Self::MissingPipe => f.write_str("worker process did not expose required IPC pipes"),
            Self::ExecutableHashMismatch => {
                f.write_str("worker executable SHA-256 does not match trusted metadata")
            }
            Self::ReceiveTimeout => f.write_str("worker IPC receive timed out"),
            Self::Protocol(error) => write!(f, "worker IPC failed: {error}"),
            Self::Payload(error) => write!(f, "worker IPC payload rejected: {error}"),
        }
    }
}

impl std::error::Error for WorkerProcessError {}

impl From<io::Error> for WorkerProcessError {
    fn from(error: io::Error) -> Self {
        Self::Process(error)
    }
}

impl From<WorkerPayloadError> for WorkerProcessError {
    fn from(error: WorkerPayloadError) -> Self {
        Self::Payload(error)
    }
}

impl From<WorkerProtocolError> for WorkerProcessError {
    fn from(error: WorkerProtocolError) -> Self {
        Self::Protocol(error)
    }
}

/// A child process connected to the bounded host worker protocol.
pub struct WorkerProcess {
    child: Child,
    stdin: ChildStdin,
    messages: Receiver<Result<WorkerMessage, WorkerProtocolError>>,
    executable: PathBuf,
}

impl WorkerProcess {
    /// Starts a worker only when its executable SHA-256 matches trusted metadata.
    ///
    /// The expected digest must be 64 lowercase hexadecimal characters. The
    /// executable must still live in a directory protected from untrusted writes:
    /// portable path-based process APIs cannot eliminate every hash-to-spawn race.
    /// This verifies artifact identity, not publisher authenticity or OS sandboxing.
    pub fn spawn_verified<I, S>(
        executable: &Path,
        expected_sha256: &str,
        arguments: I,
    ) -> Result<Self, WorkerProcessError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        if expected_sha256.len() != 64
            || !expected_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(WorkerProcessError::ExecutableHashMismatch);
        }
        if !executable.is_absolute() || !executable.is_file() {
            return Err(WorkerProcessError::InvalidExecutablePath);
        }

        let mut file = File::open(executable)?;
        let mut hasher = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        let actual = format!("{:x}", hasher.finalize());
        if actual != expected_sha256 {
            return Err(WorkerProcessError::ExecutableHashMismatch);
        }

        Self::spawn(executable, arguments)
    }

    /// Starts a worker at an absolute path with a cleared inherited environment.
    ///
    /// The caller is responsible for validating the executable's signature/hash,
    /// install location and provenance. This method intentionally does not search
    /// the executable search path, inherit environment variables, or load a library
    /// into the host.
    pub fn spawn<I, S>(executable: &Path, arguments: I) -> Result<Self, WorkerProcessError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        if !executable.is_absolute() || !executable.is_file() {
            return Err(WorkerProcessError::InvalidExecutablePath);
        }
        let executable = executable.canonicalize()?;
        let mut command = Command::new(&executable);
        command
            .args(arguments)
            .env_clear()
            .current_dir(
                executable
                    .parent()
                    .ok_or(WorkerProcessError::InvalidExecutablePath)?,
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        let mut child = command.spawn()?;
        let stdin = child.stdin.take().ok_or(WorkerProcessError::MissingPipe)?;
        let mut stdout = child.stdout.take().ok_or(WorkerProcessError::MissingPipe)?;
        let (sender, messages) = mpsc::sync_channel(1);
        let _reader = std::thread::spawn(move || loop {
            let message = read_message(&mut stdout);
            let finished = message.is_err();
            if sender.send(message).is_err() || finished {
                break;
            }
        });
        Ok(Self {
            child,
            stdin,
            messages,
            executable,
        })
    }

    /// Returns the canonical path of the executable launched for this worker.
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    /// Sends one size-limited protocol message to the worker.
    pub fn send(&mut self, message: &WorkerMessage) -> Result<(), WorkerProcessError> {
        validate_worker_message(message)?;
        write_message(&mut self.stdin, message)?;
        Ok(())
    }

    /// Receives one size-limited protocol message from the worker.
    ///
    /// This call blocks until a complete message arrives or the worker's pipe closes.
    /// A supervising thread or platform-specific job object is still needed for hard
    /// timeouts and resource limits.
    pub fn receive(&mut self) -> Result<WorkerMessage, WorkerProcessError> {
        let message = match self.messages.recv() {
            Ok(message) => message,
            Err(_) => {
                // A disconnected reader means the worker can no longer be trusted
                // to complete this operation; terminate and reap it before returning.
                let _ = self.terminate();
                return Err(WorkerProcessError::Process(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "worker IPC reader stopped",
                )));
            }
        };
        let message = match message {
            Ok(message) => message,
            Err(error) => {
                let _ = self.terminate();
                return Err(WorkerProcessError::Protocol(error));
            }
        };
        if let Err(error) = validate_worker_message(&message) {
            let _ = self.terminate();
            return Err(WorkerProcessError::Payload(error));
        }
        Ok(message)
    }

    /// Receives one message, killing and reaping the worker if the timeout expires.
    ///
    /// This fail-closed behavior prevents a worker that stops responding from being
    /// reused after the caller's operation deadline has elapsed.
    pub fn receive_timeout(
        &mut self,
        timeout: Duration,
    ) -> Result<WorkerMessage, WorkerProcessError> {
        let message = match self.messages.recv_timeout(timeout) {
            Ok(Ok(message)) => message,
            Ok(Err(error)) => {
                let _ = self.terminate();
                return Err(WorkerProcessError::Protocol(error));
            }
            Err(RecvTimeoutError::Timeout) => {
                self.terminate()?;
                return Err(WorkerProcessError::ReceiveTimeout);
            }
            Err(RecvTimeoutError::Disconnected) => {
                // Treat a vanished IPC reader as a failed worker, not as a reusable
                // process handle: stdout may have closed before the child exited.
                let _ = self.terminate();
                return Err(WorkerProcessError::Process(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "worker IPC reader stopped",
                )))
            }
        };
        if let Err(error) = validate_worker_message(&message) {
            let _ = self.terminate();
            return Err(WorkerProcessError::Payload(error));
        }
        Ok(message)
    }

    /// Requests an orderly shutdown without waiting indefinitely for the child.
    pub fn request_shutdown(&mut self) -> Result<(), WorkerProcessError> {
        self.send(&WorkerMessage::new(
            WorkerMessageKind::Shutdown,
            Vec::new(),
        )?)
    }

    /// Requests shutdown, waits at most the given duration, and kills/reaps a stuck worker.
    ///
    /// A timeout is fail-closed. This bounds wall-clock shutdown time, not CPU or memory
    /// consumption enforced by the operating system.
    pub fn shutdown(
        &mut self,
        timeout: Duration,
    ) -> Result<std::process::ExitStatus, WorkerProcessError> {
        self.request_shutdown()?;
        let started = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait()? {
                return Ok(status);
            }
            let elapsed = started.elapsed();
            if elapsed >= timeout {
                self.terminate()?;
                return Err(WorkerProcessError::ReceiveTimeout);
            }
            let remaining = timeout.saturating_sub(elapsed);
            std::thread::sleep(remaining.min(Duration::from_millis(10)));
        }
    }

    /// Returns the exit status if the worker has exited.
    pub fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>, WorkerProcessError> {
        Ok(self.child.try_wait()?)
    }

    /// Terminates and reaps the worker immediately.
    pub fn terminate(&mut self) -> Result<(), WorkerProcessError> {
        match self.child.try_wait()? {
            Some(_) => Ok(()),
            None => {
                self.child.kill()?;
                self.child.wait()?;
                Ok(())
            }
        }
    }
}

impl Drop for WorkerProcess {
    fn drop(&mut self) {
        // Never leave an orphan worker behind when the host releases its handle.
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{WorkerProcess, WorkerProcessError};
    use std::path::Path;

    #[test]
    fn refuses_executable_with_wrong_digest_before_startup() {
        let executable = std::env::current_exe().unwrap();
        let error = match WorkerProcess::spawn_verified(
            &executable,
            &"0".repeat(64),
            std::iter::empty::<&str>(),
        ) {
            Err(error) => error,
            Ok(_) => panic!("digest mismatch must prevent process startup"),
        };
        assert!(matches!(error, WorkerProcessError::ExecutableHashMismatch));
    }

    #[test]
    fn refuses_malformed_digest_and_relative_executable() {
        assert!(matches!(
            WorkerProcess::spawn_verified(
                Path::new("worker.exe"),
                "not-a-digest",
                std::iter::empty::<&str>(),
            ),
            Err(WorkerProcessError::ExecutableHashMismatch)
        ));
        assert!(matches!(
            WorkerProcess::spawn(Path::new("worker.exe"), std::iter::empty::<&str>()),
            Err(WorkerProcessError::InvalidExecutablePath)
        ));
    }
}
