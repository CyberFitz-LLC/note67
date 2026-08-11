use rusqlite::Connection;

#[allow(dead_code)]
pub const SCHEMA_VERSION: i32 = 16;

pub fn run_migrations(conn: &Connection) -> rusqlite::Result<()> {
    let version = get_schema_version(conn)?;

    if version < 1 {
        migrate_v1(conn)?;
    }
    if version < 2 {
        migrate_v2(conn)?;
    }
    if version < 3 {
        migrate_v3(conn)?;
    }
    if version < 4 {
        migrate_v4(conn)?;
    }
    if version < 5 {
        migrate_v5(conn)?;
    }
    if version < 6 {
        migrate_v6(conn)?;
    }
    if version < 7 {
        migrate_v7(conn)?;
    }
    if version < 8 {
        migrate_v8(conn)?;
    }
    if version < 9 {
        migrate_v9(conn)?;
    }
    if version < 10 {
        migrate_v10(conn)?;
    }
    if version < 11 {
        migrate_v11(conn)?;
    }
    if version < 12 {
        migrate_v12(conn)?;
    }
    if version < 13 {
        migrate_v13(conn)?;
    }
    if version < 14 {
        migrate_v14(conn)?;
    }
    if version < 15 {
        migrate_v15(conn)?;
    }
    if version < 16 {
        migrate_v16(conn)?;
    }

    Ok(())
}

fn get_schema_version(conn: &Connection) -> rusqlite::Result<i32> {
    // Create schema_version table if it doesn't exist
    conn.execute(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER PRIMARY KEY
        )",
        [],
    )?;

    let version: i32 = conn
        .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
            row.get(0)
        })
        .unwrap_or(0);

    Ok(version)
}

fn set_schema_version(conn: &Connection, version: i32) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM schema_version", [])?;
    conn.execute("INSERT INTO schema_version (version) VALUES (?1)", [version])?;
    Ok(())
}

fn migrate_v1(conn: &Connection) -> rusqlite::Result<()> {
    // Notes table
    conn.execute(
        "CREATE TABLE IF NOT EXISTS notes (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            started_at TEXT NOT NULL,
            ended_at TEXT,
            audio_path TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
        [],
    )?;

    // Transcript segments table
    conn.execute(
        "CREATE TABLE IF NOT EXISTS transcript_segments (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            note_id TEXT NOT NULL,
            start_time REAL NOT NULL,
            end_time REAL NOT NULL,
            text TEXT NOT NULL,
            speaker TEXT,
            created_at TEXT NOT NULL,
            FOREIGN KEY (note_id) REFERENCES notes(id) ON DELETE CASCADE
        )",
        [],
    )?;

    // Index for faster transcript lookups
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_transcript_note
         ON transcript_segments(note_id)",
        [],
    )?;

    // Summaries table
    conn.execute(
        "CREATE TABLE IF NOT EXISTS summaries (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            note_id TEXT NOT NULL,
            summary_type TEXT NOT NULL,
            content TEXT NOT NULL,
            created_at TEXT NOT NULL,
            FOREIGN KEY (note_id) REFERENCES notes(id) ON DELETE CASCADE
        )",
        [],
    )?;

    // Index for faster summary lookups
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_summary_note
         ON summaries(note_id)",
        [],
    )?;

    set_schema_version(conn, 1)?;

    Ok(())
}

fn migrate_v2(conn: &Connection) -> rusqlite::Result<()> {
    // Add description and participants columns to notes
    conn.execute(
        "ALTER TABLE notes ADD COLUMN description TEXT",
        [],
    )?;
    conn.execute(
        "ALTER TABLE notes ADD COLUMN participants TEXT",
        [],
    )?;

    // Create full-text search index for note search
    conn.execute(
        "CREATE VIRTUAL TABLE IF NOT EXISTS notes_fts USING fts5(
            title,
            description,
            participants,
            content='notes',
            content_rowid='rowid'
        )",
        [],
    )?;

    // Create triggers to keep FTS in sync
    conn.execute(
        "CREATE TRIGGER IF NOT EXISTS notes_ai AFTER INSERT ON notes BEGIN
            INSERT INTO notes_fts(rowid, title, description, participants)
            VALUES (NEW.rowid, NEW.title, NEW.description, NEW.participants);
        END",
        [],
    )?;

    conn.execute(
        "CREATE TRIGGER IF NOT EXISTS notes_ad AFTER DELETE ON notes BEGIN
            INSERT INTO notes_fts(notes_fts, rowid, title, description, participants)
            VALUES ('delete', OLD.rowid, OLD.title, OLD.description, OLD.participants);
        END",
        [],
    )?;

    conn.execute(
        "CREATE TRIGGER IF NOT EXISTS notes_au AFTER UPDATE ON notes BEGIN
            INSERT INTO notes_fts(notes_fts, rowid, title, description, participants)
            VALUES ('delete', OLD.rowid, OLD.title, OLD.description, OLD.participants);
            INSERT INTO notes_fts(rowid, title, description, participants)
            VALUES (NEW.rowid, NEW.title, NEW.description, NEW.participants);
        END",
        [],
    )?;

    set_schema_version(conn, 2)?;

    Ok(())
}

