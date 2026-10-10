#!/usr/bin/env python3
"""Render #3634 reviewed SQL to stdout; never connect to a database."""
import argparse
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
OWNER = "fe4de0a3-0cf4-4d79-92e4-4be3fae2c634"


def render(action, snapshot=None, commit=False):
    if action == "preview":
        return (ROOT / "preview.sql").read_text().replace(
            "\\ir common.sql", (ROOT / "common.sql").read_text()
        )
    if not isinstance(snapshot, dict) or snapshot.get("owner") != OWNER:
        raise ValueError("snapshot must belong to the approved owner")
    if set(snapshot) != {"owner", "projects", "notes", "extractions"}:
        raise ValueError("snapshot must contain full projects, notes and extractions")
    for table in ("projects", "notes", "extractions"):
        if not isinstance(snapshot[table], list) or any(
            row.get("user_id") != OWNER for row in snapshot[table]
        ):
            raise ValueError(f"invalid owner preimages in {table}")
    # SQL literal, not shell interpolation. standard_conforming_strings is fixed below.
    encoded = json.dumps(snapshot, ensure_ascii=False).replace("'", "''")
    return (
        "\\set ON_ERROR_STOP on\n\\set commit " + ("true" if commit else "false")
        + "\nBEGIN;\nSET LOCAL standard_conforming_strings = on;\n"
        + "CREATE TEMP TABLE approved(data jsonb NOT NULL) ON COMMIT DROP;\n"
        + "INSERT INTO approved VALUES ('" + encoded + "'::jsonb);\n"
        + (ROOT / f"{action}.sql").read_text().replace(
            "\\ir common.sql", (ROOT / "common.sql").read_text()
        )
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("preview", "apply", "rollback"))
    parser.add_argument("--snapshot", type=Path)
    parser.add_argument("--commit", action="store_true", help="explicitly emit COMMIT instead of default ROLLBACK")
    args = parser.parse_args()
    if args.action == "preview" and (args.snapshot or args.commit):
        parser.error("preview accepts neither snapshot nor commit")
    if args.action != "preview" and not args.snapshot:
        parser.error("apply/rollback require a reviewed full snapshot")
    print(render(args.action, json.loads(args.snapshot.read_text()) if args.snapshot else None, args.commit))
