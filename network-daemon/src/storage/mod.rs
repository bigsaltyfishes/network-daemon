//! Sensitive-config storage: encrypted Wi-Fi credentials in SQLite.
//!
//! Ownership split (user decision):
//! - **SQLite (this module)** holds ONLY sensitive data — Wi-Fi PSK and EAP
//!   credentials — encrypted at rest with AES-256-GCM. The key is a 0600
//!   root-owned secret file.
//! - Everything else (daemon behaviour, interface config, known-network
//!   metadata without passwords) lives in the TOML `config` module.
//!
//! Threat model: the secret key sits next to the database on disk, so this
//! protects against *casual* disk reads, not against root/daemon compromise.

use std::{
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{
        Aead, AeadCore, KeyInit,
        rand_core::{OsRng, RngCore},
    },
};
use rusqlite::{Connection, OptionalExtension};
use thiserror::Error;

/// One saved credential (plaintext only in memory).
#[derive(Debug, Clone)]
pub struct Credential {
    /// MAC address of the WLAN interface that owns this credential.
    /// `None` represents a legacy entry created before per-interface scope.
    pub interface_mac: Option<String>,
    pub ssid: String,
    /// `None` = applies to any BSSID, `Some` = locked to one BSSID.
    pub bssid: Option<String>,
    /// Security variant name: "Open" | "Psk" | "Eap" | "Unknown".
    pub security: String,
    pub psk: Option<String>,
    pub identity: Option<String>,
}

/// Errors from the storage layer.
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("failed to read secret key: {0}")]
    KeyRead(std::io::Error),
    #[error("failed to create secret key: {0}")]
    KeyCreate(std::io::Error),
    #[error("invalid secret key length: {0}")]
    KeyLength(usize),
    #[error("encryption failed: {0}")]
    Encrypt(String),
    #[error("decryption failed: {0}")]
    Decrypt(String),
    #[error("database error: {0}")]
    Sql(#[from] rusqlite::Error),
}

impl From<aes_gcm::Error> for StorageError {
    fn from(e: aes_gcm::Error) -> Self {
        StorageError::Encrypt(e.to_string())
    }
}

// ---------------------------------------------------------------------------
// Crypto — AES-256-GCM around a 32-byte key
// ---------------------------------------------------------------------------

/// AES-256-GCM encrypt/decrypt helper.
pub struct CryptoBox {
    cipher: Aes256Gcm,
}

impl CryptoBox {
    /// Build a box from a raw 32-byte key.
    pub fn new(key: [u8; 32]) -> Self {
        Self {
            cipher: Aes256Gcm::new_from_slice(&key).expect("valid 32-byte key"),
        }
    }

    /// Load (or create) the secret key file, enforcing 0600 mode.
    pub fn load_or_create_key(path: &Path) -> Result<[u8; 32], StorageError> {
        if let Ok(text) = fs::read_to_string(path) {
            let bytes = text.trim();
            let mut key = [0u8; 32];
            if bytes.len() != 64 {
                return Err(StorageError::KeyLength(bytes.len()));
            }
            for (i, b) in bytes.as_bytes().chunks(2).enumerate() {
                let hex = match std::str::from_utf8(b) {
                    Ok(h) => h,
                    Err(_) => return Err(StorageError::KeyLength(bytes.len())),
                };
                key[i] = u8::from_str_radix(hex, 16)
                    .map_err(|_| StorageError::KeyLength(bytes.len()))?;
            }
            return Ok(key);
        }

        // Generate a fresh 32-byte key and write it hex-encoded, 0600.
        let mut key = [0u8; 32];
        OsRng.fill_bytes(&mut key);
        let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
        write_private(path, hex.as_bytes()).map_err(StorageError::KeyCreate)?;
        Ok(key)
    }

