use anstyle::{AnsiColor, Style};
use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use serde::Deserialize;
use std::env;
use std::fs;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::{self, Command, ExitCode};
use std::time::{Duration, Instant};

const GREEN: Style = Style::new().fg_color(Some(anstyle::Color::Ansi(AnsiColor::Green)));
const RED: Style = Style::new().fg_color(Some(anstyle::Color::Ansi(AnsiColor::Red)));
const YELLOW: Style = Style::new().fg_color(Some(anstyle::Color::Ansi(AnsiColor::Yellow)));
const DIM: Style = Style::new().dimmed();
const BOLD: Style = Style::new().bold();

// ── CLI ─────────────────────────────────────────────────────

#[derive(Parser)]
#[command(about = "git hooks from pyproject.toml", version)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Install git hooks
    Install {
        #[arg(short, long)]
        force: bool,
    },
    /// Remove all prehook-managed git hooks
    Uninstall,
    /// Run hooks
    Run {
        /// Run only this hook by name
        hook: Option<String>,
        /// Git hook type to run
        #[arg(long, default_value_t = GitHook::PreCommit)]
        on: GitHook,
        /// Run hooks in parallel
        #[arg(long)]
        parallel: bool,
        /// Stop after first failure
        #[arg(long)]
        fail_fast: bool,
        /// Arguments passed by git (forwarded to hook commands)
        #[arg(last = true)]
        git_args: Vec<String>,
    },
}

// ── Config ──────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
#[clap(rename_all = "kebab-case")]
enum GitHook {
    PreCommit,
    PrePush,
    CommitMsg,
    PreRebase,
    PostMerge,
    PostCheckout,
    PrepareCommitMsg,
}

impl std::fmt::Display for GitHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::PreCommit => "pre-commit",
            Self::PrePush => "pre-push",
            Self::CommitMsg => "commit-msg",
            Self::PreRebase => "pre-rebase",
            Self::PostMerge => "post-merge",
            Self::PostCheckout => "post-checkout",
            Self::PrepareCommitMsg => "prepare-commit-msg",
        };
        f.write_str(name)
    }
}

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
        on: Option<Vec<GitHook>>,
        #[serde(default)]
        verbose: bool,
    },
}

#[derive(Deserialize)]
struct RawConfig {
    #[serde(default)]
    hooks: Vec<HookEntry>,
    #[serde(default)]
    fail_fast: bool,
    #[serde(default)]
    parallel: bool,
}

struct Hook {
    name: String,
    cmd: String,
    on: Vec<GitHook>,
    verbose: bool,
}

impl From<HookEntry> for Hook {
    fn from(entry: HookEntry) -> Self {
        let (cmd, name, on, verbose) = match entry {
            HookEntry::Simple(cmd) => (cmd, None, None, false),
            HookEntry::Full {
                run,
                name,
                on,
                verbose,
            } => (run, name, on, verbose),
        };
        Hook {
            name: name.unwrap_or_else(|| cmd.clone()),
            on: on.unwrap_or_else(|| vec![GitHook::PreCommit]),
            verbose,
            cmd,
        }
    }
}

impl Hook {
    fn spawn(&self, parent_is_tty: bool, git_args: &[String]) -> Result<process::Child> {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", &self.cmd]);
        if parent_is_tty {
            cmd.env("FORCE_COLOR", "1").env("CLICOLOR_FORCE", "1");
        }
        if !git_args.is_empty() {
            cmd.env("PREHOOK_ARGS", git_args.join(" "));
        }
        cmd.stdout(process::Stdio::piped())
            .stderr(process::Stdio::piped());
        Ok(cmd.spawn()?)
    }
}

fn find_pyproject_path() -> Result<PathBuf> {
    env::current_dir()?
        .ancestors()
        .map(|dir| dir.join("pyproject.toml"))
        .find(|path| path.exists())
        .context("no pyproject.toml found")
}

struct Config {
    hooks: Vec<Hook>,
    fail_fast: bool,
    parallel: bool,
}

impl TryFrom<PyProject> for Config {
    type Error = anyhow::Error;

    fn try_from(pyproject: PyProject) -> Result<Self> {
        let raw = pyproject
            .tool
            .and_then(|tool| tool.prehook)
            .context("no [tool.prehook] in pyproject.toml")?;

        if raw.hooks.is_empty() {
            bail!("[tool.prehook] needs 'hooks'");
        }

        Ok(Self {
            hooks: raw.hooks.into_iter().map(Hook::from).collect(),
            fail_fast: raw.fail_fast,
            parallel: raw.parallel,
        })
    }
}

impl Config {
    fn load() -> Result<Self> {
        let path = find_pyproject_path()?;
        let contents = fs::read_to_string(&path)?;
        let pyproject: PyProject =
            toml::from_str(&contents).with_context(|| format!("parsing {path:?}"))?;
        pyproject.try_into()
    }
}

