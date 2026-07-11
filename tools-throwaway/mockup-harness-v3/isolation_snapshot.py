#!/usr/bin/env python3
"""Record and verify the active worker-session forbidden-path snapshot."""

from __future__ import annotations

import hashlib
import json
import os
import re
import stat
import subprocess
import sys
from pathlib import Path
from typing import Any, NoReturn

ALLOWED_PREFIXES = (
    "tools-throwaway/mockup-harness-v3/",
    "plans/assets/mockups-v3/",
    "plans/assets/mockups-v3-gates/",
)
SNAPSHOT_FIELDS = {
    "schema_version",
    "snapshot_kind",
    "head_commit",
    "status_format",
    "fingerprint_algorithm",
    "excluded_v3_prefixes",
    "entries",
    "tracked_fingerprints",
}
FINGERPRINT_FIELDS = {"worktree_sha256", "index_sha256"}
HEX_RE = re.compile(r"^[0-9a-f]{40,64}$")


def fail(message: str) -> NoReturn:
    raise SystemExit(f"check_forbidden_paths.sh: {message}")


def path_bytes(path: str) -> bytes:
    return path.encode("utf-8", "surrogateescape")


def validate_relative_path(path: str) -> None:
    candidate = Path(path)
    if not path or candidate.is_absolute() or ".." in candidate.parts:
        fail(f"snapshot contains an unsafe repository-relative path: {path!r}")


def is_allowed(path: str) -> bool:
    return path.startswith(ALLOWED_PREFIXES)


def git(repository: Path, *arguments: str) -> bytes:
    try:
        return subprocess.check_output(
            ["git", "-C", str(repository), *arguments],
            stderr=subprocess.PIPE,
        )
    except subprocess.CalledProcessError as error:
        detail = error.stderr.decode("utf-8", "replace").strip()
        fail(f"git {' '.join(arguments)} failed: {detail}")


def parse_status_entries(repository: Path) -> list[dict[str, str]]:
    raw = git(repository, "status", "--porcelain=v1", "-z")
    fields = raw.split(b"\0")
    entries: list[dict[str, str]] = []
    index = 0
    while index < len(fields):
        field = fields[index]
        index += 1
        if not field:
            continue
        if len(field) < 4:
            fail(f"invalid porcelain record: {field!r}")
        status = field[:2].decode("ascii")
        path = field[3:].decode("utf-8", "surrogateescape")
        validate_relative_path(path)
        original_path = None
        if "R" in status or "C" in status:
            if index >= len(fields) or not fields[index]:
                fail(f"missing original path for {path!r}")
            original_path = fields[index].decode("utf-8", "surrogateescape")
            validate_relative_path(original_path)
            index += 1

        # A rename/copy is forbidden when either side is forbidden. Filtering
        # only the destination would let a forbidden source enter a v3 root.
        if is_allowed(path) and (
            original_path is None or is_allowed(original_path)
        ):
            continue
        entry = {"path": path, "status": status}
        if original_path is not None:
            entry["original_path"] = original_path
        entries.append(entry)

    return sorted(
        entries,
        key=lambda entry: (
            entry["path"],
            entry["status"],
            entry.get("original_path", ""),
        ),
    )


def index_entry(repository: Path, relative_path: str) -> bytes | None:
    raw = git(repository, "ls-files", "--stage", "-z", "--", relative_path)
    records = [record for record in raw.split(b"\0") if record]
    if not records:
        return None

    expected_path = path_bytes(relative_path)
    for record in records:
        try:
            header, record_path = record.split(b"\t", 1)
        except ValueError:
            fail(f"invalid Git index entry for {relative_path!r}")
        fields = header.split()
        if len(fields) != 3 or record_path != expected_path:
            fail(f"unexpected Git index entry for {relative_path!r}")
        mode, object_id, stage = fields
        if (
            len(mode) != 6
            or not mode.isdigit()
            or len(object_id) not in (40, 64)
            or any(byte not in b"0123456789abcdef" for byte in object_id)
            or stage not in (b"0", b"1", b"2", b"3")
        ):
            fail(f"invalid Git index entry for {relative_path!r}")
    return raw


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def worktree_fingerprint(repository: Path, relative_path: str) -> str:
    path = repository / Path(relative_path)
    try:
        metadata = os.lstat(path)
    except FileNotFoundError:
        payload = (
            b"forbidden-path-worktree-v1\0"
            + path_bytes(relative_path)
            + b"\0absent\0"
        )
        return sha256(payload)

    if stat.S_ISREG(metadata.st_mode):
        file_type = b"regular"
        content = hashlib.sha256()
        with path.open("rb") as source:
            while chunk := source.read(1024 * 1024):
                content.update(chunk)
        content_digest = content.digest()
    elif stat.S_ISLNK(metadata.st_mode):
        file_type = b"symlink"
        content_digest = hashlib.sha256(os.fsencode(os.readlink(path))).digest()
    elif stat.S_ISDIR(metadata.st_mode):
        file_type = b"directory"
        content_digest = b""
    else:
        file_type = b"other"
        content_digest = b""

    payload = b"\0".join(
        (
            b"forbidden-path-worktree-v1",
            path_bytes(relative_path),
            file_type,
            f"{stat.S_IMODE(metadata.st_mode):04o}".encode("ascii"),
            content_digest,
        )
    )
    return sha256(payload)


