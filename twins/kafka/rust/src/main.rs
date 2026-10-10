//! `lawtwin-kafka`: the Kafka twin's Rust core as a program.
//!
//! Every core of the Kafka twin, whatever its language, takes the same
//! options and environment, listens on a Kafka port and an admin port, and
//! serves the same control plane (docs/reference/kafka-core.md):
//!
//!   lawtwin-kafka [--host <addr>] [--port <n>] [--admin-port <n>]
//!                 [--advertise <host:port>] [--seed <n>] [--clock frozen|wall]
//!                 [--partitions <n>] [--no-auto-create] [--rebalance-wait <duration>]
//!
//! `TWIN_CONFIG` (a JSON object, as Ironframe passes it), then `LAWTWIN_SEED`,
//! `LAWTWIN_CLOCK` and `LAWTWIN_ADVERTISE`, give the same settings; options
//! on the command line win.

use lawtwin_kafka::Config;
use lawtwin_kafka::clock::parse_duration;
use std::process::ExitCode;

const USAGE: &str = "usage: lawtwin-kafka [--host <addr>] [--port <n>] [--admin-port <n>] [--advertise <host:port>]
                     [--seed <n>] [--clock frozen|wall] [--partitions <n>] [--no-auto-create]
                     [--rebalance-wait <duration>]";

/// Settings from the environment.
fn from_environment(config: &mut Config) -> Result<(), String> {
    if let Ok(text) = std::env::var("TWIN_CONFIG") {
        let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("TWIN_CONFIG is not JSON: {e}"))?;
        let obj = v.as_object().ok_or("TWIN_CONFIG is a JSON object")?;
        for (k, v) in obj {
            match (k.as_str(), v) {
                ("seed", serde_json::Value::Number(n)) => config.seed = n.as_u64().ok_or("TWIN_CONFIG seed is a whole number")?,
                ("clock", serde_json::Value::String(c)) => config.clock = c.parse()?,
                ("advertise" | "advertised", serde_json::Value::String(a)) => config.advertised = Some(a.clone()),
                ("partitions" | "defaultPartitions", serde_json::Value::Number(n)) => {
                    config.default_partitions = n.as_i64().ok_or("TWIN_CONFIG partitions is a whole number")? as i32
                }
                ("autoCreateTopics", serde_json::Value::Bool(b)) => config.auto_create_topics = *b,
                ("rebalanceWait", serde_json::Value::String(d)) => config.rebalance_wait_ms = parse_duration(d)? as u64,
                (other, _) => return Err(format!("TWIN_CONFIG has {other}, which the Kafka twin does not take")),
            }
        }
    }
    if let Ok(seed) = std::env::var("LAWTWIN_SEED") {
        config.seed = seed.parse().map_err(|_| "LAWTWIN_SEED is a whole number")?;
    }
    if let Ok(clock) = std::env::var("LAWTWIN_CLOCK") {
        config.clock = clock.parse()?;
    }
    if let Ok(a) = std::env::var("LAWTWIN_ADVERTISE") {
        config.advertised = Some(a);
    }
    Ok(())
}

/// The core's configuration, from the environment and the command line.
fn options(args: &[String]) -> Result<Config, String> {
    let mut config = Config::default();
    from_environment(&mut config)?;
    let (mut host, mut port, mut admin_port) = ("127.0.0.1".to_string(), 9092u16, 9644u16);
    let mut i = 0;
    while i < args.len() {
        let value = |i: usize| args.get(i + 1).cloned().ok_or_else(|| format!("{} needs a value", args[i]));
        match args[i].as_str() {
            "--port" => port = value(i)?.parse().map_err(|_| "--port is a port number")?,
            "--admin-port" => admin_port = value(i)?.parse().map_err(|_| "--admin-port is a port number")?,
            "--host" => host = value(i)?,
            "--advertise" => config.advertised = Some(value(i)?),
            "--seed" => config.seed = value(i)?.parse().map_err(|_| "--seed is a whole number")?,
            "--clock" => config.clock = value(i)?.parse()?,
            "--partitions" => config.default_partitions = value(i)?.parse().map_err(|_| "--partitions is a whole number")?,
            "--rebalance-wait" => config.rebalance_wait_ms = parse_duration(&value(i)?)? as u64,
            "--no-auto-create" => {
                config.auto_create_topics = false;
                i += 1;
                continue;
            }
            "--help" | "-h" => return Err(USAGE.into()),
            other => return Err(format!("lawtwin-kafka does not take {other}\n{USAGE}")),
        }
        i += 2;
    }
    config.kafka_listen = format!("{host}:{port}");
    config.admin_listen = format!("{host}:{admin_port}");
    Ok(config)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--version") {
        println!("lawtwin-kafka {} (rust core)", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    match options(&args).and_then(|c| lawtwin_kafka::run(c).map_err(|e| e.to_string())) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("lawtwin-kafka: {e}");
            ExitCode::FAILURE
        }
    }
}
