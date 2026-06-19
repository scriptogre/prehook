"""Install/uninstall the prehook git hook.

The hook itself (hook.sh) is self-contained POSIX sh: it reads [tool.prehook]
from pyproject.toml and runs the hooks at commit time. This module only writes
that script into the repo's hook paths, so committing never needs prehook, and
editing the hook list never needs a reinstall.
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
from importlib.resources import files
from pathlib import Path

from . import __version__

STAGES = (
    "pre-commit",
    "pre-push",
    "commit-msg",
    "pre-rebase",
    "post-merge",
    "post-checkout",
    "prepare-commit-msg",
)

CONFIG_TEMPLATE = """
[tool.prehook]
hooks = [
    "echo 'hello from prehook'",
]
"""

HOOK_SCRIPT = files(__package__).joinpath("hook.sh").read_text(encoding="utf-8")


class PrehookError(Exception):
    """A user-facing failure; reported as a single line, exits non-zero."""


# ── output ──────────────────────────────────────────────────


def _styled(code: str, text: str) -> str:
    if os.environ.get("NO_COLOR"):
        return text
    forced = os.environ.get("FORCE_COLOR") or os.environ.get("CLICOLOR_FORCE")
    if not (forced or sys.stdout.isatty()):
        return text
    return f"\033[{code}m{text}\033[0m"


def _check(message: str) -> None:
    print(f"{_styled('32', '✓')} {message}")


def _skip(message: str) -> None:
    print(_styled("2", f"↷ {message}"))


def _error(message: str) -> None:
    print(f"{_styled('31', '✗')} {message}", file=sys.stderr)


# ── git / config discovery ──────────────────────────────────


def find_pyproject() -> Path:
    cwd = Path.cwd()
    for directory in (cwd, *cwd.parents):
        candidate = directory / "pyproject.toml"
        if candidate.exists():
            return candidate
    raise PrehookError("no pyproject.toml found")


def git_hooks_dir() -> Path:
    result = subprocess.run(
        ["git", "rev-parse", "--git-dir"], capture_output=True, text=True
    )
    if result.returncode != 0:
        raise PrehookError("not a git repository")
    return Path(result.stdout.strip()) / "hooks"


# ── commands ────────────────────────────────────────────────


def install(force: bool) -> None:
    pyproject = find_pyproject()
    hooks = git_hooks_dir()

    if "[tool.prehook]" in pyproject.read_text(encoding="utf-8"):
        _check("pyproject.toml already has [tool.prehook]")
    else:
        with pyproject.open("a", encoding="utf-8") as handle:
            handle.write(CONFIG_TEMPLATE)
        _check("added [tool.prehook] to pyproject.toml")

    hooks.mkdir(parents=True, exist_ok=True)
    for stage in STAGES:
        _install_stage(hooks, stage, force)

    _check("git hooks installed")


def _install_stage(hooks: Path, stage: str, force: bool) -> None:
    path = hooks / stage

    if path.exists() and "prehook" not in path.read_text(encoding="utf-8"):
        if not force:
            raise PrehookError(
                f"{stage} hook already exists (not managed by prehook). "
                "Use --force to overwrite"
            )
        path.rename(path.with_name(f"{stage}.backup"))
        _check(f"backed up existing {stage} hook to {stage}.backup")

    # A prehook-managed hook is rewritten in place, so script updates apply
    # without uninstalling first.
    path.write_text(HOOK_SCRIPT, encoding="utf-8")
    path.chmod(0o755)


def uninstall() -> None:
    hooks = git_hooks_dir()
    if not hooks.exists():
        print("no hooks directory")
        return

    removed = restored = 0
    for stage in STAGES:
        path = hooks / stage
        if not path.exists() or "prehook" not in path.read_text(encoding="utf-8"):
            continue

        path.unlink()
        removed += 1

        backup = path.with_name(f"{stage}.backup")
        if backup.exists():
            backup.rename(path)
            restored += 1

    if removed:
        _check("git hooks removed")
        if restored:
            _check("previous hooks restored")
    else:
        _skip("no hooks managed by prehook")


# ── entry point ─────────────────────────────────────────────


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="prehook", description="git hooks from pyproject.toml"
    )
    parser.add_argument("--version", action="version", version=f"prehook {__version__}")
    commands = parser.add_subparsers(dest="command", required=True)

    install_cmd = commands.add_parser("install", help="Install git hooks")
    install_cmd.add_argument(
        "-f", "--force", action="store_true", help="overwrite existing git hooks"
    )
    commands.add_parser("uninstall", help="Remove all prehook-managed git hooks")

    args = parser.parse_args(argv)
    try:
        if args.command == "install":
            install(args.force)
        else:
            uninstall()
    except PrehookError as exc:
        _error(str(exc))
        return 1
    return 0