// ── Git ─────────────────────────────────────────────────────

fn find_git_hooks_path() -> Result<PathBuf> {
    let out = Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .output()?;
    if !out.status.success() {
        bail!("not a git repository");
    }
    Ok(PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()).join("hooks"))
}

const CONFIG_TEMPLATE: &str = r#"
[tool.prehook]
hooks = [
    "echo 'hello from prehook'",
]
"#;

fn install(force: bool) -> Result<()> {
    let path = find_pyproject_path()?;
    find_git_hooks_path()?;

    let contents = fs::read_to_string(&path)?;

    if contents.contains("[tool.prehook]") {
        print_check(&format!(
            "{BOLD}pyproject.toml{BOLD:#} already has {BOLD}[tool.prehook]{BOLD:#}"
        ));
    } else {
        let mut file = fs::OpenOptions::new().append(true).open(&path)?;
        use std::io::Write;
        file.write_all(CONFIG_TEMPLATE.as_bytes())?;
        print_check(&format!(
            "added {BOLD}[tool.prehook]{BOLD:#} to {BOLD}pyproject.toml{BOLD:#}"
        ));
    }

    for hook_type in GitHook::value_variants() {
        install_git_hook(*hook_type, force)?;
    }

    print_check("git hooks installed");
    Ok(())
}

fn install_git_hook(hook_type: GitHook, force: bool) -> Result<()> {
    let hooks_dir = find_git_hooks_path()?;
    fs::create_dir_all(&hooks_dir)?;
    let hook_path = hooks_dir.join(hook_type.to_string());

    if hook_path.exists() {
        let contents = fs::read_to_string(&hook_path)?;
        if contents.contains("prehook") {
            return Ok(());
        }
        if !force {
            bail!(
                "{hook_type} hook already exists (not managed by prehook). Use --force to overwrite"
            );
        }
        let backup = hook_path.with_extension("backup");
        fs::rename(&hook_path, &backup)?;
        print_check(&format!(
            "backed up existing {hook_type} hook to {hook_type}.backup"
        ));
    }

    let bin = env::current_exe()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "prehook".into());

    fs::write(
        &hook_path,
        format!("#!/bin/sh\n\"{bin}\" run --on {hook_type} -- \"$@\"\n"),
    )?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&hook_path, fs::Permissions::from_mode(0o755))?;
    }

    Ok(())
}

fn uninstall() -> Result<()> {
    let hooks_dir = find_git_hooks_path()?;
    if !hooks_dir.exists() {
        anstream::println!("no hooks directory");
        return Ok(());
    }

    let mut removed = 0;
    let mut restored = 0;
    for hook_type in GitHook::value_variants() {
        let hook_path = hooks_dir.join(hook_type.to_string());
        if !hook_path.exists() {
            continue;
        }
        if !fs::read_to_string(&hook_path)?.contains("prehook") {
            continue;
        }

        fs::remove_file(&hook_path)?;
        removed += 1;

        let backup = hook_path.with_extension("backup");
        if backup.exists() {
            fs::rename(&backup, &hook_path)?;
            restored += 1;
        }
    }

    if removed > 0 {
        print_check("git hooks removed");
        if restored > 0 {
            print_check("previous hooks restored");
        }
    } else {
        print_skip("no hooks managed by prehook");
    }
    Ok(())
}

// ── Types ───────────────────────────────────────────────────

enum Status {
    Pending,
    Running { start: Instant },
    Skipped,
    Done { success: bool, elapsed: Duration },
}

impl Status {
    fn is_done(&self) -> bool {
        matches!(self, Self::Skipped | Self::Done { .. })
    }

    fn is_failed(&self) -> bool {
        matches!(self, Self::Done { success: false, .. })
    }

    fn render(&self, name: &str) -> String {
        match self {
            Self::Done { success, elapsed } => {
                let (style, symbol) = if *success {
                    (GREEN, "\u{2713}")
                } else {
                    (RED, "\u{2717}")
                };
                let time = format_duration(*elapsed);
                format!("{style}{symbol}{style:#} {name}{DIM} {time}{DIM:#}")
            }
            Self::Running { start } => {
                let time = format_duration(start.elapsed());
                format!("{DIM}\u{25cb} {name} {time}{DIM:#}")
            }
            Self::Pending => format!("{DIM}\u{25cb} {name}{DIM:#}"),
            Self::Skipped => format!("{DIM}\u{21b7} {name} (skipped){DIM:#}"),
        }
    }
}

struct HookRun<'a> {
    hook: &'a Hook,
    interactive: bool,
    git_args: &'a [String],
    status: Status,
    child: Option<process::Child>,
    output: String,
}

