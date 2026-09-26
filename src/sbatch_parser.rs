use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;

/// Maps #SBATCH long option names to form state keys. Short options are
/// first resolved to their long name via [`SHORT_TO_LONG`].
static DIRECTIVE_MAP: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
    HashMap::from([
        ("job-name", "name"),
        ("partition", "partition"),
        ("time", "time"),
        ("nodes", "nodes"),
        ("ntasks-per-node", "ntasks"),
        ("cpus-per-task", "cpus"),
        ("mem", "memory"),
        ("output", "output"),
        ("error", "error"),
    ])
});

/// Short option → long option, as listed in sbatch(1). Every extra directive
/// is stored under its long name, because `build_params` re-emits extras as
/// `--<name>=<value>` (a short key would become an invalid `--n=4`).
pub static SHORT_TO_LONG: LazyLock<HashMap<char, &'static str>> = LazyLock::new(|| {
    HashMap::from([
        ('A', "account"),
        ('a', "array"),
        ('B', "extra-node-info"),
        ('b', "begin"),
        ('C', "constraint"),
        ('c', "cpus-per-task"),
        ('D', "chdir"),
        ('d', "dependency"),
        ('e', "error"),
        ('F', "nodefile"),
        ('G', "gpus"),
        ('H', "hold"),
        ('h', "help"),
        ('i', "input"),
        ('J', "job-name"),
        ('k', "no-kill"),
        ('L', "licenses"),
        ('M', "clusters"),
        ('m', "distribution"),
        ('N', "nodes"),
        ('n', "ntasks"),
        ('O', "overcommit"),
        ('o', "output"),
        ('p', "partition"),
        ('Q', "quiet"),
        ('q', "qos"),
        ('S', "core-spec"),
        ('s', "oversubscribe"),
        ('t', "time"),
        ('V', "version"),
        ('v', "verbose"),
        ('W', "wait"),
        ('w', "nodelist"),
        ('x', "exclude"),
    ])
});

/// Short options that never take a value; anything following them on the
/// line is not an argument.
const SHORT_FLAGS: &[char] = &['H', 'h', 'k', 'O', 'Q', 's', 'V', 'v', 'W'];

/// `--name`, `--name=value` or `--name value`.
static LONG_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^--([a-zA-Z][a-zA-Z0-9_-]*)(?:(?:=|\s+)(.*))?$").unwrap()
});
/// `-X`, `-X value` or `-Xvalue` (getopt style).
static SHORT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^-([a-zA-Z])\s*(.*)$").unwrap());
static MODULE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\s*module\s+load\s+(.+)$").unwrap());
static EXPORT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*export\s+([A-Za-z_][A-Za-z0-9_]*=.+)$").unwrap());

/// Remove a trailing ` # comment` from a directive argument, respecting
/// single and double quotes; then drop one level of surrounding quotes.
fn clean_value(raw: &str) -> String {
    let mut quote: Option<char> = None;
    let mut prev_ws = true;
    let mut cut = raw.len();
    for (i, c) in raw.char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == '#' && prev_ws => {
                cut = i;
                break;
            }
            None => {}
        }
        prev_ws = c.is_whitespace();
    }
    let v = raw[..cut].trim();
    for q in ['"', '\''] {
        if v.len() >= 2 && v.starts_with(q) && v.ends_with(q) {
            return v[1..v.len() - 1].to_string();
        }
    }
    v.to_string()
}

/// Split an `#SBATCH` line into (long option name, value). Returns None for
/// lines that are not a recognisable option.
pub fn parse_directive(line: &str) -> Option<(String, String)> {
    let rest = line.trim().strip_prefix("#SBATCH")?;
    // "#SBATCHfoo" is just a comment
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = rest.trim();
    if let Some(caps) = LONG_RE.captures(rest) {
        let key = caps.get(1).unwrap().as_str().to_string();
        let value = caps.get(2).map(|m| clean_value(m.as_str())).unwrap_or_default();
        return Some((key, value));
    }
    if let Some(caps) = SHORT_RE.captures(rest) {
        let ch = caps.get(1).unwrap().as_str().chars().next()?;
        let long = SHORT_TO_LONG.get(&ch)?;
        let value = if SHORT_FLAGS.contains(&ch) {
            String::new()
        } else {
            let raw = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            // getopt allows "-N=2" too; strip a leading '='
            clean_value(raw.strip_prefix('=').unwrap_or(raw))
        };
        return Some(((*long).to_string(), value));
    }
    None
}

/// Per-node GPU requests that feed the form's GPU field. `--gpus`/`-G` is a
/// per-JOB count and stays an extra parameter so its meaning is preserved.
const GPU_DIRECTIVES: &[&str] = &["gres", "gpus-per-node"];

