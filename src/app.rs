use crate::{client::RuntimeProcess, session::SessionManager};
use std::io::IsTerminal;
pub async fn run() -> anyhow::Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("--version") => {
            println!("rous {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some("--help") => {
            println!(
                "Rous terminal client\nSAKI_RUNTIME_BIN selects the external Saki stdio service.\nSAKI_UI=console|tui; SAKI_SESSION=<id>|new; SAKI_THEME=auto|light|dark."
            );
            return Ok(());
        }
        _ => {}
    }
    let mode = match std::env::var("SAKI_UI").as_deref() {
        Ok("console") => "console",
        Ok("tui") if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() => "tui",
        Ok("tui") => anyhow::bail!("SAKI_UI=tui requires a terminal"),
        Ok(other) => {
            eprintln!("unknown SAKI_UI={other:?}; using console");
            "console"
        }
        Err(_) if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() => "tui",
        Err(_) => "console",
    };
    let binary = std::env::var_os("SAKI_RUNTIME_BIN").unwrap_or_else(|| "Saki".into());
    let process = RuntimeProcess::start(tokio::process::Command::new(binary)).await?;
    let initial = std::env::var("SAKI_SESSION").ok().filter(|id| id != "new");
    let manager = SessionManager::new(process, initial);
    let (tx, rx) = tokio::sync::mpsc::channel(32);
    let (events, receiver) = crate::session::channel::channel();
    let worker = tokio::spawn(manager.run(rx, events));
    let result = if mode == "console" {
        crate::frontend::console::run(tx, receiver).await
    } else {
        crate::frontend::tui::run(tx, receiver).await
    };
    worker.await??;
    result
}
