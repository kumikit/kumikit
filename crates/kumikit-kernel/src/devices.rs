//! Paired devices (`docs/spec/03-kernel.md#storage`, `docs/spec/02-protocol.md#pairing`).
//!
//! The kernel stores what the core's authentication decides: a device's id, name and the
//! SHA-256 of its token. It never sees a token. Revoking a device deletes its row and
//! announces the id, so the core can close the device's live connections.

use kumikit_protocol::model::Timestamp;
use rusqlite::{Connection, OptionalExtension, params};
use tokio::sync::broadcast;

use crate::error::{KernelError, KernelResult};
use crate::kernel::Kernel;

/// Revocations buffered per listener before it lags.
pub(crate) const REVOCATIONS: usize = 64;

/// A paired device, as the kernel stores it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRecord {
    /// Device id (a ULID).
    pub id: String,
    /// The name the device gave when it paired.
    pub name: String,
    /// When it paired.
    pub created: Timestamp,
    /// When it last authenticated, to the minute.
    pub last_seen: Option<Timestamp>,
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<DeviceRecord> {
    Ok(DeviceRecord {
        id: r.get(0)?,
        name: r.get(1)?,
        created: Timestamp(r.get(2)?),
        last_seen: r.get::<_, Option<String>>(3)?.map(Timestamp),
    })
}

fn by_hash(c: &Connection, hash: &str) -> KernelResult<Option<DeviceRecord>> {
    Ok(c.query_row(
        "SELECT id, name, created, last_seen FROM devices WHERE token_hash = ?1",
        [hash],
        row,
    )
    .optional()?)
}

/// The minute of a timestamp (`2026-11-15T10:00`), for the last-seen throttle.
fn minute(t: &str) -> &str {
    t.get(..16).unwrap_or(t)
}

impl Kernel {
    /// Records a newly paired device whose token hashes to `token_hash` (hex SHA-256).
    pub async fn device_insert(&self, name: &str, token_hash: &str) -> KernelResult<DeviceRecord> {
        let record = DeviceRecord {
            id: self.inner.ids.next(),
            name: name.to_owned(),
            created: self.now(),
            last_seen: None,
        };
        let (r, hash) = (record.clone(), token_hash.to_owned());
        self.inner
            .store
            .write(move |c| -> KernelResult<()> {
                c.execute(
                    "INSERT INTO devices (id, name, token_hash, created) VALUES (?1, ?2, ?3, ?4)",
                    params![r.id, r.name, hash, r.created.0],
                )?;
                Ok(())
            })
            .await?;
        Ok(record)
    }

    /// The device whose token hashes to `token_hash`, if any. Records that it was seen,
    /// at most once a minute.
    pub async fn device_authenticate(
        &self,
        token_hash: &str,
    ) -> KernelResult<Option<DeviceRecord>> {
        let hash = token_hash.to_owned();
        let Some(mut device) = self.inner.store.read(move |c| by_hash(c, &hash)).await? else {
            return Ok(None);
        };
        let now = self.now();
        let seen_this_minute = device
            .last_seen
            .as_ref()
            .is_some_and(|t| minute(&t.0) == minute(&now.0));
        if !seen_this_minute {
            let (id, at) = (device.id.clone(), now.0.clone());
            self.inner
                .store
                .write(move |c| -> KernelResult<()> {
                    c.execute(
                        "UPDATE devices SET last_seen = ?2 WHERE id = ?1",
                        [&id, &at],
                    )?;
                    Ok(())
                })
                .await?;
            device.last_seen = Some(now);
        }
        Ok(Some(device))
    }

    /// Whether a device is still paired. Used by a connection that missed revocations.
    pub async fn device_exists(&self, id: &str) -> KernelResult<bool> {
        let id = id.to_owned();
        self.inner
            .store
            .read(move |c| -> KernelResult<bool> {
                let n: i64 =
                    c.query_row("SELECT count(*) FROM devices WHERE id = ?1", [&id], |r| {
                        r.get(0)
                    })?;
                Ok(n > 0)
            })
            .await
    }

