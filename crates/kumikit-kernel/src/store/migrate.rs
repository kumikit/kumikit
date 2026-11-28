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
    include_str!("../../migrations/0006_plugin_settings.sql"),
    include_str!("../../migrations/0007_plugin_workspaces.sql"),
    include_str!("../../migrations/0008_schedules.sql"),
    include_str!("../../migrations/0009_thread_permission_mode.sql"),
    include_str!("../../migrations/0010_core_settings.sql"),
    include_str!("../../migrations/0011_thread_worktree.sql"),
    include_str!("../../migrations/0012_workspace_kind.sql"),
    include_str!("../../migrations/0013_queued_messages.sql"),
    include_str!("../../migrations/0014_handoff.sql"),
    include_str!("../../migrations/0015_deferred_messages.sql"),
    include_str!("../../migrations/0016_title_source.sql"),
    include_str!("../../migrations/0017_forks_and_side_threads.sql"),
    include_str!("../../migrations/0018_thread_options.sql"),
    include_str!("../../migrations/0019_usage.sql"),
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
        assert_eq!(migrate(&mut conn, MIGRATIONS).unwrap(), 19);
        assert_eq!(version(&conn), 19);
        let tables: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'table'
                 AND name IN ('workspaces','threads','events','turns','approvals',
                              'plugins','plugin_kv','seeded_plugins','devices',
                              'plugin_settings','plugin_workspaces','schedules',
                              'core_settings','queued_messages')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tables, 14);
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
        assert_eq!(migrate(&mut conn, MIGRATIONS).unwrap(), 19);
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
    fn schema_19_backfills_each_turns_last_usage_once() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn, &MIGRATIONS[..18]).unwrap();
        conn.execute_batch(
            "INSERT INTO workspaces (id, name, root, created) VALUES ('w', 'w', '/w', 't');
             INSERT INTO threads (id, workspace, provider, model, status, created, updated)
                 VALUES ('a', 'w', 'p/x', 'm1', 'idle', 't', 't'),
                        ('f', 'w', 'p/x', 'm1', 'idle', 't', 't');
             INSERT INTO turns (id, thread, started_seq, started, provider, model)
                 VALUES ('t1', 'a', 1, '2026-11-15T10:00:00.000Z', 'p/y', 'm2'),
                        ('t2', 'a', 4, '2026-11-15T11:00:00.000Z', NULL, NULL),
                        ('t3', 'a', 6, '2026-11-15T12:00:00.000Z', NULL, NULL);
             INSERT INTO events (thread, seq, time, kind, data) VALUES
                 ('a', 1, 't', 'turn.started', '{\"turn\":\"t1\"}'),
                 ('a', 2, 't', 'usage', '{\"turn\":\"t1\",\"inputTokens\":5,\"outputTokens\":1}'),
                 ('a', 3, 't', 'usage', '{\"turn\":\"t1\",\"inputTokens\":10,\"outputTokens\":2,\"cost\":0.5}'),
                 ('a', 4, 't', 'turn.started', '{\"turn\":\"t2\"}'),
                 ('a', 5, 't', 'usage', '{\"turn\":\"t2\",\"inputTokens\":7,\"outputTokens\":3}'),
                 ('a', 6, 't', 'turn.started', '{\"turn\":\"t3\"}'),
                 ('f', 1, 't', 'usage', '{\"turn\":\"t1\",\"inputTokens\":10,\"outputTokens\":2}');",
        )
        .unwrap();
        migrate(&mut conn, MIGRATIONS).unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT turn, thread, provider, model, input_tokens, output_tokens, cost
                 FROM usage_records ORDER BY turn",
            )
            .unwrap();
        type Row = (String, String, String, String, i64, i64, Option<f64>);
        let rows: Vec<Row> = stmt
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(
            rows,
            vec![
                (
                    "t1".into(),
                    "a".into(),
                    "p/y".into(),
                    "m2".into(),
                    10,
                    2,
                    Some(0.5)
                ),
                (
                    "t2".into(),
                    "a".into(),
                    "p/x".into(),
                    "m1".into(),
                    7,
                    3,
                    None
                ),
            ]
        );
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
        let newer = u32::try_from(MIGRATIONS.len()).unwrap() + 1;
        conn.pragma_update(None, "user_version", newer).unwrap();
        let err = migrate(&mut conn, MIGRATIONS).unwrap_err();
        assert!(matches!(
            err,
            StoreError::NewerSchema { found, known } if found == newer && known == newer - 1
        ));
    }
}