    /// Encrypt `plaintext`, returning `nonce||ciphertext`.
    pub fn encrypt(&self, plaintext: &str) -> Result<Vec<u8>, StorageError> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ct = self
            .cipher
            .encrypt(&nonce, plaintext.as_bytes())
            .map_err(StorageError::from)?;
        let mut out = Vec::with_capacity(nonce.len() + ct.len());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        Ok(out)
    }

    /// Decrypt `nonce||ciphertext`.
    pub fn decrypt(&self, blob: &[u8]) -> Result<String, StorageError> {
        if blob.len() < 12 {
            return Err(StorageError::Decrypt("blob too short".into()));
        }
        let nonce = Nonce::from_slice(&blob[..12]);
        let pt = self.cipher.decrypt(nonce, &blob[12..]).map_err(|_| {
            StorageError::Decrypt("authentication failed".into())
        })?;
        String::from_utf8(pt)
            .map_err(|_| StorageError::Decrypt("invalid utf-8".into()))
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    // Enforce 0600 regardless of umask.
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

// ---------------------------------------------------------------------------
// SQLite credential store
// ---------------------------------------------------------------------------

const SCHEMA_VERSION: i64 = 2;

/// SQLite-backed credential store (encrypted at rest).
pub struct CredentialStore {
    conn: Connection,
    crypto: CryptoBox,
}

/// Key under which a credential lives: (interface-mac, ssid, bssid,
/// security). Empty interface MAC is the legacy/global namespace.
fn cred_key(
    interface_mac: Option<&str>,
    ssid: &str,
    bssid: Option<&str>,
    security: &str,
) -> (String, String, String, String) {
    (
        interface_mac.unwrap_or("").to_string(),
        ssid.to_string(),
        bssid.unwrap_or("").to_string(),
        security.to_string(),
    )
}

impl CredentialStore {
    /// Open (and create if missing) the database and schema.
    pub fn open(path: &Path, crypto: CryptoBox) -> Result<Self, StorageError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(StorageError::KeyRead)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        let version: i64 =
            conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version < SCHEMA_VERSION {
            match version {
                0 => conn.execute_batch(
                    "CREATE TABLE IF NOT EXISTS credentials (
                    interface_mac TEXT NOT NULL DEFAULT '',
                    ssid TEXT NOT NULL,
                    bssid TEXT NOT NULL DEFAULT '',
                    security TEXT NOT NULL DEFAULT 'Unknown',
                    psk_encrypted BLOB,
                    identity TEXT,
                    PRIMARY KEY (interface_mac, ssid, bssid, security)
                );
                PRAGMA user_version = 2;",
                )?,
                1 => {
                    // Version 1 keyed rows only by (ssid, bssid). Preserve those
                    // credentials in the legacy namespace while adding the
                    // interface-MAC and security dimensions.
                    conn.execute_batch(
                        "ALTER TABLE credentials RENAME TO credentials_v1;
                CREATE TABLE credentials (
                    interface_mac TEXT NOT NULL DEFAULT '',
                    ssid TEXT NOT NULL,
                    bssid TEXT NOT NULL DEFAULT '',
                    security TEXT NOT NULL DEFAULT 'Unknown',
                    psk_encrypted BLOB,
                    identity TEXT,
                    PRIMARY KEY (interface_mac, ssid, bssid, security)
                );
                INSERT INTO credentials
                    (interface_mac, ssid, bssid, security,
                     psk_encrypted, identity)
                SELECT '', ssid, bssid, security, psk_encrypted, identity
                FROM credentials_v1;
                DROP TABLE credentials_v1;
                PRAGMA user_version = 2;",
                    )?;
                }
                _ => {}
            }
        }
        Ok(Self { conn, crypto })
    }

    /// Upsert a credential (encrypting secret fields).
    pub fn save(&mut self, cred: &Credential) -> Result<(), StorageError> {
        let (interface_mac, ssid, bssid, security) = cred_key(
            cred.interface_mac.as_deref(),
            &cred.ssid,
            cred.bssid.as_deref(),
            &cred.security,
        );
        let previous = self
            .conn
            .query_row(
                "SELECT psk_encrypted, identity FROM credentials
                 WHERE interface_mac = ?1 AND ssid = ?2 AND bssid = ?3
                   AND security = ?4",
                rusqlite::params![interface_mac, ssid, bssid, security],
                |row| {
                    Ok((
                        row.get::<_, Option<Vec<u8>>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                    ))
                },
            )
            .optional()?;
        let previous_psk = previous
            .as_ref()
            .and_then(|(blob, _)| blob.as_deref())
            .map(|blob| self.crypto.decrypt(blob))
            .transpose()?;
        let psk = cred.psk.clone().or(previous_psk);
        let identity = cred
            .identity
            .clone()
            .or_else(|| previous.and_then(|(_, identity)| identity));
        let psk_blob = match &psk {
            Some(p) => Some(self.crypto.encrypt(p)?),
            None => None,
        };
        self.conn.execute(
            "INSERT INTO credentials
                (interface_mac, ssid, bssid, security, psk_encrypted, identity)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(interface_mac, ssid, bssid, security) DO UPDATE SET
               security = excluded.security,
               psk_encrypted = excluded.psk_encrypted,
               identity = excluded.identity",
            rusqlite::params![
                interface_mac,
                ssid,
                bssid,
                security,
                psk_blob,
                identity
            ],
        )?;
        Ok(())
    }

    /// Remove a credential. Returns whether a row was deleted.
    pub fn remove(
        &mut self,
        interface_mac: Option<&str>,
        ssid: &str,
        bssid: Option<&str>,
        security: Option<&str>,
    ) -> Result<bool, StorageError> {
        let interface_mac = interface_mac.unwrap_or("");
        let bssid = bssid.unwrap_or("");
        let n = if let Some(security) = security {
            self.conn.execute(
                "DELETE FROM credentials
                 WHERE interface_mac = ?1 AND ssid = ?2 AND bssid = ?3
                   AND security = ?4",
                rusqlite::params![interface_mac, ssid, bssid, security],
            )?
        } else {
            self.conn.execute(
                "DELETE FROM credentials
                 WHERE interface_mac = ?1 AND ssid = ?2 AND bssid = ?3",
                rusqlite::params![interface_mac, ssid, bssid],
            )?
        };
        Ok(n > 0)
    }

    /// Load all stored credentials (decrypting secret fields).
    pub fn load_all(&mut self) -> Result<Vec<Credential>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT interface_mac, ssid, bssid, security, psk_encrypted,
                    identity
             FROM credentials",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<Vec<u8>>>(4)?,
                r.get::<_, Option<String>>(5)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (interface_mac, ssid, bssid, security, psk_blob, identity) =
                row?;
            let bssid = if bssid.is_empty() { None } else { Some(bssid) };
            let interface_mac = if interface_mac.is_empty() {
                None
            } else {
                Some(interface_mac)
            };
            let psk = psk_blob
                .as_deref()
                .map(|b| self.crypto.decrypt(b))
                .transpose()?;
            out.push(Credential {
                interface_mac,
                ssid,
                bssid,
                security,
                psk,
                identity,
            });
        }
        Ok(out)
    }
}

