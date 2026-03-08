use std::collections::HashMap;
use std::path::PathBuf;

use regex::Regex;

fn templates_dir() -> PathBuf {
    if let Ok(p) = std::env::var("SLURMTERM_TEMPLATES_DIR") {
        return PathBuf::from(p);
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("slurmterm")
        .join("templates")
}

fn sanitize_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Template name must not be empty".into());
    }
    let re = Regex::new(r"^[a-zA-Z0-9][a-zA-Z0-9_ -]*$").unwrap();
    if !re.is_match(name) {
        return Err(format!(
            "Invalid template name: {name:?} (only letters, digits, underscores, hyphens, spaces)"
        ));
    }
    if name.len() > 100 {
        return Err("Template name too long (max 100 chars)".into());
    }
    Ok(name.to_string())
}

pub fn list_templates() -> Vec<String> {
    let dir = templates_dir();
    if !dir.is_dir() {
        return Vec::new();
    }
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let e = e.ok()?;
            let path = e.path();
            if path.extension()?.to_str()? == "json" {
                Some(path.file_stem()?.to_str()?.to_string())
            } else {
                None
            }
        })
        .collect();
    names.sort();
    names
}

pub fn save_template(name: &str, data: &HashMap<String, String>) -> Result<(), String> {
    let name = sanitize_name(name)?;
    let dir = templates_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Cannot create templates dir: {e}"))?;
    let path = dir.join(format!("{name}.json"));
    let content =
        serde_json::to_string_pretty(data).map_err(|e| format!("Serialize error: {e}"))?;
    std::fs::write(path, content).map_err(|e| format!("Write error: {e}"))
}

pub fn load_template(name: &str) -> Option<HashMap<String, String>> {
    let name = sanitize_name(name).ok()?;
    let path = templates_dir().join(format!("{name}.json"));
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

pub fn delete_template(name: &str) -> bool {
    let Ok(name) = sanitize_name(name) else {
        return false;
    };
    let path = templates_dir().join(format!("{name}.json"));
    if path.is_file() {
        std::fs::remove_file(path).is_ok()
    } else {
        false
    }
}

/// Default templates seeded on first run.
fn default_templates() -> Vec<(&'static str, HashMap<String, String>)> {
    vec![
        (
            "Quick CPU Job",
            HashMap::from([
                ("mode".into(), "sbatch".into()),
                ("name".into(), "quick-test".into()),
                ("partition".into(), "".into()),
                ("time".into(), "00:30:00".into()),
                ("nodes".into(), "1".into()),
                ("ntasks".into(), "1".into()),
                ("cpus".into(), "1".into()),
                ("memory".into(), "4G".into()),
                ("gpus".into(), "".into()),
                ("script".into(), "".into()),
                ("output".into(), "%x-%j.out".into()),
                ("error".into(), "%x-%j.err".into()),
                ("modules".into(), "".into()),
                ("env".into(), "".into()),
                ("init".into(), "".into()),
            ]),
        ),
        (
            "Multi-Node MPI",
            HashMap::from([
                ("mode".into(), "sbatch".into()),
                ("name".into(), "mpi-job".into()),
                ("partition".into(), "".into()),
                ("time".into(), "04:00:00".into()),
                ("nodes".into(), "4".into()),
                ("ntasks".into(), "16".into()),
                ("cpus".into(), "1".into()),
                ("memory".into(), "8G".into()),
                ("gpus".into(), "".into()),
                ("script".into(), "".into()),
                ("output".into(), "%x-%j.out".into()),
                ("error".into(), "%x-%j.err".into()),
                ("modules".into(), "openmpi".into()),
                ("env".into(), "".into()),
                ("init".into(), "srun ./my_mpi_program".into()),
            ]),
        ),
        (
            "Single GPU Training",
            HashMap::from([
                ("mode".into(), "sbatch".into()),
                ("name".into(), "gpu-training".into()),
                ("partition".into(), "".into()),
                ("time".into(), "08:00:00".into()),
                ("nodes".into(), "1".into()),
                ("ntasks".into(), "1".into()),
                ("cpus".into(), "4".into()),
                ("memory".into(), "32G".into()),
                ("gpus".into(), "1".into()),
                ("script".into(), "".into()),
                ("output".into(), "%x-%j.out".into()),
                ("error".into(), "%x-%j.err".into()),
                ("modules".into(), "cuda\npython".into()),
                ("env".into(), "".into()),
                ("init".into(), "python train.py".into()),
            ]),
        ),
        (
            "Large Memory Job",
            HashMap::from([
                ("mode".into(), "sbatch".into()),
                ("name".into(), "highmem-job".into()),
                ("partition".into(), "".into()),
                ("time".into(), "12:00:00".into()),
                ("nodes".into(), "1".into()),
                ("ntasks".into(), "1".into()),
                ("cpus".into(), "8".into()),
                ("memory".into(), "128G".into()),
                ("gpus".into(), "".into()),
                ("script".into(), "".into()),
                ("output".into(), "%x-%j.out".into()),
                ("error".into(), "%x-%j.err".into()),
                ("modules".into(), "".into()),
                ("env".into(), "".into()),
                ("init".into(), "".into()),
            ]),
        ),
        (
            "Interactive Session",
            HashMap::from([
                ("mode".into(), "srun".into()),
                ("name".into(), "".into()),
                ("partition".into(), "".into()),
                ("time".into(), "01:00:00".into()),
                ("nodes".into(), "1".into()),
                ("ntasks".into(), "1".into()),
                ("cpus".into(), "2".into()),
                ("memory".into(), "8G".into()),
                ("gpus".into(), "".into()),
                ("script".into(), "".into()),
                ("output".into(), "".into()),
                ("error".into(), "".into()),
                ("modules".into(), "".into()),
                ("env".into(), "".into()),
                ("init".into(), "".into()),
            ]),
        ),
    ]
}

pub fn ensure_default_templates() {
    let dir = templates_dir();
    if dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            if entries
                .filter_map(|e| e.ok())
                .any(|e| {
                    e.path()
                        .extension()
                        .is_some_and(|ext| ext == "json")
                })
            {
                return;
            }
        }
    }
    for (name, data) in default_templates() {
        let _ = save_template(name, &data);
    }
}
