-- Deployed v1 disposable index, retained only as an upgrade test fixture.
        CREATE TABLE IF NOT EXISTS note_search_meta (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            schema_version INTEGER NOT NULL
        );
        CREATE VIRTUAL TABLE IF NOT EXISTS note_search_fts USING fts5(
            uuid UNINDEXED, short_id UNINDEXED, project_id UNINDEXED,
            created_at UNINDEXED, updated_at UNINDEXED,
            title, summary, content,
            tokenize='better_trigram', detail=full
        );
        CREATE TRIGGER IF NOT EXISTS note_search_insert AFTER INSERT ON ps_data__notes
        WHEN json_extract(new.data, '$.deleted_at') IS NULL
        BEGIN
            INSERT INTO note_search_fts
                (rowid, uuid, short_id, project_id, created_at, updated_at, title, summary, content)
            VALUES (new.rowid, new.id,
                json_extract(new.data, '$.short_id'), json_extract(new.data, '$.project_id'),
                json_extract(new.data, '$.created_at'), json_extract(new.data, '$.updated_at'),
                json_extract(new.data, '$.title'), json_extract(new.data, '$.summary'),
                json_extract(new.data, '$.content'));
        END;
        CREATE TRIGGER IF NOT EXISTS note_search_update AFTER UPDATE ON ps_data__notes
        BEGIN
            DELETE FROM note_search_fts WHERE rowid=old.rowid;
            INSERT INTO note_search_fts
                (rowid, uuid, short_id, project_id, created_at, updated_at, title, summary, content)
            SELECT new.rowid, new.id,
                json_extract(new.data, '$.short_id'), json_extract(new.data, '$.project_id'),
                json_extract(new.data, '$.created_at'), json_extract(new.data, '$.updated_at'),
                json_extract(new.data, '$.title'), json_extract(new.data, '$.summary'),
                json_extract(new.data, '$.content')
            WHERE json_extract(new.data, '$.deleted_at') IS NULL;
        END;
        CREATE TRIGGER IF NOT EXISTS note_search_delete AFTER DELETE ON ps_data__notes
        BEGIN
            DELETE FROM note_search_fts WHERE rowid=old.rowid;
        END;
        INSERT INTO note_search_fts
            (rowid, uuid, short_id, project_id, created_at, updated_at, title, summary, content)
        SELECT n.rowid, n.id,
            json_extract(n.data, '$.short_id'), json_extract(n.data, '$.project_id'),
            json_extract(n.data, '$.created_at'), json_extract(n.data, '$.updated_at'),
            json_extract(n.data, '$.title'), json_extract(n.data, '$.summary'),
            json_extract(n.data, '$.content')
        FROM ps_data__notes n
        WHERE json_extract(n.data, '$.deleted_at') IS NULL
          AND NOT EXISTS (SELECT 1 FROM note_search_fts f WHERE f.rowid=n.rowid);

INSERT INTO note_search_meta(id, schema_version) VALUES(1, 1);