impl<'a> HookRun<'a> {
    fn new(hook: &'a Hook, interactive: bool, git_args: &'a [String]) -> Self {
        Self {
            hook,
            interactive,
            git_args,
            status: Status::Pending,
            child: None,
            output: String::new(),
        }
    }

    fn skip(&mut self) {
        self.status = Status::Skipped;
    }

    fn start(&mut self) -> Result<()> {
        self.status = Status::Running {
            start: Instant::now(),
        };
        self.child = Some(self.hook.spawn(self.interactive, self.git_args)?);
        Ok(())
    }

    fn finalize(&mut self, success: bool) {
        if self.status.is_done() {
            return;
        }
        let elapsed = match self.status {
            Status::Running { start } => start.elapsed(),
            _ => Duration::ZERO,
        };
        self.status = Status::Done { success, elapsed };
    }

    fn poll(&mut self) -> bool {
        if self.status.is_done() {
            return true;
        }
        if let Some(exit_status) = self
            .child
            .as_mut()
            .and_then(|proc| proc.try_wait().ok().flatten())
        {
            self.finalize(exit_status.success());
            true
        } else {
            false
        }
    }

    fn finish(&mut self) -> Result<bool> {
        if let Some(child) = self.child.take() {
            let out = child.wait_with_output()?;
            let success = out.status.success();

            if self.output.is_empty() {
                self.output = concat_output(&out.stdout, &out.stderr);
            }

            self.finalize(success);
            Ok(success)
        } else {
            Ok(!self.status.is_failed())
        }
    }

    fn render(&self) -> String {
        self.status.render(&self.hook.name)
    }

    fn print_final(&self) {
        anstream::println!("{}", self.render());

        let show_detail = self.status.is_failed() || self.hook.verbose;
        if show_detail && !self.output.is_empty() {
            for line in self.output.lines() {
                anstream::println!("  {line}");
            }
        }
    }
}

fn print_summary(runs: &[HookRun], elapsed: Duration) {
    let count = |f: fn(&Status) -> bool| runs.iter().filter(|r| f(&r.status)).count();
    let passed = count(|s| matches!(s, Status::Done { success: true, .. }));
    let failed = count(|s| s.is_failed());
    let skipped = count(|s| matches!(s, Status::Skipped));

    let parts: Vec<String> = [
        (passed, "passed", GREEN),
        (failed, "failed", RED),
        (skipped, "skipped", YELLOW),
    ]
    .into_iter()
    .filter(|(n, _, _)| *n > 0)
    .map(|(n, label, style)| format!("{style}{n} {label}{style:#}"))
    .collect();

    let time = format_duration(elapsed);
    anstream::println!("\n{} {DIM}({time}){DIM:#}", parts.join(", "));
}

fn all_passed(runs: &[HookRun]) -> bool {
    runs.iter().all(|run| !run.status.is_failed())
}

// ── Runner ──────────────────────────────────────────────────

fn run_hooks(
    hooks: &[&Hook],
    parallel: bool,
    fail_fast: bool,
    git_args: &[String],
) -> Result<bool> {
    let skip_env = env::var("SKIP").unwrap_or_default();
    let skip: Vec<&str> = skip_env.split(',').filter(|s| !s.is_empty()).collect();

    let total_start = Instant::now();
    let interactive = std::io::stdout().is_terminal();
    let mut runs: Vec<HookRun> = hooks
        .iter()
        .map(|hook| HookRun::new(hook, interactive, git_args))
        .collect();

    let passed = if parallel && runs.len() > 1 {
        run_parallel(&mut runs, &skip, fail_fast)?
    } else {
        run_sequential(&mut runs, &skip, fail_fast)?
    };

    if runs.len() > 1 {
        print_summary(&runs, total_start.elapsed());
    }

    Ok(passed)
}

fn run_sequential(runs: &mut [HookRun], skip: &[&str], fail_fast: bool) -> Result<bool> {
    for run in runs.iter_mut() {
        if skip.contains(&run.hook.name.as_str()) {
            run.skip();
            anstream::println!("{}", run.render());
            continue;
        }

        run.start()?;

        if run.interactive {
            use std::io::{Read, Write};

            let stdout_pipe = run.child.as_mut().unwrap().stdout.take().unwrap();
            let stderr_pipe = run.child.as_mut().unwrap().stderr.take().unwrap();

            std::thread::scope(|s| {
                let stdout_reader = s.spawn(move || {
                    let mut pipe = stdout_pipe;
                    let mut buf = Vec::new();
                    pipe.read_to_end(&mut buf).ok();
                    buf
                });
                let stderr_reader = s.spawn(move || {
                    let mut pipe = stderr_pipe;
                    let mut buf = Vec::new();
                    pipe.read_to_end(&mut buf).ok();
                    buf
                });

                while !run.poll() {
                    anstream::print!("\r\x1b[2K{}", run.render());
                    std::io::stdout().flush().ok();
                    std::thread::sleep(Duration::from_millis(50));
                }
                anstream::print!("\r\x1b[2K");

                run.output = concat_output(
                    &stdout_reader.join().unwrap(),
                    &stderr_reader.join().unwrap(),
                );
            });
        }

        run.finish()?;
        run.print_final();
        if run.status.is_failed() && fail_fast {
            break;
        }
    }

    Ok(all_passed(runs))
}

