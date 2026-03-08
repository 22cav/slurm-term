use std::collections::HashMap;

use regex::Regex;

/// Maps #SBATCH long keys to form state keys.
fn directive_map() -> HashMap<&'static str, &'static str> {
    let mut m = HashMap::new();
    m.insert("job-name", "name");
    m.insert("J", "name");
    m.insert("partition", "partition");
    m.insert("p", "partition");
    m.insert("time", "time");
    m.insert("t", "time");
    m.insert("nodes", "nodes");
    m.insert("N", "nodes");
    m.insert("ntasks-per-node", "ntasks");
    m.insert("cpus-per-task", "cpus");
    m.insert("c", "cpus");
    m.insert("mem", "memory");
    m.insert("output", "output");
    m.insert("o", "output");
    m.insert("error", "error");
    m.insert("e", "error");
    m
}

const GPU_DIRECTIVES: &[&str] = &["gres", "gpus", "gpus-per-node", "G"];

/// Parse raw sbatch script text into a form state dict.
pub fn parse_sbatch_text(text: &str) -> HashMap<String, String> {
    let dmap = directive_map();

    let long_re = Regex::new(r"^\s*#SBATCH\s+--([a-zA-Z][a-zA-Z0-9_-]*)(?:=|\s+)(.+)?$").unwrap();
    let short_re = Regex::new(r"^\s*#SBATCH\s+-([a-zA-Z])(?:\s+(.+))?$").unwrap();
    let module_re = Regex::new(r"(?i)^\s*module\s+load\s+(.+)$").unwrap();
    let export_re = Regex::new(r"^\s*export\s+([A-Za-z_][A-Za-z0-9_]*=.+)$").unwrap();

    let mut state: HashMap<String, String> = HashMap::new();
    state.insert("mode".into(), "sbatch".into());
    for key in &["name", "partition", "time", "nodes", "ntasks", "cpus",
                  "memory", "gpus", "output", "error", "script"] {
        state.insert((*key).into(), String::new());
    }

    let mut extra_directives: HashMap<String, String> = HashMap::new();
    let mut modules: Vec<String> = Vec::new();
    let mut env_vars: Vec<String> = Vec::new();
    let mut init_cmds: Vec<String> = Vec::new();
    let mut past_directives = false;

    for line in text.lines() {
        let stripped = line.trim();

        if stripped.starts_with("#!") {
            continue;
        }
        if stripped.starts_with('#') && !stripped.starts_with("#SBATCH") {
            continue;
        }

        if stripped.starts_with("#SBATCH") {
            if let Some(caps) = long_re.captures(stripped) {
                let key = caps.get(1).unwrap().as_str();
                let value = caps.get(2).map(|m| m.as_str().trim()).unwrap_or("");
                apply_directive(key, value, &dmap, &mut state, &mut extra_directives);
                continue;
            }
            if let Some(caps) = short_re.captures(stripped) {
                let key = caps.get(1).unwrap().as_str();
                let value = caps.get(2).map(|m| m.as_str().trim()).unwrap_or("");
                apply_directive(key, value, &dmap, &mut state, &mut extra_directives);
                continue;
            }
            continue;
        }

        if stripped.is_empty() && !past_directives {
            continue;
        }
        past_directives = true;

        if stripped.is_empty() {
            if !init_cmds.is_empty() {
                init_cmds.push(String::new());
            }
            continue;
        }

        if let Some(caps) = module_re.captures(stripped) {
            modules.push(caps.get(1).unwrap().as_str().trim().to_string());
            continue;
        }

        if let Some(caps) = export_re.captures(stripped) {
            env_vars.push(caps.get(1).unwrap().as_str().trim().to_string());
            continue;
        }

        init_cmds.push(line.trim_end().to_string());
    }

    // Strip trailing blanks from init_cmds
    while init_cmds.last().is_some_and(|l| l.trim().is_empty()) {
        init_cmds.pop();
    }

    state.insert("modules".into(), modules.join("\n"));
    state.insert("env".into(), env_vars.join("\n"));
    state.insert("init".into(), init_cmds.join("\n"));

    for (k, v) in &extra_directives {
        state.insert(format!("extra.{k}"), v.clone());
    }

    state
}

/// Parse a .sbatch file.
pub fn parse_sbatch_file(path: &str) -> Result<HashMap<String, String>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("Cannot read file: {e}"))?;
    Ok(parse_sbatch_text(&text))
}

fn apply_directive(
    key: &str,
    value: &str,
    dmap: &HashMap<&str, &str>,
    state: &mut HashMap<String, String>,
    extras: &mut HashMap<String, String>,
) {
    if let Some(form_key) = dmap.get(key) {
        state.insert((*form_key).to_string(), value.to_string());
        return;
    }

    if GPU_DIRECTIVES.contains(&key) {
        let gpu_val = if let Some(rest) = value.strip_prefix("gpu:") {
            rest
        } else {
            value
        };
        state.insert("gpus".into(), gpu_val.to_string());
        return;
    }

    extras.insert(key.to_string(), value.to_string());
}
