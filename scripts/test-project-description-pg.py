#!/usr/bin/env python3
"""Exercise #3634 operator SQL only in the container owned by test-private-pg.sh."""
import atexit
import importlib.util
import os
import shutil
import tempfile
import json
from pathlib import Path
import subprocess
import sys
import uuid

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("description_sql", Path(__file__).parent / "project-description/render.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
engine, container = sys.argv[1:]
if not container.startswith("flicknote-cli-pg-test-"):
    raise SystemExit("refusing any container not owned by the private PG harness")
command = [engine, "exec", "-i", container, "psql", "-X", "-qAt", "-h", "127.0.0.1", "-U", "postgres", "-d", "supabase", "-v", "ON_ERROR_STOP=1"]
OWNER = module.OWNER
FOREIGN = str(uuid.uuid4())

evidence_parent = os.environ.get("FLICKNOTE_TEST_DESCRIPTION_EVIDENCE_PARENT")
evidence = Path(tempfile.mkdtemp(prefix="fn3634-sql-", dir=evidence_parent))
if not evidence_parent:
    atexit.register(shutil.rmtree, evidence)
sequence = 0


def sql(text, fail=False):
    result = subprocess.run(command, input=text, text=True, capture_output=True)
    if fail:
        assert result.returncode != 0, "expected transaction to fail"
        return result.stderr
    assert result.returncode == 0, result.stderr
    return result.stdout.strip()


def quote(value):
    return "'" + value.replace("'", "''") + "'"


def snapshot():
    return json.loads(sql(module.render("preview")))


def action(kind, before, commit=True, fail=False):
    global sequence
    sequence += 1
    rendered = module.render(kind, before, commit)
    (evidence / f"{sequence:02}-{kind}.sql").write_text(rendered)
    output = sql(rendered, fail=fail)
    (evidence / f"{sequence:02}-{kind}.result").write_text(output)
    return output if fail else json.loads(output)


def update(table, identity, metadata):
    sql(f"UPDATE {table} SET metadata={quote(json.dumps(metadata))}::jsonb WHERE id='{identity}';")


projects = [str(uuid.uuid4()) for _ in range(4)]
notes = [str(uuid.uuid4()) for _ in range(10)]
(evidence / "fixture-identities.json").write_text(json.dumps({"owner": OWNER, "foreign": FOREIGN, "projects": projects, "notes": notes}))
sql(f"INSERT INTO auth.users(id) VALUES('{OWNER}'),('{FOREIGN}');")
for i, (owner, archived, metadata) in enumerate([
    (OWNER, False, {"summary": "  exact '\\n🙂  ", "sibling": {"keep": True}}),
    (OWNER, True, {"summary": "archived", "color": "unchanged"}),
    (OWNER, False, {"description": "already new", "other": 4}),
    (FOREIGN, True, {"summary": "foreign", "other": 5}),
]):
    sql(f"INSERT INTO projects(id,user_id,name,is_archived,metadata,created_at) VALUES('{projects[i]}','{owner}','SQL fixture {i}',{str(archived).lower()},{quote(json.dumps(metadata))}::jsonb,'2026-01-01');")
markers = [
    {"probability": 0.9, "routed": True},
    {"probability": 0.7, "routed": True},
    {"probability": 0.8, "routed": True},
    {"project_id": projects[0], "reason": "new assigned", "routed": True},
    {"project_id": None, "reason": "new none", "routed": True},
    {"probability": 0.5, "routed": True, "unknown": "keep"},
    {"probability": 0.5, "routed": False},
    {"routed": True},
    ["unknown"],
    {"probability": 0.9, "routed": True},
]
for i, marker in enumerate(markers):
    owner = FOREIGN if i == 9 else OWNER
    assigned = quote(projects[0]) if i in (0, 3) else "NULL"
    deleted = "'2026-01-03'" if i == 2 else "NULL"
    metadata = {"project_routing": marker, "created_by_ai": True, "nested": {"keep": [1, 2]}}
    sql(f"INSERT INTO notes(id,user_id,type,status,content,title,summary,source,project_id,metadata,created_at,updated_at,deleted_at,is_flagged) VALUES('{notes[i]}','{owner}','normal','ready','body','title','note summary','{{\"keep\":true}}',{assigned},{quote(json.dumps(metadata))}::jsonb,'2026-01-01','2026-01-02',{deleted},true);")
sql(f"INSERT INTO note_extractions(note_id,user_id,key,value) VALUES('{notes[0]}','{OWNER}','::topic','unchanged');")
foreign_query = f"SELECT jsonb_build_object('project',(SELECT to_jsonb(p) FROM projects p WHERE id='{projects[3]}'),'note',(SELECT to_jsonb(n) FROM notes n WHERE id='{notes[9]}'));"
foreign = sql(foreign_query)
before = snapshot()
(evidence / "preimages.json").write_text(json.dumps(before, ensure_ascii=False, indent=2))
# Default dry-run executes all assertions but leaves every full row unchanged.
dry = action("apply", before, commit=False)
assert dry["projects_changed"] == 2 and dry["notes_changed"] == 3
assert snapshot() == before
# Conflicts, including JSON null description and non-string summary, fail atomically.
for bad in [{"summary": "old", "description": "new"}, {"summary": "old", "description": None}, {"summary": 7}, {"summary": None}]:
    original = next(r for r in before["projects"] if r["id"] == projects[1])["metadata"]
    update("projects", projects[1], bad)
    conflict = snapshot()
    assert "conflict or non-string" in action("apply", conflict, fail=True)
    assert snapshot() == conflict
    update("projects", projects[1], original)
# A later edit and a new target invalidate the exact approved preimage.
update("projects", projects[2], {"description": "later"})
assert "snapshot changed" in action("apply", before, fail=True)
update("projects", projects[2], {"description": "already new", "other": 4})
new_id = str(uuid.uuid4())
sql(f"INSERT INTO projects(id,user_id,name,metadata) VALUES('{new_id}','{OWNER}','new target','{{\"summary\":\"new\"}}');")
new_snapshot = snapshot()
assert "snapshot changed" in action("apply", before, fail=True)
assert snapshot() == new_snapshot
sql(f"DELETE FROM projects WHERE id='{new_id}';")
# Detect a post-update trigger changing unrelated timestamps; all changes roll back.
sql("CREATE FUNCTION public.fixture_3634_bad_update() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN NEW.updated_at='2030-01-01'; RETURN NEW; END $$; CREATE TRIGGER fixture_3634_bad_update BEFORE UPDATE ON notes FOR EACH ROW EXECUTE FUNCTION public.fixture_3634_bad_update();")
assert "postimage mismatch" in action("apply", before, fail=True)
assert snapshot() == before
sql("DROP TRIGGER fixture_3634_bad_update ON notes; DROP FUNCTION public.fixture_3634_bad_update();")
result = action("apply", before)
after = snapshot()
(evidence / "postimages.json").write_text(json.dumps(after, ensure_ascii=False, indent=2))
assert result["postimage"] == after
assert sql(foreign_query) == foreign
for table in ("projects", "notes"):
    for old, new in zip(before[table], after[table]):
        assert {k: v for k, v in old.items() if k != "metadata"} == {k: v for k, v in new.items() if k != "metadata"}
        if table == "projects" and "summary" in old["metadata"]:
            assert new["metadata"] == {**{k: v for k, v in old["metadata"].items() if k != "summary"}, "description": old["metadata"]["summary"]}
        elif table == "notes" and old["id"] in notes[:3]:
            assert new["metadata"] == {k: v for k, v in old["metadata"].items() if k != "project_routing"}
        else:
            assert old["metadata"] == new["metadata"]
assert before["extractions"] == after["extractions"]
noop = action("apply", after)
assert noop["projects_changed"] == noop["notes_changed"] == 0
assert snapshot() == after
assert "snapshot changed" in action("apply", before, fail=True)
# Rerun uses a fresh empty preview; replaying stale approval is deliberately rejected.
restored = action("rollback", before)
assert restored["projects_restored"] == 2 and restored["notes_restored"] == 3
assert snapshot() == before
assert action("rollback", before)["projects_restored"] == 0
# Protect description edits, new backend markers and unrelated user edits.
action("apply", before)
update("projects", projects[0], {"description": "later user edit", "sibling": {"keep": True}})
update("notes", notes[0], {"project_routing": {"project_id": projects[0], "reason": "later backend", "routed": True}})
sql(f"UPDATE notes SET content='later content' WHERE id='{notes[1]}';")
changed = snapshot()
rollback = action("rollback", before)
assert rollback["projects_restored"] == 1 and rollback["notes_restored"] == 1
assert set(rollback["projects_skipped"]) == {projects[0]}
assert set(rollback["notes_skipped"]) == {notes[0], notes[1]}
final = snapshot()
for table, ids in [("projects", projects[:1]), ("notes", notes[:2])]:
    assert [r for r in final[table] if r["id"] in ids] == [r for r in changed[table] if r["id"] in ids]
assert sql(foreign_query) == foreign
sql(f"DELETE FROM auth.users WHERE id IN ('{OWNER}','{FOREIGN}');")
print("PROJECT_DESCRIPTION_SQL_PASS: dry-run, active/archive, exact values, owner isolation, old assigned/none, new/unknown, conflicts, concurrent edit/new target, postimage failure atomicity, no-op rerun, full restoration, constrained rollback")

if evidence_parent:
    print(f"PROJECT_DESCRIPTION_SQL_EVIDENCE={evidence.resolve()}")
