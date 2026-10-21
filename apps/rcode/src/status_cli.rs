//! `rcode status` — the workspace mode, the data directory, and engine liveness.
//!
//! There is no account and no sign-in. This file used to carry `login` and
//! `logout` as well; both had already been reduced to saying that there was
//! nothing to sign in to and tidying up a `session.json` left by a build that
//! had them. A command whose whole body is that is a command to delete, not to
//! keep and explain.

use rcode_engine::{Engine, EngineConfig, InstanceLock};

/// `rcode status`.
pub async fn status(config: EngineConfig) -> anyhow::Result<()> {
    let auth = Engine::build_auth(&config).await;
    println!("Config:   {}", config.roots.config().display());
    println!("Data:     {}", config.roots.data().display());
    println!("Mode:     local only");
    // Which store root the runtime opens is the half a name cannot carry, and
    // a path answers it better than any label would.
    let profile = Engine::resolve_profile(&config, &auth);
    println!("Store:    {}", profile.store_root().display());
    match InstanceLock::holder(config.roots.config()) {
        Some(pid) => println!("Engine:   running (pid {pid})"),
        None => println!("Engine:   not running"),
    }
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], config.ipc_port));
    let ipc = std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(500));
    println!(
        "IPC:      {} 127.0.0.1:{}",
        if ipc.is_ok() {
            "listening on"
        } else {
            "not listening on"
        },
        config.ipc_port
    );
    Ok(())
}
