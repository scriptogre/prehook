# `prehook`

[![CI](https://github.com/scriptogre/prehook/actions/workflows/ci.yml/badge.svg)](https://github.com/scriptogre/prehook/actions/workflows/ci.yml)
[![PyPI](https://img.shields.io/pypi/v/prehook)](https://pypi.org/project/prehook/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

Run git hooks as shell commands from `pyproject.toml`.

## Usage

1. Install:

    ```sh
    uvx prehook install
    ```
    _Adds `[tool.prehook]` to `pyproject.toml` (if missing) and installs git hooks. Use `--force` to overwrite existing hooks._

2. Edit the hooks in `pyproject.toml`:

    ```toml
    [tool.prehook]
    hooks = [
        "uvx ruff check --fix",
        "uvx ruff format",
    ]
    ```

3. `git commit -m "unfinished commit"`

To run hooks manually:

```sh
uvx prehook run
```

To uninstall:

```sh
uvx prehook uninstall
```

## Why?

I've used `pre-commit` for a long time, and it's a great tool. 

But for projects where I just need to run `ruff check` and `ruff format`, setting up a separate config file with repo URLs and rev hashes felt like too much.

So I made this. Hooks live in `pyproject.toml`, and they're just shell commands.


## Configuration

### Simple form

Commands run in order. If any command exits non-zero, the commit is blocked.

```toml
[tool.prehook]
hooks = [
    "uvx ruff check --fix",
    "uvx ruff format",
]
```

A single command works too:

```toml
[tool.prehook]
hooks = ["just lint"]
```

### Full form

For naming, stages, or per-hook options:

```toml
[tool.prehook]
hooks = [
    { name = "lint", run = "uvx ruff check --fix" },
    { name = "typecheck", run = "uvx pyright" },
    { name = "format", run = "uvx ruff format" },
]
```

| Key       | Default              | Description                      |
|-----------|----------------------|----------------------------------|
| `run`     | required             | Command to execute.              |
| `name`    | derived from command | Label for output and `SKIP`.     |
| `stages`  | `["pre-commit"]`     | Which git hook stages to run in. |
| `verbose` | `false`              | Show output even on success.     |

### Global options

```toml
[tool.prehook]
fail_fast = true
parallel = true
hooks = [...]
```

| Key         | Default | Description                 |
|-------------|---------|-----------------------------|
| `fail_fast` | `false` | Stop after first failure.   |
| `parallel`  | `false` | Run all hooks concurrently. |

### Parallel mode

When `parallel = true`, all hooks run at the same time. If two commands must run in order (e.g. fix then format), combine them:

```toml
[tool.prehook]
parallel = true
hooks = [
    { name = "lint+format", run = "uvx ruff check --fix && uvx ruff format" },
    { name = "typecheck", run = "uvx pyright" },
]
```

### Stages

Hooks run on `pre-commit` by default. To run on other git hooks (e.g. `pre-push`), set `stages`:

```toml
[tool.prehook]
hooks = [
    { name = "lint", run = "uvx ruff check" },
    { name = "test", run = "pytest", stages = ["pre-push"] },
]
```

`prehook init` installs hooks for all common git stages, so adding new stages to your config works without re-running init.

### Skipping hooks

Skip all hooks:

```sh
git commit --no-verify -m "wip"
```

Skip specific hooks by name:

```sh
SKIP=typecheck git commit -m "wip"
SKIP=lint,typecheck git commit -m "wip"
```

## Alternatives

If this tool doesn't do what you need, these are worth a look:

| Tool                                                 | Config                    | What it does well                                                         |
|------------------------------------------------------|---------------------------|---------------------------------------------------------------------------|
| [pre-commit](https://pre-commit.com)                 | `.pre-commit-config.yaml` | Huge ecosystem of ready-made hooks, multi-language virtualenv management. |
| [lefthook](https://github.com/evilmartians/lefthook) | `lefthook.yml`            | Fast, language-agnostic, great parallel execution.                        |
| [prek](https://github.com/j178/prek)                 | `prek.toml`               | Compatible with pre-commit configs, written in Rust, parallel execution.  |
