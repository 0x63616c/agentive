//! Isolated stdio transport for the locally installed Codex App Server.

use crate::{CodexSession, CodexTransport, protocol_error, transport_error};
use agentive::ProviderError;
use futures::future::BoxFuture;
use serde_json::Value;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::time::{Duration, timeout};

/// The default local stdio transport.
#[derive(Clone, Debug)]
pub struct StdioTransport {
    pub(crate) executable: PathBuf,
}

impl Default for StdioTransport {
    fn default() -> Self {
        Self {
            executable: PathBuf::from("codex"),
        }
    }
}

impl CodexTransport for StdioTransport {
    fn open(&self) -> BoxFuture<'static, Result<Box<dyn CodexSession>, ProviderError>> {
        let executable = self.executable.clone();
        Box::pin(async move {
            let mut child = Command::new(executable)
                .arg("app-server")
                .env_remove("CODEX_API_KEY")
                .env_remove("OPENAI_API_KEY")
                .env_remove("CODEX_ACCESS_TOKEN")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .map_err(|_| transport_error())?;
            let stdin = child.stdin.take().ok_or_else(transport_error)?;
            let stdout = child.stdout.take().ok_or_else(transport_error)?;
            if let Some(stderr) = child.stderr.take() {
                tokio::spawn(async move {
                    let mut lines = BufReader::new(stderr).lines();
                    while lines.next_line().await.ok().flatten().is_some() {}
                });
            }
            Ok(Box::new(StdioSession {
                child,
                stdin,
                stdout: BufReader::new(stdout).lines(),
            }) as Box<dyn CodexSession>)
        })
    }
}

struct StdioSession {
    child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
}
impl Drop for StdioSession {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}
impl CodexSession for StdioSession {
    fn send(&mut self, message: Value) -> BoxFuture<'_, Result<(), ProviderError>> {
        Box::pin(async move {
            let mut encoded = serde_json::to_vec(&message).map_err(|_| protocol_error())?;
            encoded.push(b'\n');
            self.stdin
                .write_all(&encoded)
                .await
                .map_err(|_| transport_error())?;
            self.stdin.flush().await.map_err(|_| transport_error())
        })
    }
    fn receive(&mut self) -> BoxFuture<'_, Result<Option<Value>, ProviderError>> {
        Box::pin(async move {
            let Some(line) = self
                .stdout
                .next_line()
                .await
                .map_err(|_| transport_error())?
            else {
                return Ok(None);
            };
            serde_json::from_str(&line)
                .map(Some)
                .map_err(|_| protocol_error())
        })
    }

    fn shutdown(&mut self) -> BoxFuture<'_, Result<(), ProviderError>> {
        Box::pin(async move {
            if self
                .child
                .try_wait()
                .map_err(|_| transport_error())?
                .is_some()
            {
                return Ok(());
            }
            // Closing stdin asks App Server to exit normally. A concurrent exit
            // may make the close itself fail, but the authoritative operation is
            // still waiting for and reaping the child below.
            let _ = self.stdin.shutdown().await;
            match timeout(Duration::from_secs(2), self.child.wait()).await {
                Ok(Ok(_status)) => {}
                Ok(Err(_)) => return Err(transport_error()),
                Err(_) => {
                    self.child.kill().await.map_err(|_| transport_error())?;
                    self.child.wait().await.map_err(|_| transport_error())?;
                }
            }
            Ok(())
        })
    }
}