/// Storage directory layout (runtime vs config split).
#[derive(Debug, Clone)]
pub struct StoragePaths {
    /// Runtime dir for sockets/pid, e.g. /var/run/network-daemon.
    #[allow(dead_code)]
    // split documented for the caller; consumed when the TUI/rc.d path lands
    pub runtime_dir: PathBuf,
    /// Config dir for config.toml + secret.key + credentials.db, e.g.
    /// /var/db/network-daemon.
    pub config_dir: PathBuf,
}

impl StoragePaths {
    pub fn secret_key_path(&self) -> PathBuf {
        self.config_dir.join("secret.key")
    }
    pub fn db_path(&self) -> PathBuf {
        self.config_dir.join("credentials.db")
    }
}

// ---------------------------------------------------------------------------
// StorageManager actor
// ---------------------------------------------------------------------------

use kameo::{Actor, actor::ActorRef, prelude::Message};

/// Commands handled by the [`StorageManager`] actor.
pub enum StorageCommand {
    /// Persist (upsert) a credential.
    SaveCredential(Credential),
    /// Delete a credential.
    RemoveCredential {
        interface_mac: Option<String>,
        ssid: String,
        bssid: Option<String>,
        security: Option<String>,
    },
}

/// Responses to storage commands.
pub enum StorageResult {
    Ok,
    /// All loaded credentials, consumed at startup to seed wifi managers.
    #[allow(dead_code)]
    // returned by the load command; wired by startup seeding
    Credentials(Vec<Credential>),
}
/// Owner of the credential store. Serializes all DB access so no other actor
/// touches SQLite directly.
pub struct StorageManager {
    store: CredentialStore,
}

impl StorageManager {
    /// Open the credential store, creating the DB and key if absent.
    ///
    /// On any failure the actor returns the error and the daemon aborts rather
    /// than silently losing persisted state.
    pub async fn new(paths: &StoragePaths) -> Result<Self, StorageError> {
        let key = CryptoBox::load_or_create_key(&paths.secret_key_path())?;
        let crypto = CryptoBox::new(key);
        let store = CredentialStore::open(&paths.db_path(), crypto)?;
        Ok(Self { store })
    }
}

impl Actor for StorageManager {
    type Args = Self;
    type Error = StorageError;

    async fn on_start(
        args: Self::Args,
        _actor_ref: ActorRef<Self>,
    ) -> Result<Self, Self::Error> {
        Ok(args)
    }
}

impl Message<StorageCommand> for StorageManager {
    type Reply = Result<StorageResult, StorageError>;

    async fn handle(
        &mut self,
        msg: StorageCommand,
        _ctx: &mut kameo::prelude::Context<Self, Self::Reply>,
    ) -> Self::Reply {
        match msg {
            StorageCommand::SaveCredential(cred) => {
                self.store.save(&cred)?;
                Ok(StorageResult::Ok)
            }
            StorageCommand::RemoveCredential {
                interface_mac,
                ssid,
                bssid,
                security,
            } => {
                self.store.remove(
                    interface_mac.as_deref(),
                    &ssid,
                    bssid.as_deref(),
                    security.as_deref(),
                )?;
                Ok(StorageResult::Ok)
            }
        }
    }
}

impl Message<()> for StorageManager {
    type Reply = Result<StorageResult, StorageError>;

