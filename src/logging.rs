use crate::config::LoggingConfig;
use std::fs::{create_dir_all, File};
use std::io::BufWriter;
use chrono::Local;
use tracing_subscriber::{fmt, EnvFilter};

pub fn setup_logging(cfg: &LoggingConfig) {
    let level = cfg
        .level
        .parse::<tracing::Level>()
        .unwrap_or(tracing::Level::INFO);

    let env_filter = EnvFilter::from_default_env().add_directive(level.into());

    if cfg.save_logs {
        if let Some(dir) = std::path::Path::new(&cfg.log_file_path).parent() {
            create_dir_all(dir).expect("Could not create log directory");
        }

        let log_file_path = if cfg.rewrite_last_logs {
            cfg.log_file_path.clone()
        } else {
            let timestamp = Local::now().format("%Y-%m-%d_%H-%M-%S");
            format!(
                "{}-{}.log",
                cfg.log_file_path.trim_end_matches(".log"),
                timestamp
            )
        };

        let file = File::create(&log_file_path).expect("Error while creating file");

        let writer = move || BufWriter::new(file.try_clone().expect("Could not clone file"));

        fmt()
            .with_env_filter(env_filter)
            .with_ansi(false)
            .with_writer(writer)
            .init();
    } else {
        fmt()
            .with_env_filter(env_filter)
            .init();
    }
}