fn migrate_v3(conn: &Connection) -> rusqlite::Result<()> {
    // Settings table for app preferences
    conn.execute(
        "CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        )",
        [],
    )?;

    // Insert default theme preference
    conn.execute(
        "INSERT OR IGNORE INTO settings (key, value) VALUES ('theme', 'system')",
        [],
    )?;

    set_schema_version(conn, 3)?;

    Ok(())
}

fn migrate_v4(conn: &Connection) -> rusqlite::Result<()> {
    // Audio segments table for multi-session recordings (pause/resume/continue)
    conn.execute(
        "CREATE TABLE IF NOT EXISTS audio_segments (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            note_id TEXT NOT NULL,
            segment_index INTEGER NOT NULL,
            mic_path TEXT NOT NULL,
            system_path TEXT,
            start_offset_ms INTEGER NOT NULL,
            duration_ms INTEGER,
            created_at TEXT NOT NULL,
            FOREIGN KEY (note_id) REFERENCES notes(id) ON DELETE CASCADE
        )",
        [],
    )?;

    // Index for faster segment lookups by note
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_audio_segments_note
         ON audio_segments(note_id)",
        [],
    )?;

    set_schema_version(conn, 4)?;

    Ok(())
}

fn migrate_v5(conn: &Connection) -> rusqlite::Result<()> {
    // Uploaded audio table for imported audio files
    conn.execute(
        "CREATE TABLE IF NOT EXISTS uploaded_audio (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            note_id TEXT NOT NULL,
            file_path TEXT NOT NULL,
            original_filename TEXT NOT NULL,
            duration_ms INTEGER,
            speaker_label TEXT NOT NULL DEFAULT 'Uploaded',
            transcription_status TEXT NOT NULL DEFAULT 'pending',
            created_at TEXT NOT NULL,
            FOREIGN KEY (note_id) REFERENCES notes(id) ON DELETE CASCADE
        )",
        [],
    )?;

    // Index for faster lookups by note
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_uploaded_audio_note
         ON uploaded_audio(note_id)",
        [],
    )?;

    set_schema_version(conn, 5)?;

    Ok(())
}

fn migrate_v6(conn: &Connection) -> rusqlite::Result<()> {
    // Add display_order to audio_segments for reordering
    conn.execute(
        "ALTER TABLE audio_segments ADD COLUMN display_order INTEGER NOT NULL DEFAULT 0",
        [],
    )?;

    // Add display_order to uploaded_audio for reordering
    conn.execute(
        "ALTER TABLE uploaded_audio ADD COLUMN display_order INTEGER NOT NULL DEFAULT 0",
        [],
    )?;

    // Set initial display_order based on creation order
    conn.execute(
        "UPDATE audio_segments SET display_order = segment_index",
        [],
    )?;

    conn.execute(
        "UPDATE uploaded_audio SET display_order = id",
        [],
    )?;

    set_schema_version(conn, 6)?;

    Ok(())
}

fn migrate_v7(conn: &Connection) -> rusqlite::Result<()> {
    // Add source tracking columns to transcript_segments
    // source_type: 'upload' (from uploaded_audio), 'segment' (from audio_segments), 'live' (from live transcription)
    // source_id: the id of the source record (uploaded_audio.id or audio_segments.id)
    conn.execute(
        "ALTER TABLE transcript_segments ADD COLUMN source_type TEXT",
        [],
    )?;

    conn.execute(
        "ALTER TABLE transcript_segments ADD COLUMN source_id INTEGER",
        [],
    )?;

    // Create index for faster deletion by source
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_transcript_source
         ON transcript_segments(source_type, source_id)",
        [],
    )?;

    set_schema_version(conn, 7)?;

    Ok(())
}

fn migrate_v8(conn: &Connection) -> rusqlite::Result<()> {
    // Tags table (unique tag names)
    conn.execute(
        "CREATE TABLE IF NOT EXISTS tags (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT UNIQUE NOT NULL,
            color TEXT,
            created_at TEXT NOT NULL
        )",
        [],
    )?;

    // Note-tag junction table
    conn.execute(
        "CREATE TABLE IF NOT EXISTS note_tags (
            note_id TEXT NOT NULL,
            tag_id INTEGER NOT NULL,
            created_at TEXT NOT NULL,
            PRIMARY KEY (note_id, tag_id),
            FOREIGN KEY (note_id) REFERENCES notes(id) ON DELETE CASCADE,
            FOREIGN KEY (tag_id) REFERENCES tags(id) ON DELETE CASCADE
        )",
        [],
    )?;

    // Indexes for faster lookups
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_note_tags_note ON note_tags(note_id)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_note_tags_tag ON note_tags(tag_id)",
        [],
    )?;

    set_schema_version(conn, 8)?;

    Ok(())
}

