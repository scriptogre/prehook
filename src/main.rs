use serde::Deserialize;
use std::env;
use std::fs;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::{self, Command};
use std::time::{Duration, Instant};

// ── Config ──────────────────────────────────────────────────

#[derive(Deserialize)]
struct PyProject {
    tool: Option<ToolTable>,
}

#[derive(Deserialize)]
struct ToolTable {
    precommit: Option<RawConfig>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum HookEntry {
    Simple(String),
    Full {
        run: String,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        stages: Option<Vec<String>>,
        #[serde(default)]
        verbose: bool,
    },
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawConfig {
    hooks: Option<Vec<HookEntry>>,
    fail_fast: bool,
    parallel: bool,
}

struct Config {
    hooks: Vec<Hook>,
    fail_fast: bool,
    parallel: bool,
}

struct Hook {
    name: String,
    cmd: String,
    stages: Vec<String>,
    verbose: bool,
}

fn find_pyproject() -> Result<PathBuf, String> {
    env::current_dir()
        .map_err(|e| e.to_string())?
        .ancestors()
        .map(|d| d.join("pyproject.toml"))
        .find(|p| p.exists())
        .ok_or_else(|| "no pyproject.toml found".into())
}

fn derive_name(cmd: &str) -> String {
    cmd.split_whitespace().next().unwrap_or("hook").to_string()
}

fn dedupe_names(hooks: &mut [Hook]) {
    let mut seen = std::collections::HashMap::<String, u32>::new();
    for h in hooks.iter_mut() {
        let n = seen.entry(h.name.clone()).or_insert(0);
        if *n > 0 {
            h.name = format!("{}-{n}", h.name);
        }
        *n += 1;
    }
}

fn load_config() -> Result<Config, String> {
    let path = find_pyproject()?;
    let text = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let doc: PyProject = toml::from_str(&text).map_err(|e| format!("{path:?}: {e}"))?;

    let raw = doc
        .tool
        .and_then(|t| t.precommit)
        .ok_or("no [tool.precommit] in pyproject.toml")?;

    let entries = raw.hooks.ok_or("[tool.precommit] needs 'hooks'")?;

    let mut hooks: Vec<Hook> = entries
        .into_iter()
        .map(|entry| {
            let (cmd, name, stages, verbose) = match entry {
                HookEntry::Simple(cmd) => (cmd, None, None, false),
                HookEntry::Full { run, name, stages, verbose } => (run, name, stages, verbose),
            };
            Hook {
                name: name.unwrap_or_else(|| derive_name(&cmd)),
                stages: stages.unwrap_or_else(|| vec!["pre-commit".into()]),
                verbose,
                cmd,
            }
        })
        .collect();

    dedupe_names(&mut hooks);

    Ok(Config {
        hooks,
        fail_fast: raw.fail_fast,
        parallel: raw.parallel,
    })
}

// ── Git ─────────────────────────────────────────────────────

fn find_git_hooks_dir() -> Result<PathBuf, String> {
    let out = Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err("not a git repository".into());
    }
    Ok(PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()).join("hooks"))
}

fn install_hooks() -> Result<(), String> {
    let config = load_config()?;

    let stages: std::collections::BTreeSet<&str> = config
        .hooks
        .iter()
        .flat_map(|h| h.stages.iter().map(|s| s.as_str()))
        .collect();

    for stage in stages {
        install_hook(stage)?;
    }
    Ok(())
}

fn install_hook(stage: &str) -> Result<(), String> {
    let git_hooks_dir = find_git_hooks_dir()?;
    fs::create_dir_all(&git_hooks_dir).map_err(|e| e.to_string())?;
    let hook_path = git_hooks_dir.join(stage);

    if hook_path.exists() {
        let content = fs::read_to_string(&hook_path).map_err(|e| e.to_string())?;
        if content.contains("precommit") {
            println!("already installed at {}", hook_path.display());
            return Ok(());
        }
        let backup = hook_path.with_extension("legacy");
        fs::rename(&hook_path, &backup).map_err(|e| e.to_string())?;
        println!("backed up existing hook to {}", backup.display());
    }

    let bin = env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "precommit".into());

    fs::write(&hook_path, format!("#!/bin/sh\n\"{bin}\" run --stage {stage}\n"))
        .map_err(|e| e.to_string())?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&hook_path, fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
    }

    println!("installed {stage} hook");
    Ok(())
}

fn uninstall_hooks() -> Result<(), String> {
    let git_hooks_dir = find_git_hooks_dir()?;
    if !git_hooks_dir.exists() {
        println!("no hooks directory");
        return Ok(());
    }

    let mut found = false;
    for entry in fs::read_dir(&git_hooks_dir).map_err(|e| e.to_string())? {
        let hook_path = entry.map_err(|e| e.to_string())?.path();

        if !hook_path.is_file() { continue; }
        if matches!(hook_path.extension().and_then(|e| e.to_str()), Some("legacy" | "sample")) { continue; }
        if !fs::read_to_string(&hook_path).unwrap_or_default().contains("precommit") { continue; }

        found = true;
        let stage = hook_path.file_name().unwrap().to_string_lossy();
        fs::remove_file(&hook_path).map_err(|e| e.to_string())?;

        let backup = hook_path.with_extension("legacy");
        if backup.exists() {
            fs::rename(&backup, &hook_path).map_err(|e| e.to_string())?;
            println!("restored previous {stage} hook");
        }
        println!("uninstalled {stage} hook");
    }

    if !found { println!("no hooks managed by precommit"); }
    Ok(())
}

