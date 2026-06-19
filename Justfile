build:
    uv build

run *args:
    uv run python -m prehook {{args}}

check:
    uvx ruff check
    uvx ruff format --check
    uv run pytest -q

test:
    uv run pytest -q

fmt:
    uvx ruff format

fix:
    uvx ruff check --fix

release version:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ ! "{{version}}" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
        echo "Error: version must be semver (e.g. 0.2.0), got '{{version}}'"
        exit 1
    fi
    if [ -n "$(git status --porcelain)" ]; then
        echo "Error: working tree is dirty. Commit or stash changes first."
        exit 1
    fi
    just check
    sed -i '' 's/^version = ".*"/version = "{{version}}"/' pyproject.toml
    git add pyproject.toml
    git commit -m "Release v{{version}}"
    git tag "v{{version}}"
    echo ""
    echo "Ready to publish. Run:"
    echo "  git push && git push --tags"