fn migrate_v9(conn: &Connection) -> rusqlite::Result<()> {
    // Note links table for wiki-style [[Note Title]] links
    conn.execute(
        "CREATE TABLE IF NOT EXISTS note_links (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source_note_id TEXT NOT NULL,
            target_note_id TEXT,
            target_title TEXT NOT NULL,
            created_at TEXT NOT NULL,
            FOREIGN KEY (source_note_id) REFERENCES notes(id) ON DELETE CASCADE,
            FOREIGN KEY (target_note_id) REFERENCES notes(id) ON DELETE SET NULL
        )",
        [],
    )?;

    // Indexes for faster lookups
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_note_links_source ON note_links(source_note_id)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_note_links_target ON note_links(target_note_id)",
        [],
    )?;

    set_schema_version(conn, 9)?;

    Ok(())
}

fn migrate_v10(conn: &Connection) -> rusqlite::Result<()> {
    // Make audio_segments.mic_path nullable to support listen-only (system-audio-only) recordings.
    // SQLite cannot drop NOT NULL in place, so recreate the table.
    conn.execute_batch(
        "BEGIN;
         CREATE TABLE audio_segments_new (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             note_id TEXT NOT NULL,
             segment_index INTEGER NOT NULL,
             mic_path TEXT,
             system_path TEXT,
             start_offset_ms INTEGER NOT NULL,
             duration_ms INTEGER,
             display_order INTEGER NOT NULL DEFAULT 0,
             created_at TEXT NOT NULL,
             FOREIGN KEY (note_id) REFERENCES notes(id) ON DELETE CASCADE
         );
         INSERT INTO audio_segments_new
             (id, note_id, segment_index, mic_path, system_path, start_offset_ms, duration_ms, display_order, created_at)
         SELECT id, note_id, segment_index, mic_path, system_path, start_offset_ms, duration_ms, display_order, created_at
         FROM audio_segments;
         DROP TABLE audio_segments;
         ALTER TABLE audio_segments_new RENAME TO audio_segments;
         CREATE INDEX IF NOT EXISTS idx_audio_segments_note ON audio_segments(note_id);
         COMMIT;",
    )?;

    set_schema_version(conn, 10)?;

    Ok(())
}

fn migrate_v11(conn: &Connection) -> rusqlite::Result<()> {
    // Action items derived from note bodies (inline GFM checkboxes are the
    // source of truth; this table is a queryable index for the global Tasks
    // view). `stable_id` is a hash of note_id + normalized text so a note can
    // be re-synced idempotently without losing checked state.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS action_items (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             note_id TEXT NOT NULL,
             stable_id TEXT NOT NULL,
             text TEXT NOT NULL,
             assignee TEXT,
             due_date TEXT,
             done INTEGER NOT NULL DEFAULT 0,
             sort_order INTEGER NOT NULL DEFAULT 0,
             created_at TEXT NOT NULL,
             updated_at TEXT NOT NULL,
             FOREIGN KEY (note_id) REFERENCES notes(id) ON DELETE CASCADE,
             UNIQUE (note_id, stable_id)
         );
         CREATE INDEX IF NOT EXISTS idx_action_items_note ON action_items(note_id);
         CREATE INDEX IF NOT EXISTS idx_action_items_open ON action_items(done, due_date);",
    )?;

    set_schema_version(conn, 11)?;

    Ok(())
}

fn migrate_v12(conn: &Connection) -> rusqlite::Result<()> {
    // Action items gain a description and can nest as subtasks (parent_id points
    // at another action item; deleting a parent deletes its children in code).
    conn.execute_batch(
        "ALTER TABLE action_items ADD COLUMN description TEXT;
         ALTER TABLE action_items ADD COLUMN parent_id INTEGER;
         CREATE INDEX IF NOT EXISTS idx_action_items_parent ON action_items(parent_id);",
    )?;

    set_schema_version(conn, 12)?;

    Ok(())
}