// ── Runner ──────────────────────────────────────────────────

fn run_hooks(config: &Config, stage: &str, only: Option<&str>) -> Result<bool, String> {
    let skip_env = env::var("SKIP").unwrap_or_default();
    let skip: Vec<&str> = skip_env.split(',').filter(|s| !s.is_empty()).collect();

    let hooks: Vec<&Hook> = config
        .hooks
        .iter()
        .filter(|h| only.map_or(true, |name| h.name == name))
        .filter(|h| h.stages.iter().any(|s| s == stage))
        .collect();

    if let Some(name) = only {
        if hooks.is_empty() {
            return Err(format!("unknown hook: {name}"));
        }
    }

    if config.parallel && hooks.len() > 1 {
        run_parallel(&hooks, &skip, config.fail_fast)
    } else {
        run_sequential(&hooks, &skip, config.fail_fast)
    }
}

fn run_sequential(hooks: &[&Hook], skip: &[&str], fail_fast: bool) -> Result<bool, String> {
    let mut ok = true;

    for hook in hooks {
        if skip.contains(&hook.name.as_str()) {
            print_status(&hook.name, "skipped", None, None);
            continue;
        }

        let start = Instant::now();
        let out = Command::new("sh")
            .args(["-c", &hook.cmd])
            .output()
            .map_err(|e| e.to_string())?;
        let elapsed = start.elapsed();
        let output = concat_output(&out.stdout, &out.stderr);

        if out.status.success() {
            let detail = if hook.verbose { Some(output.as_str()) } else { None };
            print_status(&hook.name, "passed", Some(elapsed), detail);
        } else {
            print_status(&hook.name, "failed", Some(elapsed), Some(&output));
            ok = false;
            if fail_fast { break; }
        }
    }

    Ok(ok)
}

fn run_parallel(hooks: &[&Hook], skip: &[&str], fail_fast: bool) -> Result<bool, String> {
    use std::thread;

    let handles: Vec<_> = hooks
        .iter()
        .map(|hook| {
            let name = hook.name.clone();
            let cmd = hook.cmd.clone();
            let verbose = hook.verbose;
            let skipped = skip.contains(&hook.name.as_str());

            let handle = thread::spawn(move || -> Result<Option<(bool, String, Duration)>, String> {
                if skipped { return Ok(None); }
                let start = Instant::now();
                let out = Command::new("sh").args(["-c", &cmd]).output().map_err(|e| e.to_string())?;
                Ok(Some((out.status.success(), concat_output(&out.stdout, &out.stderr), start.elapsed())))
            });

            (name, verbose, handle)
        })
        .collect();

    let mut ok = true;
    for (name, verbose, handle) in handles {
        let result = handle.join().map_err(|_| "hook panicked")??;

        let Some((passed, output, elapsed)) = result else {
            print_status(&name, "skipped", None, None);
            continue;
        };

        if passed {
            let detail = if verbose { Some(output.as_str()) } else { None };
            print_status(&name, "passed", Some(elapsed), detail);
        } else {
            print_status(&name, "failed", Some(elapsed), Some(&output));
            ok = false;
            if fail_fast { break; }
        }
    }
    Ok(ok)
}

fn concat_output(stdout: &[u8], stderr: &[u8]) -> String {
    [stdout, stderr]
        .map(|b| String::from_utf8_lossy(b).trim().to_string())
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

// ── Output ──────────────────────────────────────────────────

fn print_status(name: &str, status: &str, elapsed: Option<Duration>, detail: Option<&str>) {
    let dots = ".".repeat(55usize.saturating_sub(name.len() + status.len()).max(1));
    let time = elapsed.map(|d| format!(" ({:.2}s)", d.as_secs_f64())).unwrap_or_default();

    if std::io::stdout().is_terminal() && env::var("NO_COLOR").is_err() {
        let c = match status { "passed" => "32", "failed" => "31", _ => "33" };
        println!("{name}\x1b[2m{dots}\x1b[0m\x1b[{c}m{status}\x1b[0m\x1b[2m{time}\x1b[0m");
    } else {
        println!("{name}{dots}{status}{time}");
    }

    if let Some(text) = detail.filter(|t| !t.is_empty()) {
        for line in text.lines() {
            println!("  {line}");
        }
    }
}

// ── CLI ─────────────────────────────────────────────────────

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();

    match args.get(1).map(|s| s.as_str()) {
        Some("install") => install_hooks(),
        Some("uninstall") => uninstall_hooks(),
        Some("run") => {
            let rest = &args[2..];
            let mut hook_name: Option<&str> = None;
            let mut stage = "pre-commit";
            let mut i = 0;

            while i < rest.len() {
                match rest[i].as_str() {
                    "--stage" => {
                        i += 1;
                        stage = rest.get(i).map(|s| s.as_str()).unwrap_or("pre-commit");
                    }
                    s if !s.starts_with('-') => hook_name = Some(s),
                    _ => {}
                }
                i += 1;
            }

            let config = load_config()?;
            if !run_hooks(&config, stage, hook_name)? {
                process::exit(1);
            }
            Ok(())
        }
        _ => {
            eprintln!("precommit - git hooks from pyproject.toml\n");
            eprintln!("usage:");
            eprintln!("  precommit install");
            eprintln!("  precommit uninstall");
            eprintln!("  precommit run [<hook>] [--stage <stage>]");
            process::exit(1);
        }
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        process::exit(1);
    }
}
