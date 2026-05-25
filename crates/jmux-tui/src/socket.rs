use std::path::PathBuf;

use anyhow::Result;
use jmux_core::{parse_set_status, AgentState, Request};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::mpsc;

#[derive(Debug)]
pub enum SocketEvent {
    SetStatus {
        state: AgentState,
        session_id: Option<usize>,
        pane_id: Option<usize>,
    },
    Flash {
        session_id: Option<usize>,
        pane_id: Option<usize>,
    },
    NewSession {
        path: PathBuf,
    },
    SetCwd {
        path: PathBuf,
    },
}

pub async fn run_socket_server(path: PathBuf, tx: mpsc::Sender<SocketEvent>) -> Result<()> {
    // Remove stale socket file if it exists
    if path.exists() {
        std::fs::remove_file(&path)?;
    }

    let listener = UnixListener::bind(&path)?;

    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let tx = tx.clone();
                tokio::spawn(async move {
                    let reader = BufReader::new(stream);
                    let mut lines = reader.lines();

                    if let Ok(Some(line)) = lines.next_line().await {
                        match Request::parse(&line) {
                            Ok(req) => match req.method.as_str() {
                                "set-status" => match parse_set_status(&req.params) {
                                    Ok(state) => {
                                        let session_id = req
                                            .params
                                            .get("session_id")
                                            .and_then(|v| v.as_u64())
                                            .map(|v| v as usize);
                                        let pane_id = req
                                            .params
                                            .get("pane_id")
                                            .and_then(|v| v.as_u64())
                                            .map(|v| v as usize);
                                        let _ = tx
                                            .send(SocketEvent::SetStatus {
                                                state,
                                                session_id,
                                                pane_id,
                                            })
                                            .await;
                                    }
                                    Err(e) => {
                                        eprintln!("jmux socket: bad set-status params: {}", e);
                                    }
                                },
                                "flash" => {
                                    let session_id = req
                                        .params
                                        .get("session_id")
                                        .and_then(|v| v.as_u64())
                                        .map(|v| v as usize);
                                    let pane_id = req
                                        .params
                                        .get("pane_id")
                                        .and_then(|v| v.as_u64())
                                        .map(|v| v as usize);
                                    let _ = tx
                                        .send(SocketEvent::Flash {
                                            session_id,
                                            pane_id,
                                        })
                                        .await;
                                }
                                "new-session" => {
                                    if let Some(path_str) =
                                        req.params.get("path").and_then(|v| v.as_str())
                                    {
                                        let path = PathBuf::from(path_str);
                                        let _ = tx.send(SocketEvent::NewSession { path }).await;
                                    } else {
                                        eprintln!("jmux socket: new-session missing path param");
                                    }
                                }
                                "set-cwd" => {
                                    if let Some(path_str) =
                                        req.params.get("path").and_then(|v| v.as_str())
                                    {
                                        let path = PathBuf::from(path_str);
                                        let _ = tx.send(SocketEvent::SetCwd { path }).await;
                                    } else {
                                        eprintln!("jmux socket: set-cwd missing path param");
                                    }
                                }
                                other => {
                                    eprintln!("jmux socket: unknown method: {}", other);
                                }
                            },
                            Err(e) => {
                                eprintln!("jmux socket: failed to parse request: {}", e);
                            }
                        }
                    }
                });
            }
            Err(e) => {
                eprintln!("jmux socket: accept error: {}", e);
            }
        }
    }
}
