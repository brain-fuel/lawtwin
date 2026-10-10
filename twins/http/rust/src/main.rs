//! `lawtwin-http`: the HTTP twin's Rust core as a program.
//!
//! Every core of the HTTP twin, whatever its language, takes the same
//! options and environment, serves one imported contract's API on its port
//! and the control plane on an admin port (docs/reference/http-core.md):
//!
//!   lawtwin-http --contract <name> [--host <addr>] [--port <n>] [--admin-port <n>]
//!                [--advertise <host:port>] [--seed <n>] [--clock frozen|wall]
//!
//! `TWIN_CONFIG` (a JSON object, as Ironframe passes it), then
//! `LAWTWIN_CONTRACT`, `LAWTWIN_SEED`, `LAWTWIN_CLOCK` and
//! `LAWTWIN_ADVERTISE`, give the same settings; options on the command line
//! win. ref:DEC-core-per-target

use lawtwin_http::Config;
use std::process::ExitCode;

const USAGE: &str = "usage: lawtwin-http --contract <name> [--host <addr>] [--port <n>] [--admin-port <n>]
                    [--advertise <host:port>] [--seed <n>] [--clock frozen|wall]
       lawtwin-http --contracts";

/// Settings from the environment.
fn from_environment(config: &mut Config) -> Result<(), String> {
    if let Ok(text) = std::env::var("TWIN_CONFIG") {
        let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("TWIN_CONFIG is not JSON: {e}"))?;
        let obj = v.as_object().ok_or("TWIN_CONFIG is a JSON object")?;
        for (k, v) in obj {
            match (k.as_str(), v) {
                ("contract", serde_json::Value::String(c)) => config.contract = c.clone(),
                ("seed", serde_json::Value::Number(n)) => config.seed = n.as_u64().ok_or("TWIN_CONFIG seed is a whole number")?,
                ("clock", serde_json::Value::String(c)) => config.clock = c.parse()?,
                ("advertise" | "advertised", serde_json::Value::String(a)) => config.advertised = Some(a.clone()),
                (other, _) => return Err(format!("TWIN_CONFIG has {other}, which the HTTP twin does not take")),
            }
        }
    }
    if let Ok(c) = std::env::var("LAWTWIN_CONTRACT") {
        config.contract = c;
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
    if let Ok(m) = std::env::var("LAWTWIN_MUTANT") {
        config.mutant = Some(m);
    }
    Ok(())
}

/// The core's configuration, from the environment and the command line.
fn options(args: &[String]) -> Result<Config, String> {
    let mut config = Config::default();
    from_environment(&mut config)?;
    let (mut host, mut port, mut admin_port) = ("127.0.0.1".to_string(), 8080u16, 9644u16);
    let mut i = 0;
    while i < args.len() {
        let value = |i: usize| args.get(i + 1).cloned().ok_or_else(|| format!("{} needs a value", args[i]));
        match args[i].as_str() {
            "--contract" => config.contract = value(i)?,
            "--port" => port = value(i)?.parse().map_err(|_| "--port is a port number")?,
            "--admin-port" => admin_port = value(i)?.parse().map_err(|_| "--admin-port is a port number")?,
            "--host" => host = value(i)?,
            "--advertise" => config.advertised = Some(value(i)?),
            "--seed" => config.seed = value(i)?.parse().map_err(|_| "--seed is a whole number")?,
            "--clock" => config.clock = value(i)?.parse()?,
            // A deliberate defect, for showing that conformance and replay catch it.
            "--mutant" => config.mutant = Some(value(i)?),
            "--help" | "-h" => return Err(USAGE.into()),
            other => return Err(format!("lawtwin-http does not take {other}\n{USAGE}")),
        }
        i += 2;
    }
    if config.contract.is_empty() {
        let all = lawtwin_http::contracts::registry();
        if all.len() == 1 {
            config.contract = all[0].name.to_string();
        } else {
            return Err(format!(
                "name the contract to serve with --contract, LAWTWIN_CONTRACT or TWIN_CONFIG: one of {}\n{USAGE}",
                all.iter().map(|c| c.name).collect::<Vec<_>>().join(", ")
            ));
        }
    }
    config.listen = format!("{host}:{port}");
    config.admin_listen = format!("{host}:{admin_port}");
    Ok(config)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version") => {
            println!("lawtwin-http {} (rust core)", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Some("--contracts") => {
            for c in lawtwin_http::contracts::registry() {
                println!("{}\t{} {}", c.name, c.package, c.version);
            }
            return ExitCode::SUCCESS;
        }
        _ => {}
    }
    match options(&args).and_then(|c| lawtwin_http::run(c).map_err(|e| e.to_string())) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("lawtwin-http: {e}");
            ExitCode::FAILURE
        }
    }
}
