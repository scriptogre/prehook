use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_precommit"))
}

struct TempRepo {
    dir: tempfile::TempDir,
}

impl TempRepo {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "--allow-empty", "-m", "init"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        TempRepo { dir }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn write_config(&self, toml: &str) {
        fs::write(self.path().join("pyproject.toml"), toml).unwrap();
    }

    fn run_cmd(&self, args: &[&str]) -> (i32, String, String) {
        let out = Command::new(binary())
            .args(args)
            .current_dir(self.path())
            .env("NO_COLOR", "1")
            .output()
            .unwrap();
        let code = out.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        (code, stdout, stderr)
    }
}

// ── Config parsing ──────────────────────────────────────────

#[test]
fn simple_form_single_hook() {
    let repo = TempRepo::new();
    repo.write_config(r#"
[tool.precommit]
hooks = ["echo hello"]
"#);
    let (code, stdout, _) = repo.run_cmd(&["run"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("passed"));
}

#[test]
fn simple_form_multiple_hooks() {
    let repo = TempRepo::new();
    repo.write_config(r#"
[tool.precommit]
hooks = ["echo hello", "echo world"]
"#);
    let (code, stdout, _) = repo.run_cmd(&["run"]);
    assert_eq!(code, 0);
    assert_eq!(stdout.matches("passed").count(), 2);
}

#[test]
fn full_form_with_name() {
    let repo = TempRepo::new();
    repo.write_config(r#"
[tool.precommit]
hooks = [
    { name = "greet", run = "echo hi" },
]
"#);
    let (code, stdout, _) = repo.run_cmd(&["run"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("greet"));
    assert!(stdout.contains("passed"));
}

#[test]
fn full_form_name_derived_from_command() {
    let repo = TempRepo::new();
    repo.write_config(r#"
[tool.precommit]
hooks = [
    { run = "echo hi" },
]
"#);
    let (code, stdout, _) = repo.run_cmd(&["run"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("echo"));
    assert!(stdout.contains("passed"));
}

#[test]
fn mixed_simple_and_full() {
    let repo = TempRepo::new();
    repo.write_config(r#"
[tool.precommit]
hooks = [
    "echo hello",
    { name = "greet", run = "echo hi" },
]
"#);
    let (code, stdout, _) = repo.run_cmd(&["run"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("echo"));
    assert!(stdout.contains("greet"));
}

#[test]
fn errors_when_no_hooks() {
    let repo = TempRepo::new();
    repo.write_config(r#"
[tool.precommit]
fail_fast = true
"#);
    let (code, _, stderr) = repo.run_cmd(&["run"]);
    assert_ne!(code, 0);
    assert!(stderr.contains("hooks"));
}

#[test]
fn errors_when_no_section() {
    let repo = TempRepo::new();
    repo.write_config("[project]\nname = \"test\"\n");
    let (code, _, stderr) = repo.run_cmd(&["run"]);
    assert_ne!(code, 0);
    assert!(stderr.contains("no [tool.precommit]"));
}

#[test]
fn duplicate_names_get_suffixed() {
    let repo = TempRepo::new();
    repo.write_config(r#"
[tool.precommit]
hooks = ["echo first", "echo second", "echo third"]
"#);
    let (code, stdout, _) = repo.run_cmd(&["run"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("echo-1"));
    assert!(stdout.contains("echo-2"));
}

// ── SKIP + fail_fast ────────────────────────────────────────

#[test]
fn skip_env_skips_hooks() {
    let repo = TempRepo::new();
    repo.write_config(r#"
[tool.precommit]
hooks = [
    { name = "fail", run = "exit 1" },
]
"#);
    let out = Command::new(binary())
        .args(["run"])
        .current_dir(repo.path())
        .env("NO_COLOR", "1")
        .env("SKIP", "fail")
        .output()
        .unwrap();
    assert_eq!(out.status.code().unwrap(), 0);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("skipped"));
}

#[test]
fn skip_multiple_hooks() {
    let repo = TempRepo::new();
    repo.write_config(r#"
[tool.precommit]
hooks = [
    { name = "a", run = "exit 1" },
    { name = "b", run = "exit 1" },
    { name = "c", run = "echo ok" },
]
"#);
    let out = Command::new(binary())
        .args(["run"])
        .current_dir(repo.path())
        .env("NO_COLOR", "1")
        .env("SKIP", "a,b")
        .output()
        .unwrap();
    assert_eq!(out.status.code().unwrap(), 0);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout.matches("skipped").count(), 2);
    assert!(stdout.contains("passed"));
}

#[test]
fn verbose_shows_output_on_success() {
    let repo = TempRepo::new();
    repo.write_config(r#"
[tool.precommit]
hooks = [
    { name = "loud", run = "echo hello world", verbose = true },
]
"#);
    let (code, stdout, _) = repo.run_cmd(&["run"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("passed"));
    assert!(stdout.contains("hello world"));
}

#[test]
fn fail_fast_stops_after_first_failure() {
    let repo = TempRepo::new();
    repo.write_config(r#"
[tool.precommit]
fail_fast = true
hooks = [
    { name = "bad", run = "exit 1" },
    { name = "never", run = "echo should not run" },
]
"#);
    let (code, stdout, _) = repo.run_cmd(&["run"]);
    assert_ne!(code, 0);
    assert!(stdout.contains("bad"));
    assert!(!stdout.contains("never"));
}

#[test]
fn failing_hook_returns_nonzero() {
    let repo = TempRepo::new();
    repo.write_config(r#"
[tool.precommit]
hooks = ["exit 1"]
"#);
    let (code, stdout, _) = repo.run_cmd(&["run"]);
    assert_ne!(code, 0);
    assert!(stdout.contains("failed"));
}

// ── Install / uninstall ─────────────────────────────────────

#[test]
fn install_creates_hook_file() {
    let repo = TempRepo::new();
    repo.write_config(r#"
[tool.precommit]
hooks = ["echo hi"]
"#);
    repo.run_cmd(&["install"]);

    let hook = repo.path().join(".git/hooks/pre-commit");
    assert!(hook.exists());
    let content = fs::read_to_string(&hook).unwrap();
    assert!(content.contains("precommit"));
}

#[test]
fn install_is_idempotent() {
    let repo = TempRepo::new();
    repo.write_config(r#"
[tool.precommit]
hooks = ["echo hi"]
"#);
    repo.run_cmd(&["install"]);
    let (_, stdout, _) = repo.run_cmd(&["install"]);
    assert!(stdout.contains("already installed"));
}

#[test]
fn install_backs_up_existing_hook() {
    let repo = TempRepo::new();
    repo.write_config(r#"
[tool.precommit]
hooks = ["echo hi"]
"#);
    let hooks_dir = repo.path().join(".git/hooks");
    fs::create_dir_all(&hooks_dir).unwrap();
    fs::write(hooks_dir.join("pre-commit"), "#!/bin/sh\necho old\n").unwrap();

    let (_, stdout, _) = repo.run_cmd(&["install"]);
    assert!(stdout.contains("backed up"));
    assert!(hooks_dir.join("pre-commit.legacy").exists());
}

#[test]
fn install_detects_stages() {
    let repo = TempRepo::new();
    repo.write_config(r#"
[tool.precommit]
hooks = [
    { name = "lint", run = "echo lint" },
    { name = "test", run = "echo test", stages = ["pre-push"] },
]
"#);
    repo.run_cmd(&["install"]);

    assert!(repo.path().join(".git/hooks/pre-commit").exists());
    assert!(repo.path().join(".git/hooks/pre-push").exists());
}

#[test]
fn uninstall_removes_hook() {
    let repo = TempRepo::new();
    repo.write_config(r#"
[tool.precommit]
hooks = ["echo hi"]
"#);
    repo.run_cmd(&["install"]);
    repo.run_cmd(&["uninstall"]);

    let hook = repo.path().join(".git/hooks/pre-commit");
    assert!(!hook.exists());
}

#[test]
fn uninstall_restores_legacy_hook() {
    let repo = TempRepo::new();
    repo.write_config(r#"
[tool.precommit]
hooks = ["echo hi"]
"#);
    let hooks_dir = repo.path().join(".git/hooks");
    fs::create_dir_all(&hooks_dir).unwrap();
    fs::write(hooks_dir.join("pre-commit"), "#!/bin/sh\necho old\n").unwrap();

    repo.run_cmd(&["install"]);
    repo.run_cmd(&["uninstall"]);

    let content = fs::read_to_string(hooks_dir.join("pre-commit")).unwrap();
    assert!(content.contains("echo old"));
}

#[test]
fn uninstall_ignores_non_precommit_hook() {
    let repo = TempRepo::new();
    let hooks_dir = repo.path().join(".git/hooks");
    fs::create_dir_all(&hooks_dir).unwrap();
    fs::write(hooks_dir.join("pre-commit"), "#!/bin/sh\necho custom\n").unwrap();

    let (_, stdout, _) = repo.run_cmd(&["uninstall"]);
    assert!(stdout.contains("no hooks managed by precommit"));
    assert!(hooks_dir.join("pre-commit").exists());
}

// ── Run options ─────────────────────────────────────────────

#[test]
fn run_single_hook_by_name() {
    let repo = TempRepo::new();
    repo.write_config(r#"
[tool.precommit]
hooks = [
    { name = "a", run = "echo aaa" },
    { name = "b", run = "echo bbb" },
]
"#);
    let (code, stdout, _) = repo.run_cmd(&["run", "a"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("a"));
    assert!(!stdout.contains("b"));
}

#[test]
fn run_unknown_hook_errors() {
    let repo = TempRepo::new();
    repo.write_config(r#"
[tool.precommit]
hooks = [
    { name = "a", run = "echo hi" },
]
"#);
    let (code, _, stderr) = repo.run_cmd(&["run", "nope"]);
    assert_ne!(code, 0);
    assert!(stderr.contains("unknown hook"));
}

#[test]
fn stage_filtering() {
    let repo = TempRepo::new();
    repo.write_config(r#"
[tool.precommit]
hooks = [
    { name = "lint", run = "echo lint" },
    { name = "tests", run = "echo tests", stages = ["pre-push"] },
]
"#);

    let (code, stdout, _) = repo.run_cmd(&["run"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("lint"));
    assert!(!stdout.contains("tests"));

    let (code, stdout, _) = repo.run_cmd(&["run", "--stage", "pre-push"]);
    assert_eq!(code, 0);
    assert!(!stdout.contains("lint"));
    assert!(stdout.contains("tests"));
}

// ── Parallel ────────────────────────────────────────────────

#[test]
fn parallel_runs_all_hooks() {
    let repo = TempRepo::new();
    repo.write_config(r#"
[tool.precommit]
parallel = true
hooks = ["echo one", "echo two", "echo three"]
"#);
    let (code, stdout, _) = repo.run_cmd(&["run"]);
    assert_eq!(code, 0);
    assert_eq!(stdout.matches("passed").count(), 3);
}

#[test]
fn parallel_reports_failure() {
    let repo = TempRepo::new();
    repo.write_config(r#"
[tool.precommit]
parallel = true
hooks = [
    { name = "good", run = "echo ok" },
    { name = "bad", run = "exit 1" },
]
"#);
    let (code, stdout, _) = repo.run_cmd(&["run"]);
    assert_ne!(code, 0);
    assert!(stdout.contains("good"));
    assert!(stdout.contains("bad"));
    assert!(stdout.contains("failed"));
}