    /// Every paired device, oldest first.
    pub async fn device_list(&self) -> KernelResult<Vec<DeviceRecord>> {
        self.inner
            .store
            .read(|c| -> KernelResult<Vec<DeviceRecord>> {
                let mut stmt = c.prepare(
                    "SELECT id, name, created, last_seen FROM devices ORDER BY created, id",
                )?;
                let rows = stmt.query_map([], row)?.collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
    }

    /// Unpairs a device and announces it on [`Kernel::device_revocations`]. Fails with
    /// `not_found` if no such device is paired.
    pub async fn device_revoke(&self, id: &str) -> KernelResult<()> {
        let key = id.to_owned();
        let deleted = self
            .inner
            .store
            .write(move |c| -> KernelResult<usize> {
                Ok(c.execute("DELETE FROM devices WHERE id = ?1", [&key])?)
            })
            .await?;
        if deleted == 0 {
            return Err(KernelError::NotFound(format!("no device {id}")));
        }
        // No listener is fine: no connection of that device is open.
        let _ = self.inner.revocations.send(id.to_owned());
        Ok(())
    }

    /// Ids of devices revoked from now on. A listener that lags behind gets `Lagged` and
    /// must check its device with [`Kernel::device_exists`].
    pub fn device_revocations(&self) -> broadcast::Receiver<String> {
        self.inner.revocations.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::KernelConfig;
    use crate::clock::{SequentialIds, StepClock};

    async fn kernel(dir: &tempfile::TempDir) -> Kernel {
        Kernel::open(KernelConfig {
            clock: Arc::new(StepClock::default()),
            ids: Arc::new(SequentialIds::new("D")),
            ..KernelConfig::new(dir.path().join("core.sqlite"))
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn devices_round_trip_and_survive_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let k = kernel(&dir).await;
        let a = k.device_insert("laptop", "aa").await.unwrap();
        let b = k.device_insert("phone", "bb").await.unwrap();
        assert_eq!(a.name, "laptop");
        assert_ne!(a.id, b.id);
        assert!(a.last_seen.is_none());
        k.close().await;

        let k = kernel(&dir).await;
        let list = k.device_list().await.unwrap();
        assert_eq!(
            list.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
            ["laptop", "phone"]
        );
        let found = k.device_authenticate("bb").await.unwrap().unwrap();
        assert_eq!(found.id, b.id);
        assert!(found.last_seen.is_some());
        assert!(k.device_authenticate("cc").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn devices_store_only_the_hash_given() {
        let dir = tempfile::tempdir().unwrap();
        let k = kernel(&dir).await;
        k.device_insert("laptop", "0123abcd").await.unwrap();
        let hashes: Vec<String> = k
            .inner
            .store
            .read(|c| -> KernelResult<Vec<String>> {
                let mut s = c.prepare("SELECT token_hash FROM devices")?;
                Ok(s.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?)
            })
            .await
            .unwrap();
        assert_eq!(hashes, ["0123abcd"]);
    }

    #[tokio::test]
    async fn a_token_hash_pairs_one_device_only() {
        let dir = tempfile::tempdir().unwrap();
        let k = kernel(&dir).await;
        k.device_insert("a", "same").await.unwrap();
        assert!(k.device_insert("b", "same").await.is_err());
    }

    #[tokio::test]
    async fn revoking_deletes_and_announces() {
        let dir = tempfile::tempdir().unwrap();
        let k = kernel(&dir).await;
        let d = k.device_insert("laptop", "aa").await.unwrap();
        let mut revoked = k.device_revocations();
        k.device_revoke(&d.id).await.unwrap();
        assert_eq!(revoked.recv().await.unwrap(), d.id);
        assert!(!k.device_exists(&d.id).await.unwrap());
        assert!(k.device_authenticate("aa").await.unwrap().is_none());
        let err = k.device_revoke(&d.id).await.unwrap_err();
        assert!(matches!(err, KernelError::NotFound(_)));
    }

    #[test]
    fn last_seen_is_throttled_to_the_minute() {
        assert_eq!(
            minute("2026-11-15T10:00:59.000Z"),
            minute("2026-11-15T10:00:01.000Z")
        );
        assert_ne!(
            minute("2026-11-15T10:01:00.000Z"),
            minute("2026-11-15T10:00:59.000Z")
        );
    }
}
