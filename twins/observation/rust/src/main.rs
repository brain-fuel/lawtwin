//! Start a fresh observation receiver; intake and administration bind separately.
use lawtwin_observation::{Config, Observer};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mut config = None;
    let mut listen = "127.0.0.1:4318".to_string();
    let mut admin = "127.0.0.1:9646".to_string();
    while let Some(option) = args.next() {
        match option.as_str() {
            "--version" | "-V" => {
                println!("lawtwin-observation {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--config" => config = Some(args.next().ok_or("--config needs a file")?),
            "--listen" => listen = args.next().ok_or("--listen needs an address")?,
            "--admin-listen" => admin = args.next().ok_or("--admin-listen needs an address")?,
            "--help" => {
                println!(
                    "lawtwin-observation --config <file> [--listen 127.0.0.1:4318] [--admin-listen 127.0.0.1:9646]"
                );
                return Ok(());
            }
            other => return Err(format!("unknown option {other}").into()),
        }
    }
    let config: Config =
        serde_json::from_slice(&std::fs::read(config.ok_or("--config is required")?)?)?;
    let observer = Observer::create(config).map_err(std::io::Error::other)?;
    let intake = tokio::net::TcpListener::bind(&listen).await?;
    let administration = tokio::net::TcpListener::bind(&admin).await?;
    eprintln!(
        "observation intake {}; admin {}",
        intake.local_addr()?,
        administration.local_addr()?
    );
    let intake_server =
        axum::serve(intake, observer.intake_router()).with_graceful_shutdown(stop());
    let admin_server =
        axum::serve(administration, observer.admin_router()).with_graceful_shutdown(stop());
    tokio::try_join!(async { intake_server.await }, async { admin_server.await })?;
    Ok(())
}

async fn stop() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("SIGTERM handler");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
