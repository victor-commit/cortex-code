#!/usr/bin/env python3
"""Audit mission-authored commits for path scope and identity."""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path
from typing import NoReturn

from isolation_snapshot import ALLOWED_PREFIXES, read_snapshot, validate_session_id

MISSION_NAME = "echobt"
MISSION_EMAIL = "154886644+echobt@users.noreply.github.com"
AUDIT_ORIGIN_FIELDS = {"schema_version", "origin_kind", "origin_commit"}


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
    return path.startswith(ALLOWED_PREFIXES)


def validate_active_session(snapshot_path: Path, expected_session_id: str) -> None:
    try:
        snapshot, _ = read_snapshot(snapshot_path)
    except SystemExit as error:
        fail(f"invalid active snapshot: {error}")
    if snapshot["session_id"] != validate_session_id(expected_session_id):
        fail("active snapshot is bound to a different worker session")


def audit_origin(repository: Path, origin_path: Path) -> str:
    try:
        origin = json.loads(origin_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        fail(f"invalid persistent audit origin: {error}")
    if (
        not isinstance(origin, dict)
        or set(origin) != AUDIT_ORIGIN_FIELDS
        or origin.get("schema_version") != 1
        or origin.get("origin_kind") != "mission-audit"
        or not isinstance(origin.get("origin_commit"), str)
    ):
        fail("persistent audit origin has an invalid schema")

    start = origin["origin_commit"]
    if (
        len(start) not in (40, 64)
        or any(character not in "0123456789abcdef" for character in start)
    ):
        fail("persistent audit origin has an invalid commit")
    run(repository, "rev-parse", "--verify", f"{start}^{{commit}}")
    run(repository, "merge-base", "--is-ancestor", start, "HEAD")
    return start


def mission_commits(repository: Path, start: str) -> list[str]:
    return [
        commit
        for commit in run(repository, "rev-list", "--reverse", f"{start}..HEAD")
        .decode("ascii")
        .splitlines()
        if commit
    ]


def identity(repository: Path, commit: str) -> tuple[str, str, str, str]:
    values = run(
        repository,
        "show",
        "-s",
        "--format=%an%x00%ae%x00%cn%x00%ce",
        commit,
    ).decode("utf-8", "replace").rstrip("\n").split("\0")
    if len(values) != 4:
        fail(f"invalid identity for commit {commit}")
    return tuple(values)  # type: ignore[return-value]


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
    audit_origin_path: Path,
) -> None:
    validate_active_session(snapshot_path, expected_session_id)
    start = audit_origin(repository, audit_origin_path)
    audited = 0
    violations: list[dict[str, object]] = []
    for commit in mission_commits(repository, start):
        author_name, author_email, committer_name, committer_email = identity(
            repository, commit
        )
        author_is_mission = (author_name, author_email) == (
            MISSION_NAME,
            MISSION_EMAIL,
        )
        committer_is_mission = (committer_name, committer_email) == (
            MISSION_NAME,
            MISSION_EMAIL,
        )
        if not author_is_mission and not committer_is_mission:
            continue
        audited += 1
        paths = commit_paths(repository, commit)
        invalid = [path for path in paths if not allowed(path)]
        identity_violation = not (author_is_mission and committer_is_mission)
        if invalid or identity_violation:
            violation: dict[str, object] = {"commit": commit}
            if invalid:
                violation["paths"] = invalid
            if identity_violation:
                violation["identity"] = {
                    "author": [author_name, author_email],
                    "committer": [committer_name, committer_email],
                }
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
        f"{audited} mission commits audited from persistent origin {start}"
    )


def main(arguments: list[str]) -> None:
    if len(arguments) != 4:
        raise SystemExit(
            "usage: audit_mission_commits.py <repository> <active-snapshot> "
            "<session-id> <persistent-audit-origin>"
        )
    audit(
        Path(arguments[0]),
        Path(arguments[1]),
        arguments[2],
        Path(arguments[3]),
    )


if __name__ == "__main__":
    main(sys.argv[1:])
