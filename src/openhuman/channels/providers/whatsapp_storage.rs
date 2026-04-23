//! Custom wa-rs storage backend using OpenHuman's rusqlite
//!
//! This module implements all 4 wa-rs storage traits using rusqlite directly,
//! avoiding the Diesel/libsqlite3-sys dependency conflict from wa-rs-sqlite-storage.
//!
//! # Traits Implemented
//!
//! - [`SignalStore`]: Signal protocol cryptographic operations
//! - [`AppSyncStore`]: WhatsApp app state synchronization
//! - [`ProtocolStore`]: WhatsApp Web protocol alignment
//! - [`DeviceStore`]: Device persistence operations

#[cfg(feature = "whatsapp-web")]
use async_trait::async_trait;
#[cfg(feature = "whatsapp-web")]
use parking_lot::Mutex;
#[cfg(feature = "whatsapp-web")]
use rusqlite::{params, Connection};
#[cfg(feature = "whatsapp-web")]
use std::path::Path;
#[cfg(feature = "whatsapp-web")]
use std::sync::Arc;

#[cfg(feature = "whatsapp-web")]
use prost::Message;
#[cfg(feature = "whatsapp-web")]
use wa_rs_binary::jid::Jid;
#[cfg(feature = "whatsapp-web")]
use wa_rs_core::appstate::hash::HashState;
#[cfg(feature = "whatsapp-web")]
use wa_rs_core::appstate::processor::AppStateMutationMAC;
#[cfg(feature = "whatsapp-web")]
use wa_rs_core::store::traits::DeviceInfo;
#[cfg(feature = "whatsapp-web")]
use wa_rs_core::store::traits::DeviceStore as DeviceStoreTrait;
#[cfg(feature = "whatsapp-web")]
use wa_rs_core::store::traits::*;
#[cfg(feature = "whatsapp-web")]
use wa_rs_core::store::Device as CoreDevice;

/// Custom wa-rs storage backend using rusqlite
///
/// This implements all 4 storage traits required by wa-rs.
/// The backend uses OpenHuman's existing rusqlite setup, avoiding the
/// Diesel/libsqlite3-sys conflict from wa-rs-sqlite-storage.
#[cfg(feature = "whatsapp-web")]
#[derive(Clone)]
pub struct RusqliteStore {
    /// Database file path
    db_path: String,
    /// SQLite connection (thread-safe via Mutex)
    conn: Arc<Mutex<Connection>>,
    /// Device ID for this session
    device_id: i32,
}

/// Helper macro to convert rusqlite errors to StoreError
/// For execute statements that return usize, maps to ()
macro_rules! to_store_err {
    // For expressions returning Result<usize, E>
    (execute: $expr:expr) => {
        $expr
            .map(|_| ())
            .map_err(|e| wa_rs_core::store::error::StoreError::Database(e.to_string()))
    };
    // For other expressions
    ($expr:expr) => {
        $expr.map_err(|e| wa_rs_core::store::error::StoreError::Database(e.to_string()))
    };
}

#[cfg(feature = "whatsapp-web")]
impl RusqliteStore {
    /// Create a new rusqlite-based storage backend
    ///
    /// # Arguments
    ///
    /// * `db_path` - Path to the SQLite database file (will be created if needed)
    pub fn new<P: AsRef<Path>>(db_path: P) -> anyhow::Result<Self> {
        let db_path = db_path.as_ref().to_string_lossy().to_string();

        // Create parent directory if needed
        if let Some(parent) = Path::new(&db_path).parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(&db_path)?;

        // Enable WAL mode for better concurrency
        to_store_err!(conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;",
        ))?;

        let store = Self {
            db_path,
            conn: Arc::new(Mutex::new(conn)),
            device_id: 1, // Default device ID
        };

        store.init_schema()?;

