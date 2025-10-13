//! Minimal configuration placeholder for SiTerm.
//!
//! The original Ratatui example loaded configuration from multiple files. For
//! SiTerm we will drive behavior from command-line parameters instead. This
//! module keeps a lightweight `Config` type so components can request shared
//! settings, and provides placeholder helpers for filesystem paths that other
//! modules currently display. As we add real CLI options (e.g. default port,
//! preferred baud rate, theme), wire them into `Config::from_cli`.

use std::{env, path::PathBuf};

#[derive(Clone, Debug, Default)]
pub struct Config {
    // Example future fields:
    // pub default_port: Option<String>,
    // pub default_baud: Option<u32>,
    // pub theme: Theme,
}

impl Config {
    /// Construct a placeholder configuration.
    ///
    /// ```ignore
    /// // Pseudo-code for upcoming CLI integration:
    /// pub fn from_cli(args: &Cli) -> Self {
    ///     let mut cfg = Config::default();
    ///     cfg.default_port = args.port.clone();
    ///     cfg.default_baud = args.baud_rate;
    ///     cfg
    /// }
    /// ```
    #[allow(unused)]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Return the directory used for local data (logs, session caches, etc.).
pub fn get_data_dir() -> PathBuf {
    project_directory()
        .map(|dirs| dirs.data_local_dir().to_path_buf())
        .unwrap_or_else(default_data_dir)
}

/// Return the directory used for local configuration mirrors (currently unused).
pub fn get_config_dir() -> PathBuf {
    project_directory()
        .map(|dirs| dirs.config_local_dir().to_path_buf())
        .unwrap_or_else(default_config_dir)
}

fn project_directory() -> Option<directories::ProjectDirs> {
    directories::ProjectDirs::from("com", "kdheepak", env!("CARGO_PKG_NAME"))
}

fn default_data_dir() -> PathBuf {
    env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".data")
}

fn default_config_dir() -> PathBuf {
    env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".config")
}
