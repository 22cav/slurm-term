use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct SlurmTermConfig {
    pub monitor_poll_interval: f64,
    pub inspector_poll_interval: f64,
    pub hardware_poll_interval: f64,
    pub history_poll_interval: f64,
    pub subprocess_timeout: f64,
    pub history_window: String,
    pub gpu_monitor_enabled: bool,
    pub gpu_monitor_command: String,
    pub notify_on_complete: bool,
    pub notify_on_fail: bool,
}

impl Default for SlurmTermConfig {
    fn default() -> Self {
        Self {
            monitor_poll_interval: 3.0,
            inspector_poll_interval: 3.0,
            hardware_poll_interval: 30.0,
            history_poll_interval: 60.0,
            subprocess_timeout: 30.0,
            history_window: "now-7days".to_string(),
            gpu_monitor_enabled: false,
            gpu_monitor_command: "nvidia-smi".to_string(),
            notify_on_complete: false,
            notify_on_fail: false,
        }
    }
}

#[derive(Deserialize, Default)]
struct TomlConfig {
    poll: Option<TomlPoll>,
    general: Option<TomlGeneral>,
    gpu: Option<TomlGpu>,
    notifications: Option<TomlNotifications>,
}

#[derive(Deserialize, Default)]
struct TomlPoll {
    monitor: Option<f64>,
    inspector: Option<f64>,
    hardware: Option<f64>,
    history: Option<f64>,
}

#[derive(Deserialize, Default)]
struct TomlGeneral {
    subprocess_timeout: Option<f64>,
    history_window: Option<String>,
}

#[derive(Deserialize, Default)]
struct TomlGpu {
    enabled: Option<bool>,
    command: Option<String>,
}

#[derive(Deserialize, Default)]
struct TomlNotifications {
    on_complete: Option<bool>,
    on_fail: Option<bool>,
}

fn config_path() -> PathBuf {
    if let Ok(p) = std::env::var("SLURMTERM_CONFIG") {
        return PathBuf::from(p);
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("slurmterm")
        .join("config.toml")
}

pub fn load_config(path: Option<&str>) -> SlurmTermConfig {
    let mut cfg = SlurmTermConfig::default();
    let p = path
        .map(PathBuf::from)
        .unwrap_or_else(config_path);

    let content = match std::fs::read_to_string(&p) {
        Ok(c) => c,
        Err(_) => return cfg,
    };

    let toml_cfg: TomlConfig = match toml::from_str(&content) {
        Ok(c) => c,
        Err(_) => return cfg,
    };

    if let Some(poll) = toml_cfg.poll {
        if let Some(v) = poll.monitor {
            cfg.monitor_poll_interval = v;
        }
        if let Some(v) = poll.inspector {
            cfg.inspector_poll_interval = v;
        }
        if let Some(v) = poll.hardware {
            cfg.hardware_poll_interval = v;
        }
        if let Some(v) = poll.history {
            cfg.history_poll_interval = v;
        }
    }

    if let Some(general) = toml_cfg.general {
        if let Some(v) = general.subprocess_timeout {
            cfg.subprocess_timeout = v;
        }
        if let Some(v) = general.history_window {
            cfg.history_window = v;
        }
    }

    if let Some(gpu) = toml_cfg.gpu {
        if let Some(v) = gpu.enabled {
            cfg.gpu_monitor_enabled = v;
        }
        if let Some(v) = gpu.command {
            cfg.gpu_monitor_command = v;
        }
    }

    if let Some(notif) = toml_cfg.notifications {
        if let Some(v) = notif.on_complete {
            cfg.notify_on_complete = v;
        }
        if let Some(v) = notif.on_fail {
            cfg.notify_on_fail = v;
        }
    }

    cfg
}
