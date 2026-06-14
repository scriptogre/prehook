use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_prehook"))
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
    repo.write_config(
        r#"
[tool.prehook]
hooks = ["echo hello"]
"#,
    );
    let (code, stdout, _) = repo.run_cmd(&["run"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("\u{2713}")); // ✓
}

#[test]
fn simple_form_multiple_hooks() {
    let repo = TempRepo::new();
    repo.write_config(
        r#"
[tool.prehook]
hooks = ["echo hello", "echo world"]
"#,
    );
    let (code, stdout, _) = repo.run_cmd(&["run"]);
    assert_eq!(code, 0);
    assert_eq!(stdout.matches("\u{2713}").count(), 2);
}

#[test]
fn full_form_with_name() {
    let repo = TempRepo::new();
    repo.write_config(
        r#"
[tool.prehook]
hooks = [
    { name = "greet", run = "echo hi" },
]
"#,
    );
    let (code, stdout, _) = repo.run_cmd(&["run"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("greet"));
    assert!(stdout.contains("\u{2713}"));
}

#[test]
fn full_form_name_derived_from_command() {
    let repo = TempRepo::new();
    repo.write_config(
        r#"
[tool.prehook]
hooks = [
    { run = "echo hi" },
]
"#,
    );
    let (code, stdout, _) = repo.run_cmd(&["run"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("echo hi"));
    assert!(stdout.contains("\u{2713}"));
}

#[test]
fn mixed_simple_and_full() {
    let repo = TempRepo::new();
    repo.write_config(
        r#"
[tool.prehook]
hooks = [
    "echo hello",
    { name = "greet", run = "echo hi" },
]
"#,
    );
    let (code, stdout, _) = repo.run_cmd(&["run"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("echo hello"));
    assert!(stdout.contains("greet"));
}

#[test]
fn errors_when_no_hooks() {
    let repo = TempRepo::new();
    repo.write_config(
        r#"
[tool.prehook]
fail_fast = true
"#,
    );
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
    assert!(stderr.contains("no [tool.prehook]"));
}

#[test]
fn errors_when_no_pyproject() {
    let repo = TempRepo::new();
    let (code, _, stderr) = repo.run_cmd(&["run"]);
    assert_ne!(code, 0);
    assert!(stderr.contains("no pyproject.toml"));
}

// ── SKIP + fail_fast ────────────────────────────────────────

#[test]
fn skip_env_skips_hooks() {
    let repo = TempRepo::new();
    repo.write_config(
        r#"
[tool.prehook]
hooks = [
    { name = "fail", run = "exit 1" },
]
"#,
    );
    let out = Command::new(binary())
        .args(["run"])
        .current_dir(repo.path())
        .env("NO_COLOR", "1")
        .env("SKIP", "fail")
        .output()
        .unwrap();
    assert_eq!(out.status.code().unwrap(), 0);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("\u{21b7}")); // ↷
}

#[test]
fn skip_multiple_hooks() {
    let repo = TempRepo::new();
    repo.write_config(
        r#"
[tool.prehook]
hooks = [
    { name = "a", run = "exit 1" },
    { name = "b", run = "exit 1" },
    { name = "c", run = "echo ok" },
]
"#,
    );
    let out = Command::new(binary())
        .args(["run"])
        .current_dir(repo.path())
        .env("NO_COLOR", "1")
        .env("SKIP", "a,b")
        .output()
        .unwrap();
    assert_eq!(out.status.code().unwrap(), 0);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout.matches("\u{21b7}").count(), 2);
    assert!(stdout.contains("\u{2713}"));
}

#[test]
fn verbose_shows_output_on_success() {
    let repo = TempRepo::new();
    repo.write_config(
        r#"
[tool.prehook]
hooks = [
    { name = "loud", run = "echo hello world", verbose = true },
]
"#,
    );
    let (code, stdout, _) = repo.run_cmd(&["run"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("\u{2713}"));
    assert!(stdout.contains("hello world"));
}

#[test]
fn fail_fast_stops_after_first_failure() {
    let repo = TempRepo::new();
    repo.write_config(
        r#"
[tool.prehook]
fail_fast = true
hooks = [
    { name = "bad", run = "exit 1" },
    { name = "never", run = "echo should not run" },
]
"#,
    );
    let (code, stdout, _) = repo.run_cmd(&["run"]);
    assert_ne!(code, 0);
    assert!(stdout.contains("bad"));
    assert!(!stdout.contains("never"));
}

#[test]
fn failing_hook_returns_nonzero() {
    let repo = TempRepo::new();
    repo.write_config(
        r#"
[tool.prehook]
hooks = ["exit 1"]
"#,
    );
    let (code, stdout, _) = repo.run_cmd(&["run"]);
    assert_ne!(code, 0);
    assert!(stdout.contains("\u{2717}")); // ✗
}

#[test]
fn failed_hook_shows_output() {
    let repo = TempRepo::new();
    repo.write_config(
        r#"
[tool.prehook]
hooks = [
    { name = "broken", run = "echo 'something went wrong' >&2; exit 1" },
]
"#,
    );
    let (code, stdout, _) = repo.run_cmd(&["run"]);
    assert_ne!(code, 0);
    assert!(stdout.contains("something went wrong"));
}

// ── Init / uninstall ────────────────────────────────────────

#[test]
fn install_creates_hook_file() {
    let repo = TempRepo::new();
    repo.write_config(
        r#"
[tool.prehook]
hooks = ["echo hi"]
"#,
    );
    repo.run_cmd(&["install"]);

    let hook = repo.path().join(".git/hooks/pre-commit");
    assert!(hook.exists());
    let content = fs::read_to_string(&hook).unwrap();
    assert!(content.contains("prehook"));
}

#[test]
fn install_is_idempotent() {
    let repo = TempRepo::new();
    repo.write_config(
        r#"
[tool.prehook]
hooks = ["echo hi"]
"#,
    );
    repo.run_cmd(&["install"]);
    let (_, stdout, _) = repo.run_cmd(&["install"]);
    assert!(stdout.contains("already has [tool.prehook]"));
    assert!(stdout.contains("git hooks installed"));
}

#[test]
fn install_refuses_existing_foreign_hook() {
    let repo = TempRepo::new();
    repo.write_config(
        r#"
[tool.prehook]
hooks = ["echo hi"]
"#,
    );
    let hooks_dir = repo.path().join(".git/hooks");
    fs::create_dir_all(&hooks_dir).unwrap();
    fs::write(hooks_dir.join("pre-commit"), "#!/bin/sh\necho old\n").unwrap();

    let (code, _, stderr) = repo.run_cmd(&["install"]);
    assert_ne!(code, 0);
    assert!(stderr.contains("already exists"));
    assert!(stderr.contains("--force"));
}

#[test]
fn install_force_backs_up_existing_hook() {
    let repo = TempRepo::new();
    repo.write_config(
        r#"
[tool.prehook]
hooks = ["echo hi"]
"#,
    );
    let hooks_dir = repo.path().join(".git/hooks");
    fs::create_dir_all(&hooks_dir).unwrap();
    fs::write(hooks_dir.join("pre-commit"), "#!/bin/sh\necho old\n").unwrap();

    let (code, stdout, _) = repo.run_cmd(&["install", "--force"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("backed up"));
    assert!(hooks_dir.join("pre-commit.backup").exists());
}

#[test]
fn install_installs_all_stages() {
    let repo = TempRepo::new();
    repo.write_config(
        r#"
[tool.prehook]
hooks = ["echo hi"]
"#,
    );
    let (_, stdout, _) = repo.run_cmd(&["install"]);

    assert!(repo.path().join(".git/hooks/pre-commit").exists());
    assert!(repo.path().join(".git/hooks/pre-push").exists());
    assert!(repo.path().join(".git/hooks/commit-msg").exists());
    assert!(stdout.contains("git hooks installed"));
}

#[test]
fn install_adds_config_to_existing_pyproject() {
    let repo = TempRepo::new();
    repo.write_config("[project]\nname = \"test\"\n");
    repo.run_cmd(&["install"]);

    let content = fs::read_to_string(repo.path().join("pyproject.toml")).unwrap();
    assert!(content.contains("[project]"));
    assert!(content.contains("[tool.prehook]"));
    assert!(repo.path().join(".git/hooks/pre-commit").exists());
}

#[test]
fn uninstall_removes_hook() {
    let repo = TempRepo::new();
    repo.write_config(
        r#"
[tool.prehook]
hooks = ["echo hi"]
"#,
    );
    repo.run_cmd(&["install"]);
    repo.run_cmd(&["uninstall"]);

    let hook = repo.path().join(".git/hooks/pre-commit");
    assert!(!hook.exists());
}

#[test]
fn uninstall_restores_legacy_hook() {
    let repo = TempRepo::new();
    repo.write_config(
        r#"
[tool.prehook]
hooks = ["echo hi"]
"#,
    );
    let hooks_dir = repo.path().join(".git/hooks");
    fs::create_dir_all(&hooks_dir).unwrap();
    fs::write(hooks_dir.join("pre-commit"), "#!/bin/sh\necho old\n").unwrap();

    repo.run_cmd(&["install"]);
    repo.run_cmd(&["uninstall"]);

    let content = fs::read_to_string(hooks_dir.join("pre-commit")).unwrap();
    assert!(content.contains("echo old"));
}

#[test]
fn uninstall_ignores_non_prehook_hook() {
    let repo = TempRepo::new();
    let hooks_dir = repo.path().join(".git/hooks");
    fs::create_dir_all(&hooks_dir).unwrap();
    fs::write(hooks_dir.join("pre-commit"), "#!/bin/sh\necho custom\n").unwrap();

    let (_, stdout, _) = repo.run_cmd(&["uninstall"]);
    assert!(stdout.contains("no hooks managed by prehook"));
    assert!(hooks_dir.join("pre-commit").exists());
}

// ── Run options ─────────────────────────────────────────────

#[test]
fn run_single_hook_by_name() {
    let repo = TempRepo::new();
    repo.write_config(
        r#"
[tool.prehook]
hooks = [
    { name = "a", run = "echo aaa" },
    { name = "b", run = "echo bbb" },
]
"#,
    );
    let (code, stdout, _) = repo.run_cmd(&["run", "a"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("a"));
    assert!(!stdout.contains("b"));
}

#[test]
fn run_unknown_hook_errors() {
    let repo = TempRepo::new();
    repo.write_config(
        r#"
[tool.prehook]
hooks = [
    { name = "a", run = "echo hi" },
]
"#,
    );
    let (code, _, stderr) = repo.run_cmd(&["run", "nope"]);
    assert_ne!(code, 0);
    assert!(stderr.contains("unknown hook"));
}

#[test]
fn stage_filtering() {
    let repo = TempRepo::new();
    repo.write_config(
        r#"
[tool.prehook]
hooks = [
    { name = "lint", run = "echo lint" },
    { name = "tests", run = "echo tests", on = ["pre-push"] },
]
"#,
    );

    let (code, stdout, _) = repo.run_cmd(&["run"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("lint"));
    assert!(!stdout.contains("tests"));

    let (code, stdout, _) = repo.run_cmd(&["run", "--on", "pre-push"]);
    assert_eq!(code, 0);
    assert!(!stdout.contains("lint"));
    assert!(stdout.contains("tests"));
}

// ── Named hook across stages ───────────────────────────────

#[test]
fn named_hook_runs_regardless_of_stage() {
    let repo = TempRepo::new();
    repo.write_config(
        r#"
[tool.prehook]
hooks = [
    { name = "push-check", run = "echo pushed", on = ["pre-push"] },
]
"#,
    );
    let (code, stdout, _) = repo.run_cmd(&["run", "push-check"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("push-check"));
}

// ── Git arg forwarding ─────────────────────────────────────

#[test]
fn git_args_forwarded_as_env() {
    let repo = TempRepo::new();
    repo.write_config(
        r#"
[tool.prehook]
hooks = [
    { name = "args", run = "echo $PREHOOK_ARGS", verbose = true, on = ["commit-msg"] },
]
"#,
    );
    let (code, stdout, _) =
        repo.run_cmd(&["run", "--on", "commit-msg", "--", ".git/COMMIT_EDITMSG"]);
    assert_eq!(code, 0);
    assert!(stdout.contains(".git/COMMIT_EDITMSG"));
}

#[test]
fn install_hook_forwards_args() {
    let repo = TempRepo::new();
    repo.write_config(
        r#"
[tool.prehook]
hooks = ["echo hi"]
"#,
    );
    repo.run_cmd(&["install"]);

    let hook = repo.path().join(".git/hooks/pre-commit");
    let content = fs::read_to_string(&hook).unwrap();
    assert!(content.contains("\"$@\""));
}

// ── Parallel ────────────────────────────────────────────────

#[test]
fn parallel_runs_all_hooks() {
    let repo = TempRepo::new();
    repo.write_config(
        r#"
[tool.prehook]
parallel = true
hooks = ["echo one", "echo two", "echo three"]
"#,
    );
    let (code, stdout, _) = repo.run_cmd(&["run"]);
    assert_eq!(code, 0);
    assert_eq!(stdout.matches("\u{2713}").count(), 3);
}

#[test]
fn parallel_reports_failure() {
    let repo = TempRepo::new();
    repo.write_config(
        r#"
[tool.prehook]
parallel = true
hooks = [
    { name = "good", run = "echo ok" },
    { name = "bad", run = "exit 1" },
]
"#,
    );
    let (code, stdout, _) = repo.run_cmd(&["run"]);
    assert_ne!(code, 0);
    assert!(stdout.contains("good"));
    assert!(stdout.contains("bad"));
    assert!(stdout.contains("\u{2717}"));
}

#[test]
fn parallel_heavy_output_does_not_stall() {
    let repo = TempRepo::new();
    // Each hook writes ~100KB, well over the ~64KB pipe buffer
    repo.write_config(
        r#"
[tool.prehook]
parallel = true
hooks = [
    { name = "big-a", run = "dd if=/dev/zero bs=1024 count=100 2>/dev/null | tr '\\0' 'a'" },
    { name = "big-b", run = "dd if=/dev/zero bs=1024 count=100 2>/dev/null | tr '\\0' 'b'" },
]
"#,
    );
    let (code, stdout, _) = repo.run_cmd(&["run"]);
    assert_eq!(code, 0);
    assert_eq!(stdout.matches("\u{2713}").count(), 2);
}

// ── Summary ─────────────────────────────────────────────────

#[test]
fn summary_shown_for_multiple_hooks() {
    let repo = TempRepo::new();
    repo.write_config(
        r#"
[tool.prehook]
hooks = ["echo one", "echo two"]
"#,
    );
    let (code, stdout, _) = repo.run_cmd(&["run"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("2 passed"));
}

#[test]
fn no_summary_for_single_hook() {
    let repo = TempRepo::new();
    repo.write_config(
        r#"
[tool.prehook]
hooks = ["echo one"]
"#,
    );
    let (code, stdout, _) = repo.run_cmd(&["run"]);
    assert_eq!(code, 0);
    assert!(!stdout.contains("passed"));
}
