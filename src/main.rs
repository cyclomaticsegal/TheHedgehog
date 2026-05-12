mod ai;
mod analysis;
mod app;
mod folds;
mod help;
mod knowledge;
mod models;
mod providers;
mod dag;
mod eval;
mod obsidian;
mod storage;

pub(crate) const USER_AGENT: &str = "the-hedgehog/0.1.0-preview";

use eframe::egui;
use tracing_subscriber::prelude::*;

fn main() -> eframe::Result<()> {
    // Daily-rotating log file under ./data/logs/. Kept for forensic
    // triage after incidents — ADR 0020. The non-blocking guard must
    // stay alive for the process lifetime; we bind it to `_guard` so it
    // isn't dropped until main returns.
    let mut log_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    log_dir.push("data");
    log_dir.push("logs");
    let _guard = if std::fs::create_dir_all(&log_dir).is_ok() {
        prune_old_logs(&log_dir, 3);
        let file_appender = tracing_appender::rolling::daily(&log_dir, "hedgehog.log");
        let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
        let file_layer = tracing_subscriber::fmt::layer()
            .with_writer(non_blocking)
            .with_ansi(false)
            .with_target(true);
        let stderr_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stderr);
        let filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
        tracing_subscriber::registry()
            .with(filter)
            .with(file_layer)
            .with(stderr_layer)
            .init();
        Some(guard)
    } else {
        // Fall back to stderr-only if we can't create the log dir.
        let stderr_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stderr);
        let filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
        tracing_subscriber::registry()
            .with(filter)
            .with(stderr_layer)
            .init();
        None
    };
    tracing::info!("hedgehog starting");

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_maximized(true),
        ..Default::default()
    };

    eframe::run_native(
        "The Hedgehog",
        native_options,
        Box::new(|cc| Ok(Box::new(app::DashboardApp::new(cc)))),
    )
}

/// Delete rotated log files older than `keep_days` from `log_dir`.
/// Best-effort: errors are silently ignored — this runs before the
/// tracing subscriber is wired up, so there's nowhere to log failures.
fn prune_old_logs(log_dir: &std::path::Path, keep_days: u64) {
    let cutoff = match std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(keep_days * 24 * 60 * 60))
    {
        Some(t) => t,
        None => return,
    };
    let Ok(entries) = std::fs::read_dir(log_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if !name.starts_with("hedgehog.log") {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(mtime) = meta.modified() else { continue };
        if mtime < cutoff {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}
