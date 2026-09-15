use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::PayloadError;

/// A validated logical identity for one configured storage driver.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct StorageId(String);

impl StorageId {
    /// Creates a lowercase logical storage identity.
    ///
    /// # Errors
    /// Returns an error for an empty, overlong, or malformed identity.
    pub fn new(value: impl Into<String>) -> Result<Self, PayloadError> {
        let value = value.into();
        if !value.is_empty()
            && value.len() <= 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            Ok(Self(value))
        } else {
            Err(PayloadError::InvalidInput {
                kind: "storage id",
                reason: "must be 1 to 64 lowercase ASCII letters, digits, or dashes",
            })
        }
    }

    #[must_use]
    /// Returns the validated storage identity text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for StorageId {
    type Error = PayloadError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<StorageId> for String {
    fn from(value: StorageId) -> Self {
        value.0
    }
}

/// An opaque, randomly generated 128-bit external object identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ObjectId([u8; 16]);

impl ObjectId {
    /// Generates a random, non-content-derived identity.
    #[must_use]
    pub fn random() -> Self {
        Self(*Uuid::new_v4().as_bytes())
    }

    /// Parses exactly 32 canonical lowercase hexadecimal characters.
    ///
    /// # Errors
    /// Returns an error if text is not canonical lowercase hexadecimal.
    pub fn parse(value: &str) -> Result<Self, PayloadError> {
        if value.len() != 32
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(PayloadError::InvalidInput {
                kind: "object id",
                reason: "must be exactly 32 lowercase hexadecimal characters",
            });
        }
        let uuid = Uuid::parse_str(value).map_err(|_| PayloadError::InvalidInput {
            kind: "object id",
            reason: "must be valid hexadecimal",
        })?;
        Ok(Self(*uuid.as_bytes()))
    }

    #[must_use]
    /// Renders the canonical 32-character lowercase hexadecimal form.
    pub fn to_hex(self) -> String {
        Uuid::from_bytes(self.0).simple().to_string()
    }
}

impl TryFrom<String> for ObjectId {
    type Error = PayloadError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<ObjectId> for String {
    fn from(value: ObjectId) -> Self {
        value.to_hex()
    }
}

/// A validated, relative logical storage location.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectKey(String);

impl ObjectKey {
    /// Derives the locked v1 object layout from an object ID.
    #[must_use]
    pub fn v1_for(id: ObjectId) -> Self {
        let id = id.to_hex();
        Self(format!("v1/objects/{}/{id}", &id[..3]))
    }

    #[must_use]
    /// Returns the relative logical key text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn relative_path(&self) -> PathBuf {
        PathBuf::from(&self.0)
    }
}

/// Immutable byte storage keyed by a validated logical object key.
pub trait StorageDriver: Send + Sync + std::fmt::Debug + 'static {
    /// Creates an object or confirms an existing object has identical bytes.
    ///
    /// # Errors
    /// Returns a typed conflict or I/O error. Drivers never retry internally.
    fn create_immutable(&self, key: &ObjectKey, bytes: &[u8]) -> Result<(), PayloadError>;

    /// Reads one complete immutable object, bounded by the requested maximum.
    ///
    /// # Errors
    /// Returns typed missing, size, or I/O errors.
    fn read(&self, key: &ObjectKey, max_bytes: usize) -> Result<Vec<u8>, PayloadError>;
}

/// A deterministic, in-process storage driver for tests and local conformance.
#[derive(Debug, Clone, Default)]
pub struct InMemoryStorage {
    objects: Arc<Mutex<BTreeMap<ObjectKey, Vec<u8>>>>,
}

impl InMemoryStorage {
    #[must_use]
    /// Creates an empty in-memory test driver.
    pub fn new() -> Self {
        Self::default()
    }
}

impl StorageDriver for InMemoryStorage {
    fn create_immutable(&self, key: &ObjectKey, bytes: &[u8]) -> Result<(), PayloadError> {
        let mut objects = self
            .objects
            .lock()
            .map_err(|_| PayloadError::Codec("in-memory storage lock poisoned".to_owned()))?;
        match objects.get(key) {
            Some(existing) if existing == bytes => Ok(()),
            Some(_) => Err(PayloadError::ImmutableConflict),
            None => {
                objects.insert(key.clone(), bytes.to_vec());
                Ok(())
            }
        }
    }

