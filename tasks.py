from __future__ import annotations

from pathlib import Path
import re

from invoke import Collection, Exit, task

WARNING_PATTERN = re.compile(r"^\s*warning(?:\[|:)", re.IGNORECASE)
MAX_FAILURE_LINES = 5
MANIFEST_PATH = "Cargo.toml"


def _warning_lines(output: str) -> list[str]:
    return [line.strip() for line in output.splitlines() if WARNING_PATTERN.match(line)]


def _failure_summary(output: str) -> str:
    lines = [line.strip() for line in output.splitlines() if line.strip()]
    if not lines:
        return "no output captured"
    return " | ".join(lines[-MAX_FAILURE_LINES:])


def _run_step(ctx, label: str, command: str) -> dict[str, object]:
    result = ctx.run(command, warn=True, hide=True)
    output = "\n".join(part for part in (result.stdout, result.stderr) if part)
    return {
        "label": label,
        "command": command,
        "ok": result.ok,
        "warnings": _warning_lines(output),
        "failure": _failure_summary(output) if not result.ok else None,
        "exit_code": result.exited,
    }


def _print_summary(name: str, results: list[dict[str, object]]) -> None:
    print(f"{name} summary")
    print("Warnings:")
    warning_results = [result for result in results if result["warnings"]]
    if warning_results:
        for result in warning_results:
            warnings = result["warnings"]
            print(f"- {result['label']}: {len(warnings)} warning(s)")
    else:
        print("- none")

    print("Failures:")
    failed_results = [result for result in results if not result["ok"]]
    if failed_results:
        for result in failed_results:
            print(
                f"- {result['label']} (exit {result['exit_code']}): {result['failure']}"
            )
    else:
        print("- none")

    successful_tasks = sum(1 for result in results if result["ok"])
    print(f"Successful tasks: {successful_tasks}")

    if failed_results:
        raise Exit(code=1)


def _run_group(ctx, name: str, steps: list[tuple[str, str]]) -> None:
    results = [_run_step(ctx, label, command) for label, command in steps]
    _print_summary(name, results)


def _lockfile_path() -> Path | None:
    for candidate in (Path("Cargo.lock"), Path("..") / "Cargo.lock"):
        if candidate.exists():
            return candidate
    return None


@task
def build(ctx):
    """Run the build and lint checks with a terse summary."""
    _run_group(
        ctx,
        "Build",
        [
            (
                "cargo build",
                f"cargo build --manifest-path {MANIFEST_PATH} --all-targets",
            ),
            (
                "cargo fmt",
                f"cargo fmt --manifest-path {MANIFEST_PATH} -- --check",
            ),
            (
                "cargo clippy",
                f"cargo clippy --manifest-path {MANIFEST_PATH} --all-targets --all-features -- -D warnings",
            ),
        ],
    )


@task
def test(ctx):
    """Run the test suite with a terse summary."""
    _run_group(
        ctx,
        "Test",
        [("cargo test", f"cargo test --manifest-path {MANIFEST_PATH}")],
    )


@task
def security(ctx):
    """Run security checks with a terse summary."""
    results = []
    if _lockfile_path() is None:
        results.append(
            _run_step(
                ctx,
                "cargo generate-lockfile",
                f"cargo generate-lockfile --manifest-path {MANIFEST_PATH}",
            )
        )

    lockfile = _lockfile_path()
    audit_command = "cargo audit"
    if lockfile is not None:
        audit_command = f"cargo audit --file {lockfile}"

    results.append(_run_step(ctx, "cargo audit", audit_command))
    _print_summary("Security", results)


ns = Collection()
ns.add_task(build)
ns.add_task(test)
ns.add_task(security)
