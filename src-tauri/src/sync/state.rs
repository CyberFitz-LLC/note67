//! Where sync keeps its place.
//!
//! The cursor, the signed-in account, and the guard that stops incoming
//! changes being mistaken for local edits.

use rusqlite::Connection;

/// Keys in `sync_state`. Strings rather than columns so a new piece of state
/// does not need a migration.
pub const CURSOR: &str = "cursor";
pub const USER_OID: &str = "userOid";
pub const TENANT_ID: &str = "tenantId";
pub const ACCOUNT_NAME: &str = "accountName";
pub const DEVICE_REGISTERED: &str = "deviceRegistered";

pub fn get(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT value FROM sync_state WHERE key = ?1", [key], |r| {
        r.get(0)
    })
    .ok()
    .map_or(Ok(None), |v: String| Ok(Some(v)))
}

pub fn set(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO sync_state (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [key, value],
    )?;
    Ok(())
}

pub fn clear(conn: &Connection, key: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM sync_state WHERE key = ?1", [key])?;
    Ok(())
}

/// How far this device has read the archive's feed.
///
/// Zero when absent, which is what makes a fresh install pull everything rather
/// than nothing — the failure that would look like sync working while showing
/// an empty library.
pub fn cursor(conn: &Connection) -> rusqlite::Result<i64> {
    Ok(get(conn, CURSOR)?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0))
}

/// Move the cursor forward.
///
/// Never backwards. A page that came back empty reports the cursor it was
/// given, and a lower value arriving from anywhere else would replay changes
/// this device has already applied.
pub fn advance_cursor(conn: &Connection, to: i64) -> rusqlite::Result<i64> {
    let current = cursor(conn)?;
    let next = current.max(to);
    set(conn, CURSOR, &next.to_string())?;
    Ok(next)
}

/// Suspends change tracking for as long as it is held.
///
/// Incoming changes are written to the same tables a local edit touches, so
/// without this the triggers would queue them straight back to the archive and
/// two devices would bounce a note between them for ever.
///
/// Released on drop, including when the apply panics part way. A guard left set
/// would be worse than none: local edits would silently stop syncing, and only
/// on machines that had ever pulled.
pub struct ApplyGuard<'a> {
    conn: &'a Connection,
}

impl<'a> ApplyGuard<'a> {
    pub fn new(conn: &'a Connection) -> rusqlite::Result<Self> {
        conn.execute("INSERT OR IGNORE INTO sync_applying (active) VALUES (1)", [])?;
        Ok(Self { conn })
    }
}

impl Drop for ApplyGuard<'_> {
    fn drop(&mut self) {
        // Nothing useful to do with a failure here, and propagating it would
        // mean a panic during unwinding. The next guard's INSERT OR IGNORE and
        // its own drop will clear it.
        let _ = self.conn.execute("DELETE FROM sync_applying", []);
    }
}

/// Whether tracking is currently suspended.
pub fn is_applying(conn: &Connection) -> rusqlite::Result<bool> {
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM sync_applying", [], |r| r.get(0))?;
    Ok(n > 0)
}

