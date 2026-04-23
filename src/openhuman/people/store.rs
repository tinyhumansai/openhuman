//! SQLite-backed store for people + handle aliases + interactions.
//!
//! Connection is wrapped in `Arc<Mutex<Connection>>` so handlers and tests
//! can share ownership across tokio tasks; operations are synchronous and
//! fast (all single-row CRUD or small aggregates).

use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{params, Connection, OptionalExtension, Result as SqlResult};
use tokio::sync::Mutex;

use crate::openhuman::people::migrations;
use crate::openhuman::people::types::{Handle, Interaction, Person, PersonId};

pub type ConnHandle = Arc<Mutex<Connection>>;

/// Process-global handle to the `PeopleStore`. Controller handlers are
/// free functions with no `&self`, so they fetch the store via `get()`
/// — seeded once at startup with `init`. Absent at test time; tests
/// construct stores directly and call `rpc::*` helpers instead of going
/// through the schema adapters.
static GLOBAL: tokio::sync::OnceCell<Arc<PeopleStore>> = tokio::sync::OnceCell::const_new();

pub async fn init(store: Arc<PeopleStore>) -> Result<(), &'static str> {
    GLOBAL
        .set(store)
        .map_err(|_| "people store already initialised")
}

pub fn get() -> Result<Arc<PeopleStore>, &'static str> {
    GLOBAL
        .get()
        .cloned()
        .ok_or("people store not initialised — core startup hasn't completed")
}

pub struct PeopleStore {
    pub conn: ConnHandle,
}

impl PeopleStore {
    pub fn open_in_memory() -> SqlResult<Self> {
        let conn = Connection::open_in_memory()?;
        migrations::run(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn open_at(path: &std::path::Path) -> SqlResult<Self> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(path)?;
        migrations::run(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Insert a new person and its initial set of handles, atomically.
    pub async fn insert_person(&self, person: &Person, handles: &[Handle]) -> SqlResult<()> {
        let conn = self.conn.clone();
        let person = person.clone();
        let handles: Vec<Handle> = handles.iter().map(|h| h.canonicalize()).collect();
        tokio::task::spawn_blocking(move || {
            let mut guard = conn.blocking_lock();
            let tx = guard.transaction()?;
            tx.execute(
                "INSERT INTO people(id, display_name, primary_email, primary_phone, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    person.id.to_string(),
                    person.display_name,
                    person.primary_email,
                    person.primary_phone,
                    person.created_at.timestamp(),
                    person.updated_at.timestamp(),
                ],
            )?;
            for h in &handles {
                let (kind, value) = h.as_key();
                tx.execute(
                    "INSERT OR IGNORE INTO handle_aliases(kind, value, person_id, created_at) \
                     VALUES (?1, ?2, ?3, CAST(strftime('%s','now') AS INTEGER))",
                    params![kind, value, person.id.to_string()],
                )?;
            }
            tx.commit()
        })
        .await
        .map_err(|e| rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ffi::ErrorCode::SystemIoFailure,
                extended_code: 0,
            },
            Some(e.to_string()),
        ))?
    }

