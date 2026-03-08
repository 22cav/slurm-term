use regex::Regex;

/// Parse a Slurm-style time string and return total seconds.
/// Formats: SS, MM:SS, HH:MM:SS, D-HH:MM:SS
pub fn parse_time(time_str: &str) -> Result<i64, String> {
    let time_str = time_str.trim();
    if time_str.is_empty() {
        return Err("Empty time string".into());
    }

    let (days, rest) = if time_str.contains('-') {
        let mut parts = time_str.splitn(2, '-');
        let d: i64 = parts
            .next()
            .unwrap()
            .parse()
            .map_err(|_| format!("Invalid day part in time: {time_str:?}"))?;
        (d, parts.next().unwrap_or(""))
    } else {
        (0i64, time_str)
    };

    let parts: Vec<&str> = rest.split(':').collect();
    let secs = match parts.len() {
        1 => parts[0]
            .parse::<i64>()
            .map_err(|_| format!("Invalid time: {time_str:?}"))?,
        2 => {
            let m: i64 = parts[0]
                .parse()
                .map_err(|_| format!("Invalid time: {time_str:?}"))?;
            let s: i64 = parts[1]
                .parse()
                .map_err(|_| format!("Invalid time: {time_str:?}"))?;
            m * 60 + s
        }
        3 => {
            let h: i64 = parts[0]
                .parse()
                .map_err(|_| format!("Invalid time: {time_str:?}"))?;
            let m: i64 = parts[1]
                .parse()
                .map_err(|_| format!("Invalid time: {time_str:?}"))?;
            let s: i64 = parts[2]
                .parse()
                .map_err(|_| format!("Invalid time: {time_str:?}"))?;
            h * 3600 + m * 60 + s
        }
        _ => return Err(format!("Invalid time format: {time_str:?}")),
    };

    Ok(days * 86400 + secs)
}

/// Parse a memory string like "4G" into megabytes.
pub fn parse_memory(mem_str: &str) -> Result<i64, String> {
    let re = Regex::new(r"(?i)^\s*(\d+)\s*([KMGT]?)B?\s*$").unwrap();
    let caps = re
        .captures(mem_str)
        .ok_or_else(|| format!("Invalid memory format: {mem_str:?}"))?;
    let value: i64 = caps[1]
        .parse()
        .map_err(|_| format!("Invalid memory value: {mem_str:?}"))?;
    let suffix = caps.get(2).map(|m| m.as_str().to_uppercase()).unwrap_or_default();
    let mb = match suffix.as_str() {
        "" | "M" => value,
        "K" => std::cmp::max(1, value / 1024),
        "G" => value * 1024,
        "T" => value * 1024 * 1024,
        _ => value,
    };
    Ok(std::cmp::max(1, mb))
}

/// Validate a Slurm job name.
pub fn validate_job_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Job name must not be empty".into());
    }
    if name.len() > 200 {
        return Err("Job name too long (max 200 chars)".into());
    }
    let re = Regex::new(r"^[a-zA-Z0-9][a-zA-Z0-9_.@:+/-]*$").unwrap();
    if !re.is_match(name) {
        return Err(
            "Job name contains invalid characters (use letters, digits, dots, underscores, @, colons, +, /, hyphens)"
                .into(),
        );
    }
    Ok(name.to_string())
}

/// Return a color name for a Slurm job state.
pub fn state_color(state: &str) -> ratatui::style::Color {
    use crate::theme;
    match state.to_uppercase().as_str() {
        "RUNNING" | "COMPLETING" => theme::GREEN,
        "COMPLETED" => theme::TEAL,
        "PENDING" => theme::YELLOW,
        "SUSPENDED" => theme::PEACH,
        "FAILED" | "TIMEOUT" | "NODE_FAIL" | "OUT_OF_MEMORY" => theme::RED,
        "CANCELLED" => theme::MAUVE,
        "PREEMPTED" => theme::LAVENDER,
        _ => theme::DIM,
    }
}

/// Parse a MaxRSS string to a percentage of total memory.
pub fn parse_rss_to_pct(rss_str: &str, total_mb: i64) -> f64 {
    if rss_str.is_empty() || total_mb <= 0 {
        return 0.0;
    }
    let rss_str = rss_str.trim();
    let mb = if let Some(s) = rss_str.strip_suffix('G') {
        s.parse::<f64>().unwrap_or(0.0) * 1024.0
    } else if let Some(s) = rss_str.strip_suffix('M') {
        s.parse::<f64>().unwrap_or(0.0)
    } else if let Some(s) = rss_str.strip_suffix('K') {
        s.parse::<f64>().unwrap_or(0.0) / 1024.0
    } else {
        rss_str.parse::<f64>().unwrap_or(0.0) / (1024.0 * 1024.0)
    };
    (mb / total_mb as f64 * 100.0).min(100.0)
}

/// Parse a Slurm duration string to seconds.
pub fn parse_duration_to_seconds(duration: &str) -> f64 {
    if duration.is_empty() {
        return 0.0;
    }
    let duration = duration.trim();
    let (days, rest) = if duration.contains('-') {
        let mut parts = duration.splitn(2, '-');
        let d = parts
            .next()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);
        (d, parts.next().unwrap_or(""))
    } else {
        (0.0, duration)
    };

    let parts: Vec<&str> = rest.split(':').collect();
    let secs = match parts.len() {
        3 => {
            let h = parts[0].parse::<f64>().unwrap_or(0.0);
            let m = parts[1].parse::<f64>().unwrap_or(0.0);
            let s = parts[2].parse::<f64>().unwrap_or(0.0);
            h * 3600.0 + m * 60.0 + s
        }
        2 => {
            let m = parts[0].parse::<f64>().unwrap_or(0.0);
            let s = parts[1].parse::<f64>().unwrap_or(0.0);
            m * 60.0 + s
        }
        _ => 0.0,
    };

    days * 86400.0 + secs
}

/// Parse an AveCPU string to a percentage.
pub fn parse_cpu_pct(cpu_str: &str, elapsed_seconds: f64) -> f64 {
    if cpu_str.is_empty() {
        return 0.0;
    }
    let cpu_str = cpu_str.trim();
    if let Some(s) = cpu_str.strip_suffix('%') {
        return s.parse::<f64>().unwrap_or(0.0).min(100.0);
    }
    let cpu_seconds = parse_duration_to_seconds(cpu_str);
    if cpu_seconds > 0.0 && elapsed_seconds > 0.0 {
        (cpu_seconds / elapsed_seconds * 100.0).min(100.0)
    } else {
        0.0
    }
}
