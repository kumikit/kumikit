//! Forward-only schema migrations; the version lives in `PRAGMA user_version`.

use rusqlite::Connection;

use super::{StoreError, StoreResult};

/// The kernel's migrations, in order. Migration `i` (0-based) moves the schema from
/// version `i` to `i + 1`. Never edit one that has shipped; append a new one.
pub const MIGRATIONS: &[&str] = &[
    include_str!("../../migrations/0001_init.sql"),
    include_str!("../../migrations/0002_plugins.sql"),
    include_str!("../../migrations/0003_seeded_plugins.sql"),
    include_str!("../../migrations/0004_devices.sql"),
    include_str!("../../migrations/0005_plugin_publisher.sql"),
];

/// Applies the migrations `conn` has not seen yet, each in its own transaction. Fails
/// with [`StoreError::NewerSchema`] if the database is newer than `migrations`.
pub fn migrate(conn: &mut Connection, migrations: &[&str]) -> StoreResult<u32> {
    let known = u32::try_from(migrations.len())
        .map_err(|_| StoreError::Corrupt("too many migrations".into()))?;
    let found: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if found > known {
        return Err(StoreError::NewerSchema { found, known });
    }
    for (version, sql) in (found..known).zip(&migrations[found as usize..]) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", version + 1)?;
        tx.commit()?;
    }
    Ok(known)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(conn: &Connection) -> u32 {
        conn.pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn fresh_database_reaches_latest() {
        let mut conn = Connection::open_in_memory().unwrap();
        assert_eq!(migrate(&mut conn, MIGRATIONS).unwrap(), 5);
        assert_eq!(version(&conn), 5);
        let tables: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'table'
                 AND name IN ('workspaces','threads','events','turns','approvals',
                              'plugins','plugin_kv','seeded_plugins','devices')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tables, 9);
    }

    #[test]
    fn schema_1_upgrades_to_the_latest_and_keeps_its_rows() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn, &MIGRATIONS[..1]).unwrap();
        conn.execute(
            "INSERT INTO workspaces (id, name, root, created) VALUES ('w', 'w', '/w', 't')",
            [],
        )
        .unwrap();
        assert_eq!(migrate(&mut conn, MIGRATIONS).unwrap(), 5);
        let n: i64 = conn
            .query_row("SELECT count(*) FROM workspaces", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
        conn.execute(
            "INSERT INTO plugins (id, version, state, sha256, installed)
             VALUES ('a.b', '0.1.0', 'installed', 'x', 't')",
            [],
        )
        .unwrap();
    }

    #[test]
    fn schema_4_marks_seeded_plugins_as_bundled() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn, &MIGRATIONS[..4]).unwrap();
        for id in ["kumikit.seeded", "acme.local"] {
            conn.execute(
                "INSERT INTO plugins (id, version, state, sha256, installed)
                 VALUES (?1, '0.1.0', 'installed', 'x', 't')",
                [id],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO seeded_plugins (id, version, seeded) VALUES ('kumikit.seeded', '0.1.0', 't')",
            [],
        )
        .unwrap();
        migrate(&mut conn, MIGRATIONS).unwrap();
        let source = |id: &str| -> (String, Option<String>) {
            conn.query_row(
                "SELECT source, publisher_key FROM plugins WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
        };
        assert_eq!(source("kumikit.seeded"), ("bundled".into(), None));
        assert_eq!(source("acme.local"), ("file".into(), None));
    }

    #[test]
    fn upgrades_only_missing_steps() {
        let mut conn = Connection::open_in_memory().unwrap();
        let v1: &[&str] = &["CREATE TABLE a (x INTEGER);"];
        let v2: &[&str] = &[
            "CREATE TABLE a (x INTEGER);",
            "ALTER TABLE a ADD COLUMN y TEXT;",
        ];
        migrate(&mut conn, v1).unwrap();
        conn.execute("INSERT INTO a (x) VALUES (1)", []).unwrap();
        assert_eq!(migrate(&mut conn, v2).unwrap(), 2);
        let y: Option<String> = conn.query_row("SELECT y FROM a", [], |r| r.get(0)).unwrap();
        assert_eq!(y, None);
        // Idempotent at the latest version.
        assert_eq!(migrate(&mut conn, v2).unwrap(), 2);
    }

    #[test]
    fn failed_migration_leaves_version_unchanged() {
        let mut conn = Connection::open_in_memory().unwrap();
        let bad: &[&str] = &["CREATE TABLE a (x INTEGER); NOT SQL;"];
        assert!(migrate(&mut conn, bad).is_err());
        assert_eq!(version(&conn), 0);
    }

    #[test]
    fn refuses_newer_database() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", 7).unwrap();
        let err = migrate(&mut conn, MIGRATIONS).unwrap_err();
        assert!(matches!(
            err,
            StoreError::NewerSchema { found: 7, known: 5 }
        ));
    }
}