    fn read(&self, key: &ObjectKey, max_bytes: usize) -> Result<Vec<u8>, PayloadError> {
        let objects = self
            .objects
            .lock()
            .map_err(|_| PayloadError::Codec("in-memory storage lock poisoned".to_owned()))?;
        let bytes = objects.get(key).ok_or(PayloadError::MissingObject)?;
        if bytes.len() > max_bytes {
            return Err(PayloadError::SizeLimit {
                kind: "stored payload",
            });
        }
        Ok(bytes.clone())
    }
}

/// A local or shared-filesystem immutable storage driver.
#[derive(Debug, Clone)]
pub struct FilesystemStorage {
    root: Arc<PathBuf>,
}

impl FilesystemStorage {
    /// Creates a filesystem storage driver rooted at an existing or new directory.
    ///
    /// # Errors
    /// Returns an error if the root cannot be created or is not a directory.
    pub fn new(root: impl AsRef<Path>) -> Result<Self, PayloadError> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root).map_err(PayloadError::from_io)?;
        if !root.is_dir() {
            return Err(PayloadError::InvalidInput {
                kind: "storage root",
                reason: "must be a directory",
            });
        }
        Ok(Self {
            root: Arc::new(root),
        })
    }

    fn path_for(&self, key: &ObjectKey) -> Result<PathBuf, PayloadError> {
        let relative = key.relative_path();
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(PayloadError::InvalidInput {
                kind: "object key",
                reason: "must be a relative normal path",
            });
        }
        Ok(self.root.join(relative))
    }

    fn compare_existing(path: &Path, bytes: &[u8]) -> Result<(), PayloadError> {
        match fs::read(path) {
            Ok(existing) if existing == bytes => Ok(()),
            Ok(_) => Err(PayloadError::ImmutableConflict),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Err(PayloadError::MissingObject)
            }
            Err(error) => Err(PayloadError::from_io(error)),
        }
    }
}

impl StorageDriver for FilesystemStorage {
    fn create_immutable(&self, key: &ObjectKey, bytes: &[u8]) -> Result<(), PayloadError> {
        let final_path = self.path_for(key)?;
        let parent = final_path
            .parent()
            .ok_or(PayloadError::MalformedReference)?;
        fs::create_dir_all(parent).map_err(PayloadError::from_io)?;
        let temp_path = parent.join(format!(".agentive-{}.tmp", ObjectId::random().to_hex()));
        let write_result = (|| -> Result<(), PayloadError> {
            let mut temporary = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp_path)
                .map_err(PayloadError::from_io)?;
            temporary.write_all(bytes).map_err(PayloadError::from_io)?;
            temporary.sync_all().map_err(PayloadError::from_io)?;
            match fs::hard_link(&temp_path, &final_path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    Self::compare_existing(&final_path, bytes)
                }
                Err(error) => Err(PayloadError::from_io(error)),
            }
        })();
        let cleanup_result = fs::remove_file(&temp_path);
        if let Err(error) = cleanup_result
            && error.kind() != std::io::ErrorKind::NotFound
            && write_result.is_ok()
        {
            return Err(PayloadError::from_io(error));
        }
        write_result
    }

    fn read(&self, key: &ObjectKey, max_bytes: usize) -> Result<Vec<u8>, PayloadError> {
        let path = self.path_for(key)?;
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(PayloadError::MissingObject);
            }
            Err(error) => return Err(PayloadError::from_io(error)),
        };
        let metadata = file.metadata().map_err(PayloadError::from_io)?;
        let size = usize::try_from(metadata.len()).map_err(|_| PayloadError::SizeLimit {
            kind: "stored payload",
        })?;
        if size > max_bytes {
            return Err(PayloadError::SizeLimit {
                kind: "stored payload",
            });
        }
        let read_limit = u64::try_from(max_bytes)
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        let mut bytes = Vec::with_capacity(size.min(max_bytes));
        file.take(read_limit)
            .read_to_end(&mut bytes)
            .map_err(PayloadError::from_io)?;
        if bytes.len() > max_bytes {
            return Err(PayloadError::SizeLimit {
                kind: "stored payload",
            });
        }
        Ok(bytes)
    }
}

pub(crate) fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