/// Forget the account, but keep the notes.
///
/// Signing out is not a delete. The local library is the authoritative copy and
/// the app works signed out, so everything stays; only the ability to reach the
/// archive goes away.
///
/// The cursor goes too. A different account would see a different feed, and
/// resuming from a stale position would skip everything the new account had
/// before this moment.
pub fn sign_out(conn: &Connection) -> rusqlite::Result<()> {
    for key in [
        CURSOR,
        USER_OID,
        TENANT_ID,
        ACCOUNT_NAME,
        DEVICE_REGISTERED,
    ] {
        clear(conn, key)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn
    }

    #[test]
    fn a_missing_key_reads_as_absent() {
        assert_eq!(get(&db(), "nothing").unwrap(), None);
    }

    #[test]
    fn a_value_round_trips() {
        let conn = db();
        set(&conn, USER_OID, "abc").unwrap();
        assert_eq!(get(&conn, USER_OID).unwrap().as_deref(), Some("abc"));
    }

    #[test]
    fn setting_a_key_twice_replaces_it() {
        let conn = db();
        set(&conn, CURSOR, "1").unwrap();
        set(&conn, CURSOR, "2").unwrap();
        assert_eq!(cursor(&conn).unwrap(), 2);
    }

    #[test]
    fn a_fresh_install_starts_at_the_beginning_of_the_feed() {
        // Not at the end. Starting there would show an empty library while
        // reporting that sync had worked.
        assert_eq!(cursor(&db()).unwrap(), 0);
    }

    #[test]
    fn an_unparseable_cursor_reads_as_the_beginning() {
        // Re-reading the whole feed is wasteful; skipping it silently is not
        // recoverable.
        let conn = db();
        set(&conn, CURSOR, "not a number").unwrap();
        assert_eq!(cursor(&conn).unwrap(), 0);
    }

    #[test]
    fn the_cursor_moves_forward() {
        let conn = db();
        assert_eq!(advance_cursor(&conn, 12).unwrap(), 12);
        assert_eq!(cursor(&conn).unwrap(), 12);
    }

    #[test]
    fn the_cursor_never_moves_backwards() {
        // A lower value would replay changes already applied.
        let conn = db();
        advance_cursor(&conn, 30).unwrap();
        assert_eq!(advance_cursor(&conn, 5).unwrap(), 30);
        assert_eq!(cursor(&conn).unwrap(), 30);
    }

    #[test]
    fn the_guard_suspends_tracking_while_it_lives() {
        let conn = db();
        assert!(!is_applying(&conn).unwrap());
        {
            let _guard = ApplyGuard::new(&conn).unwrap();
            assert!(is_applying(&conn).unwrap());
        }
        assert!(!is_applying(&conn).unwrap());
    }

    #[test]
    fn the_guard_is_released_even_when_the_apply_panics() {
        // A guard left set would silently stop local edits from ever syncing,
        // and only on machines that had pulled at least once.
        let conn = db();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = ApplyGuard::new(&conn).unwrap();
            panic!("apply failed part way");
        }));
        assert!(result.is_err());
        assert!(!is_applying(&conn).unwrap(), "tracking stayed suspended");
    }

    #[test]
    fn nested_guards_do_not_double_count() {
        // The marker is a single row, so an inner guard finishing must not look
        // like the outer one finishing. It does end tracking early — which is
        // why applies are never nested, and this pins the behaviour rather than
        // pretending it is safe.
        let conn = db();
        let outer = ApplyGuard::new(&conn).unwrap();
        {
            let _inner = ApplyGuard::new(&conn).unwrap();
        }
        assert!(!is_applying(&conn).unwrap());
        drop(outer);
    }

    #[test]
    fn signing_out_keeps_the_notes_and_forgets_the_account() {
        let conn = db();
        conn.execute(
            "INSERT INTO notes (id, title, started_at, created_at, updated_at)
             VALUES ('n1','Weekly','t','t','t')",
            [],
        )
        .unwrap();
        set(&conn, USER_OID, "user").unwrap();
        set(&conn, CURSOR, "42").unwrap();

        sign_out(&conn).unwrap();

        let notes: i64 = conn
            .query_row("SELECT COUNT(*) FROM notes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(notes, 1, "signing out is not a delete");
        assert_eq!(get(&conn, USER_OID).unwrap(), None);
    }

    #[test]
    fn signing_out_resets_the_cursor() {
        // A different account sees a different feed. Resuming from a stale
        // position would skip everything that account already had.
        let conn = db();
        set(&conn, CURSOR, "42").unwrap();
        sign_out(&conn).unwrap();
        assert_eq!(cursor(&conn).unwrap(), 0);
    }
}
