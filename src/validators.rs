use std::sync::LazyLock;

use regex::Regex;

/// A memory size like "4G", "512M", "1024".
static MEMORY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\s*(\d+)\s*([KMGT]?)B?\s*$").unwrap());
/// A Slurm job name.
static JOB_NAME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9][a-zA-Z0-9_.@:+/-]*$").unwrap());

/// Parse a Slurm `--time` string and return total seconds.
///
/// Slurm accepts: "minutes", "minutes:seconds", "hours:minutes:seconds",
/// "days-hours", "days-hours:minutes" and "days-hours:minutes:seconds".
/// In particular a bare integer means MINUTES (sbatch(1)), and with a day
/// prefix the first field means HOURS.
pub fn parse_time(time_str: &str) -> Result<i64, String> {
    let time_str = time_str.trim();
    if time_str.is_empty() {
        return Err("Empty time string".into());
    }

    let (days, rest) = match time_str.split_once('-') {
        Some((d, rest)) => {
            let d: i64 = d
                .parse()
                .map_err(|_| format!("Invalid day part in time: {time_str:?}"))?;
            (Some(d), rest)
        }
        None => (None, time_str),
    };

    let nums = rest
        .split(':')
        .map(|p| {
            p.parse::<i64>()
                .map_err(|_| format!("Invalid time: {time_str:?}"))
        })
        .collect::<Result<Vec<i64>, String>>()?;

    let secs = match (days, nums.as_slice()) {
        (None, [m]) => m * 60,
        (None, [m, s]) => m * 60 + s,
        (None, [h, m, s]) => h * 3600 + m * 60 + s,
        (Some(_), [h]) => h * 3600,
        (Some(_), [h, m]) => h * 3600 + m * 60,
        (Some(_), [h, m, s]) => h * 3600 + m * 60 + s,
        _ => return Err(format!("Invalid time format: {time_str:?}")),
    };

    Ok(days.unwrap_or(0) * 86400 + secs)
}

/// Format a duration in seconds as Slurm's HH:MM:SS.
pub fn format_hms(total_secs: i64) -> String {
    let h = total_secs / 3600;
    let m = (total_secs % 3600) / 60;
    let s = total_secs % 60;
    format!("{h:02}:{m:02}:{s:02}")
}

/// Parse a memory string like "4G" into megabytes.
pub fn parse_memory(mem_str: &str) -> Result<i64, String> {
    let caps = MEMORY_RE
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
    if !JOB_NAME_RE.is_match(name) {
        return Err(
            "Job name contains invalid characters (use letters, digits, dots, underscores, @, colons, +, /, hyphens)"
                .into(),
        );
    }
    Ok(name.to_string())
}

/// Return a color for a Slurm job state (covers the states listed in the
/// squeue/sacct JOB STATE CODES sections).
pub fn state_color(state: &str) -> ratatui::style::Color {
    use crate::theme;
    // sacct appends context to some states, e.g. "CANCELLED by 1000" —
    // classify on the first word.
    let state = state.to_uppercase();
    match state.split_whitespace().next().unwrap_or("") {
        "RUNNING" | "COMPLETING" => theme::GREEN,
        "COMPLETED" => theme::TEAL,
        "PENDING" | "REQUEUED" | "CONFIGURING" | "STAGE_OUT" => theme::YELLOW,
        "SUSPENDED" | "RESIZING" | "SIGNALING" | "STOPPED" => theme::PEACH,
        "FAILED" | "TIMEOUT" | "NODE_FAIL" | "OUT_OF_MEMORY" | "BOOT_FAIL"
        | "DEADLINE" | "REVOKED" => theme::RED,
        "CANCELLED" | "SPECIAL_EXIT" => theme::MAUVE,
        "PREEMPTED" => theme::LAVENDER,
        _ => theme::DIM,
    }
}