/// Parse raw sbatch script text into a form state dict.
pub fn parse_sbatch_text(text: &str) -> HashMap<String, String> {
    // Normalize line endings: CRLF → LF, stray CR → LF
    let text = &text.replace("\r\n", "\n").replace('\r', "\n");
    let dmap = &*DIRECTIVE_MAP;
    let (module_re, export_re) = (&*MODULE_RE, &*EXPORT_RE);

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

        if stripped.starts_with("#!") && !past_directives {
            continue;
        }

        // sbatch(1): "The batch script may contain options preceded with
        // #SBATCH before any executable commands in the script. sbatch will
        // stop processing further #SBATCH directives once the first
        // non-comment non-whitespace line has been reached." Later directive
        // lines are ordinary comments and stay in the body verbatim.
        if !past_directives && stripped.starts_with('#') {
            if let Some((key, value)) = parse_directive(stripped) {
                apply_directive(&key, &value, dmap, &mut state, &mut extra_directives);
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
        if key == "gres" {
            // Only a single plain "gpu[:type]:count" request maps onto the
            // form field; anything else (several GRES, non-GPU GRES) is kept
            // verbatim as an extra so nothing is lost.
            match value.strip_prefix("gpu:") {
                Some(rest) if !rest.contains(',') => {
                    state.insert("gpus".into(), rest.to_string());
                }
                _ if value == "gpu" => {
                    state.insert("gpus".into(), "1".into());
                }
                _ => {
                    extras.insert(key.to_string(), value.to_string());
                }
            }
        } else {
            extras.insert(key.to_string(), value.to_string());
        }
        return;
    }

    extras.insert(key.to_string(), value.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> HashMap<String, String> {
        parse_sbatch_text(text)
    }

    #[test]
    fn long_directives_map_to_form_fields() {
        let s = parse("#!/bin/bash\n#SBATCH --job-name=train\n#SBATCH --time 01:00:00\n#SBATCH --mem=4G\n");
        assert_eq!(s["name"], "train");
        assert_eq!(s["time"], "01:00:00");
        assert_eq!(s["memory"], "4G");
        assert_eq!(s["mode"], "sbatch");
    }

    #[test]
    fn short_directives_resolve_to_long_names() {
        let s = parse("#SBATCH -n 4\n#SBATCH -A proj\n#SBATCH -q high\n#SBATCH -J job\n#SBATCH -p gpu\n");
        assert_eq!(s["extra.ntasks"], "4");
        assert_eq!(s["extra.account"], "proj");
        assert_eq!(s["extra.qos"], "high");
        assert_eq!(s["name"], "job");
        assert_eq!(s["partition"], "gpu");
        // No short key ever leaks into extras (would become "--n=4")
        assert!(s.keys().all(|k| !k.starts_with("extra.") || k.len() > "extra.".len() + 1));
    }

    #[test]
    fn attached_short_values_are_parsed() {
        let s = parse("#SBATCH -N2\n#SBATCH -t30\n#SBATCH -c=8\n");
        assert_eq!(s["nodes"], "2");
        assert_eq!(s["time"], "30");
        assert_eq!(s["cpus"], "8");
    }

    #[test]
    fn flag_directives_are_kept() {
        let s = parse("#SBATCH --exclusive\n#SBATCH --requeue\n#SBATCH -H\n");
        assert_eq!(s["extra.exclusive"], "");
        assert_eq!(s["extra.requeue"], "");
        assert_eq!(s["extra.hold"], "");
    }

    #[test]
    fn trailing_comments_and_quotes_are_stripped() {
        let s = parse("#SBATCH --time=10:00 # ten minutes\n#SBATCH --comment=\"a # b\"\n#SBATCH --constraint='a100|h100'\n");
        assert_eq!(s["time"], "10:00");
        assert_eq!(s["extra.comment"], "a # b");
        assert_eq!(s["extra.constraint"], "a100|h100");
    }

    #[test]
    fn directives_after_first_command_are_ignored() {
        let s = parse("#SBATCH --nodes=1\necho hi\n#SBATCH --nodes=5\n");
        assert_eq!(s["nodes"], "1");
        assert_eq!(s["init"], "echo hi\n#SBATCH --nodes=5");
    }

    #[test]
    fn comments_between_directives_do_not_end_the_header() {
        let s = parse("#!/bin/bash\n# resources\n\n#SBATCH --nodes=3\n");
        assert_eq!(s["nodes"], "3");
    }

    #[test]
    fn gpu_semantics_are_preserved() {
        let s = parse("#SBATCH --gres=gpu:a100:2\n");
        assert_eq!(s["gpus"], "a100:2");
        // --gpus / -G is per job, not per node
        let s = parse("#SBATCH -G 4\n");
        assert_eq!(s["gpus"], "");
        assert_eq!(s["extra.gpus"], "4");
        let s = parse("#SBATCH --gpus-per-node=2\n");
        assert_eq!(s["extra.gpus-per-node"], "2");
        // Several GRES stay verbatim
        let s = parse("#SBATCH --gres=gpu:1,nvme:1\n");
        assert_eq!(s["gpus"], "");
        assert_eq!(s["extra.gres"], "gpu:1,nvme:1");
    }

    #[test]
    fn crlf_and_modules_env_init() {
        let s = parse("#SBATCH -N 1\r\n\r\nmodule load cuda/12\r\nexport OMP_NUM_THREADS=4\r\npython train.py\r\n");
        assert_eq!(s["nodes"], "1");
        assert_eq!(s["modules"], "cuda/12");
        assert_eq!(s["env"], "OMP_NUM_THREADS=4");
        assert_eq!(s["init"], "python train.py");
    }

    #[test]
    fn unknown_short_options_and_sbatchlike_comments_are_skipped() {
        let s = parse("#SBATCH -Z foo\n#SBATCHX --nodes=9\n#SBATCH --nodes=2\n");
        assert_eq!(s["nodes"], "2");
        assert!(!s.contains_key("extra.Z"));
    }
}