        Ok(store)
    }

    /// Initialize all database tables
    fn init_schema(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock();
        to_store_err!(conn.execute_batch(
            "-- Main device table
            CREATE TABLE IF NOT EXISTS device (
                id INTEGER PRIMARY KEY,
                lid TEXT,
                pn TEXT,
                registration_id INTEGER NOT NULL,
                noise_key BLOB NOT NULL,
                identity_key BLOB NOT NULL,
                signed_pre_key BLOB NOT NULL,
                signed_pre_key_id INTEGER NOT NULL,
                signed_pre_key_signature BLOB NOT NULL,
                adv_secret_key BLOB NOT NULL,
                account BLOB,
                push_name TEXT NOT NULL,
                app_version_primary INTEGER NOT NULL,
                app_version_secondary INTEGER NOT NULL,
                app_version_tertiary INTEGER NOT NULL,
                app_version_last_fetched_ms INTEGER NOT NULL,
                edge_routing_info BLOB,
                props_hash TEXT
            );

            -- Signal identity keys
            CREATE TABLE IF NOT EXISTS identities (
                address TEXT NOT NULL,
                key BLOB NOT NULL,
                device_id INTEGER NOT NULL,
                PRIMARY KEY (address, device_id)
            );

            -- Signal protocol sessions
            CREATE TABLE IF NOT EXISTS sessions (
                address TEXT NOT NULL,
                record BLOB NOT NULL,
                device_id INTEGER NOT NULL,
                PRIMARY KEY (address, device_id)
            );

            -- Pre-keys for key exchange
            CREATE TABLE IF NOT EXISTS prekeys (
                id INTEGER NOT NULL,
                key BLOB NOT NULL,
                uploaded INTEGER NOT NULL DEFAULT 0,
                device_id INTEGER NOT NULL,
                PRIMARY KEY (id, device_id)
            );

            -- Signed pre-keys
            CREATE TABLE IF NOT EXISTS signed_prekeys (
                id INTEGER NOT NULL,
                record BLOB NOT NULL,
                device_id INTEGER NOT NULL,
                PRIMARY KEY (id, device_id)
            );

            -- Sender keys for group messaging
            CREATE TABLE IF NOT EXISTS sender_keys (
                address TEXT NOT NULL,
                record BLOB NOT NULL,
                device_id INTEGER NOT NULL,
                PRIMARY KEY (address, device_id)
            );

            -- App state sync keys
            CREATE TABLE IF NOT EXISTS app_state_keys (
                key_id BLOB NOT NULL,
                key_data BLOB NOT NULL,
                device_id INTEGER NOT NULL,
                PRIMARY KEY (key_id, device_id)
            );

            -- App state versions
            CREATE TABLE IF NOT EXISTS app_state_versions (
                name TEXT NOT NULL,
                state_data BLOB NOT NULL,
                device_id INTEGER NOT NULL,
                PRIMARY KEY (name, device_id)
            );

            -- App state mutation MACs
            CREATE TABLE IF NOT EXISTS app_state_mutation_macs (
                name TEXT NOT NULL,
                version INTEGER NOT NULL,
                index_mac BLOB NOT NULL,
                value_mac BLOB NOT NULL,
                device_id INTEGER NOT NULL,
                PRIMARY KEY (name, index_mac, device_id)
            );

            -- LID to phone number mapping
            CREATE TABLE IF NOT EXISTS lid_pn_mapping (
                lid TEXT NOT NULL,
                phone_number TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                learning_source TEXT NOT NULL,
                updated_at INTEGER NOT NULL,
                device_id INTEGER NOT NULL,
                PRIMARY KEY (lid, device_id)
            );

            -- SKDM recipients tracking
            CREATE TABLE IF NOT EXISTS skdm_recipients (
                group_jid TEXT NOT NULL,
                device_jid TEXT NOT NULL,
                device_id INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                PRIMARY KEY (group_jid, device_jid, device_id)
            );

            -- Device registry for multi-device
            CREATE TABLE IF NOT EXISTS device_registry (
                user_id TEXT NOT NULL,
                devices_json TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                phash TEXT,
                device_id INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (user_id, device_id)
            );

            -- Base keys for collision detection
            CREATE TABLE IF NOT EXISTS base_keys (
                address TEXT NOT NULL,
                message_id TEXT NOT NULL,
                base_key BLOB NOT NULL,
                device_id INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                PRIMARY KEY (address, message_id, device_id)
            );

            -- Sender key status for lazy deletion
            CREATE TABLE IF NOT EXISTS sender_key_status (
                group_jid TEXT NOT NULL,
                participant TEXT NOT NULL,
                device_id INTEGER NOT NULL,
                marked_at INTEGER NOT NULL,
                PRIMARY KEY (group_jid, participant, device_id)
            );

            -- Trusted contact tokens
            CREATE TABLE IF NOT EXISTS tc_tokens (
                jid TEXT NOT NULL,
                token BLOB NOT NULL,
                token_timestamp INTEGER NOT NULL,
                sender_timestamp INTEGER,
                device_id INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (jid, device_id)
            );",
        ))?;
        Ok(())
    }
}

#[cfg(feature = "whatsapp-web")]
#[async_trait]
impl SignalStore for RusqliteStore {
    // --- Identity Operations ---

    async fn put_identity(
        &self,
        address: &str,
        key: [u8; 32],
    ) -> wa_rs_core::store::error::Result<()> {
        let conn = self.conn.lock();
        to_store_err!(execute: conn.execute(
            "INSERT OR REPLACE INTO identities (address, key, device_id)
             VALUES (?1, ?2, ?3)",
            params![address, key.to_vec(), self.device_id],
        ))
    }

