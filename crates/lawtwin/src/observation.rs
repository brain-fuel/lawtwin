//! Launch the observation core without interpreting telemetry in the CLI host.
use std::path::PathBuf;

pub const USAGE: &str = "  lawtwin observe --config <receiver.json> [--listen 127.0.0.1:4318] [--admin-listen 127.0.0.1:9646]";

pub fn run(command: &str, args: &[String]) -> Option<crate::control::Outcome> {
    if command != "observe" {
        return None;
    }
    let core = std::env::var_os("LAWTWIN_OBSERVATION_CORE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|parent| parent.join("lawtwin-observation")))
                .unwrap_or_else(|| PathBuf::from("lawtwin-observation"))
        });
    let result = std::process::Command::new(&core).args(args).status();
    Some(match result {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err((
            String::new(),
            status
                .code()
                .and_then(|c| u8::try_from(c).ok())
                .unwrap_or(1),
        )),
        Err(error) => Err((
            format!(
                "cannot launch {}: {error}; build with make build-observation",
                core.display()
            ),
            1,
        )),
    })
}
