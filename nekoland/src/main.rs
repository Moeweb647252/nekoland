use std::process::ExitCode;

use nekoland::build_default_app;
use tracing_subscriber::EnvFilter;

/// Binary entry point for the compositor executable.
fn main() -> ExitCode {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,nekoland=debug"));

    tracing_subscriber::fmt().with_env_filter(filter).compact().init();

    // Build the protocol registry before constructing the app so startup logging can surface the
    // effective protocol set early.
    let supported_protocols = nekoland_protocol::supported_protocols();
    tracing::info!(count = supported_protocols.len(), "protocol registry initialized");
    let mut app = build_default_app();

    match app.run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("nekoland failed: {error}");
            ExitCode::FAILURE
        }
    }
}