/// Color for a node state as reported by `scontrol show node`
/// ("IDLE", "MIXED+DRAIN") or `sinfo %T` ("idle", "draining", "allocated*").
/// Case-insensitive substring match so base-state+flag combos are covered;
/// problem flags win over the base state (an "IDLE+DRAIN" node is draining).
pub fn node_state_color(state: &str) -> ratatui::style::Color {
    use crate::theme;
    let s = state.to_uppercase();
    if ["DOWN", "DRAIN", "FAIL", "ERROR", "INVAL", "NOT_RESPONDING"]
        .iter()
        .any(|p| s.contains(p))
    {
        theme::RED
    } else if ["MIXED", "ALLOC", "COMPLETING"].iter().any(|p| s.contains(p)) {
        theme::YELLOW
    } else if s.contains("IDLE") {
        theme::GREEN
    } else {
        theme::DIM
    }
}

/// Convert a MiB count string, as reported by `sinfo %m` and scontrol's
/// RealMemory/FreeMem (e.g. "64000" or "64000+"), to a GiB string
/// ("62.5" / "62.5+"). Non-numeric input is returned unchanged.
pub fn mib_to_gib(mib_str: &str) -> String {
    let s = mib_str.trim();
    let (num, plus) = match s.strip_suffix('+') {
        Some(n) => (n, "+"),
        None => (s, ""),
    };
    match num.parse::<f64>() {
        Ok(mib) => format!("{:.1}{}", mib / 1024.0, plus),
        Err(_) => mib_str.to_string(),
    }
}

/// Parse a Slurm RSS size string (sacct/sstat MaxRSS, e.g. "123456K",
/// "1.37M", "2G") into kibibytes. A bare number is treated as bytes.
/// Returns None for empty or unparseable input.
pub fn parse_rss_kb(rss_str: &str) -> Option<f64> {
    let s = rss_str.trim();
    if s.is_empty() {
        return None;
    }
    let (num, mult) = if let Some(n) = s.strip_suffix(['T', 't']) {
        (n, 1024.0 * 1024.0 * 1024.0)
    } else if let Some(n) = s.strip_suffix(['G', 'g']) {
        (n, 1024.0 * 1024.0)
    } else if let Some(n) = s.strip_suffix(['M', 'm']) {
        (n, 1024.0)
    } else if let Some(n) = s.strip_suffix(['K', 'k']) {
        (n, 1.0)
    } else {
        (s, 1.0 / 1024.0)
    };
    num.trim().parse::<f64>().ok().map(|v| v * mult)
}

