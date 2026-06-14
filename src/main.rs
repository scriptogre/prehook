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
    prehook: Option<RawConfig>,
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
        .and_then(|t| t.prehook)
        .ok_or("no [tool.prehook] in pyproject.toml")?;

    let entries = raw.hooks.ok_or("[tool.prehook] needs 'hooks'")?;

    let mut hooks: Vec<Hook> = entries
        .into_iter()
        .map(|entry| {
            let (cmd, name, stages, verbose) = match entry {
                HookEntry::Simple(cmd) => (cmd, None, None, false),
                HookEntry::Full {
                    run,
                    name,
                    stages,
                    verbose,
                } => (run, name, stages, verbose),
            };
            Hook {
                name: name.unwrap_or_else(|| cmd.clone()),
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

const CONFIG_TEMPLATE: &str = r#"
[tool.prehook]
hooks = [
    "echo 'hello from prehook'",
]
"#;

fn install(force: bool) -> Result<(), String> {
    let path = find_pyproject()?;
    find_git_hooks_dir()?;

    let text = fs::read_to_string(&path).map_err(|e| e.to_string())?;

    let color = use_color();
    let (b, r) = if color {
        ("\x1b[1m", "\x1b[0m")
    } else {
        ("", "")
    };

    if text.contains("[tool.prehook]") {
        print_status(
            &format!("{b}pyproject.toml{r} already has {b}[tool.prehook]{r}"),
            "passed",
            None,
            None,
        );
    } else {
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .map_err(|e| e.to_string())?;
        use std::io::Write;
        file.write_all(CONFIG_TEMPLATE.as_bytes())
            .map_err(|e| e.to_string())?;
        print_status(
            &format!("added {b}[tool.prehook]{r} to {b}pyproject.toml{r}"),
            "passed",
            None,
            None,
        );
    }

    let stages = [
        "pre-commit",
        "pre-push",
        "commit-msg",
        "pre-rebase",
        "post-merge",
        "post-checkout",
        "prepare-commit-msg",
    ];

    for stage in stages {
        install_hook(stage, force)?;
    }

    print_status("git hooks installed", "passed", None, None);
    Ok(())
}

fn install_hook(stage: &str, force: bool) -> Result<bool, String> {
    let git_hooks_dir = find_git_hooks_dir()?;
    fs::create_dir_all(&git_hooks_dir).map_err(|e| e.to_string())?;
    let hook_path = git_hooks_dir.join(stage);

    if hook_path.exists() {
        let content = fs::read_to_string(&hook_path).map_err(|e| e.to_string())?;
        if content.contains("prehook") {
            return Ok(false);
        }
        if !force {
            return Err(format!(
                "{stage} hook already exists (not managed by prehook). Use --force to overwrite"
            ));
        }
        let backup = hook_path.with_extension("backup");
        fs::rename(&hook_path, &backup).map_err(|e| e.to_string())?;
        print_status(
            &format!("backed up existing {stage} hook to {stage}.backup"),
            "passed",
            None,
            None,
        );
    }

    let bin = env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "prehook".into());

    fs::write(
        &hook_path,
        format!("#!/bin/sh\n\"{bin}\" run --stage {stage}\n"),
    )
    .map_err(|e| e.to_string())?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&hook_path, fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
    }

    Ok(true)
}

fn uninstall_hooks() -> Result<(), String> {
    let git_hooks_dir = find_git_hooks_dir()?;
    if !git_hooks_dir.exists() {
        println!("no hooks directory");
        return Ok(());
    }

    let mut removed = 0;
    let mut restored = 0;
    for entry in fs::read_dir(&git_hooks_dir).map_err(|e| e.to_string())? {
        let hook_path = entry.map_err(|e| e.to_string())?.path();

        if !hook_path.is_file() {
            continue;
        }
        if matches!(
            hook_path.extension().and_then(|e| e.to_str()),
            Some("backup" | "sample")
        ) {
            continue;
        }
        if !fs::read_to_string(&hook_path)
            .unwrap_or_default()
            .contains("prehook")
        {
            continue;
        }

        fs::remove_file(&hook_path).map_err(|e| e.to_string())?;
        removed += 1;

        let backup = hook_path.with_extension("backup");
        if backup.exists() {
            fs::rename(&backup, &hook_path).map_err(|e| e.to_string())?;
            restored += 1;
        }
    }

    if removed > 0 {
        print_status("git hooks removed", "passed", None, None);
        if restored > 0 {
            print_status("previous hooks restored", "passed", None, None);
        }
    } else {
        print_status("no hooks managed by prehook", "skipped", None, None);
    }
    Ok(())
}

// ── Runner ──────────────────────────────────────────────────

fn run_hooks(config: &Config, stage: &str, only: Option<&str>) -> Result<bool, String> {
    let skip_env = env::var("SKIP").unwrap_or_default();
    let skip: Vec<&str> = skip_env.split(',').filter(|s| !s.is_empty()).collect();

    let hooks: Vec<&Hook> = config
        .hooks
        .iter()
        .filter(|h| only.is_none_or(|name| h.name == name))
        .filter(|h| h.stages.iter().any(|s| s == stage))
        .collect();

    if let Some(name) = only {
        if hooks.is_empty() {
            return Err(format!("unknown hook: {name}"));
        }
    }

    let total_start = Instant::now();

    let ok = if config.parallel && hooks.len() > 1 {
        run_parallel(&hooks, &skip, config.fail_fast)?
    } else {
        run_sequential(&hooks, &skip, config.fail_fast)?
    };

    if hooks.len() > 1 {
        print_summary(&COUNTS.with(|c| c.take()), total_start.elapsed());
    }

    Ok(ok)
}

#[derive(Default)]
struct Counts {
    passed: u32,
    failed: u32,
    skipped: u32,
}

std::thread_local! {
    static COUNTS: std::cell::Cell<Counts> = const { std::cell::Cell::new(Counts { passed: 0, failed: 0, skipped: 0 }) };
}

fn track(status: &str) {
    COUNTS.with(|c| {
        let mut counts = c.take();
        match status {
            "passed" => counts.passed += 1,
            "failed" => counts.failed += 1,
            _ => counts.skipped += 1,
        }
        c.set(counts);
    });
}

fn use_color() -> bool {
    std::io::stdout().is_terminal() && env::var("NO_COLOR").is_err()
}

fn force_color(cmd: &mut Command) {
    cmd.env("FORCE_COLOR", "1").env("CLICOLOR_FORCE", "1");
}

fn run_sequential(hooks: &[&Hook], skip: &[&str], fail_fast: bool) -> Result<bool, String> {
    let mut ok = true;
    let color = use_color();

    for hook in hooks {
        if skip.contains(&hook.name.as_str()) {
            print_status(&hook.name, "skipped", None, None);
            continue;
        }

        if color {
            use std::io::Write;
            print!("\x1b[2m\u{25cb} {}\x1b[0m", hook.name);
            std::io::stdout().flush().ok();
        }

        let start = Instant::now();
        let mut cmd = Command::new("sh");
        cmd.args(["-c", &hook.cmd]);
        if color {
            force_color(&mut cmd);
        }
        let out = cmd.output().map_err(|e| e.to_string())?;

        if color {
            print!("\r\x1b[2K");
        }
        let elapsed = start.elapsed();
        let output = concat_output(&out.stdout, &out.stderr);

        if out.status.success() {
            let detail = if hook.verbose {
                Some(output.as_str())
            } else {
                None
            };
            print_status(&hook.name, "passed", Some(elapsed), detail);
        } else {
            print_status(&hook.name, "failed", Some(elapsed), Some(&output));
            ok = false;
            if fail_fast {
                break;
            }
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

            let color = use_color();
            let handle = thread::spawn(
                move || -> Result<Option<(bool, String, Duration)>, String> {
                    if skipped {
                        return Ok(None);
                    }
                    let start = Instant::now();
                    let mut proc = Command::new("sh");
                    proc.args(["-c", &cmd]);
                    if color {
                        force_color(&mut proc);
                    }
                    let out = proc.output().map_err(|e| e.to_string())?;
                    Ok(Some((
                        out.status.success(),
                        concat_output(&out.stdout, &out.stderr),
                        start.elapsed(),
                    )))
                },
            );

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
            if fail_fast {
                break;
            }
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
    let (symbol, color_code) = match status {
        "passed" => ("\u{2713}", "32"), // ✓
        "failed" => ("\u{2717}", "31"), // ✗
        _ => ("\u{21b7}", "33"),        // ↷
    };
    let time = elapsed
        .map(|d| format!(" {:.1}s", d.as_secs_f64()))
        .unwrap_or_default();

    track(status);

    if use_color() {
        println!("\x1b[{color_code}m{symbol}\x1b[0m {name}\x1b[2m{time}\x1b[0m");
    } else {
        println!("{symbol} {name}{time}");
    }

    if let Some(text) = detail.filter(|t| !t.is_empty()) {
        for line in text.lines() {
            println!("  {line}");
        }
    }
}

fn print_summary(counts: &Counts, elapsed: Duration) {
    let color = use_color();
    let mut parts = Vec::new();

    if counts.passed > 0 {
        if color {
            parts.push(format!("\x1b[32m{} passed\x1b[0m", counts.passed));
        } else {
            parts.push(format!("{} passed", counts.passed));
        }
    }
    if counts.failed > 0 {
        if color {
            parts.push(format!("\x1b[31m{} failed\x1b[0m", counts.failed));
        } else {
            parts.push(format!("{} failed", counts.failed));
        }
    }
    if counts.skipped > 0 {
        if color {
            parts.push(format!("\x1b[33m{} skipped\x1b[0m", counts.skipped));
        } else {
            parts.push(format!("{} skipped", counts.skipped));
        }
    }

    let time = format!("{:.1}s", elapsed.as_secs_f64());
    if color {
        println!("\n{} \x1b[2m({time})\x1b[0m", parts.join(", "));
    } else {
        println!("\n{} ({time})", parts.join(", "));
    }
}

// ── CLI ─────────────────────────────────────────────────────

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();

    match args.get(1).map(|s| s.as_str()) {
        Some("install") => {
            let force = args.iter().any(|a| a == "--force" || a == "-f");
            install(force)
        }
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
            eprintln!("prehook - git hooks from pyproject.toml\n");
            eprintln!("usage:");
            eprintln!("  prehook init");
            eprintln!("  prehook uninstall");
            eprintln!("  prehook run [<hook>] [--stage <stage>]");
            process::exit(1);
        }
    }
}

fn main() {
    if let Err(e) = run() {
        if std::io::stderr().is_terminal() && env::var("NO_COLOR").is_err() {
            eprintln!("\x1b[31m\u{2717}\x1b[0m {e}");
        } else {
            eprintln!("\u{2717} {e}");
        }
        process::exit(1);
    }
}
