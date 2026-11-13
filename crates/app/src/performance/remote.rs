//! Real authenticated loopback requests while the diagnostic editor keeps painting.
use super::{Result, clock_ms};
use craft_control::loopback::{Connection, Token};
use serde_json::Value;
use std::{
    sync::mpsc::{self, Receiver, TryRecvError},
    time::Duration,
};

/// A single bounded client reply. No UI callback waits on a socket or joins this worker.
pub(super) struct RemoteEdit {
    receive: Receiver<Result<f64>>,
    started: f64,
}

impl RemoteEdit {
    /// Run one production control request with the live adapter's normal authentication.
    /// A disconnected UI receiver releases the worker after the bounded socket timeout.
    pub(super) fn start(port: u16, secret: &str, request: Value, ctx: egui::Context) -> Result<Self> {
        let token = Token::parse(secret).map_err(|e| e.to_string())?;
        let (send, receive) = mpsc::sync_channel(1);
        let started = clock_ms();
        std::thread::Builder::new()
            .name("pcbcraft-perf-client".into())
            .spawn(move || {
                let call = || {
                    let mut peer = Connection::connect(port, &token, request, Duration::from_secs(5)).map_err(|e| e.to_string())?;
                    let reply = peer.receive(1 << 20).map_err(|e| e.to_string())?.ok_or("MCP peer closed without a reply")?;
                    if let Some(error) = reply.get("error") {
                        return Err(format!("MCP edit failed: {error}"));
                    }
                    if reply.get("result").is_none() {
                        return Err("MCP edit reply has no result".into());
                    }
                    Ok(clock_ms() - started)
                };
                let _ = send.try_send(call());
                ctx.request_repaint();
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { receive, started })
    }

    /// Poll without blocking. The outer deadline also catches a worker that stops replying.
    pub(super) fn poll(&self) -> Result<Option<f64>> {
        match self.receive.try_recv() {
            Ok(result) => result.map(Some),
            Err(TryRecvError::Empty) if clock_ms() - self.started < 12_000.0 => Ok(None),
            Err(TryRecvError::Empty) => Err("MCP diagnostic request timed out".into()),
            Err(TryRecvError::Disconnected) => Err("MCP diagnostic worker disconnected".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A lost worker and a worker that never replies must both fail without blocking the UI.
    #[test]
    fn disconnected_and_stalled_clients_fail() {
        let (send, receive) = mpsc::sync_channel(1);
        let mut remote = RemoteEdit { receive, started: clock_ms() };
        assert!(matches!(remote.poll(), Ok(None)));
        remote.started -= 13_000.0;
        assert!(remote.poll().is_err_and(|error| error.contains("timed out")));
        drop(send);
        assert!(remote.poll().is_err_and(|error| error.contains("disconnected")));
    }
}