def fingerprints_for(
    repository: Path, relative_paths: list[str]
) -> dict[str, dict[str, str]]:
    fingerprints = {}
    for relative_path in dict.fromkeys(relative_paths):
        entry = index_entry(repository, relative_path)
        index_payload = b"absent" if entry is None else b"present\0" + entry
        fingerprints[relative_path] = {
            "worktree_sha256": worktree_fingerprint(repository, relative_path),
            "index_sha256": sha256(
                b"forbidden-path-index-v1\0"
                + path_bytes(relative_path)
                + b"\0"
                + index_payload
            ),
        }
    return fingerprints


def tracked_forbidden_paths(repository: Path) -> list[str]:
    raw = git(repository, "ls-files", "-z")
    paths = []
    for value in raw.split(b"\0"):
        if not value:
            continue
        path = value.decode("utf-8", "surrogateescape")
        validate_relative_path(path)
        if not is_allowed(path):
            paths.append(path)
    return sorted(paths)


def record(repository: Path, head_commit: str) -> dict[str, Any]:
    entries: list[dict[str, Any]] = []
    for status_entry in parse_status_entries(repository):
        entry: dict[str, Any] = dict(status_entry)
        if status_entry["status"] != "??":
            paths = [status_entry["path"]]
            if "original_path" in status_entry:
                paths.append(status_entry["original_path"])
            entry["fingerprints"] = fingerprints_for(repository, paths)
        entries.append(entry)

    return {
        "schema_version": 3,
        "snapshot_kind": "worker-session",
        "head_commit": head_commit,
        "status_format": "git status --porcelain=v1 -z",
        "fingerprint_algorithm": "sha256",
        "excluded_v3_prefixes": list(ALLOWED_PREFIXES),
        "entries": entries,
        "tracked_fingerprints": fingerprints_for(
            repository, tracked_forbidden_paths(repository)
        ),
    }


def validate_fingerprint(fingerprint: object, path: str) -> None:
    if not isinstance(fingerprint, dict) or set(fingerprint) != FINGERPRINT_FIELDS:
        fail(f"invalid fingerprints for {path!r}")
    if any(
        not isinstance(value, str)
        or len(value) != 64
        or any(character not in "0123456789abcdef" for character in value)
        for value in fingerprint.values()
    ):
        fail(f"invalid fingerprints for {path!r}")


def validate_snapshot(snapshot: object) -> list[dict[str, Any]]:
    if not isinstance(snapshot, dict):
        fail("active snapshot root must be an object")
    if set(snapshot) != SNAPSHOT_FIELDS:
        fail("active snapshot fields are invalid")
    if snapshot.get("schema_version") != 3:
        fail("unsupported active snapshot schema_version")
    if snapshot.get("snapshot_kind") != "worker-session":
        fail("unsupported active snapshot kind")
    head_commit = snapshot.get("head_commit")
    if not isinstance(head_commit, str) or not HEX_RE.fullmatch(head_commit):
        fail("active snapshot has an invalid head commit")
    if snapshot.get("status_format") != "git status --porcelain=v1 -z":
        fail("unsupported status format")
    if snapshot.get("fingerprint_algorithm") != "sha256":
        fail("unsupported fingerprint algorithm")
    if snapshot.get("excluded_v3_prefixes") != list(ALLOWED_PREFIXES):
        fail("v3 exclusions do not match contract")

    entries = snapshot.get("entries")
    if not isinstance(entries, list):
        fail("active snapshot entries are missing")
    tracked_fingerprints = snapshot.get("tracked_fingerprints")
    if not isinstance(tracked_fingerprints, dict) or not tracked_fingerprints:
        fail("active snapshot tracked fingerprints are missing")
    for path, fingerprint in tracked_fingerprints.items():
        if not isinstance(path, str):
            fail("active snapshot tracked fingerprint path is invalid")
        validate_relative_path(path)
        if is_allowed(path):
            fail(f"active snapshot tracks an allowed path: {path!r}")
        validate_fingerprint(fingerprint, path)

    prior_key: tuple[str, str, str] | None = None
    validated: list[dict[str, Any]] = []
    for entry in entries:
        if not isinstance(entry, dict):
            fail("active snapshot contains a non-object entry")
        path = entry.get("path")
        status = entry.get("status")
        original_path = entry.get("original_path")
        if (
            not isinstance(path, str)
            or not isinstance(status, str)
            or len(status) != 2
            or (original_path is not None and not isinstance(original_path, str))
        ):
            fail("active snapshot contains an invalid status entry")
        validate_relative_path(path)
        if original_path is not None:
            validate_relative_path(original_path)
        key = (path, status, original_path or "")
        if prior_key is not None and key <= prior_key:
            fail("active snapshot entries are not uniquely sorted")
        prior_key = key

        expected_keys = {"path", "status"}
        if original_path is not None:
            expected_keys.add("original_path")
        fingerprints = entry.get("fingerprints")
        if fingerprints is not None:
            expected_keys.add("fingerprints")
            if status == "??" or not isinstance(fingerprints, dict):
                fail(f"untracked or invalid fingerprints for {path!r}")
            expected_paths = [path]
            if original_path is not None:
                expected_paths.append(original_path)
            if set(fingerprints) != set(expected_paths):
                fail(f"invalid fingerprint paths for {path!r}")
            for fingerprint_path, fingerprint in fingerprints.items():
                validate_relative_path(fingerprint_path)
                validate_fingerprint(fingerprint, fingerprint_path)
        if set(entry) != expected_keys:
            fail(f"unexpected active snapshot fields for {path!r}")
        validated.append(entry)
    return validated