fn migrate_v13(conn: &Connection) -> rusqlite::Result<()> {
    // Allow standalone tasks (note_id NULL) so the central Tasks page can add
    // tasks not tied to any note. SQLite can't drop NOT NULL in place; recreate.
    conn.execute_batch(
        "BEGIN;
         CREATE TABLE action_items_new (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             note_id TEXT,
             stable_id TEXT NOT NULL,
             text TEXT NOT NULL,
             assignee TEXT,
             due_date TEXT,
             done INTEGER NOT NULL DEFAULT 0,
             sort_order INTEGER NOT NULL DEFAULT 0,
             created_at TEXT NOT NULL,
             updated_at TEXT NOT NULL,
             description TEXT,
             parent_id INTEGER,
             FOREIGN KEY (note_id) REFERENCES notes(id) ON DELETE CASCADE
         );
         INSERT INTO action_items_new
             (id, note_id, stable_id, text, assignee, due_date, done, sort_order, created_at, updated_at, description, parent_id)
         SELECT id, note_id, stable_id, text, assignee, due_date, done, sort_order, created_at, updated_at, description, parent_id
         FROM action_items;
         DROP TABLE action_items;
         ALTER TABLE action_items_new RENAME TO action_items;
         CREATE INDEX IF NOT EXISTS idx_action_items_note ON action_items(note_id);
         CREATE INDEX IF NOT EXISTS idx_action_items_open ON action_items(done, due_date);
         CREATE INDEX IF NOT EXISTS idx_action_items_parent ON action_items(parent_id);
         COMMIT;",
    )?;

    set_schema_version(conn, 13)?;

    Ok(())
}

fn migrate_v14(conn: &Connection) -> rusqlite::Result<()> {
    // The transcript version chain. Append-only: rows are never updated except
    // to record a receipt hash once a version is attested.
    //
    // content_hash is the hash of the canonical bytes (see
    // exochain::transcript), and serialization records which canonical form
    // produced it, so a future format can coexist with receipts minted under
    // this one instead of invalidating them.
    conn.execute_batch(
        "BEGIN;
         CREATE TABLE IF NOT EXISTS transcript_versions (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             note_id TEXT NOT NULL,
             version INTEGER NOT NULL,
             content_hash TEXT NOT NULL,
             parent_hash TEXT,
             serialization TEXT NOT NULL,
             origin TEXT NOT NULL,
             reason TEXT NOT NULL,
             segment_count INTEGER NOT NULL,
             created_at TEXT NOT NULL,
             receipt_hash TEXT,
             attested_at TEXT,
             UNIQUE(note_id, version),
             FOREIGN KEY (note_id) REFERENCES notes(id) ON DELETE CASCADE
         );
         CREATE INDEX IF NOT EXISTS idx_transcript_versions_note
             ON transcript_versions(note_id, version);
         COMMIT;",
    )?;

    set_schema_version(conn, 14)?;
    Ok(())
}

fn migrate_v15(conn: &Connection) -> rusqlite::Result<()> {
    // Where an imported transcript came from. A receipt over an import can only
    // claim that this content arrived and has not changed since, so it has to
    // name what produced it — otherwise the receipt reads like one over a
    // transcript we made ourselves.
    //
    // Null for recorded transcripts, which have no external source.
    conn.execute_batch(
        "BEGIN;
         ALTER TABLE transcript_versions ADD COLUMN source_tool TEXT;
         ALTER TABLE transcript_versions ADD COLUMN source_filename TEXT;
         COMMIT;",
    )?;

    set_schema_version(conn, 15)?;
    Ok(())
}

