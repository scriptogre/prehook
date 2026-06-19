"""Integration tests: drive the CLI, then execute the generated git hook."""

import os
import subprocess
import sys
from pathlib import Path

import pytest

SRC = str(Path(__file__).resolve().parent.parent / "src")


def _git(args, cwd):
    subprocess.run(["git", *args], cwd=cwd, capture_output=True, check=False)


@pytest.fixture
def repo(tmp_path):
    _git(["init"], tmp_path)
    _git(["config", "user.email", "test@example.com"], tmp_path)
    _git(["config", "user.name", "Test"], tmp_path)
    _git(["commit", "--allow-empty", "-m", "init"], tmp_path)
    return tmp_path


def write_config(repo: Path, toml: str) -> None:
    (repo / "pyproject.toml").write_text(toml)


def prehook(repo: Path, *args: str):
    """Run the prehook CLI (install / uninstall)."""
    env = {**os.environ, "NO_COLOR": "1", "PYTHONPATH": SRC}
    result = subprocess.run(
        [sys.executable, "-m", "prehook", *args],
        cwd=repo,
        env=env,
        capture_output=True,
        text=True,
    )
    return result.returncode, result.stdout, result.stderr


def fire(repo: Path, stage: str, env_extra=None, args=()):
    """Execute the installed hook for `stage` the way git would."""
    env = {**os.environ, "NO_COLOR": "1"}
    if env_extra:
        env.update(env_extra)
    hook = repo / ".git" / "hooks" / stage
    result = subprocess.run(
        ["sh", str(hook), *args], cwd=repo, env=env, capture_output=True, text=True
    )
    return result.returncode, result.stdout + result.stderr


def run_pre_commit(repo: Path):
    prehook(repo, "install")
    return fire(repo, "pre-commit")


# ── hook execution: forms ───────────────────────────────────


def test_simple_form_single_hook(repo):
    write_config(repo, '[tool.prehook]\nhooks = ["echo hello"]\n')
    code, out = run_pre_commit(repo)
    assert code == 0
    assert "✓" in out  # check mark


def test_simple_form_multiple_hooks(repo):
    write_config(repo, '[tool.prehook]\nhooks = ["echo hello", "echo world"]\n')
    code, out = run_pre_commit(repo)
    assert code == 0
    assert out.count("✓") == 2


def test_full_form_with_name(repo):
    write_config(
        repo, '[tool.prehook]\nhooks = [ { name = "greet", run = "echo hi" } ]\n'
    )
    code, out = run_pre_commit(repo)
    assert code == 0
    assert "greet" in out
    assert "✓" in out


def test_full_form_name_derived(repo):
    write_config(repo, '[tool.prehook]\nhooks = [ { run = "echo hi" } ]\n')
    code, out = run_pre_commit(repo)
    assert code == 0
    assert "echo hi" in out


def test_mixed_simple_and_full(repo):
    write_config(
        repo,
        '[tool.prehook]\nhooks = [ "echo hello", { name = "greet", run = "echo hi" } ]\n',
    )
    code, out = run_pre_commit(repo)
    assert code == 0
    assert "echo hello" in out
    assert "greet" in out


def test_no_hooks_is_silent_noop(repo):
    write_config(repo, "[tool.prehook]\nfail_fast = true\nhooks = []\n")
    code, out = run_pre_commit(repo)
    assert code == 0
    assert "✓" not in out


# ── SKIP + fail_fast + verbose ──────────────────────────────


def test_skip_env_skips_hooks(repo):
    write_config(
        repo, '[tool.prehook]\nhooks = [ { name = "fail", run = "exit 1" } ]\n'
    )
    prehook(repo, "install")
    code, out = fire(repo, "pre-commit", {"SKIP": "fail"})
    assert code == 0
    assert "↷" in out  # skip arrow


def test_skip_multiple_hooks(repo):
    write_config(
        repo,
        '[tool.prehook]\nhooks = [ { name = "a", run = "exit 1" }, '
        '{ name = "b", run = "exit 1" }, { name = "c", run = "echo ok" } ]\n',
    )
    prehook(repo, "install")
    code, out = fire(repo, "pre-commit", {"SKIP": "a,b"})
    assert code == 0
    assert out.count("↷") == 2
    assert "✓" in out


def test_verbose_shows_output_on_success(repo):
    write_config(
        repo,
        '[tool.prehook]\nhooks = [ { name = "loud", run = "echo hello world", verbose = true } ]\n',
    )
    code, out = run_pre_commit(repo)
    assert code == 0
    assert "✓" in out
    assert "hello world" in out


