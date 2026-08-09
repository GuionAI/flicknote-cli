use powersync::schema::{Column, Index, IndexedColumn, Schema, Table};

pub fn app_schema() -> Schema {
    Schema {
        tables: vec![
            notes_table(),
            projects_table(),
            note_extractions_table(),
            taskchampion_tasks_table(),
            taskchampion_operations_table(),
            settings_table(),
        ],
        ..Schema::default()
    }
}

fn notes_table() -> Table {
    Table::create(
        "notes",
        vec![
            Column::integer("short_id"),
            Column::text("user_id"),
            Column::text("type"),
            Column::text("status"),
            Column::text("title"),
            Column::text("content"),
            Column::text("summary"),
            Column::integer("is_flagged"),
            Column::text("project_id"),
            Column::text("metadata"),
            Column::text("source"),
            Column::text("created_at"),
            Column::text("updated_at"),
            Column::text("deleted_at"),
        ],
        |table| {
            table.options.track_metadata = true;
            table.indexes = vec![
                compound_index(
                    "notes_user_short_id_idx",
                    &[("user_id", "TEXT"), ("short_id", "INTEGER")],
                ),
                index("type", "type", "TEXT"),
                index("project", "project_id", "TEXT"),
                index("status", "status", "TEXT"),
                index("created", "created_at", "TEXT"),
                index("notes_deleted_at_idx", "deleted_at", "TEXT"),
                index("notes_updated_at_idx", "updated_at", "TEXT"),
            ];
        },
    )
}

fn projects_table() -> Table {
    Table::create(
        "projects",
        vec![
            Column::text("user_id"),
            Column::text("name"),
            Column::text("color"),
            Column::integer("is_archived"),
            Column::text("created_at"),
        ],
        |_| {},
    )
}

fn note_extractions_table() -> Table {
    Table::create(
        "note_extractions",
        vec![
            Column::text("note_id"),
            Column::text("user_id"),
            Column::text("key"),
            Column::text("value"),
        ],
        |table| {
            table.options.track_metadata = true;
            table.indexes = vec![
                index("note_extractions_note_id_idx", "note_id", "TEXT"),
                index("note_extractions_key_idx", "key", "TEXT"),
            ];
        },
    )
}

fn taskchampion_tasks_table() -> Table {
    Table::create(
        "tc_tasks",
        vec![
            Column::integer("short_id"),
            Column::text("user_id"),
            Column::text("data"),
            Column::text("entry_at"),
            Column::text("status"),
            Column::text("description"),
            Column::text("priority"),
            Column::text("modified_at"),
            Column::text("due_at"),
            Column::text("scheduled_at"),
            Column::text("start_at"),
            Column::text("end_at"),
            Column::text("wait_at"),
            Column::text("parent_id"),
            Column::text("note_id"),
            Column::text("project_id"),
        ],
        |table| {
            table.indexes = vec![
                compound_index(
                    "tc_tasks_user_short_id_idx",
                    &[("user_id", "TEXT"), ("short_id", "INTEGER")],
                ),
                index("tc_tasks_status", "status", "TEXT"),
                index("tc_tasks_parent", "parent_id", "TEXT"),
            ];
        },
    )
}

fn taskchampion_operations_table() -> Table {
    Table::create(
        "tc_operations",
        vec![
            Column::text("user_id"),
            Column::text("data"),
            Column::text("created_at"),
        ],
        |table| {
            table.options.local_only = true;
        },
    )
}

fn settings_table() -> Table {
    Table::create(
        "settings",
        vec![
            Column::text("language"),
            Column::text("iana_tz"),
            Column::text("base_keyterms"),
            Column::text("role"),
            Column::text("asr_model"),
            Column::text("tc_config"),
        ],
        |_| {},
    )
}

fn index(name: &str, column: &str, type_name: &str) -> Index {
    compound_index(name, &[(column, type_name)])
}

fn compound_index(name: &str, columns: &[(&str, &str)]) -> Index {
    Index {
        name: name.to_string().into(),
        columns: columns
            .iter()
            .map(|(name, type_name)| IndexedColumn {
                name: (*name).to_string().into(),
                ascending: true,
                type_name: (*type_name).to_string().into(),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_committed_tables_track_crud_metadata() {
        let schema = app_schema();

        for name in ["notes", "note_extractions"] {
            let table = schema
                .tables
                .iter()
                .find(|table| table.name == name)
                .unwrap_or_else(|| panic!("missing {name} table"));
            assert!(table.options.track_metadata, "{name} must track metadata");
        }
    }
}