    async fn load_identity(
        &self,
        address: &str,
    ) -> wa_rs_core::store::error::Result<Option<Vec<u8>>> {
        let conn = self.conn.lock();
        let result = conn.query_row(
            "SELECT key FROM identities WHERE address = ?1 AND device_id = ?2",
            params![address, self.device_id],
            |row| row.get::<_, Vec<u8>>(0),
        );

        match result {
            Ok(key) => Ok(Some(key)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(wa_rs_core::store::error::StoreError::Database(
                e.to_string(),
            )),
        }
    }

    async fn delete_identity(&self, address: &str) -> wa_rs_core::store::error::Result<()> {
        let conn = self.conn.lock();
        to_store_err!(execute: conn.execute(
            "DELETE FROM identities WHERE address = ?1 AND device_id = ?2",
            params![address, self.device_id],
        ))
    }

    // --- Session Operations ---

    async fn get_session(
        &self,
        address: &str,
    ) -> wa_rs_core::store::error::Result<Option<Vec<u8>>> {
        let conn = self.conn.lock();
        let result = conn.query_row(
            "SELECT record FROM sessions WHERE address = ?1 AND device_id = ?2",
            params![address, self.device_id],
            |row| row.get::<_, Vec<u8>>(0),
        );

        match result {
            Ok(record) => Ok(Some(record)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(wa_rs_core::store::error::StoreError::Database(
                e.to_string(),
            )),
        }
    }

    async fn put_session(
        &self,
        address: &str,
        session: &[u8],
    ) -> wa_rs_core::store::error::Result<()> {
        let conn = self.conn.lock();
        to_store_err!(execute: conn.execute(
            "INSERT OR REPLACE INTO sessions (address, record, device_id)
             VALUES (?1, ?2, ?3)",
            params![address, session, self.device_id],
        ))
    }

    async fn delete_session(&self, address: &str) -> wa_rs_core::store::error::Result<()> {
        let conn = self.conn.lock();
        to_store_err!(execute: conn.execute(
            "DELETE FROM sessions WHERE address = ?1 AND device_id = ?2",
            params![address, self.device_id],
        ))
    }

    // --- PreKey Operations ---

    async fn store_prekey(
        &self,
        id: u32,
        record: &[u8],
        uploaded: bool,
    ) -> wa_rs_core::store::error::Result<()> {
        let conn = self.conn.lock();
        to_store_err!(execute: conn.execute(
            "INSERT OR REPLACE INTO prekeys (id, key, uploaded, device_id)
             VALUES (?1, ?2, ?3, ?4)",
            params![id, record, uploaded, self.device_id],
        ))
    }

    async fn load_prekey(&self, id: u32) -> wa_rs_core::store::error::Result<Option<Vec<u8>>> {
        let conn = self.conn.lock();
        let result = conn.query_row(
            "SELECT key FROM prekeys WHERE id = ?1 AND device_id = ?2",
            params![id, self.device_id],
            |row| row.get::<_, Vec<u8>>(0),
        );

        match result {
            Ok(key) => Ok(Some(key)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(wa_rs_core::store::error::StoreError::Database(
                e.to_string(),
            )),
        }
    }

    async fn remove_prekey(&self, id: u32) -> wa_rs_core::store::error::Result<()> {
        let conn = self.conn.lock();
        to_store_err!(execute: conn.execute(
            "DELETE FROM prekeys WHERE id = ?1 AND device_id = ?2",
            params![id, self.device_id],
        ))
    }

    // --- Signed PreKey Operations ---

    async fn store_signed_prekey(
        &self,
        id: u32,
        record: &[u8],
    ) -> wa_rs_core::store::error::Result<()> {
        let conn = self.conn.lock();
        to_store_err!(execute: conn.execute(
            "INSERT OR REPLACE INTO signed_prekeys (id, record, device_id)
             VALUES (?1, ?2, ?3)",
            params![id, record, self.device_id],
        ))
    }

    async fn load_signed_prekey(
        &self,
        id: u32,
    ) -> wa_rs_core::store::error::Result<Option<Vec<u8>>> {
        let conn = self.conn.lock();
        let result = conn.query_row(
            "SELECT record FROM signed_prekeys WHERE id = ?1 AND device_id = ?2",
            params![id, self.device_id],
            |row| row.get::<_, Vec<u8>>(0),
        );

        match result {
            Ok(record) => Ok(Some(record)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(wa_rs_core::store::error::StoreError::Database(
                e.to_string(),
            )),
        }
    }

    async fn load_all_signed_prekeys(
        &self,
    ) -> wa_rs_core::store::error::Result<Vec<(u32, Vec<u8>)>> {
        let conn = self.conn.lock();
        let mut stmt = to_store_err!(
            conn.prepare("SELECT id, record FROM signed_prekeys WHERE device_id = ?1")
        )?;

        let rows = to_store_err!(stmt.query_map(params![self.device_id], |row| {
            Ok((row.get::<_, u32>(0)?, row.get::<_, Vec<u8>>(1)?))
        }))?;

        let mut result = Vec::new();
        for row in rows {
            result.push(to_store_err!(row)?);
        }

        Ok(result)
    }

    async fn remove_signed_prekey(&self, id: u32) -> wa_rs_core::store::error::Result<()> {
        let conn = self.conn.lock();
        to_store_err!(execute: conn.execute(
            "DELETE FROM signed_prekeys WHERE id = ?1 AND device_id = ?2",
            params![id, self.device_id],
        ))
    }

    // --- Sender Key Operations ---

    async fn put_sender_key(
        &self,
        address: &str,
        record: &[u8],
    ) -> wa_rs_core::store::error::Result<()> {
        let conn = self.conn.lock();
        to_store_err!(execute: conn.execute(
            "INSERT OR REPLACE INTO sender_keys (address, record, device_id)
             VALUES (?1, ?2, ?3)",
            params![address, record, self.device_id],
        ))
    }

    async fn get_sender_key(
        &self,
        address: &str,
    ) -> wa_rs_core::store::error::Result<Option<Vec<u8>>> {
        let conn = self.conn.lock();
        let result = conn.query_row(
            "SELECT record FROM sender_keys WHERE address = ?1 AND device_id = ?2",
            params![address, self.device_id],
            |row| row.get::<_, Vec<u8>>(0),
        );

        match result {
            Ok(record) => Ok(Some(record)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(wa_rs_core::store::error::StoreError::Database(
                e.to_string(),
            )),
        }
    }

    async fn delete_sender_key(&self, address: &str) -> wa_rs_core::store::error::Result<()> {
        let conn = self.conn.lock();
        to_store_err!(execute: conn.execute(
            "DELETE FROM sender_keys WHERE address = ?1 AND device_id = ?2",
            params![address, self.device_id],
        ))
    }
}

#[cfg(feature = "whatsapp-web")]
#[async_trait]
impl AppSyncStore for RusqliteStore {
    async fn get_sync_key(
        &self,
        key_id: &[u8],
    ) -> wa_rs_core::store::error::Result<Option<AppStateSyncKey>> {
        let conn = self.conn.lock();
        let result = conn.query_row(
            "SELECT key_data FROM app_state_keys WHERE key_id = ?1 AND device_id = ?2",
            params![key_id, self.device_id],
            |row| {
                let key_data: Vec<u8> = row.get(0)?;
                serde_json::from_slice(&key_data)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
            },
        );

        match result {
            Ok(key) => Ok(Some(key)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(wa_rs_core::store::error::StoreError::Database(
                e.to_string(),
            )),
        }
    }

    async fn set_sync_key(
        &self,
        key_id: &[u8],
        key: AppStateSyncKey,
    ) -> wa_rs_core::store::error::Result<()> {
        let conn = self.conn.lock();
        let key_data = to_store_err!(serde_json::to_vec(&key))?;

        to_store_err!(execute: conn.execute(
            "INSERT OR REPLACE INTO app_state_keys (key_id, key_data, device_id)
             VALUES (?1, ?2, ?3)",
            params![key_id, key_data, self.device_id],
        ))
    }

    async fn get_version(&self, name: &str) -> wa_rs_core::store::error::Result<HashState> {
        let conn = self.conn.lock();
        let state_data: Vec<u8> = to_store_err!(conn.query_row(
            "SELECT state_data FROM app_state_versions WHERE name = ?1 AND device_id = ?2",
            params![name, self.device_id],
            |row| row.get(0),
        ))?;

        to_store_err!(serde_json::from_slice(&state_data))
    }

    async fn set_version(
        &self,
        name: &str,
        state: HashState,
    ) -> wa_rs_core::store::error::Result<()> {
        let conn = self.conn.lock();
        let state_data = to_store_err!(serde_json::to_vec(&state))?;

        to_store_err!(execute: conn.execute(
            "INSERT OR REPLACE INTO app_state_versions (name, state_data, device_id)
             VALUES (?1, ?2, ?3)",
            params![name, state_data, self.device_id],
        ))
    }

    async fn put_mutation_macs(
        &self,
        name: &str,
        version: u64,
        mutations: &[AppStateMutationMAC],
    ) -> wa_rs_core::store::error::Result<()> {
        let conn = self.conn.lock();

        for mutation in mutations {
            let index_mac = to_store_err!(serde_json::to_vec(&mutation.index_mac))?;
            let value_mac = to_store_err!(serde_json::to_vec(&mutation.value_mac))?;

            to_store_err!(execute: conn.execute(
                "INSERT OR REPLACE INTO app_state_mutation_macs
                 (name, version, index_mac, value_mac, device_id)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![name, i64::try_from(version).unwrap_or(i64::MAX), index_mac, value_mac, self.device_id],
            ))?;
        }

        Ok(())
    }

    async fn get_mutation_mac(
        &self,
        name: &str,
        index_mac: &[u8],
    ) -> wa_rs_core::store::error::Result<Option<Vec<u8>>> {
        let conn = self.conn.lock();
        let index_mac_json = to_store_err!(serde_json::to_vec(index_mac))?;

        let result = conn.query_row(
            "SELECT value_mac FROM app_state_mutation_macs
             WHERE name = ?1 AND index_mac = ?2 AND device_id = ?3",
            params![name, index_mac_json, self.device_id],
            |row| row.get::<_, Vec<u8>>(0),
        );

        match result {
            Ok(mac) => Ok(Some(mac)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(wa_rs_core::store::error::StoreError::Database(
                e.to_string(),
            )),
        }
    }

    async fn delete_mutation_macs(
        &self,
        name: &str,
        index_macs: &[Vec<u8>],
    ) -> wa_rs_core::store::error::Result<()> {
        let conn = self.conn.lock();

        for index_mac in index_macs {
            let index_mac_json = to_store_err!(serde_json::to_vec(index_mac))?;

            to_store_err!(execute: conn.execute(
                "DELETE FROM app_state_mutation_macs
                 WHERE name = ?1 AND index_mac = ?2 AND device_id = ?3",
                params![name, index_mac_json, self.device_id],
            ))?;
        }

        Ok(())
    }
}

#[cfg(feature = "whatsapp-web")]
#[async_trait]
impl ProtocolStore for RusqliteStore {
    // --- SKDM Tracking ---

    async fn get_skdm_recipients(
        &self,
        group_jid: &str,
    ) -> wa_rs_core::store::error::Result<Vec<Jid>> {
        let conn = self.conn.lock();
        let mut stmt = to_store_err!(conn.prepare(
            "SELECT device_jid FROM skdm_recipients WHERE group_jid = ?1 AND device_id = ?2"
        ))?;

        let rows = to_store_err!(stmt.query_map(params![group_jid, self.device_id], |row| {
            row.get::<_, String>(0)
        }))?;

        let mut result = Vec::new();
        for row in rows {
            let jid_str = to_store_err!(row)?;
            if let Ok(jid) = jid_str.parse() {
                result.push(jid);
            }
        }

        Ok(result)
    }

    async fn add_skdm_recipients(
        &self,
        group_jid: &str,
        device_jids: &[Jid],
    ) -> wa_rs_core::store::error::Result<()> {
        let conn = self.conn.lock();
        let now = chrono::Utc::now().timestamp();

        for device_jid in device_jids {
            to_store_err!(execute: conn.execute(
                "INSERT OR IGNORE INTO skdm_recipients (group_jid, device_jid, device_id, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![group_jid, device_jid.to_string(), self.device_id, now],
            ))?;
        }

        Ok(())
    }

    async fn clear_skdm_recipients(&self, group_jid: &str) -> wa_rs_core::store::error::Result<()> {
        let conn = self.conn.lock();
        to_store_err!(execute: conn.execute(
            "DELETE FROM skdm_recipients WHERE group_jid = ?1 AND device_id = ?2",
            params![group_jid, self.device_id],
        ))
    }

    // --- LID-PN Mapping ---

    async fn get_lid_mapping(
        &self,
        lid: &str,
    ) -> wa_rs_core::store::error::Result<Option<LidPnMappingEntry>> {
        let conn = self.conn.lock();
        let result = conn.query_row(
            "SELECT lid, phone_number, created_at, learning_source, updated_at
             FROM lid_pn_mapping WHERE lid = ?1 AND device_id = ?2",
            params![lid, self.device_id],
            |row| {
                Ok(LidPnMappingEntry {
                    lid: row.get(0)?,
                    phone_number: row.get(1)?,
                    created_at: row.get(2)?,
                    learning_source: row.get(3)?,
                    updated_at: row.get(4)?,
                })
            },
        );

        match result {
            Ok(entry) => Ok(Some(entry)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(wa_rs_core::store::error::StoreError::Database(
                e.to_string(),
            )),
        }
    }

    async fn get_pn_mapping(
        &self,
        phone: &str,
    ) -> wa_rs_core::store::error::Result<Option<LidPnMappingEntry>> {
        let conn = self.conn.lock();
        let result = conn.query_row(
            "SELECT lid, phone_number, created_at, learning_source, updated_at
             FROM lid_pn_mapping WHERE phone_number = ?1 AND device_id = ?2
             ORDER BY updated_at DESC LIMIT 1",
            params![phone, self.device_id],
            |row| {
                Ok(LidPnMappingEntry {
                    lid: row.get(0)?,
                    phone_number: row.get(1)?,
                    created_at: row.get(2)?,
                    learning_source: row.get(3)?,
                    updated_at: row.get(4)?,
                })
            },
        );

        match result {
            Ok(entry) => Ok(Some(entry)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(wa_rs_core::store::error::StoreError::Database(
                e.to_string(),
            )),
        }
    }

    async fn put_lid_mapping(
        &self,
        entry: &LidPnMappingEntry,
    ) -> wa_rs_core::store::error::Result<()> {
        let conn = self.conn.lock();
        to_store_err!(execute: conn.execute(
            "INSERT OR REPLACE INTO lid_pn_mapping
             (lid, phone_number, created_at, learning_source, updated_at, device_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                entry.lid,
                entry.phone_number,
                entry.created_at,
                entry.learning_source,
                entry.updated_at,
                self.device_id,
            ],
        ))
    }

    async fn get_all_lid_mappings(
        &self,
    ) -> wa_rs_core::store::error::Result<Vec<LidPnMappingEntry>> {
        let conn = self.conn.lock();
        let mut stmt = to_store_err!(conn.prepare(
            "SELECT lid, phone_number, created_at, learning_source, updated_at
             FROM lid_pn_mapping WHERE device_id = ?1"
        ))?;

        let rows = to_store_err!(stmt.query_map(params![self.device_id], |row| {
            Ok(LidPnMappingEntry {
                lid: row.get(0)?,
                phone_number: row.get(1)?,
                created_at: row.get(2)?,
                learning_source: row.get(3)?,
                updated_at: row.get(4)?,
            })
        }))?;

        let mut result = Vec::new();
        for row in rows {
            result.push(to_store_err!(row)?);
        }

        Ok(result)
    }

    // --- Base Key Collision Detection ---

    async fn save_base_key(
        &self,
        address: &str,
        message_id: &str,
        base_key: &[u8],
    ) -> wa_rs_core::store::error::Result<()> {
        let conn = self.conn.lock();
        let now = chrono::Utc::now().timestamp();

        to_store_err!(execute: conn.execute(
            "INSERT OR REPLACE INTO base_keys (address, message_id, base_key, device_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![address, message_id, base_key, self.device_id, now],
        ))
    }

    async fn has_same_base_key(
        &self,
        address: &str,
        message_id: &str,
        current_base_key: &[u8],
    ) -> wa_rs_core::store::error::Result<bool> {
        let conn = self.conn.lock();
        let result = conn.query_row(
            "SELECT base_key FROM base_keys
             WHERE address = ?1 AND message_id = ?2 AND device_id = ?3",
            params![address, message_id, self.device_id],
            |row| {
                let saved_key: Vec<u8> = row.get(0)?;
                Ok(saved_key == current_base_key)
            },
        );

        match result {
            Ok(same) => Ok(same),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
            Err(e) => Err(wa_rs_core::store::error::StoreError::Database(
                e.to_string(),
            )),
        }
    }

    async fn delete_base_key(
        &self,
        address: &str,
        message_id: &str,
    ) -> wa_rs_core::store::error::Result<()> {
        let conn = self.conn.lock();
        to_store_err!(execute: conn.execute(
            "DELETE FROM base_keys WHERE address = ?1 AND message_id = ?2 AND device_id = ?3",
            params![address, message_id, self.device_id],
        ))
    }

    // --- Device Registry ---

    async fn update_device_list(
        &self,
        record: DeviceListRecord,
    ) -> wa_rs_core::store::error::Result<()> {
        let conn = self.conn.lock();
        let devices_json = to_store_err!(serde_json::to_string(&record.devices))?;
        let now = chrono::Utc::now().timestamp();

        to_store_err!(execute: conn.execute(
            "INSERT OR REPLACE INTO device_registry
             (user_id, devices_json, timestamp, phash, device_id, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                record.user,
                devices_json,
                record.timestamp,
                record.phash,
                self.device_id,
                now,
            ],
        ))
    }

    async fn get_devices(
        &self,
        user: &str,
    ) -> wa_rs_core::store::error::Result<Option<DeviceListRecord>> {
        let conn = self.conn.lock();
        let result = conn.query_row(
            "SELECT user_id, devices_json, timestamp, phash
             FROM device_registry WHERE user_id = ?1 AND device_id = ?2",
            params![user, self.device_id],
            |row| {
                // Helper to convert errors to rusqlite::Error
                fn to_rusqlite_err<E: std::error::Error + Send + Sync + 'static>(
                    e: E,
                ) -> rusqlite::Error {
                    rusqlite::Error::ToSqlConversionFailure(Box::new(e))
                }

                let devices_json: String = row.get(1)?;
                let devices: Vec<DeviceInfo> =
                    serde_json::from_str(&devices_json).map_err(to_rusqlite_err)?;
                Ok(DeviceListRecord {
                    user: row.get(0)?,
                    devices,
                    timestamp: row.get(2)?,
                    phash: row.get(3)?,
                })
            },
        );

        match result {
            Ok(record) => Ok(Some(record)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(wa_rs_core::store::error::StoreError::Database(
                e.to_string(),
            )),
        }
    }

    // --- Sender Key Status (Lazy Deletion) ---

    async fn mark_forget_sender_key(
        &self,
        group_jid: &str,
        participant: &str,
    ) -> wa_rs_core::store::error::Result<()> {
        let conn = self.conn.lock();
        let now = chrono::Utc::now().timestamp();

        to_store_err!(execute: conn.execute(
            "INSERT OR REPLACE INTO sender_key_status (group_jid, participant, device_id, marked_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![group_jid, participant, self.device_id, now],
        ))
    }

    async fn consume_forget_marks(
        &self,
        group_jid: &str,
    ) -> wa_rs_core::store::error::Result<Vec<String>> {
        let conn = self.conn.lock();
        let mut stmt = to_store_err!(conn.prepare(
            "SELECT participant FROM sender_key_status
             WHERE group_jid = ?1 AND device_id = ?2"
        ))?;

        let rows = to_store_err!(stmt.query_map(params![group_jid, self.device_id], |row| {
            row.get::<_, String>(0)
        }))?;

        let mut result = Vec::new();
        for row in rows {
            result.push(to_store_err!(row)?);
        }

        // Delete the marks after consuming them
        to_store_err!(execute: conn.execute(
            "DELETE FROM sender_key_status WHERE group_jid = ?1 AND device_id = ?2",
            params![group_jid, self.device_id],
        ))?;

        Ok(result)
    }

    // --- TcToken Storage ---

    async fn get_tc_token(
        &self,
        jid: &str,
    ) -> wa_rs_core::store::error::Result<Option<TcTokenEntry>> {
        let conn = self.conn.lock();
        let result = conn.query_row(
            "SELECT token, token_timestamp, sender_timestamp FROM tc_tokens
             WHERE jid = ?1 AND device_id = ?2",
            params![jid, self.device_id],
            |row| {
                Ok(TcTokenEntry {
                    token: row.get(0)?,
                    token_timestamp: row.get(1)?,
                    sender_timestamp: row.get(2)?,
                })
            },
        );

        match result {
            Ok(entry) => Ok(Some(entry)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(wa_rs_core::store::error::StoreError::Database(
                e.to_string(),
            )),
        }
    }

    async fn put_tc_token(
        &self,
        jid: &str,
        entry: &TcTokenEntry,
    ) -> wa_rs_core::store::error::Result<()> {
        let conn = self.conn.lock();
        let now = chrono::Utc::now().timestamp();

        to_store_err!(execute: conn.execute(
            "INSERT OR REPLACE INTO tc_tokens
             (jid, token, token_timestamp, sender_timestamp, device_id, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                jid,
                entry.token,
                entry.token_timestamp,
                entry.sender_timestamp,
                self.device_id,
                now,
            ],
        ))
    }

    async fn delete_tc_token(&self, jid: &str) -> wa_rs_core::store::error::Result<()> {
        let conn = self.conn.lock();
        to_store_err!(execute: conn.execute(
            "DELETE FROM tc_tokens WHERE jid = ?1 AND device_id = ?2",
            params![jid, self.device_id],
        ))
    }

    async fn get_all_tc_token_jids(&self) -> wa_rs_core::store::error::Result<Vec<String>> {
        let conn = self.conn.lock();
        let mut stmt =
            to_store_err!(conn.prepare("SELECT jid FROM tc_tokens WHERE device_id = ?1"))?;

        let rows = to_store_err!(
            stmt.query_map(params![self.device_id], |row| { row.get::<_, String>(0) })
        )?;

        let mut result = Vec::new();
        for row in rows {
            result.push(to_store_err!(row)?);
        }

        Ok(result)
    }

    async fn delete_expired_tc_tokens(
        &self,
        cutoff_timestamp: i64,
    ) -> wa_rs_core::store::error::Result<u32> {
        let conn = self.conn.lock();
        let deleted = conn
            .execute(
                "DELETE FROM tc_tokens WHERE token_timestamp < ?1 AND device_id = ?2",
                params![cutoff_timestamp, self.device_id],
            )
            .map_err(|e| wa_rs_core::store::error::StoreError::Database(e.to_string()))?;

        let deleted = u32::try_from(deleted).map_err(|_| {
            wa_rs_core::store::error::StoreError::Database(format!(
                "Affected row count overflowed u32: {deleted}"
            ))
        })?;

        Ok(deleted)
    }
}

#[cfg(feature = "whatsapp-web")]
#[async_trait]
impl DeviceStoreTrait for RusqliteStore {
    async fn save(&self, device: &CoreDevice) -> wa_rs_core::store::error::Result<()> {
        let conn = self.conn.lock();

        // Serialize KeyPairs to bytes
        let noise_key = {
            let mut bytes = Vec::new();
            let priv_key = device.noise_key.private_key.serialize();
            bytes.extend_from_slice(priv_key.as_slice());
            bytes.extend_from_slice(device.noise_key.public_key.public_key_bytes());
            bytes
        };

        let identity_key = {
            let mut bytes = Vec::new();
            let priv_key = device.identity_key.private_key.serialize();
            bytes.extend_from_slice(priv_key.as_slice());
            bytes.extend_from_slice(device.identity_key.public_key.public_key_bytes());
            bytes
        };

        let signed_pre_key = {
            let mut bytes = Vec::new();
            let priv_key = device.signed_pre_key.private_key.serialize();
            bytes.extend_from_slice(priv_key.as_slice());
            bytes.extend_from_slice(device.signed_pre_key.public_key.public_key_bytes());
            bytes
        };

        let account = device.account.as_ref().map(|a| a.encode_to_vec());

        to_store_err!(execute: conn.execute(
            "INSERT OR REPLACE INTO device (
                id, lid, pn, registration_id, noise_key, identity_key,
                signed_pre_key, signed_pre_key_id, signed_pre_key_signature,
                adv_secret_key, account, push_name, app_version_primary,
                app_version_secondary, app_version_tertiary, app_version_last_fetched_ms,
                edge_routing_info, props_hash
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
            params![
                self.device_id,
                device.lid.as_ref().map(|j| j.to_string()),
                device.pn.as_ref().map(|j| j.to_string()),
                device.registration_id,
                noise_key,
                identity_key,
                signed_pre_key,
                device.signed_pre_key_id,
                device.signed_pre_key_signature.to_vec(),
                device.adv_secret_key.to_vec(),
                account,
                &device.push_name,
                device.app_version_primary,
                device.app_version_secondary,
                device.app_version_tertiary,
                device.app_version_last_fetched_ms,
                device.edge_routing_info.as_ref().map(|v| v.clone()),
                device.props_hash.as_ref().map(|v| v.clone()),
            ],
        ))
    }

    async fn load(&self) -> wa_rs_core::store::error::Result<Option<CoreDevice>> {
        let conn = self.conn.lock();
        let result = conn.query_row(
            "SELECT * FROM device WHERE id = ?1",
            params![self.device_id],
            |row| {
                // Helper to convert errors to rusqlite::Error
                fn to_rusqlite_err<E: std::error::Error + Send + Sync + 'static>(
                    e: E,
                ) -> rusqlite::Error {
                    rusqlite::Error::ToSqlConversionFailure(Box::new(e))
                }

                // Deserialize KeyPairs from bytes (64 bytes each)
                let noise_key_bytes: Vec<u8> = row.get("noise_key")?;
                let identity_key_bytes: Vec<u8> = row.get("identity_key")?;
                let signed_pre_key_bytes: Vec<u8> = row.get("signed_pre_key")?;

                if noise_key_bytes.len() != 64
                    || identity_key_bytes.len() != 64
                    || signed_pre_key_bytes.len() != 64
                {
                    return Err(rusqlite::Error::InvalidParameterName("key_pair".into()));
                }

                use wa_rs_core::libsignal::protocol::{KeyPair, PrivateKey, PublicKey};

                let noise_key = KeyPair::new(
                    PublicKey::from_djb_public_key_bytes(&noise_key_bytes[32..64])
                        .map_err(to_rusqlite_err)?,
                    PrivateKey::deserialize(&noise_key_bytes[0..32]).map_err(to_rusqlite_err)?,
                );

                let identity_key = KeyPair::new(
                    PublicKey::from_djb_public_key_bytes(&identity_key_bytes[32..64])
                        .map_err(to_rusqlite_err)?,
                    PrivateKey::deserialize(&identity_key_bytes[0..32]).map_err(to_rusqlite_err)?,
                );

                let signed_pre_key = KeyPair::new(
                    PublicKey::from_djb_public_key_bytes(&signed_pre_key_bytes[32..64])
                        .map_err(to_rusqlite_err)?,
                    PrivateKey::deserialize(&signed_pre_key_bytes[0..32])
                        .map_err(to_rusqlite_err)?,
                );

                let lid_str: Option<String> = row.get("lid")?;
                let pn_str: Option<String> = row.get("pn")?;
                let signature_bytes: Vec<u8> = row.get("signed_pre_key_signature")?;
                let adv_secret_bytes: Vec<u8> = row.get("adv_secret_key")?;
                let account_bytes: Option<Vec<u8>> = row.get("account")?;

                let mut signature = [0u8; 64];
                let mut adv_secret = [0u8; 32];
                signature.copy_from_slice(&signature_bytes);
                adv_secret.copy_from_slice(&adv_secret_bytes);

                let account = if let Some(bytes) = account_bytes {
                    Some(
                        wa_rs_proto::whatsapp::AdvSignedDeviceIdentity::decode(&*bytes)
                            .map_err(to_rusqlite_err)?,
                    )
                } else {
                    None
                };

                Ok(CoreDevice {
                    lid: lid_str.and_then(|s| s.parse().ok()),
                    pn: pn_str.and_then(|s| s.parse().ok()),
                    registration_id: row.get("registration_id")?,
                    noise_key,
                    identity_key,
                    signed_pre_key,
                    signed_pre_key_id: row.get("signed_pre_key_id")?,
                    signed_pre_key_signature: signature,
                    adv_secret_key: adv_secret,
                    account,
                    push_name: row.get("push_name")?,
                    app_version_primary: row.get("app_version_primary")?,
                    app_version_secondary: row.get("app_version_secondary")?,
                    app_version_tertiary: row.get("app_version_tertiary")?,
                    app_version_last_fetched_ms: row.get("app_version_last_fetched_ms")?,
                    edge_routing_info: row.get("edge_routing_info")?,
                    props_hash: row.get("props_hash")?,
                    ..Default::default()
                })
            },
        );

        match result {
            Ok(device) => Ok(Some(device)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(wa_rs_core::store::error::StoreError::Database(
                e.to_string(),
            )),
        }
    }

    async fn exists(&self) -> wa_rs_core::store::error::Result<bool> {
        let conn = self.conn.lock();
        let count: i64 = to_store_err!(conn.query_row(
            "SELECT COUNT(*) FROM device WHERE id = ?1",
            params![self.device_id],
            |row| row.get(0),
        ))?;

        Ok(count > 0)
    }

    async fn create(&self) -> wa_rs_core::store::error::Result<i32> {
        // Device already created in constructor, just return the ID
        Ok(self.device_id)
    }

    async fn snapshot_db(
        &self,
        name: &str,
        extra_content: Option<&[u8]>,
    ) -> wa_rs_core::store::error::Result<()> {
        // Create a snapshot by copying the database file
        let snapshot_path = format!("{}.snapshot.{}", self.db_path, name);

        to_store_err!(std::fs::copy(&self.db_path, &snapshot_path))?;

        // If extra_content is provided, save it alongside
        if let Some(content) = extra_content {
            let content_path = format!("{}.extra", snapshot_path);
            to_store_err!(std::fs::write(&content_path, content))?;
        }

        Ok(())
    }
}

#[cfg(test)]
#[path = "whatsapp_storage_tests.rs"]
mod tests;
