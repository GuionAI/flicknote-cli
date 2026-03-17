---
name: flicktask-cli
description: "FlickTask CLI for tree-based task management via TaskChampion + PowerSync"
---

# FlickTask CLI

Tree-based task management stored in the local FlickNote SQLite database and synced to the cloud. Requires `flicknote login` first.

## Adding Tasks

```bash
# Add a root task
flicktask add "Implement OAuth2 flow"

# Add with options
flicktask add "Write tests" --parent a1b2c3d4 --priority H --due 2026-03-20
flicktask add "Research API" --tag backend --project auth
flicktask add "Task with UDA" --set foo=bar --set baz=qux
flicktask add "Focus today" --scheduled today

# Options:
#   --parent <id>          Parent task (8-char hex or full UUID)
#   --due <date>           Due date (YYYY-MM-DD or relative: today, tomorrow, 2days, eod, etc.)
#   --scheduled <date>     Scheduled date — puts task in today's list
#   --priority H|M|L       Priority
#   --tag <name>           Tag (repeatable: --tag a --tag b)
#   --project <name>       Project name
#   --set <KEY=VALUE>      UDA value (repeatable)
```

## Listing Tasks

```bash
flicktask list                          # all pending root tasks
flicktask list --completed              # completed tasks
flicktask list --tag backend            # filter by tag
flicktask list --priority H             # filter by priority (H, M, L)
flicktask list --due today              # due today
flicktask list --due week               # due within 7 days
flicktask list --due overdue            # past due date
```

## Viewing Tasks

```bash
# Show full task details + subtree
flicktask get a1b2c3d4

# Limit subtree depth
flicktask get a1b2c3d4 --depth 2

# Show all pending tasks as a tree
flicktask tree

# Show subtree of a specific task
flicktask tree a1b2c3d4

# Limit tree depth
flicktask tree --depth 3
```

## Editing Tasks

```bash
# Edit task properties (all flags optional)
flicktask edit a1b2c3d4 --description "New description"
flicktask edit a1b2c3d4 --due 2026-04-01
flicktask edit a1b2c3d4 --priority M
flicktask edit a1b2c3d4 --parent b2c3d4e5
flicktask edit a1b2c3d4 --wait 2026-03-25    # hide until date
flicktask edit a1b2c3d4 --project myproject
flicktask edit a1b2c3d4 --set key=value
```

## Completing and Deleting

```bash
flicktask done a1b2c3d4       # mark as completed
flicktask delete a1b2c3d4     # delete a task
flicktask undo                # undo the last change
```

## Tags

```bash
flicktask tag a1b2c3d4 backend     # add a tag
flicktask untag a1b2c3d4 backend   # remove a tag
```

## Annotations

```bash
# Add an annotation (timestamped note on the task)
flicktask annotate a1b2c3d4 "Blocked by upstream API change"

# Pipe annotation from stdin (for multiline)
echo "Long note here" | flicktask annotate a1b2c3d4
```

## Moving Tasks

```bash
# Move task to a new parent
flicktask move a1b2c3d4 b2c3d4e5

# Move task to root (no parent)
flicktask move a1b2c3d4
```

## Time Tracking

```bash
flicktask start a1b2c3d4    # start tracking time
flicktask stop a1b2c3d4     # stop tracking time
```

## Creating Subtask Trees from Markdown

`flicktask plan` reads markdown from stdin and creates a subtask tree under the given parent. Each heading becomes a task; indented headings become subtasks. Body text under a heading becomes the task's annotation.

```bash
# Pipe a markdown plan to create subtasks
cat <<'EOF' | flicktask plan a1b2c3d4
## Research
Look into existing solutions.

## Implementation
### Backend
Write the API endpoints.

### Frontend
Build the UI components.

## Testing
Write integration tests.
EOF

# Replace existing subtasks (deletes current children first)
cat plan.md | flicktask plan a1b2c3d4 --replace
```

## Importing

```bash
# Import from taskwarrior export JSON (stdin)
task export | flicktask import
```

## Finding Tasks

```bash
flicktask find <keyword> [keyword...]           # OR match, pending only
flicktask find --completed <keyword>            # search completed tasks
```

## Today

```bash
flicktask today list                            # tasks scheduled for today
flicktask today add <id> [id...]                # schedule tasks for today
flicktask today remove <id> [id...]             # unschedule tasks
flicktask today completed                       # tasks completed today
```

## Relative Dates

All date flags (`--due`, `--wait`, `--scheduled`) support:
- Absolute: `2026-03-20`
- Relative: `today`, `tomorrow`, `yesterday`, `now`
- Duration: `2days`, `1wk`, `3mo`, `9hrs`
- Boundaries: `eod`, `eow`, `eom`, `eoy`, `sow`, `som`, `soy`
- Weekdays: `mon`, `tue`, `wed`, `thu`, `fri`, `sat`, `sun`
- Far future: `later`, `someday` (for `--wait`)

```bash
flicktask add "desc" --due tomorrow --scheduled today
flicktask edit <id> --due eow --scheduled 2days
flicktask add "someday" --wait later
```

## Exporting

```bash
flicktask export                    # all pending tasks as taskwarrior-compatible JSON
flicktask export --completed        # completed tasks
flicktask export <id>               # single task
```

## ID Format

Task IDs are 8-character hex strings (e.g. `a1b2c3d4`). Full UUIDs are also accepted. IDs are printed after every create/edit operation.
