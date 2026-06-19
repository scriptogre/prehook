# `prehook`

[![CI](https://github.com/scriptogre/prehook/actions/workflows/ci.yml/badge.svg)](https://github.com/scriptogre/prehook/actions/workflows/ci.yml)
[![PyPI](https://img.shields.io/pypi/v/prehook)](https://pypi.org/project/prehook/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

Run git hooks configured in `pyproject.toml` using `uv`.

## Usage

1. Install (one-time):

    ```sh
    uvx prehook install
    ```

   _Use `--force` to overwrite existing git hooks._

   Installed hooks are self-contained `sh` (they read `pyproject.toml` directly), so committing never needs `prehook` itself, and editing the hook list needs no reinstall.

2. Update hooks in `pyproject.toml`:

    ```toml
    [tool.prehook]
    hooks = [
        "uvx ruff check",
        "uvx ruff format",
    ]
    ```

3. Commit:
    ```sh
    git commit -m "unfinished commit"
    ```

To uninstall:

```sh
uvx prehook uninstall
```

## Configuration

Each hook can have a name, target git hook type, and other options:

```toml
[tool.prehook]
hooks = [
    { name = "lint", run = "uvx ruff check --fix" },
    { name = "format", run = "uvx ruff format" },
    { name = "typecheck", run = "uvx pyright" },
    { name = "test", run = "pytest", on = ["pre-push"] },
]
```

| Key       | Default              | Description                     |
| --------- | -------------------- | ------------------------------- |
| `run`     | required             | Command to execute.             |
| `name`    | derived from command | Label for output and `SKIP`.    |
| `on`      | `["pre-commit"]`     | Which git [hook types](https://git-scm.com/docs/githooks#_hooks) to run on. |
| `verbose` | `false`              | Show output even on success.    |

`uvx prehook install` installs hooks for all supported git hook types, so adding new `on` values works without reinstalling.

### Skipping hooks

```sh
SKIP=typecheck git commit -m "wip"         # skip by name
SKIP=lint,typecheck git commit -m "wip"    # skip multiple
git commit --no-verify -m "wip"            # skip all
```

### Parallel and fail fast

```toml
[tool.prehook]
parallel = true
fail_fast = true
hooks = [
    { name = "lint+format", run = "uvx ruff check --fix && uvx ruff format" },
    { name = "typecheck", run = "uvx pyright" },
]
```

| Key         | Default | Description                 |
| ----------- | ------- | --------------------------- |
| `fail_fast` | `false` | Stop after first failure.   |
| `parallel`  | `false` | Run all hooks concurrently. |

## Why?

I used [`pre-commit`](https://pre-commit.com) for a long time, and it's a great tool.

But I prefer less configuration files, and for most projects I just want a quick:
```sh
uvx ruff check
uvx ruff format
```
Which are fast enough to always run on all files (rather than just staged ones).

## Alternatives

If this tool doesn't do what you need, these are worth a look:

| Tool                                                 | Config                    | What it does well                                                         |
|------------------------------------------------------|---------------------------|---------------------------------------------------------------------------|
| [pre-commit](https://pre-commit.com)                 | `.pre-commit-config.yaml` | Huge ecosystem of ready-made hooks, multi-language virtualenv management. |
| [lefthook](https://github.com/evilmartians/lefthook) | `lefthook.yml`            | Fast, language-agnostic, great parallel execution.                        |
| [prek](https://github.com/j178/prek)                 | `prek.toml`               | Compatible with pre-commit configs, written in Rust, parallel execution.  |