use powersync::schema::{Column, Index, IndexedColumn, Schema, Table};

pub fn app_schema() -> Schema {
    let mut schema = Schema::default();

    schema.tables.push(Table::create(
        "notes",
        vec![
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
            Column::text("external_id"),
            Column::text("created_at"),
            Column::text("updated_at"),
            Column::text("deleted_at"),
        ],
        |t| {
            t.indexes = vec![
                Index {
                    name: "type".into(),
                    columns: vec![IndexedColumn {
                        name: "type".into(),
                        ascending: true,
                        type_name: "TEXT".into(),
                    }],
                },
                Index {
                    name: "project".into(),
                    columns: vec![IndexedColumn {
                        name: "project_id".into(),
                        ascending: true,
                        type_name: "TEXT".into(),
                    }],
                },
                Index {
                    name: "status".into(),
                    columns: vec![IndexedColumn {
                        name: "status".into(),
                        ascending: true,
                        type_name: "TEXT".into(),
                    }],
                },
                Index {
                    name: "created".into(),
                    columns: vec![IndexedColumn {
                        name: "created_at".into(),
                        ascending: true,
                        type_name: "TEXT".into(),
                    }],
                },
                Index {
                    name: "notes_deleted_at_idx".into(),
                    columns: vec![IndexedColumn {
                        name: "deleted_at".into(),
                        ascending: true,
                        type_name: "TEXT".into(),
                    }],
                },
                Index {
                    name: "notes_updated_at_idx".into(),
                    columns: vec![IndexedColumn {
                        name: "updated_at".into(),
                        ascending: true,
                        type_name: "TEXT".into(),
                    }],
                },
            ];
        },
    ));

    schema.tables.push(Table::create(
        "projects",
        vec![
            Column::text("user_id"),
            Column::text("name"),
            Column::text("color"),
            Column::integer("is_archived"),
            Column::text("created_at"),
            Column::text("prompt_id"),
            Column::text("keyterm_id"),
        ],
        |_| {},
    ));

    schema.tables.push(Table::create(
        "prompts",
        vec![
            Column::text("user_id"),
            Column::text("title"),
            Column::text("description"),
            Column::text("prompt"),
            Column::text("created_at"),
        ],
        |_| {},
    ));

    schema.tables.push(Table::create(
        "keyterms",
        vec![
            Column::text("user_id"),
            Column::text("name"),
            Column::text("description"),
            Column::text("content"),
            Column::text("created_at"),
            Column::text("updated_at"),
        ],
        |t| {
            t.indexes = vec![Index {
                name: "keyterms_user".into(),
                columns: vec![IndexedColumn {
                    name: "user_id".into(),
                    ascending: true,
                    type_name: "TEXT".into(),
                }],
            }];
        },
    ));

    schema.tables.push(Table::create(
        "note_extractions",
        vec![
            Column::text("note_id"),
            Column::text("user_id"),
            Column::text("type"),
            Column::text("value"),
        ],
        |t| {
            t.indexes = vec![
                Index {
                    name: "note_extractions_note_id_idx".into(),
                    columns: vec![IndexedColumn {
                        name: "note_id".into(),
                        ascending: true,
                        type_name: "TEXT".into(),
                    }],
                },
                Index {
                    name: "note_extractions_type_idx".into(),
                    columns: vec![IndexedColumn {
                        name: "type".into(),
                        ascending: true,
                        type_name: "TEXT".into(),
                    }],
                },
            ];
        },
    ));

    schema.tables.push(Table::create(
        "tc_tasks",
        vec![
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
        |t| {
            t.indexes = vec![
                Index {
                    name: "tc_tasks_status".into(),
                    columns: vec![IndexedColumn {
                        name: "status".into(),
                        ascending: true,
                        type_name: "TEXT".into(),
                    }],
                },
                Index {
                    name: "tc_tasks_parent".into(),
                    columns: vec![IndexedColumn {
                        name: "parent_id".into(),
                        ascending: true,
                        type_name: "TEXT".into(),
                    }],
                },
            ];
        },
    ));

    schema.tables.push(Table::create(
        "tc_operations",
        vec![
            Column::text("user_id"),
            Column::text("data"),
            Column::text("created_at"),
        ],
        |t| {
            t.options.local_only = true;
        },
    ));

    schema.tables.push(Table::create(
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
    ));

    schema
}