    /// Attach a handle alias to an existing person. Idempotent via
    /// `INSERT OR IGNORE` on `(kind, value)`.
    pub async fn add_alias(&self, person_id: PersonId, handle: Handle) -> SqlResult<()> {
        let conn = self.conn.clone();
        let handle = handle.canonicalize();
        tokio::task::spawn_blocking(move || {
            let guard = conn.blocking_lock();
            let (kind, value) = handle.as_key();
            guard.execute(
                "INSERT OR IGNORE INTO handle_aliases(kind, value, person_id, created_at) \
                 VALUES (?1, ?2, ?3, CAST(strftime('%s','now') AS INTEGER))",
                params![kind, value, person_id.to_string()],
            )?;
            Ok(())
        })
        .await
        .map_err(|e| {
            rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error {
                    code: rusqlite::ffi::ErrorCode::SystemIoFailure,
                    extended_code: 0,
                },
                Some(e.to_string()),
            )
        })?
    }

    /// Resolve a canonicalized handle to a `PersonId`, or `None` if unknown.
    pub async fn lookup(&self, handle: &Handle) -> SqlResult<Option<PersonId>> {
        let conn = self.conn.clone();
        let handle = handle.canonicalize();
        tokio::task::spawn_blocking(move || {
            let guard = conn.blocking_lock();
            let (kind, value) = handle.as_key();
            let id: Option<String> = guard
                .query_row(
                    "SELECT person_id FROM handle_aliases WHERE kind = ?1 AND value = ?2",
                    params![kind, value],
                    |row| row.get(0),
                )
                .optional()?;
            Ok(id.and_then(|s| uuid::Uuid::parse_str(&s).ok().map(PersonId)))
        })
        .await
        .map_err(|e| {
            rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error {
                    code: rusqlite::ffi::ErrorCode::SystemIoFailure,
                    extended_code: 0,
                },
                Some(e.to_string()),
            )
        })?
    }

    /// Load a person and all their aliases.
    pub async fn get(&self, person_id: PersonId) -> SqlResult<Option<Person>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> SqlResult<Option<Person>> {
            let guard = conn.blocking_lock();
            let row: Option<(String, Option<String>, Option<String>, Option<String>, i64, i64)> =
                guard
                    .query_row(
                        "SELECT id, display_name, primary_email, primary_phone, created_at, updated_at \
                         FROM people WHERE id = ?1",
                        params![person_id.to_string()],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
                    )
                    .optional()?;
            let Some((id_str, display_name, primary_email, primary_phone, created, updated)) = row
            else {
                return Ok(None);
            };
            let id = uuid::Uuid::parse_str(&id_str)
                .map(PersonId)
                .map_err(|e| rusqlite::Error::InvalidColumnName(e.to_string()))?;
            let handles = load_handles(&guard, &id)?;
            Ok(Some(Person {
                id,
                display_name,
                primary_email,
                primary_phone,
                handles,
                created_at: ts_to_dt(created),
                updated_at: ts_to_dt(updated),
            }))
        })
        .await
        .map_err(|e| rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ffi::ErrorCode::SystemIoFailure,
                extended_code: 0,
            },
            Some(e.to_string()),
        ))?
    }

    /// List all people (unordered — scorer applies ranking separately).
    pub async fn list(&self) -> SqlResult<Vec<Person>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> SqlResult<Vec<Person>> {
            let guard = conn.blocking_lock();
            let mut stmt = guard.prepare(
                "SELECT id, display_name, primary_email, primary_phone, created_at, updated_at \
                 FROM people ORDER BY display_name",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(5)?,
                ))
            })?;
            let mut out = Vec::new();
            for r in rows {
                let (id_str, display_name, primary_email, primary_phone, created, updated) = r?;
                let id = uuid::Uuid::parse_str(&id_str)
                    .map(PersonId)
                    .map_err(|e| rusqlite::Error::InvalidColumnName(e.to_string()))?;
                let handles = load_handles(&guard, &id)?;
                out.push(Person {
                    id,
                    display_name,
                    primary_email,
                    primary_phone,
                    handles,
                    created_at: ts_to_dt(created),
                    updated_at: ts_to_dt(updated),
                });
            }
            Ok(out)
        })
        .await
        .map_err(|e| {
            rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error {
                    code: rusqlite::ffi::ErrorCode::SystemIoFailure,
                    extended_code: 0,
                },
                Some(e.to_string()),
            )
        })?
    }

    /// Record a single interaction.
    pub async fn record_interaction(&self, i: Interaction) -> SqlResult<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let guard = conn.blocking_lock();
            guard.execute(
                "INSERT INTO interactions(person_id, ts, is_outbound, length) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    i.person_id.to_string(),
                    i.ts.timestamp(),
                    if i.is_outbound { 1_i64 } else { 0_i64 },
                    i.length as i64,
                ],
            )?;
            Ok(())
        })
        .await
        .map_err(|e| {
            rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error {
                    code: rusqlite::ffi::ErrorCode::SystemIoFailure,
                    extended_code: 0,
                },
                Some(e.to_string()),
            )
        })?
    }

    /// Fetch all interactions for a person, newest first.
    pub async fn interactions_for(&self, person_id: PersonId) -> SqlResult<Vec<Interaction>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> SqlResult<Vec<Interaction>> {
            let guard = conn.blocking_lock();
            let mut stmt = guard.prepare(
                "SELECT ts, is_outbound, length FROM interactions \
                 WHERE person_id = ?1 ORDER BY ts DESC",
            )?;
            let rows = stmt.query_map(params![person_id.to_string()], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })?;
            let mut out = Vec::new();
            for r in rows {
                let (ts, is_out, length) = r?;
                out.push(Interaction {
                    person_id,
                    ts: ts_to_dt(ts),
                    is_outbound: is_out != 0,
                    length: length.max(0) as u32,
                });
            }
            Ok(out)
        })
        .await
        .map_err(|e| {
            rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error {
                    code: rusqlite::ffi::ErrorCode::SystemIoFailure,
                    extended_code: 0,
                },
                Some(e.to_string()),
            )
        })?
    }
}

