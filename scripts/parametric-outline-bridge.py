#!/usr/bin/env python3
"""Publish a saved PCB outline and optionally adapt it to a SolidCraft linked design."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import uuid
import zipfile


MANIFEST = "manifest.json"
BODY = "bodies/0.step"
BOARD = "board.json"
PARAMS = "params.json"
PARTS = {BODY, BOARD, PARAMS}
MAX_BYTES = 16 * 1024 * 1024


def json_bytes(value):
    return (json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False) + "\n").encode()


def digest(data):
    return hashlib.sha256(data).hexdigest()


def part_index(parts):
    return {name: digest(data) for name, data in parts.items()}


def read_exchange(path):
    """Read only the bounded, hash-checked payload used by this prototype."""
    if path.stat().st_size > MAX_BYTES:
        raise ValueError("Exchange file exceeds the prototype's size limit.")
    with zipfile.ZipFile(path) as archive:
        entries = archive.infolist()
        if len(entries) != len(PARTS) + 1 or {entry.filename for entry in entries} != PARTS | {MANIFEST}:
            raise ValueError("Unexpected exchange members.")
        if sum(entry.file_size for entry in entries) > MAX_BYTES:
            raise ValueError("Exchange payload exceeds the prototype's size limit.")
        manifest = json.loads(archive.read(MANIFEST))
        parts = {name: archive.read(name) for name in PARTS}
    if (
        manifest["format"] != "craft-exchange/1"
        or manifest["requires"] != ["board/1"]
        or manifest["units"] != "mm"
        or manifest["parts"] != part_index(parts)
        or manifest["content_hash"] != digest(json_bytes(manifest["parts"]))
    ):
        raise ValueError("Incompatible exchange contract or mismatched content hash.")
    if not uuid.UUID(manifest["source_id"]).int or type(manifest["revision"]) is not int or manifest["revision"] < 1:
        raise ValueError("Invalid exchange identity or revision.")
    return manifest, parts


def publish(source, exchange, outline_tool, solidcraft_cli):
    """Advance the exchange on geometry changes and publish its adapter file atomically."""
    linked_design = exchange.with_suffix(".sc")
    previous, payload = read_exchange(exchange) if exchange.exists() else (None, None)
    if solidcraft_cli and linked_design.exists() and previous is None:
        raise ValueError("Choose an exchange path without an existing linked design.")
    if previous and previous["project_location"] != str(source):
        raise ValueError("The exchange belongs to a different project path.")

    with tempfile.TemporaryDirectory(prefix=".pcb-outline-", dir=exchange.parent) as name:
        scratch = Path(name)
        step = scratch / "board.step"
        reply = subprocess.run([outline_tool, "snapshot", source, step], check=True, stdout=subprocess.PIPE)
        board = json.loads(reply.stdout)
        parts = {
            BODY: step.read_bytes(),
            BOARD: json_bytes(board),
            PARAMS: json_bytes([
                {"name": "BoardWidth", "value": board["width_mm"], "unit": "mm"},
                {"name": "BoardHeight", "value": board["height_mm"], "unit": "mm"},
            ]),
        }
        index = part_index(parts)
        content_hash = digest(json_bytes(index))
        if previous and (previous["project"], previous["board"]) != (board["project"], board["board"]):
            raise ValueError("The exchange belongs to a different board.")
        changed = previous is None or previous["content_hash"] != content_hash
        if changed:
            manifest = {
                "format": "craft-exchange/1",
                "source_app": "pcbcraft",
                "source_version": "0.1.0-outline-prototype",
                "source_id": previous["source_id"] if previous else str(uuid.uuid4()),
                "revision": previous["revision"] + 1 if previous else 1,
                "project": board["project"],
                "board": board["board"],
                "project_location": str(source),
                "units": "mm",
                "requires": ["board/1"],
                "parts": index,
                "content_hash": content_hash,
            }
            archive_path = scratch / "board.craftx"
            with zipfile.ZipFile(archive_path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
                archive.writestr(MANIFEST, json_bytes(manifest))
                for member, data in parts.items():
                    archive.writestr(member, data)
            payload = read_exchange(archive_path)[1]
        else:
            manifest = previous

        stamp = exchange.with_suffix(".linked-hash")
        linked_hash = stamp.read_text().strip() if stamp.exists() else None
        refresh_link = solidcraft_cli and (linked_hash != content_hash or not linked_design.exists())
        if refresh_link:
            # Feed the verified exchange payload to SolidCraft's real STEP importer.
            step.write_bytes(payload[BODY])
            staged_design = scratch / "board.sc"
            subprocess.run([solidcraft_cli, "run", "--in", step, "--save", staged_design, "--quiet"], check=True)
            staged_stamp = scratch / "linked-hash"
            staged_stamp.write_text(content_hash + "\n")
        if changed:
            os.replace(archive_path, exchange)
        if refresh_link:
            # A missing stamp forces regeneration if publishing stops before the new stamp lands.
            stamp.unlink(missing_ok=True)
            os.replace(staged_design, linked_design)
            os.replace(staged_stamp, stamp)
        return manifest, changed


def follow(args):
    """Publish a new saved revision when the packed project's file signature changes."""
    prior = None
    last_error = None
    while True:
        try:
            file = args.project.stat()
            key = (file.st_ino, file.st_mtime_ns, file.st_size)
            if key != prior:
                run_publish(args)
                prior = key
                last_error = None
        except (OSError, ValueError, KeyError, TypeError, subprocess.CalledProcessError, zipfile.BadZipFile) as error:
            if str(error) != last_error:
                print(f"Publish failed: {error}", file=sys.stderr, flush=True)
                last_error = str(error)
        time.sleep(0.5)


def run_publish(args):
    manifest, changed = publish(args.project, args.exchange, args.outline_tool, args.solidcraft_cli)
    print(json.dumps({"revision": manifest["revision"], "source_id": manifest["source_id"], "published": changed}), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["publish", "watch"])
    parser.add_argument("project", type=Path)
    parser.add_argument("exchange", type=Path)
    parser.add_argument("--outline-tool", required=True, type=Path)
    parser.add_argument("--solidcraft-cli", type=Path, help="also produce the linked SolidCraft design beside the exchange")
    args = parser.parse_args()
    args.project = args.project.resolve()
    args.exchange = args.exchange.resolve()
    if args.project.suffix != ".pc" or args.exchange.suffix != ".craftx":
        parser.error("Use a packed .pc project and a .craftx output.")
    args.outline_tool = args.outline_tool.resolve()
    if args.solidcraft_cli:
        args.solidcraft_cli = args.solidcraft_cli.resolve()
    try:
        if args.mode == "watch":
            follow(args)
        else:
            run_publish(args)
    except KeyboardInterrupt:
        return 0
    except (OSError, ValueError, KeyError, TypeError, subprocess.CalledProcessError, zipfile.BadZipFile) as error:
        print(f"Publish failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
