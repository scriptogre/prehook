"""prehook: git hooks from pyproject.toml, run by a self-contained sh script."""

try:
    from importlib.metadata import PackageNotFoundError, version

    __version__ = version("prehook")
except (ImportError, PackageNotFoundError):  # not installed (e.g. run from source)
    __version__ = "0+unknown"