fn migrate_v16(conn: &Connection) -> rusqlite::Result<()> {
    // Sync bookkeeping.
    //
    // Change tracking is done with triggers rather than by editing every
    // command that writes. The app works today; threading a "mark dirty" call
    // through forty call sites is forty chances to miss one, and a missed one
    // is a note that silently never syncs. The database sees every write by
    // definition.
    //
    // Nothing here runs unless the user signs in. Recording, transcription and
    // the local chain never need any of it — the rows simply accumulate and are
    // read by nobody.
    conn.execute_batch(
        "BEGIN;

         -- Cursor, account, device registration. One row per key so a new piece
         -- of state does not need a migration.
         CREATE TABLE IF NOT EXISTS sync_state (
             key   TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );

         -- What has changed locally and not yet reached the archive.
         --
         -- client_change_id is regenerated on every write, which is what makes
         -- a retry safe: if the record is edited again while a push is in
         -- flight, the new edit carries a new id and is pushed on its own. Were
         -- the id stable per record, the server would replay the first
         -- outcome and the second edit would never land.
         CREATE TABLE IF NOT EXISTS sync_dirty (
             kind             TEXT NOT NULL,
             note_id          TEXT NOT NULL,
             entity_id        TEXT NOT NULL,
             client_change_id TEXT NOT NULL,
             deleted          INTEGER NOT NULL DEFAULT 0,
             queued_at        TEXT NOT NULL,
             PRIMARY KEY (kind, note_id, entity_id)
         );

         -- Held while incoming changes are being written.
         --
         -- Without this the tracking triggers cannot tell a local edit from a
         -- change that just arrived from the archive: applying one would queue
         -- it to be pushed straight back, and two devices would bounce the same
         -- note between them for ever. Every trigger below is guarded on this
         -- table being empty.
         --
         -- A table rather than a Rust flag because the guard has to be visible
         -- to SQLite, inside the trigger, at the moment the write happens.
         CREATE TABLE IF NOT EXISTS sync_applying (
             active INTEGER PRIMARY KEY CHECK (active = 1)
         );

         -- Notes removed from this device but deliberately left in the archive.
         --
         -- Without this, 'Remove from this device' would pull the note straight
         -- back on the next sync, which would read as the app ignoring you.
         CREATE TABLE IF NOT EXISTS sync_suppressed (
             note_id       TEXT PRIMARY KEY,
             suppressed_at TEXT NOT NULL
         );

         -- Summaries have no identity that survives leaving this machine: the
         -- primary key is a local autoincrement, and a note can hold several
         -- summaries of the same type, so nothing about the content
         -- distinguishes them either. Everything else already has one —
         -- action items carry stable_id, and tags and links are identified by
         -- their content, which is what makes the same tag applied on two
         -- devices converge rather than duplicate.
         ALTER TABLE summaries ADD COLUMN sync_uid TEXT;
         COMMIT;",
    )?;

    // Existing rows predate the column. Done as an UPDATE rather than a column
    // default because SQLite will not accept a non-constant default on ADD
    // COLUMN, and randomblob is the point.
    conn.execute(
        "UPDATE summaries SET sync_uid = lower(hex(randomblob(16))) WHERE sync_uid IS NULL",
        [],
    )?;

    conn.execute_batch(
        "BEGIN;

         CREATE TRIGGER IF NOT EXISTS summaries_sync_uid AFTER INSERT ON summaries
         WHEN new.sync_uid IS NULL
         BEGIN
             UPDATE summaries SET sync_uid = lower(hex(randomblob(16))) WHERE id = new.id;
         END;

         -- Notes.
         CREATE TRIGGER IF NOT EXISTS notes_sync_ins AFTER INSERT ON notes
         WHEN NOT EXISTS (SELECT 1 FROM sync_applying)
         BEGIN
             INSERT OR REPLACE INTO sync_dirty (kind, note_id, entity_id, client_change_id, deleted, queued_at)
             VALUES ('note', new.id, '', lower(hex(randomblob(16))), 0, datetime('now'));
         END;
         CREATE TRIGGER IF NOT EXISTS notes_sync_upd AFTER UPDATE ON notes
         WHEN NOT EXISTS (SELECT 1 FROM sync_applying)
         BEGIN
             INSERT OR REPLACE INTO sync_dirty (kind, note_id, entity_id, client_change_id, deleted, queued_at)
             VALUES ('note', new.id, '', lower(hex(randomblob(16))), 0, datetime('now'));
         END;

         -- A local delete is NOT an archive delete. It records a suppression so
         -- sync does not pull the note back, and leaves the archive alone.
         -- Deleting from the archive is a separate, explicit action that
         -- enqueues its own tombstone before removing the row.
         CREATE TRIGGER IF NOT EXISTS notes_sync_del AFTER DELETE ON notes
         WHEN NOT EXISTS (SELECT 1 FROM sync_applying)
         BEGIN
             INSERT OR REPLACE INTO sync_suppressed (note_id, suppressed_at)
             VALUES (old.id, datetime('now'));
             -- Every queued change for this note goes, its own included. The
             -- content is gone from this device, so there is nothing left to
             -- push; leaving the note's row would send an update describing a
             -- note this machine can no longer see. An edit made just before
             -- the removal is abandoned, which is the honest outcome of
             -- choosing to remove it.
             DELETE FROM sync_dirty WHERE note_id = old.id;
         END;

         -- Summaries.
         CREATE TRIGGER IF NOT EXISTS summaries_sync_ins AFTER INSERT ON summaries
         WHEN NOT EXISTS (SELECT 1 FROM sync_applying)
         BEGIN
             INSERT OR REPLACE INTO sync_dirty (kind, note_id, entity_id, client_change_id, deleted, queued_at)
             SELECT 'summary', new.note_id, s.sync_uid, lower(hex(randomblob(16))), 0, datetime('now')
             FROM summaries s WHERE s.id = new.id AND s.sync_uid IS NOT NULL;
         END;
         CREATE TRIGGER IF NOT EXISTS summaries_sync_upd AFTER UPDATE ON summaries
         WHEN new.sync_uid IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sync_applying)
         BEGIN
             INSERT OR REPLACE INTO sync_dirty (kind, note_id, entity_id, client_change_id, deleted, queued_at)
             VALUES ('summary', new.note_id, new.sync_uid, lower(hex(randomblob(16))), 0, datetime('now'));
         END;
         CREATE TRIGGER IF NOT EXISTS summaries_sync_del AFTER DELETE ON summaries
         WHEN old.sync_uid IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sync_applying)
         BEGIN
             INSERT OR REPLACE INTO sync_dirty (kind, note_id, entity_id, client_change_id, deleted, queued_at)
             VALUES ('summary', old.note_id, old.sync_uid, lower(hex(randomblob(16))), 1, datetime('now'));
         END;

         -- Action items. Those with no note are standalone tasks: they have no
         -- parent to authorize against, so they stay on this device.
         CREATE TRIGGER IF NOT EXISTS action_items_sync_ins AFTER INSERT ON action_items
         WHEN new.note_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sync_applying)
         BEGIN
             INSERT OR REPLACE INTO sync_dirty (kind, note_id, entity_id, client_change_id, deleted, queued_at)
             VALUES ('actionItem', new.note_id, new.stable_id, lower(hex(randomblob(16))), 0, datetime('now'));
         END;
         CREATE TRIGGER IF NOT EXISTS action_items_sync_upd AFTER UPDATE ON action_items
         WHEN new.note_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sync_applying)
         BEGIN
             INSERT OR REPLACE INTO sync_dirty (kind, note_id, entity_id, client_change_id, deleted, queued_at)
             VALUES ('actionItem', new.note_id, new.stable_id, lower(hex(randomblob(16))), 0, datetime('now'));
         END;
         CREATE TRIGGER IF NOT EXISTS action_items_sync_del AFTER DELETE ON action_items
         WHEN old.note_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sync_applying)
         BEGIN
             INSERT OR REPLACE INTO sync_dirty (kind, note_id, entity_id, client_change_id, deleted, queued_at)
             VALUES ('actionItem', old.note_id, old.stable_id, lower(hex(randomblob(16))), 1, datetime('now'));
         END;

         -- Tags, identified by name. The tag row's id is local, but the name is
         -- unique and is what the tag actually is, so the same tag applied on
         -- two devices converges instead of duplicating.
         CREATE TRIGGER IF NOT EXISTS note_tags_sync_ins AFTER INSERT ON note_tags
         WHEN NOT EXISTS (SELECT 1 FROM sync_applying)
         BEGIN
             INSERT OR REPLACE INTO sync_dirty (kind, note_id, entity_id, client_change_id, deleted, queued_at)
             SELECT 'tag', new.note_id, t.name, lower(hex(randomblob(16))), 0, datetime('now')
             FROM tags t WHERE t.id = new.tag_id;
         END;
         CREATE TRIGGER IF NOT EXISTS note_tags_sync_del AFTER DELETE ON note_tags
         WHEN NOT EXISTS (SELECT 1 FROM sync_applying)
         BEGIN
             INSERT OR REPLACE INTO sync_dirty (kind, note_id, entity_id, client_change_id, deleted, queued_at)
             SELECT 'tag', old.note_id, t.name, lower(hex(randomblob(16))), 1, datetime('now')
             FROM tags t WHERE t.id = old.tag_id;
         END;

         -- Links, identified by what they point at.
         CREATE TRIGGER IF NOT EXISTS note_links_sync_ins AFTER INSERT ON note_links
         WHEN NOT EXISTS (SELECT 1 FROM sync_applying)
         BEGIN
             INSERT OR REPLACE INTO sync_dirty (kind, note_id, entity_id, client_change_id, deleted, queued_at)
             VALUES ('link', new.source_note_id, new.target_title, lower(hex(randomblob(16))), 0, datetime('now'));
         END;
         CREATE TRIGGER IF NOT EXISTS note_links_sync_del AFTER DELETE ON note_links
         WHEN NOT EXISTS (SELECT 1 FROM sync_applying)
         BEGIN
             INSERT OR REPLACE INTO sync_dirty (kind, note_id, entity_id, client_change_id, deleted, queued_at)
             VALUES ('link', old.source_note_id, old.target_title, lower(hex(randomblob(16))), 1, datetime('now'));
         END;

         -- The chain. Append-only, so there is nothing to track but the append:
         -- a version that could be updated or deleted would attest nothing, and
         -- the archive rejects any attempt to rewrite one.
         CREATE TRIGGER IF NOT EXISTS transcript_versions_sync_ins AFTER INSERT ON transcript_versions
         WHEN NOT EXISTS (SELECT 1 FROM sync_applying)
         BEGIN
             INSERT OR REPLACE INTO sync_dirty (kind, note_id, entity_id, client_change_id, deleted, queued_at)
             VALUES ('transcriptVersion', new.note_id, CAST(new.version AS TEXT), lower(hex(randomblob(16))), 0, datetime('now'));
         END;

         CREATE INDEX IF NOT EXISTS idx_sync_dirty_note ON sync_dirty(note_id);
         COMMIT;",
    )?;

    set_schema_version(conn, 16)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A database at the current schema, with nothing in it.
    fn db() -> Connection {
        let conn = Connection::open_in_memory().expect("open");
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        run_migrations(&conn).expect("migrate");
        conn
    }

    fn a_note(conn: &Connection, id: &str) {
        conn.execute(
            "INSERT INTO notes (id, title, started_at, created_at, updated_at)
             VALUES (?1, 'Weekly', 't', 't', 't')",
            [id],
        )
        .unwrap();
    }

    fn dirty(conn: &Connection) -> Vec<(String, String, String, i64)> {
        let mut stmt = conn
            .prepare("SELECT kind, note_id, entity_id, deleted FROM sync_dirty ORDER BY kind, entity_id")
            .unwrap();
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap();
        rows.map(|r| r.unwrap()).collect()
    }

    fn change_id(conn: &Connection, kind: &str) -> String {
        conn.query_row(
            "SELECT client_change_id FROM sync_dirty WHERE kind = ?1",
            [kind],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn migrations_reach_the_current_version() {
        assert_eq!(get_schema_version(&db()).unwrap(), SCHEMA_VERSION);
    }

    #[test]
    fn migrations_are_idempotent() {
        // Every launch runs them. A second pass must be a no-op, or an upgrade
        // would fail on exactly the machines that already upgraded.
        let conn = db();
        run_migrations(&conn).expect("second run");
        assert_eq!(get_schema_version(&conn).unwrap(), SCHEMA_VERSION);
    }

    #[test]
    fn creating_a_note_queues_it_for_sync() {
        // Tracking is done with triggers rather than by editing every command
        // that writes: a missed call site is a note that silently never syncs,
        // and the database sees every write by definition.
        let conn = db();
        a_note(&conn, "n1");
        assert_eq!(dirty(&conn), vec![("note".into(), "n1".into(), String::new(), 0)]);
    }

    #[test]
    fn every_kind_of_content_is_tracked() {
        let conn = db();
        a_note(&conn, "n1");
        conn.execute("INSERT INTO summaries (note_id, summary_type, content, created_at) VALUES ('n1','brief','x','t')", []).unwrap();
        conn.execute("INSERT INTO action_items (note_id, stable_id, text, created_at, updated_at) VALUES ('n1','ai-1','Follow up','t','t')", []).unwrap();
        conn.execute("INSERT INTO tags (name, created_at) VALUES ('standup','t')", []).unwrap();
        conn.execute("INSERT INTO note_tags (note_id, tag_id, created_at) VALUES ('n1',1,'t')", []).unwrap();
        conn.execute("INSERT INTO note_links (source_note_id, target_title, created_at) VALUES ('n1','Other','t')", []).unwrap();
        conn.execute("INSERT INTO transcript_versions (note_id, version, content_hash, serialization, origin, reason, segment_count, created_at) VALUES ('n1',1,'abc','note67.transcript.v1','recorded','initial',2,'t')", []).unwrap();

        let kinds: Vec<String> = dirty(&conn).into_iter().map(|(k, _, _, _)| k).collect();
        assert_eq!(
            kinds,
            vec!["actionItem", "link", "note", "summary", "tag", "transcriptVersion"]
        );
    }

    #[test]
    fn a_summary_gets_an_identity_that_survives_this_machine() {
        // Its primary key is a local autoincrement, and a note can hold several
        // summaries of the same type, so nothing about the content
        // distinguishes them either.
        let conn = db();
        a_note(&conn, "n1");
        conn.execute("INSERT INTO summaries (note_id, summary_type, content, created_at) VALUES ('n1','brief','x','t')", []).unwrap();
        let uid: Option<String> = conn
            .query_row("SELECT sync_uid FROM summaries", [], |r| r.get(0))
            .unwrap();
        assert_eq!(uid.map(|u| u.len()), Some(32));
    }

    #[test]
    fn a_tag_is_identified_by_its_name() {
        // The tag row's id is local. Using the name is what makes the same tag
        // applied on two devices converge instead of duplicating.
        let conn = db();
        a_note(&conn, "n1");
        conn.execute("INSERT INTO tags (name, created_at) VALUES ('standup','t')", []).unwrap();
        conn.execute("INSERT INTO note_tags (note_id, tag_id, created_at) VALUES ('n1',1,'t')", []).unwrap();
        assert!(dirty(&conn)
            .iter()
            .any(|(k, _, e, _)| k == "tag" && e == "standup"));
    }

    #[test]
    fn a_standalone_action_item_is_not_synced() {
        // It has no note to be authorized against, so it stays here.
        let conn = db();
        conn.execute("INSERT INTO action_items (note_id, stable_id, text, created_at, updated_at) VALUES (NULL,'ai-x','Personal','t','t')", []).unwrap();
        assert!(dirty(&conn).is_empty());
    }

    #[test]
    fn editing_a_record_mints_a_new_change_id() {
        // The subtle one. If the id were stable per record, an edit made while
        // a push was in flight would be swallowed: the server replays the first
        // outcome for a repeated change id, so the second edit would never land.
        let conn = db();
        a_note(&conn, "n1");
        let first = change_id(&conn, "note");
        conn.execute("UPDATE notes SET title = 'Weekly sync' WHERE id = 'n1'", []).unwrap();
        assert_ne!(first, change_id(&conn, "note"));
    }

    #[test]
    fn removing_a_tag_queues_a_tombstone() {
        let conn = db();
        a_note(&conn, "n1");
        conn.execute("INSERT INTO tags (name, created_at) VALUES ('standup','t')", []).unwrap();
        conn.execute("INSERT INTO note_tags (note_id, tag_id, created_at) VALUES ('n1',1,'t')", []).unwrap();
        conn.execute("DELETE FROM note_tags WHERE note_id='n1' AND tag_id=1", []).unwrap();
        assert!(dirty(&conn)
            .iter()
            .any(|(k, _, e, d)| k == "tag" && e == "standup" && *d == 1));
    }

    #[test]
    fn deleting_a_note_locally_suppresses_it_rather_than_the_archive() {
        // Two deletes, deliberately distinct. Removing a note from this device
        // must not reach the archive — and without the suppression the next
        // sync would pull it straight back, which reads as the app ignoring you.
        let conn = db();
        a_note(&conn, "n1");
        conn.execute("INSERT INTO summaries (note_id, summary_type, content, created_at) VALUES ('n1','brief','x','t')", []).unwrap();
        conn.execute("DELETE FROM notes WHERE id = 'n1'", []).unwrap();

        let suppressed: i64 = conn
            .query_row("SELECT COUNT(*) FROM sync_suppressed WHERE note_id='n1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(suppressed, 1);

        // Nothing queued survives, the note's own row included: the content is
        // gone from this device, so an update describing it would describe
        // something this machine can no longer see.
        assert!(dirty(&conn).is_empty(), "queued work outlived the note: {:?}", dirty(&conn));
    }

    /// Writing incoming changes, with tracking suspended.
    ///
    /// Mirrors what the sync engine does: hold the marker across the whole
    /// apply, so nothing written during it is mistaken for a local edit.
    fn while_applying(conn: &Connection, work: impl FnOnce(&Connection)) {
        conn.execute("INSERT OR IGNORE INTO sync_applying (active) VALUES (1)", [])
            .unwrap();
        work(conn);
        conn.execute("DELETE FROM sync_applying", []).unwrap();
    }

    #[test]
    fn a_change_arriving_from_the_archive_is_not_queued_back_to_it() {
        // The echo. Applying an incoming change writes to the same tables a
        // local edit does, so without a guard the triggers would queue it for
        // push and two devices would bounce the note between them for ever.
        let conn = db();
        while_applying(&conn, |c| {
            a_note(c, "n1");
            c.execute("INSERT INTO summaries (note_id, summary_type, content, created_at) VALUES ('n1','brief','x','t')", []).unwrap();
            c.execute("INSERT INTO tags (name, created_at) VALUES ('standup','t')", []).unwrap();
            c.execute("INSERT INTO note_tags (note_id, tag_id, created_at) VALUES ('n1',1,'t')", []).unwrap();
            c.execute("INSERT INTO transcript_versions (note_id, version, content_hash, serialization, origin, reason, segment_count, created_at) VALUES ('n1',1,'abc','note67.transcript.v1','recorded','initial',2,'t')", []).unwrap();
        });
        assert!(dirty(&conn).is_empty(), "an applied change was queued back: {:?}", dirty(&conn));

        // And the note really did arrive — the guard suppresses tracking, not
        // the write itself.
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM notes WHERE id='n1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn tracking_resumes_once_the_apply_is_over() {
        // A guard that leaked would be worse than no guard: local edits would
        // stop syncing silently, and only on the machines that had ever pulled.
        let conn = db();
        while_applying(&conn, |c| a_note(c, "n1"));
        assert!(dirty(&conn).is_empty());

        conn.execute("UPDATE notes SET title = 'Edited here' WHERE id = 'n1'", []).unwrap();
        assert_eq!(dirty(&conn).len(), 1);
    }

    #[test]
    fn a_note_removed_by_the_archive_is_not_suppressed_locally() {
        // Suppression means "this device chose not to hold this note". A
        // tombstone arriving from the archive is not that choice, and recording
        // it as one would block the note from ever returning if it came back.
        let conn = db();
        a_note(&conn, "n1");
        while_applying(&conn, |c| {
            c.execute("DELETE FROM notes WHERE id = 'n1'", []).unwrap();
        });
        let suppressed: i64 = conn
            .query_row("SELECT COUNT(*) FROM sync_suppressed", [], |r| r.get(0))
            .unwrap();
        assert_eq!(suppressed, 0);
    }

    #[test]
    fn suppressing_the_same_note_twice_is_harmless() {
        let conn = db();
        for _ in 0..2 {
            a_note(&conn, "n1");
            conn.execute("DELETE FROM notes WHERE id = 'n1'", []).unwrap();
        }
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM sync_suppressed", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }
}