    /// Load all credentials (used at startup to seed the wifi managers).
    async fn handle(
        &mut self,
        _msg: (),
        _ctx: &mut kameo::prelude::Context<Self, Self::Reply>,
    ) -> Self::Reply {
        let creds = self.store.load_all()?;
        Ok(StorageResult::Credentials(creds))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("nd-storage-test-{tag}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn crypto_round_trip() {
        let key = [7u8; 32];
        let box_ = CryptoBox::new(key);
        let blob = box_.encrypt("hunter2").unwrap();
        assert_ne!(blob, b"hunter2");
        assert_eq!(box_.decrypt(&blob).unwrap(), "hunter2");
    }

    #[test]
    fn decrypt_authenticates() {
        let key = [7u8; 32];
        let box_ = CryptoBox::new(key);
        let mut blob = box_.encrypt("secret").unwrap();
        let last = blob.len() - 1;
        blob[last] ^= 0xff; // corrupt ciphertext
        assert!(box_.decrypt(&blob).is_err());
    }

    #[test]
    fn key_file_created_and_reused() {
        let dir = temp_dir("key");
        let path = dir.join("secret.key");
        let k1 = CryptoBox::load_or_create_key(&path).unwrap();
        let k2 = CryptoBox::load_or_create_key(&path).unwrap();
        assert_eq!(k1, k2);
        // File is 0600.
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn store_save_remove_load() {
        let dir = temp_dir("store");
        let crypto = CryptoBox::new([1u8; 32]);
        let mut store =
            CredentialStore::open(&dir.join("creds.db"), crypto).unwrap();

        store
            .save(&Credential {
                interface_mac: Some("aa:aa:aa:aa:aa:aa".into()),
                ssid: "HomeWiFi".into(),
                bssid: None,
                security: "Psk".into(),
                psk: Some("supersecret123".into()),
                identity: None,
            })
            .unwrap();

        let all = store.load_all().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].ssid, "HomeWiFi");
        assert_eq!(all[0].psk.as_deref(), Some("supersecret123"));

        assert!(
            store
                .remove(
                    Some("aa:aa:aa:aa:aa:aa"),
                    "HomeWiFi",
                    None,
                    Some("Psk"),
                )
                .unwrap()
        );
        assert!(
            !store
                .remove(
                    Some("aa:aa:aa:aa:aa:aa"),
                    "HomeWiFi",
                    None,
                    Some("Psk"),
                )
                .unwrap()
        );
        assert!(store.load_all().unwrap().is_empty());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn store_keys_by_bssid() {
        let dir = temp_dir("bssid");
        let crypto = CryptoBox::new([2u8; 32]);
        let mut store =
            CredentialStore::open(&dir.join("creds.db"), crypto).unwrap();
        store
            .save(&Credential {
                interface_mac: Some("aa:aa:aa:aa:aa:aa".into()),
                ssid: "SSID".into(),
                bssid: Some("aa:bb:cc:dd:ee:ff".into()),
                security: "Psk".into(),
                psk: Some("pw1".into()),
                identity: None,
            })
            .unwrap();
        store
            .save(&Credential {
                interface_mac: Some("aa:aa:aa:aa:aa:aa".into()),
                ssid: "SSID".into(),
                bssid: None,
                security: "Psk".into(),
                psk: Some("pw2".into()),
                identity: None,
            })
            .unwrap();
        let all = store.load_all().unwrap();
        assert_eq!(all.len(), 2);
        // BSSID-locked and any-BSSID rows coexist.
        assert!(
            all.iter()
                .any(|c| c.bssid.as_deref() == Some("aa:bb:cc:dd:ee:ff"))
        );
        assert!(all.iter().any(|c| c.bssid.is_none()));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn store_keys_by_interface_and_security() {
        let dir = temp_dir("interface");
        let crypto = CryptoBox::new([3u8; 32]);
        let mut store =
            CredentialStore::open(&dir.join("creds.db"), crypto).unwrap();
        for (interface_mac, security, psk) in [
            ("aa:aa:aa:aa:aa:aa", "Psk", "password1"),
            ("bb:bb:bb:bb:bb:bb", "Psk", "password2"),
            ("aa:aa:aa:aa:aa:aa", "Eap", "password3"),
        ] {
            store
                .save(&Credential {
                    interface_mac: Some(interface_mac.into()),
                    ssid: "SSID".into(),
                    bssid: None,
                    security: security.into(),
                    psk: Some(psk.into()),
                    identity: None,
                })
                .unwrap();
        }
        let all = store.load_all().unwrap();
        assert_eq!(all.len(), 3);
        assert!(all.iter().any(|credential| {
            credential.interface_mac.as_deref() == Some("bb:bb:bb:bb:bb:bb")
        }));
        assert!(all.iter().any(|credential| {
            credential.security == "Eap"
                && credential.interface_mac.as_deref()
                    == Some("aa:aa:aa:aa:aa:aa")
        }));
        fs::remove_dir_all(&dir).ok();
    }
}
