//! The Kafka twin: the `lawtwin.kafka` contract's reference model behind the
//! Kafka wire protocol, with LawTwin's control plane.
//!
//! `model` is the code LawSpec generates from the contract; `wire` maps the
//! Kafka protocol onto its operations; `group` runs the classic group
//! protocol over the contract's coordinator; `admin` is the control plane
//! (`force`, `fault`, `advance`, `inspect`, `history`, `seed`); `laws`
//! checks states and histories against the contract's laws.

pub mod admin;
pub mod clock;
pub mod faults;
pub mod force;
pub mod group;
pub mod history;
pub mod laws;
pub mod model;
pub mod twin;
pub mod wire;

pub use twin::{Config, Twin};

use std::net::SocketAddr;
use std::sync::Arc;

/// A running twin: where it listens.
pub struct Running {
    /// The twin.
    pub twin: Arc<Twin>,
    /// The Kafka listener's address.
    pub kafka: SocketAddr,
    /// The admin API's address.
    pub admin: SocketAddr,
}

/// Binds the twin's two ports and serves them on the current runtime.
pub async fn start(config: Config) -> std::io::Result<Running> {
    let kafka_listener = tokio::net::TcpListener::bind(&config.kafka_listen).await?;
    let admin_listener = tokio::net::TcpListener::bind(&config.admin_listen).await?;
    let kafka = kafka_listener.local_addr()?;
    let admin = admin_listener.local_addr()?;
    let twin = Twin::new(config);
    *twin.kafka_addr.lock().unwrap() = Some(kafka);
    tokio::spawn(wire::serve(twin.clone(), kafka_listener));
    let router = admin::router(twin.clone());
    tokio::spawn(async move {
        let _ = axum::serve(admin_listener, router).await;
    });
    // On the wall clock, faults end and sessions time out as time passes.
    let ticker = twin.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            let mut core = ticker.lock();
            if core.clock.mode() == clock::ClockMode::Wall {
                core.expire_faults();
                group::expire_sessions(&mut core);
            }
        }
    });
    Ok(Running { twin, kafka, admin })
}

/// A runtime for the twin: the model's list walks recurse, so its threads
/// get room for them.
pub fn runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread().enable_all().thread_stack_size(64 << 20).build()
}

/// Runs the twin until the process is interrupted.
pub fn run(config: Config) -> std::io::Result<()> {
    runtime()?.block_on(async move {
        let running = start(config).await?;
        let (host, port) = running.twin.advertised();
        eprintln!(
            "lawtwin kafka twin: Kafka on {} (advertised {host}:{port}), admin API on http://{}, seed {}, clock {:?}",
            running.kafka, running.admin, running.twin.config.seed, running.twin.config.clock
        );
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
        Ok(())
    })
}