/// Parse a MaxRSS string to a percentage of total memory.
pub fn parse_rss_to_pct(rss_str: &str, total_mb: i64) -> f64 {
    if total_mb <= 0 {
        return 0.0;
    }
    let Some(kb) = parse_rss_kb(rss_str) else {
        return 0.0;
    };
    (kb / 1024.0 / total_mb as f64 * 100.0).min(100.0)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme;

    #[test]
    fn parse_time_bare_integer_is_minutes() {
        // sbatch(1): "--time=<minutes>"
        assert_eq!(parse_time("90").unwrap(), 90 * 60);
        assert_eq!(parse_time("1").unwrap(), 60);
    }

    #[test]
    fn parse_time_colon_forms() {
        assert_eq!(parse_time("90:30").unwrap(), 90 * 60 + 30);
        assert_eq!(parse_time("1:30:00").unwrap(), 5400);
    }

    #[test]
    fn parse_time_day_forms() {
        assert_eq!(parse_time("2-12").unwrap(), 2 * 86400 + 12 * 3600);
        assert_eq!(parse_time("2-12:30").unwrap(), 2 * 86400 + 12 * 3600 + 30 * 60);
        assert_eq!(
            parse_time("2-12:30:15").unwrap(),
            2 * 86400 + 12 * 3600 + 30 * 60 + 15
        );
    }

    #[test]
    fn parse_time_rejects_garbage() {
        assert!(parse_time("").is_err());
        assert!(parse_time("abc").is_err());
        assert!(parse_time("1:2:3:4").is_err());
        assert!(parse_time("x-12").is_err());
        assert!(parse_time("2-").is_err());
    }

    #[test]
    fn state_color_covers_documented_states() {
        assert_eq!(state_color("RUNNING"), theme::GREEN);
        assert_eq!(state_color("COMPLETED"), theme::TEAL);
        assert_eq!(state_color("BOOT_FAIL"), theme::RED);
        assert_eq!(state_color("DEADLINE"), theme::RED);
        assert_eq!(state_color("OUT_OF_MEMORY"), theme::RED);
        assert_eq!(state_color("REVOKED"), theme::RED);
        assert_eq!(state_color("REQUEUED"), theme::YELLOW);
        assert_eq!(state_color("CONFIGURING"), theme::YELLOW);
        assert_eq!(state_color("STAGE_OUT"), theme::YELLOW);
        assert_eq!(state_color("RESIZING"), theme::PEACH);
        assert_eq!(state_color("SIGNALING"), theme::PEACH);
        assert_eq!(state_color("SPECIAL_EXIT"), theme::MAUVE);
        assert_eq!(state_color("PREEMPTED"), theme::LAVENDER);
    }

    #[test]
    fn state_color_matches_first_word() {
        // sacct emits e.g. "CANCELLED by 1000"
        assert_eq!(state_color("CANCELLED by 1000"), theme::MAUVE);
        assert_eq!(state_color("cancelled by 1000"), theme::MAUVE);
    }

    #[test]
    fn node_state_color_is_case_insensitive() {
        // scontrol reports uppercase, sinfo %T lowercase
        assert_eq!(node_state_color("IDLE"), theme::GREEN);
        assert_eq!(node_state_color("idle"), theme::GREEN);
        assert_eq!(node_state_color("MIXED"), theme::YELLOW);
        assert_eq!(node_state_color("allocated*"), theme::YELLOW);
        assert_eq!(node_state_color("completing"), theme::YELLOW);
        assert_eq!(node_state_color("down~"), theme::RED);
        assert_eq!(node_state_color("draining"), theme::RED);
        assert_eq!(node_state_color("unknown"), theme::DIM);
    }

    #[test]
    fn node_state_color_flags_win_over_base_state() {
        assert_eq!(node_state_color("IDLE+DRAIN"), theme::RED);
        assert_eq!(node_state_color("MIXED+NOT_RESPONDING"), theme::RED);
    }

    #[test]
    fn mib_to_gib_converts_and_preserves_suffix() {
        assert_eq!(mib_to_gib("64000"), "62.5");
        assert_eq!(mib_to_gib("64000+"), "62.5+");
        assert_eq!(mib_to_gib("1024"), "1.0");
        assert_eq!(mib_to_gib("N/A"), "N/A");
        assert_eq!(mib_to_gib("(null)"), "(null)");
    }

    #[test]
    fn parse_rss_kb_handles_slurm_suffixes() {
        assert_eq!(parse_rss_kb("123456K"), Some(123456.0));
        assert_eq!(parse_rss_kb("1.5M"), Some(1536.0));
        assert_eq!(parse_rss_kb("2G"), Some(2.0 * 1024.0 * 1024.0));
        assert_eq!(parse_rss_kb("2048"), Some(2.0)); // bare = bytes
        assert_eq!(parse_rss_kb(""), None);
        assert_eq!(parse_rss_kb("abc"), None);
    }

    #[test]
    fn parse_rss_to_pct_scales_against_total() {
        // 512 MiB of 1024 MiB = 50%
        assert_eq!(parse_rss_to_pct("524288K", 1024), 50.0);
        assert_eq!(parse_rss_to_pct("", 1024), 0.0);
        assert_eq!(parse_rss_to_pct("1G", 0), 0.0);
    }
}
