pub mod app;
pub mod daemon;
pub mod daemon_client;
pub mod input;
pub mod layout;
pub mod pty;
pub mod socket;
pub mod widgets;

pub use app::App;
pub use daemon::run_daemon;
pub use daemon_client::DaemonClient;
pub use pty::PtyOutput;
pub use socket::{run_socket_server, SocketEvent};

#[cfg(test)]
mod tests;