fn load_handles(conn: &Connection, id: &PersonId) -> SqlResult<Vec<Handle>> {
    let mut stmt = conn.prepare(
        "SELECT kind, value FROM handle_aliases WHERE person_id = ?1 ORDER BY kind, value",
    )?;
    let rows = stmt.query_map(params![id.to_string()], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    let mut out = Vec::new();
    for r in rows {
        let (kind, value) = r?;
        let h = match kind.as_str() {
            "imessage" => Handle::IMessage(value),
            "email" => Handle::Email(value),
            "display_name" => Handle::DisplayName(value),
            other => {
                return Err(rusqlite::Error::InvalidColumnName(format!(
                    "unknown handle kind: {other}"
                )));
            }
        };
        out.push(h);
    }
    Ok(out)
}

fn ts_to_dt(ts: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(ts, 0)
        .single()
        .unwrap_or_else(|| Utc.timestamp_opt(0, 0).unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn insert_list_and_lookup_round_trip() {
        let s = PeopleStore::open_in_memory().unwrap();
        let now = Utc::now();
        let p = Person {
            id: PersonId::new(),
            display_name: Some("Sarah Lee".into()),
            primary_email: Some("sarah@example.com".into()),
            primary_phone: None,
            handles: vec![],
            created_at: now,
            updated_at: now,
        };
        s.insert_person(
            &p,
            &[
                Handle::Email("Sarah@Example.com".into()),
                Handle::DisplayName("Sarah Lee".into()),
            ],
        )
        .await
        .unwrap();

        let got = s
            .lookup(&Handle::Email("sarah@example.com".into()))
            .await
            .unwrap();
        assert_eq!(got, Some(p.id));

        let list = s.list().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].handles.len(), 2);
    }

    #[tokio::test]
    async fn interactions_round_trip() {
        let s = PeopleStore::open_in_memory().unwrap();
        let now = Utc::now();
        let pid = PersonId::new();
        let p = Person {
            id: pid,
            display_name: Some("X".into()),
            primary_email: None,
            primary_phone: None,
            handles: vec![],
            created_at: now,
            updated_at: now,
        };
        s.insert_person(&p, &[]).await.unwrap();
        s.record_interaction(Interaction {
            person_id: pid,
            ts: now,
            is_outbound: true,
            length: 100,
        })
        .await
        .unwrap();
        s.record_interaction(Interaction {
            person_id: pid,
            ts: now,
            is_outbound: false,
            length: 50,
        })
        .await
        .unwrap();
        let ints = s.interactions_for(pid).await.unwrap();
        assert_eq!(ints.len(), 2);
    }
}
