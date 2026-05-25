use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use jmux_core::AppState;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::{mpsc, watch, Mutex};

pub struct PtyMessage {
    pub session_id: usize,
    pub pane_id: usize,
    pub data: Vec<u8>,
}

pub struct DaemonClient {
    pub writer: Arc<Mutex<tokio::net::unix::OwnedWriteHalf>>,
    state_rx: watch::Receiver<Option<AppState>>,
    pub pty_rx: Arc<Mutex<mpsc::Receiver<PtyMessage>>>,
}

impl DaemonClient {
    pub async fn connect(socket_path: &Path) -> Result<Self> {
        let stream = UnixStream::connect(socket_path).await?;
        let (reader, writer) = stream.into_split();

        let (state_tx, state_rx) = watch::channel::<Option<AppState>>(None);
        let (pty_tx, pty_rx) = mpsc::channel::<PtyMessage>(1024);
        let writer = Arc::new(Mutex::new(writer));
        tokio::spawn(async move {
            let mut lines = BufReader::new(reader).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) {
                    match msg["type"].as_str() {
                        Some("state") => {
                            if let Ok(state) =
                                serde_json::from_value::<AppState>(msg["data"].clone())
                            {
                                let _ = state_tx.send(Some(state));
                            }
                        }
                        Some("pty") => {
                            let session_id = msg["session_id"].as_u64().unwrap_or(0) as usize;
                            let pane_id = msg["pane_id"].as_u64().unwrap_or(0) as usize;
                            let data: Vec<u8> = msg["data"]
                                .as_array()
                                .map(|a| {
                                    a.iter()
                                        .filter_map(|v| v.as_u64().map(|b| b as u8))
                                        .collect()
                                })
                                .unwrap_or_default();
                            if !data.is_empty() {
                                let _ = pty_tx
                                    .send(PtyMessage {
                                        session_id,
                                        pane_id,
                                        data,
                                    })
                                    .await;
                            }
                        }
                        _ => {}
                    }
                }
            }
        });

        Ok(Self {
            writer,
            state_rx,
            pty_rx: Arc::new(Mutex::new(pty_rx)),
        })
    }

    pub async fn subscribe(&self) -> Result<()> {
        let mut w = self.writer.lock().await;
        w.write_all(b"{\"method\":\"subscribe\",\"params\":{}}\n")
            .await?;
        Ok(())
    }

    pub async fn send_input(&self, session_id: usize, pane_id: usize, data: &[u8]) -> Result<()> {
        let data_arr: Vec<u8> = data.to_vec();
        let msg = serde_json::json!({
            "method": "input",
            "params": {
                "session_id": session_id,
                "pane_id": pane_id,
                "data": data_arr,
            }
        })
        .to_string()
            + "\n";
        let mut w = self.writer.lock().await;
        w.write_all(msg.as_bytes()).await?;
        Ok(())
    }

    pub async fn send_pane_resize(&self, session_id: usize, pane_id: usize, rows: u16, cols: u16) -> Result<()> {
        let msg = serde_json::json!({
            "method": "resize-pane",
            "params": { "session_id": session_id, "pane_id": pane_id, "rows": rows, "cols": cols }
        })
        .to_string()
            + "\n";
        let mut w = self.writer.lock().await;
        w.write_all(msg.as_bytes()).await?;
        Ok(())
    }

    pub async fn send_resize(&self, rows: u16, cols: u16) -> Result<()> {
        let msg = serde_json::json!({
            "method": "resize",
            "params": { "rows": rows, "cols": cols }
        })
        .to_string()
            + "\n";
        let mut w = self.writer.lock().await;
        w.write_all(msg.as_bytes()).await?;
        Ok(())
    }

    pub async fn detach(&self) -> Result<()> {
        let mut w = self.writer.lock().await;
        w.write_all(b"{\"method\":\"detach\",\"params\":{}}\n")
            .await?;
        Ok(())
    }

    pub fn current_state(&self) -> Option<AppState> {
        self.state_rx.borrow().clone()
    }

    pub fn state_rx(&self) -> watch::Receiver<Option<AppState>> {
        self.state_rx.clone()
    }
}
