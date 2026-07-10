#!/usr/bin/env python3
"""Create and verify the fail-closed forbidden-path status baseline."""

from __future__ import annotations

import hashlib
import json
import os
import stat
import subprocess
import sys
from pathlib import Path
from typing import Any

ALLOWED_PREFIXES = (
    "tools-throwaway/mockup-harness-v3/",
    "plans/assets/mockups-v3/",
    "plans/assets/mockups-v3-gates/",
)
FINGERPRINT_FIELDS = {"worktree_sha256", "index_sha256"}


def fail(message: str) -> None:
    raise SystemExit(f"check_forbidden_paths.sh: {message}")


def repository_path(repository: Path, relative_path: str) -> Path:
    return repository / Path(relative_path)


def path_bytes(path: str) -> bytes:
    return path.encode("utf-8", "surrogateescape")


def validate_relative_path(path: str) -> None:
    candidate = Path(path)
    if not path or candidate.is_absolute() or ".." in candidate.parts:
        fail(f"baseline contains an unsafe repository-relative path: {path!r}")


def parse_status_entries(repository: Path) -> list[dict[str, str]]:
    raw = subprocess.check_output(
        ["git", "-C", str(repository), "status", "--porcelain=v1", "-z"]
    )
    fields = raw.split(b"\0")
    entries = []
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
            if index >= len(fields):
                fail(f"missing original path for {path!r}")
            original_path = fields[index].decode("utf-8", "surrogateescape")
            validate_relative_path(original_path)
            index += 1
        if path.startswith(ALLOWED_PREFIXES):
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
    raw = subprocess.check_output(
        [
            "git",
            "-C",
            str(repository),
            "ls-files",
            "--stage",
            "-z",
            "--",
            relative_path,
        ]
    )
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
    path = repository_path(repository, relative_path)
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


def fingerprints_for(repository: Path, relative_path: str) -> dict[str, str]:
    entry = index_entry(repository, relative_path)
    index_payload = b"absent" if entry is None else b"present\0" + entry
    return {
        "worktree_sha256": worktree_fingerprint(repository, relative_path),
        "index_sha256": sha256(
            b"forbidden-path-index-v1\0"
            + path_bytes(relative_path)
            + b"\0"
            + index_payload
        ),
    }


def record(repository: Path, head_commit: str) -> dict[str, Any]:
    entries: list[dict[str, Any]] = []
    for status_entry in parse_status_entries(repository):
        entry: dict[str, Any] = dict(status_entry)
        if status_entry["status"] != "??":
            entry["fingerprints"] = fingerprints_for(repository, status_entry["path"])
        entries.append(entry)

    return {
        "schema_version": 2,
        "head_commit": head_commit,
        "status_format": "git status --porcelain=v1 -z",
        "fingerprint_algorithm": "sha256",
        "excluded_v3_prefixes": list(ALLOWED_PREFIXES),
        "entries": entries,
    }


def validate_baseline(baseline: object) -> list[dict[str, Any]]:
    if not isinstance(baseline, dict):
        fail("baseline root must be an object")
    if baseline.get("schema_version") != 2:
        fail("unsupported schema_version")
    if baseline.get("status_format") != "git status --porcelain=v1 -z":
        fail("unsupported status format")
    if baseline.get("fingerprint_algorithm") != "sha256":
        fail("unsupported fingerprint algorithm")
    if baseline.get("excluded_v3_prefixes") != list(ALLOWED_PREFIXES):
        fail("v3 exclusions do not match contract")

    entries = baseline.get("entries")
    if not isinstance(entries, list):
        fail("baseline entries are missing")

    prior_key: tuple[str, str, str] | None = None
    status_entries: list[dict[str, Any]] = []
    for entry in entries:
        if not isinstance(entry, dict):
            fail("baseline contains a non-object entry")
        path = entry.get("path")
        status = entry.get("status")
        original_path = entry.get("original_path")
        if (
            not isinstance(path, str)
            or not isinstance(status, str)
            or (original_path is not None and not isinstance(original_path, str))
            or len(status) != 2
        ):
            fail("baseline contains an invalid status entry")
        validate_relative_path(path)
        if original_path is not None:
            validate_relative_path(original_path)
        key = (path, status, original_path or "")
        if prior_key is not None and key <= prior_key:
            fail("baseline entries are not uniquely sorted")
        prior_key = key

        expected_keys = {"path", "status"}
        if original_path is not None:
            expected_keys.add("original_path")
        fingerprints = entry.get("fingerprints")
        if fingerprints is not None:
            expected_keys.add("fingerprints")
            if status == "??" or not isinstance(fingerprints, dict):
                fail(f"untracked or invalid fingerprint entry: {path!r}")
            if set(fingerprints) != FINGERPRINT_FIELDS or any(
                not isinstance(value, str)
                or len(value) != 64
                or any(character not in "0123456789abcdef" for character in value)
                for value in fingerprints.values()
            ):
                fail(f"invalid fingerprints for {path!r}")
        if set(entry) != expected_keys:
            fail(f"unexpected baseline fields for {path!r}")
        status_entries.append(entry)
    return status_entries


def verify(repository: Path, baseline_path: Path) -> None:
    try:
        baseline = json.loads(baseline_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        fail(f"invalid baseline: {error}")

    expected = validate_baseline(baseline)
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
        print("check_forbidden_paths.sh: forbidden status baseline mismatch", file=sys.stderr)
        print(
            json.dumps(
                {"expected": expected_status, "actual": actual_status},
                indent=2,
                sort_keys=True,
            ),
            file=sys.stderr,
        )
        raise SystemExit(1)

    fingerprinted = 0
    for entry in expected:
        fingerprints = entry.get("fingerprints")
        if fingerprints is None:
            continue
        fingerprinted += 1
        actual = fingerprints_for(repository, entry["path"])
        if actual != fingerprints:
            print(
                f"check_forbidden_paths.sh: forbidden fingerprint mismatch for {entry['path']!r}",
                file=sys.stderr,
            )
            print(
                json.dumps(
                    {"expected": fingerprints, "actual": actual},
                    indent=2,
                    sort_keys=True,
                ),
                file=sys.stderr,
            )
            raise SystemExit(1)

    print(
        "check_forbidden_paths.sh: OK, "
        f"{len(actual_status)} off-limits status entries match, "
        f"{fingerprinted} dirty tracked paths fingerprinted"
    )


def main(arguments: list[str]) -> None:
    if not arguments:
        raise SystemExit(
            "usage: forbidden_path_baseline.py record <repository> <head-commit> | "
            "verify <repository> <baseline>"
        )
    command, *values = arguments
    if command == "record" and len(values) == 2:
        repository, head_commit = values
        json.dump(record(Path(repository), head_commit), sys.stdout, indent=2, sort_keys=True)
        sys.stdout.write("\n")
        return
    if command == "verify" and len(values) == 2:
        repository, baseline = values
        verify(Path(repository), Path(baseline))
        return
    raise SystemExit("invalid forbidden-path baseline command")


if __name__ == "__main__":
    main(sys.argv[1:])