fn run_parallel(runs: &mut [HookRun], skip: &[&str], fail_fast: bool) -> Result<bool> {
    let interactive = runs.first().map(|r| r.interactive).unwrap_or(false);
    let count = runs.len();

    for run in runs.iter_mut() {
        if skip.contains(&run.hook.name.as_str()) {
            run.skip();
        } else {
            run.start()?;
        }
    }

    {
        use std::io::{Read, Write};

        if interactive {
            for run in runs.iter() {
                anstream::println!("{}", run.render());
            }
            std::io::stdout().flush().ok();
        }

        std::thread::scope(|s| {
            let drains: Vec<_> = runs
                .iter_mut()
                .enumerate()
                .filter_map(|(i, run)| {
                    let child = run.child.as_mut()?;
                    let stdout = child.stdout.take()?;
                    let stderr = child.stderr.take()?;
                    Some((
                        i,
                        s.spawn(move || {
                            let mut pipe = stdout;
                            let mut buf = Vec::new();
                            pipe.read_to_end(&mut buf).ok();
                            buf
                        }),
                        s.spawn(move || {
                            let mut pipe = stderr;
                            let mut buf = Vec::new();
                            pipe.read_to_end(&mut buf).ok();
                            buf
                        }),
                    ))
                })
                .collect();

            loop {
                std::thread::sleep(Duration::from_millis(50));
                let done = runs.iter_mut().fold(true, |acc, run| run.poll() & acc);

                if interactive {
                    anstream::print!("\x1b[{}A", count);
                    for run in runs.iter() {
                        anstream::print!("\x1b[2K");
                        anstream::println!("{}", run.render());
                    }
                    std::io::stdout().flush().ok();
                }

                if done {
                    break;
                }
            }

            for (i, out_handle, err_handle) in drains {
                runs[i].output =
                    concat_output(&out_handle.join().unwrap(), &err_handle.join().unwrap());
            }
        });

        if interactive {
            anstream::print!("\x1b[{}A", count);
            for _ in 0..count {
                anstream::println!("\x1b[2K");
            }
            anstream::print!("\x1b[{}A", count);
        }
    }

    for run in runs.iter_mut() {
        run.finish()?;
        run.print_final();
        if run.status.is_failed() && fail_fast {
            break;
        }
    }

    Ok(all_passed(runs))
}

fn concat_output(stdout: &[u8], stderr: &[u8]) -> String {
    [stdout, stderr]
        .map(|bytes| String::from_utf8_lossy(bytes).trim().to_string())
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

// ── Output ──────────────────────────────────────────────────

fn format_duration(duration: Duration) -> String {
    format!("{:.2}s", duration.as_secs_f64())
}

fn print_check(message: &str) {
    anstream::println!("{GREEN}\u{2713}{GREEN:#} {message}");
}

fn print_skip(message: &str) {
    anstream::println!("{DIM}\u{21b7} {message}{DIM:#}");
}

fn print_error(err: &anyhow::Error) {
    anstream::eprintln!("{RED}\u{2717}{RED:#} {err}");
}

// ── Main ────────────────────────────────────────────────────

fn run() -> Result<bool> {
    let cli = Cli::parse();

    match cli.command {
        Cmd::Install { force } => {
            install(force)?;
            Ok(true)
        }
        Cmd::Uninstall => {
            uninstall()?;
            Ok(true)
        }
        Cmd::Run {
            hook,
            on,
            parallel,
            fail_fast,
            git_args,
        } => {
            let config = Config::load()?;
            let parallel = parallel || config.parallel;
            let fail_fast = fail_fast || config.fail_fast;

            let hooks: Vec<&Hook> = if let Some(name) = &hook {
                let matched: Vec<&Hook> = config.hooks.iter().filter(|h| h.name == *name).collect();
                if matched.is_empty() {
                    bail!("unknown hook: {name}");
                }
                matched
            } else {
                config.hooks.iter().filter(|h| h.on.contains(&on)).collect()
            };

            run_hooks(&hooks, parallel, fail_fast, &git_args)
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(err) => {
            print_error(&err);
            ExitCode::from(1)
        }
    }
}
