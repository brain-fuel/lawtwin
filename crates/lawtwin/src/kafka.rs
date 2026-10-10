//! The Kafka twin's fidelity commands: `conform kafka` runs the core
//! conformance kit's client scenarios against real Kafka, `record kafka`
//! records a real broker's traffic through a proxy, `replay kafka` replays
//! a recorded trace through a core, and `peer kafka produce` and
//! `peer kafka consume` are simulated producers and consumers. They are run
//! by the program `lawtwin-kafka-fidelity`, found beside this command, on
//! the path, or at `LAWTWIN_KAFKA_FIDELITY` (docs/reference/kafka-fidelity.md).

use std::process::Command;

/// The usage of the Kafka fidelity commands.
pub const USAGE: &str = "  lawtwin conform kafka --real <host:port> [--real-name <name>] [--client <name>]...
          [--scenario <name>]... [--out <directory>] [--twin <command | image:<image>> | --no-twin]
          [--release <version>] [--frames]
  lawtwin record kafka --upstream <host:port> [--listen <[host:]port>] [--admin-port <n>]
          [--advertise <host:port>] [--out <prefix>] [--clock wall|frozen] [--quiet]
  lawtwin replay kafka <trace> --core <command | image:<image> | rust> [--report <file>]
  lawtwin peer kafka produce --bootstrap <host:port> --topic <topic> [--count <n> | --duration <d>]
          [--rate <n>] [--seed <n>] [--keys <n> | --no-keys] [--header <name>=<value>]...
          [--partition <n>] [--value-type <type> --spec <lawspec>] [--format json|lawspec]
          [--transactional-id <id> [--txn-size <n>]] [--idempotent] [--fault <fault>[:<p>]]...
          [--late-by <duration>] [--log <file>] [--report <file>]
  lawtwin peer kafka consume --bootstrap <host:port> --topic <topic> [--group <group>]
          [--isolation read_committed|read_uncommitted] [--value-type <type> --spec <lawspec>]
          [--format json|lawspec] [--expect <n>] [--duration <d>] [--idle <d>]
          [--from earliest|latest] [--no-commit] [--log <file>] [--report <file>]";

/// A command's outcome: success, or failure with a message and exit code.
pub type Outcome = Result<(), (String, u8)>;

/// The fidelity program: `LAWTWIN_KAFKA_FIDELITY`, or beside this command,
/// or on the path.
fn program() -> String {
    if let Ok(p) = std::env::var("LAWTWIN_KAFKA_FIDELITY") {
        return p;
    }
    let name = "lawtwin-kafka-fidelity";
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join(name)))
        .filter(|p| p.exists())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| name.to_string())
}

fn exec(sub: &str, args: &[String]) -> Outcome {
    let p = program();
    let status = Command::new(&p)
        .arg(sub)
        .args(args)
        .status()
        .map_err(|e| (format!("cannot run {p}: {e}; build it with make build"), 1))?;
    match status.code() {
        Some(0) => Ok(()),
        Some(code) => Err((String::new(), code.clamp(1, 255) as u8)),
        None => Err(("the Kafka fidelity tool was stopped".into(), 1)),
    }
}

/// Runs a Kafka fidelity command; `None` when it is not one.
pub fn run(command: &str, rest: &[String]) -> Option<Outcome> {
    let kafka = rest.first().map(String::as_str) == Some("kafka");
    Some(match command {
        "conform" | "record" | "replay" if kafka => exec(command, &rest[1..]),
        "peer" if kafka => exec("peer", &rest[1..]),
        // `lawtwin replay <trace> --core <core>`: a Kafka trace through a core.
        "replay" if rest.iter().any(|a| a == "--core") => exec("replay", rest),
        _ => return None,
    })
}
