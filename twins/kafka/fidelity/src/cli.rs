//! Reading the command line of the fidelity tools.

/// The usage of every command.
pub const USAGE: &str = "usage:
  lawtwin record kafka --upstream <host:port> [--listen <[host:]port>] [--admin-port <n>]
          [--advertise <host:port>] [--out <prefix>] [--clock wall|frozen] [--quiet]
  lawtwin replay <trace> --core <command | image:<image>> [--report <file>]
  lawtwin conform kafka --real <host:port> [--real-name <name>] [--client <name>]...
          [--scenario <name>]... [--out <directory>] [--twin <command | image:<image>> | --no-twin]
          [--release <version>] [--frames]
  lawtwin peer kafka produce --bootstrap <host:port> --topic <topic> [options]
  lawtwin peer kafka consume --bootstrap <host:port> --topic <topic> --group <group> [options]";

/// The options of a command, by name, with the bare words in order.
#[derive(Default, Debug)]
pub struct Args {
    /// Bare words.
    pub words: Vec<String>,
    /// `--name value` pairs, in order; a flag has an empty value.
    pub options: Vec<(String, String)>,
}

impl Args {
    /// Reads arguments; the names in `flags` take no value.
    pub fn parse(args: &[String], flags: &[&str]) -> Self {
        let mut out = Args::default();
        let mut i = 0;
        while i < args.len() {
            let a = &args[i];
            if let Some(name) = a.strip_prefix("--") {
                if let Some((n, v)) = name.split_once('=') {
                    out.options.push((n.to_string(), v.to_string()));
                } else if flags.contains(&name) || i + 1 >= args.len() {
                    out.options.push((name.to_string(), String::new()));
                } else {
                    out.options.push((name.to_string(), args[i + 1].clone()));
                    i += 1;
                }
            } else {
                out.words.push(a.clone());
            }
            i += 1;
        }
        out
    }

    /// The last value of an option.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.options.iter().rev().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    /// Every value of an option, in order.
    pub fn all(&self, name: &str) -> Vec<String> {
        self.options.iter().filter(|(n, _)| n == name).map(|(_, v)| v.clone()).collect()
    }

    /// Whether an option is given.
    pub fn has(&self, name: &str) -> bool {
        self.options.iter().any(|(n, _)| n == name)
    }

    /// An option parsed, or its default.
    pub fn parsed<T: std::str::FromStr>(&self, name: &str, default: T) -> Result<T, String> {
        match self.get(name) {
            None => Ok(default),
            Some(v) => v.parse().map_err(|_| format!("--{name} does not take {v}")),
        }
    }

    /// Fails on an option not in `names`.
    pub fn known(&self, names: &[&str]) -> Result<(), String> {
        match self.options.iter().find(|(n, _)| !names.contains(&n.as_str())) {
            Some((n, _)) => Err(format!("--{n} is not an option here\n{USAGE}")),
            None => Ok(()),
        }
    }
}