def test_fail_fast_stops_after_first_failure(repo):
    write_config(
        repo,
        '[tool.prehook]\nfail_fast = true\nhooks = [ { name = "bad", run = "exit 1" }, '
        '{ name = "never", run = "echo should not run" } ]\n',
    )
    code, out = run_pre_commit(repo)
    assert code != 0
    assert "bad" in out
    assert "never" not in out


def test_failing_hook_returns_nonzero(repo):
    write_config(repo, '[tool.prehook]\nhooks = ["exit 1"]\n')
    code, out = run_pre_commit(repo)
    assert code != 0
    assert "✗" in out  # cross mark


def test_failed_hook_shows_output(repo):
    write_config(
        repo,
        '[tool.prehook]\nhooks = [ { name = "broken", run = "echo something went wrong >&2; exit 1" } ]\n',
    )
    code, out = run_pre_commit(repo)
    assert code != 0
    assert "something went wrong" in out


# ── stage filtering ─────────────────────────────────────────


def test_stage_filtering(repo):
    write_config(
        repo,
        '[tool.prehook]\nhooks = [ { name = "lint", run = "echo lint" }, '
        '{ name = "tests", run = "echo tests", on = ["pre-push"] } ]\n',
    )
    prehook(repo, "install")

    code, out = fire(repo, "pre-commit")
    assert code == 0
    assert "lint" in out
    assert "tests" not in out

    code, out = fire(repo, "pre-push")
    assert code == 0
    assert "tests" in out
    assert "lint" not in out


# ── parallel ────────────────────────────────────────────────


def test_parallel_runs_all_hooks(repo):
    write_config(
        repo,
        '[tool.prehook]\nparallel = true\nhooks = ["echo one", "echo two", "echo three"]\n',
    )
    code, out = run_pre_commit(repo)
    assert code == 0
    assert out.count("✓") == 3


def test_parallel_reports_failure(repo):
    write_config(
        repo,
        '[tool.prehook]\nparallel = true\nhooks = [ { name = "good", run = "echo ok" }, '
        '{ name = "bad", run = "exit 1" } ]\n',
    )
    code, out = run_pre_commit(repo)
    assert code != 0
    assert "good" in out
    assert "bad" in out
    assert "✗" in out


def test_parallel_heavy_output_does_not_stall(repo):
    # ~120KB per hook; the hook captures to temp files, so no pipe to deadlock.
    write_config(
        repo,
        "[tool.prehook]\nparallel = true\nhooks = [ "
        '{ name = "big-a", run = "yes aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa | head -n 4000" }, '
        '{ name = "big-b", run = "yes bbbbbbbbbbbbbbbbbbbbbbbbbbbbbb | head -n 4000" } ]\n',
    )
    code, out = run_pre_commit(repo)
    assert code == 0
    assert out.count("✓") == 2


# ── summary ─────────────────────────────────────────────────


def test_summary_shown_for_multiple_hooks(repo):
    write_config(repo, '[tool.prehook]\nhooks = ["echo one", "echo two"]\n')
    code, out = run_pre_commit(repo)
    assert code == 0
    assert "2 passed" in out


def test_no_summary_for_single_hook(repo):
    write_config(repo, '[tool.prehook]\nhooks = ["echo one"]\n')
    code, out = run_pre_commit(repo)
    assert code == 0
    assert "passed" not in out


# ── git arg forwarding ──────────────────────────────────────


def test_git_args_forwarded_as_env(repo):
    write_config(
        repo,
        '[tool.prehook]\nhooks = [ { name = "args", run = "echo $PREHOOK_ARGS", '
        'verbose = true, on = ["commit-msg"] } ]\n',
    )
    prehook(repo, "install")
    code, out = fire(repo, "commit-msg", args=[".git/COMMIT_EDITMSG"])
    assert code == 0
    assert ".git/COMMIT_EDITMSG" in out


# ── install / uninstall ─────────────────────────────────────


def test_install_creates_hook_file(repo):
    write_config(repo, '[tool.prehook]\nhooks = ["echo hi"]\n')
    prehook(repo, "install")
    hook = repo / ".git/hooks/pre-commit"
    assert hook.exists()
    assert "prehook" in hook.read_text()


def test_install_is_idempotent(repo):
    write_config(repo, '[tool.prehook]\nhooks = ["echo hi"]\n')
    prehook(repo, "install")
    _, stdout, _ = prehook(repo, "install")
    assert "already has [tool.prehook]" in stdout
    assert "git hooks installed" in stdout


def test_install_refuses_existing_foreign_hook(repo):
    write_config(repo, '[tool.prehook]\nhooks = ["echo hi"]\n')
    hooks_dir = repo / ".git/hooks"
    hooks_dir.mkdir(parents=True, exist_ok=True)
    (hooks_dir / "pre-commit").write_text("#!/bin/sh\necho old\n")

    code, _, stderr = prehook(repo, "install")
    assert code != 0
    assert "already exists" in stderr
    assert "--force" in stderr


