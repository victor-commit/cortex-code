#!/usr/bin/env python3
"""Audit mission-authored commits for path scope and identity."""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path
from typing import NoReturn

ALLOWED_PREFIXES = (
    "tools-throwaway/mockup-harness-v3/",
    "plans/assets/mockups-v3/",
    "plans/assets/mockups-v3-gates/",
    "evidence/v3-human-gate/",
)
MISSION_NAME = "echobt"
MISSION_EMAIL = "154886644+echobt@users.noreply.github.com"


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


def snapshot_head(snapshot_path: Path) -> str:
    try:
        snapshot = json.loads(snapshot_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        fail(f"invalid active snapshot: {error}")
    if (
        not isinstance(snapshot, dict)
        or snapshot.get("schema_version") != 3
        or snapshot.get("snapshot_kind") != "worker-session"
        or not isinstance(snapshot.get("head_commit"), str)
    ):
        fail("active snapshot does not contain a valid worker-session head")
    return snapshot["head_commit"]


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


def audit(repository: Path, snapshot_path: Path) -> None:
    start = snapshot_head(snapshot_path)
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
        f"{audited} mission commits audited from session head {start}"
    )


def main(arguments: list[str]) -> None:
    if len(arguments) != 2:
        raise SystemExit(
            "usage: audit_mission_commits.py <repository> <active-snapshot>"
        )
    audit(Path(arguments[0]), Path(arguments[1]))


if __name__ == "__main__":
    main(sys.argv[1:])
