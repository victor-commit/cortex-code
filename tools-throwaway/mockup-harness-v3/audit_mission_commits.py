#!/usr/bin/env python3
"""Audit explicit mission-ledger commits for permitted path scope."""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path
from typing import NoReturn

from isolation_snapshot import ALLOWED_PREFIXES, read_snapshot, validate_session_id

COMMIT_RE = re.compile(r"^[0-9a-f]{40,64}$")
AUTHORIZED_MISSION_ARTIFACTS = frozenset(
    {
        "tools-throwaway/mockup-harness-v3/.isolation_state/"
        "mission_commits.ledger",
    }
)
TRUSTED_BOOTSTRAP_COPY_SOURCE = {
    "c9410a0f5417d303945b9a2120c73a7a505c8a88": frozenset(
        {"tools-throwaway/mockup-harness/Cargo.lock"}
    )
}


def fail(message: str) -> NoReturn:
    raise SystemExit(f"audit_mission_commits.sh: {message}")


def run(repository: Path, *arguments: str) -> bytes:
    try:
        return subprocess.check_output(
            ["git", "-C", str(repository), *arguments],
            stderr=subprocess.PIPE,
        )
    except subprocess.CalledProcessError as error:
        detail = error.stderr.decode("utf-8", "replace").strip()
        fail(f"git {' '.join(arguments)} failed: {detail}")


def allowed(path: str) -> bool:
    return path.startswith(ALLOWED_PREFIXES) or path in AUTHORIZED_MISSION_ARTIFACTS


def allowed_for_commit(commit: str, path: str) -> bool:
    return allowed(path) or path in TRUSTED_BOOTSTRAP_COPY_SOURCE.get(commit, ())


def validate_active_session(snapshot_path: Path, expected_session_id: str) -> None:
    try:
        snapshot, _ = read_snapshot(snapshot_path)
    except SystemExit as error:
        fail(f"invalid active snapshot: {error}")
    if snapshot["session_id"] != validate_session_id(expected_session_id):
        fail("active snapshot is bound to a different worker session")


def read_ledger(ledger_path: Path) -> list[str]:
    if ledger_path.is_symlink() or not ledger_path.is_file():
        fail("missing mission commit ledger")
    try:
        content = ledger_path.read_text(encoding="ascii")
    except OSError as error:
        fail(f"cannot read mission commit ledger: {error}")
    except UnicodeDecodeError:
        fail("mission commit ledger is not ASCII")
    if content and not content.endswith("\n"):
        fail("mission commit ledger does not end with a newline")

    commits = content.splitlines()
    seen: set[str] = set()
    for commit in commits:
        if not COMMIT_RE.fullmatch(commit):
            fail(f"mission commit ledger contains an invalid SHA: {commit!r}")
        if commit in seen:
            fail(f"mission commit ledger contains a duplicate SHA: {commit}")
        seen.add(commit)
    return commits


def verify_reachable(repository: Path, commit: str) -> None:
    result = subprocess.run(
        ["git", "-C", str(repository), "rev-parse", "--verify", f"{commit}^{{commit}}"],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode != 0:
        fail(f"ledgered commit {commit} is unreachable")
    canonical = result.stdout.decode("ascii", "strict").strip()
    if canonical != commit:
        fail(f"ledgered commit {commit} is not a canonical commit SHA")

    result = subprocess.run(
        ["git", "-C", str(repository), "merge-base", "--is-ancestor", commit, "HEAD"],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode != 0:
        fail(f"ledgered commit {commit} is unreachable from HEAD")


def commit_paths(repository: Path, commit: str) -> list[str]:
    raw = run(
        repository,
        "diff-tree",
        "--root",
        "--no-commit-id",
        "--name-status",
        "-z",
        "-r",
        "-m",
        "--find-renames",
        "--find-copies",
        "--find-copies-harder",
        commit,
    )
    fields = raw.split(b"\0")
    paths: list[str] = []
    index = 0
    while index < len(fields):
        field = fields[index]
        index += 1
        if not field:
            continue
        status = field
        if index >= len(fields) or not fields[index]:
            fail(f"missing path in commit {commit}")
        path = fields[index]
        index += 1
        paths.append(path.decode("utf-8", "surrogateescape"))
        if status.startswith((b"R", b"C")):
            if index >= len(fields) or not fields[index]:
                fail(f"missing rename destination in commit {commit}")
            paths.append(fields[index].decode("utf-8", "surrogateescape"))
            index += 1
    return sorted(set(paths))


def audit(
    repository: Path,
    snapshot_path: Path,
    expected_session_id: str,
    ledger_path: Path,
    reported_commits: list[str],
) -> None:
    validate_active_session(snapshot_path, expected_session_id)
    ledgered_commits = read_ledger(ledger_path)
    ledgered = set(ledgered_commits)
    for commit in reported_commits:
        if not COMMIT_RE.fullmatch(commit):
            fail(f"reported mission commit has an invalid SHA: {commit!r}")
        if commit not in ledgered:
            fail(f"reported mission commit {commit} is absent from mission commit ledger")

    violations: list[dict[str, object]] = []
    for commit in ledgered_commits:
        verify_reachable(repository, commit)
        paths = commit_paths(repository, commit)
        invalid = [path for path in paths if not allowed_for_commit(commit, path)]
        if invalid:
            violation: dict[str, object] = {"commit": commit}
            violation["paths"] = invalid
            violations.append(violation)

    if violations:
        print(
            "audit_mission_commits.sh: mission commit path violations",
            file=sys.stderr,
        )
        print(json.dumps(violations, indent=2, sort_keys=True), file=sys.stderr)
        raise SystemExit(1)

    print(
        "audit_mission_commits.sh: OK, "
        f"{len(ledgered_commits)} ledgered mission commits audited"
    )


def main(arguments: list[str]) -> None:
    if len(arguments) < 4:
        raise SystemExit(
            "usage: audit_mission_commits.py <repository> <active-snapshot> "
            "<session-id> <mission-commit-ledger> [reported-commit ...]"
        )
    audit(Path(arguments[0]), Path(arguments[1]), arguments[2], Path(arguments[3]), arguments[4:])


if __name__ == "__main__":
    main(sys.argv[1:])