def test_install_force_backs_up_existing_hook(repo):
    write_config(repo, '[tool.prehook]\nhooks = ["echo hi"]\n')
    hooks_dir = repo / ".git/hooks"
    hooks_dir.mkdir(parents=True, exist_ok=True)
    (hooks_dir / "pre-commit").write_text("#!/bin/sh\necho old\n")

    code, stdout, _ = prehook(repo, "install", "--force")
    assert code == 0
    assert "backed up" in stdout
    assert (hooks_dir / "pre-commit.backup").exists()


def test_install_rewrites_managed_hook_in_place(repo):
    write_config(repo, '[tool.prehook]\nhooks = ["echo hi"]\n')
    prehook(repo, "install")
    code, _, _ = prehook(repo, "install")
    assert code == 0
    assert not (repo / ".git/hooks/pre-commit.backup").exists()


def test_install_installs_all_stages(repo):
    write_config(repo, '[tool.prehook]\nhooks = ["echo hi"]\n')
    _, stdout, _ = prehook(repo, "install")
    assert (repo / ".git/hooks/pre-commit").exists()
    assert (repo / ".git/hooks/pre-push").exists()
    assert (repo / ".git/hooks/commit-msg").exists()
    assert "git hooks installed" in stdout


def test_install_adds_config_to_existing_pyproject(repo):
    write_config(repo, '[project]\nname = "test"\n')
    prehook(repo, "install")
    content = (repo / "pyproject.toml").read_text()
    assert "[project]" in content
    assert "[tool.prehook]" in content
    assert (repo / ".git/hooks/pre-commit").exists()


def test_install_errors_without_pyproject(repo):
    code, _, stderr = prehook(repo, "install")
    assert code != 0
    assert "no pyproject.toml" in stderr


def test_install_hook_forwards_args(repo):
    write_config(repo, '[tool.prehook]\nhooks = ["echo hi"]\n')
    prehook(repo, "install")
    content = (repo / ".git/hooks/pre-commit").read_text()
    assert '"$@"' in content


def test_uninstall_removes_hook(repo):
    write_config(repo, '[tool.prehook]\nhooks = ["echo hi"]\n')
    prehook(repo, "install")
    prehook(repo, "uninstall")
    assert not (repo / ".git/hooks/pre-commit").exists()


def test_uninstall_restores_legacy_hook(repo):
    write_config(repo, '[tool.prehook]\nhooks = ["echo hi"]\n')
    hooks_dir = repo / ".git/hooks"
    hooks_dir.mkdir(parents=True, exist_ok=True)
    (hooks_dir / "pre-commit").write_text("#!/bin/sh\necho old\n")

    prehook(repo, "install", "--force")
    prehook(repo, "uninstall")

    assert "echo old" in (hooks_dir / "pre-commit").read_text()


def test_uninstall_ignores_non_prehook_hook(repo):
    hooks_dir = repo / ".git/hooks"
    hooks_dir.mkdir(parents=True, exist_ok=True)
    (hooks_dir / "pre-commit").write_text("#!/bin/sh\necho custom\n")

    _, stdout, _ = prehook(repo, "uninstall")
    assert "no hooks managed by prehook" in stdout
    assert (hooks_dir / "pre-commit").exists()


# ── prehook run (manual / CI) ───────────────────────────────


def test_run_executes_hooks_without_install(repo):
    write_config(repo, '[tool.prehook]\nhooks = ["echo hello"]\n')
    code, stdout, _ = prehook(repo, "run")  # note: no install
    assert code == 0
    assert "✓" in stdout


def test_run_on_stage(repo):
    write_config(
        repo,
        '[tool.prehook]\nhooks = [ { name = "lint", run = "echo lint" }, '
        '{ name = "tests", run = "echo tests", on = ["pre-push"] } ]\n',
    )
    _, out, _ = prehook(repo, "run")
    assert "lint" in out
    assert "tests" not in out

    _, out, _ = prehook(repo, "run", "--on", "pre-push")
    assert "tests" in out
    assert "lint" not in out


def test_run_forwards_args(repo):
    write_config(
        repo,
        '[tool.prehook]\nhooks = [ { name = "args", run = "echo $PREHOOK_ARGS", '
        'verbose = true, on = ["commit-msg"] } ]\n',
    )
    code, out, _ = prehook(repo, "run", "--on", "commit-msg", ".git/COMMIT_EDITMSG")
    assert code == 0
    assert ".git/COMMIT_EDITMSG" in out


def test_run_returns_nonzero_on_failure(repo):
    write_config(repo, '[tool.prehook]\nhooks = ["exit 1"]\n')
    code, out, _ = prehook(repo, "run")
    assert code != 0
    assert "✗" in out