def read_snapshot(snapshot_path: Path) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    try:
        snapshot = json.loads(snapshot_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        fail(f"invalid active snapshot: {error}")
    entries = validate_snapshot(snapshot)
    return snapshot, entries


def verify(repository: Path, snapshot_path: Path) -> None:
    snapshot, expected = read_snapshot(snapshot_path)
    expected_status = [
        {
            key: entry[key]
            for key in ("path", "status", "original_path")
            if key in entry
        }
        for entry in expected
    ]
    actual_status = parse_status_entries(repository)
    if actual_status != expected_status:
        print(
            "check_forbidden_paths.sh: active session status mismatch",
            file=sys.stderr,
        )
        print(
            json.dumps(
                {"expected": expected_status, "actual": actual_status},
                indent=2,
                sort_keys=True,
            ),
            file=sys.stderr,
        )
        raise SystemExit(1)

    expected_tracked = snapshot["tracked_fingerprints"]
    expected_paths = sorted(expected_tracked)
    actual_paths = tracked_forbidden_paths(repository)
    if actual_paths != expected_paths:
        print(
            "check_forbidden_paths.sh: tracked forbidden path inventory mismatch",
            file=sys.stderr,
        )
        print(
            json.dumps(
                {
                    "expected": expected_paths,
                    "actual": actual_paths,
                },
                indent=2,
            ),
            file=sys.stderr,
        )
        raise SystemExit(1)
    actual_tracked = fingerprints_for(repository, expected_paths)
    if actual_tracked != expected_tracked:
        changed = sorted(
            path
            for path in set(expected_tracked) | set(actual_tracked)
            if expected_tracked.get(path) != actual_tracked.get(path)
        )
        print(
            "check_forbidden_paths.sh: tracked forbidden fingerprint mismatch",
            file=sys.stderr,
        )
        print(json.dumps({"changed_paths": changed}, indent=2), file=sys.stderr)
        raise SystemExit(1)

    fingerprinted = 0
    for entry in expected:
        fingerprints = entry.get("fingerprints")
        if fingerprints is None:
            continue
        for path, expected_fingerprint in fingerprints.items():
            fingerprinted += 1
            actual = fingerprints_for(repository, [path])[path]
            if actual != expected_fingerprint:
                print(
                    "check_forbidden_paths.sh: active session fingerprint mismatch "
                    f"for {path!r}",
                    file=sys.stderr,
                )
                print(
                    json.dumps(
                        {"expected": expected_fingerprint, "actual": actual},
                        indent=2,
                        sort_keys=True,
                    ),
                    file=sys.stderr,
                )
                raise SystemExit(1)

    print(
        "check_forbidden_paths.sh: OK, "
        f"{len(actual_status)} off-limits status entries match, "
        f"{fingerprinted} dirty status paths plus "
        f"{len(expected_tracked)} tracked paths fingerprinted from the active session"
    )


def main(arguments: list[str]) -> None:
    if not arguments:
        raise SystemExit(
            "usage: isolation_snapshot.py record <repository> <head-commit> | "
            "verify <repository> <active-snapshot>"
        )
    command, *values = arguments
    if command == "record" and len(values) == 2:
        repository, head_commit = values
        json.dump(
            record(Path(repository), head_commit),
            sys.stdout,
            indent=2,
            sort_keys=True,
        )
        sys.stdout.write("\n")
        return
    if command == "verify" and len(values) == 2:
        repository, snapshot_path = values
        verify(Path(repository), Path(snapshot_path))
        return
    raise SystemExit("invalid isolation snapshot command")


if __name__ == "__main__":
    main(sys.argv[1:])
