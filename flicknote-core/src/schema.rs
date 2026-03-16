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
        ],
        |_| {},
    ));

    schema.tables.push(Table::create(
        "note_extractions",
        vec![
            Column::text("note_id"),
            Column::text("user_id"),
            Column::text("type"),
            Column::text("value"),
        ],
        |_| {},
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
        |_| {},
    ));

    schema
}
