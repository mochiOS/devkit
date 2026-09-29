//! Atomic autosave-in-place recovery records for document applications.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 8] = b"MOCHRCV1";
const MAX_FIELD_BYTES: usize = 16 * 1024;
const MAX_CONTENT_BYTES: usize = 64 * 1024 * 1024;

/// One recoverable document snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryRecord {
    pub identifier: String,
    pub original_path: Option<PathBuf>,
    pub content_type: String,
    pub revision: u64,
    pub contents: Vec<u8>,
}

/// Stores recovery snapshots below an application-owned directory.
#[derive(Clone, Debug)]
pub struct RecoveryStore {
    directory: PathBuf,
}

impl RecoveryStore {
    #[must_use]
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    pub fn save(&self, record: &RecoveryRecord) -> std::io::Result<()> {
        validate_identifier(&record.identifier)?;
        let bytes = encode(record)?;
        fs::create_dir_all(&self.directory)?;
        let destination = self.path_for(&record.identifier);
        let temporary = self.directory.join(format!(".{}.new", record.identifier));
        let mut file = File::create(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, &destination)?;
        sync_directory(&self.directory);
        Ok(())
    }

    pub fn load(&self, identifier: &str) -> std::io::Result<Option<RecoveryRecord>> {
        validate_identifier(identifier)?;
        let path = self.path_for(identifier);
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let mut bytes = Vec::new();
        file.take((MAX_CONTENT_BYTES + MAX_FIELD_BYTES * 3 + 64) as u64)
            .read_to_end(&mut bytes)?;
        decode(&bytes).map(Some)
    }

    pub fn remove(&self, identifier: &str) -> std::io::Result<bool> {
        validate_identifier(identifier)?;
        match fs::remove_file(self.path_for(identifier)) {
            Ok(()) => {
                sync_directory(&self.directory);
                Ok(true)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub fn identifiers(&self) -> std::io::Result<Vec<String>> {
        let entries = match fs::read_dir(&self.directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let mut identifiers = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let path = entry.path();
                (path.extension().and_then(|value| value.to_str()) == Some("recovery"))
                    .then(|| path.file_stem()?.to_str().map(str::to_owned))
                    .flatten()
            })
            .collect::<Vec<_>>();
        identifiers.sort();
        identifiers.dedup();
        Ok(identifiers)
    }

    fn path_for(&self, identifier: &str) -> PathBuf {
        self.directory.join(format!("{identifier}.recovery"))
    }
}

fn validate_identifier(identifier: &str) -> std::io::Result<()> {
    if identifier.is_empty()
        || identifier.len() > 128
        || !identifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid recovery identifier",
        ));
    }
    Ok(())
}

fn encode(record: &RecoveryRecord) -> std::io::Result<Vec<u8>> {
    validate_identifier(&record.identifier)?;
    let path = record
        .original_path
        .as_deref()
        .and_then(Path::to_str)
        .unwrap_or("");
    for field in [
        record.identifier.as_str(),
        path,
        record.content_type.as_str(),
    ] {
        if field.len() > MAX_FIELD_BYTES {
            return Err(invalid_data("recovery metadata is too large"));
        }
    }
    if record.contents.len() > MAX_CONTENT_BYTES {
        return Err(invalid_data("recovery contents are too large"));
    }
    let mut bytes =
        Vec::with_capacity(40 + path.len() + record.content_type.len() + record.contents.len());
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&(record.identifier.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&(path.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&(record.content_type.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&(record.contents.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&record.revision.to_le_bytes());
    bytes.extend_from_slice(record.identifier.as_bytes());
    bytes.extend_from_slice(path.as_bytes());
    bytes.extend_from_slice(record.content_type.as_bytes());
    bytes.extend_from_slice(&record.contents);
    Ok(bytes)
}

fn decode(bytes: &[u8]) -> std::io::Result<RecoveryRecord> {
    if bytes.len() < 36 || bytes.get(..8) != Some(MAGIC) {
        return Err(invalid_data("invalid recovery record"));
    }
    let identifier_len = read_u32(bytes, 8)? as usize;
    let path_len = read_u32(bytes, 12)? as usize;
    let content_type_len = read_u32(bytes, 16)? as usize;
    let contents_len = usize::try_from(read_u64(bytes, 20)?)
        .map_err(|_| invalid_data("invalid recovery length"))?;
    let revision = read_u64(bytes, 28)?;
    if identifier_len > MAX_FIELD_BYTES
        || path_len > MAX_FIELD_BYTES
        || content_type_len > MAX_FIELD_BYTES
        || contents_len > MAX_CONTENT_BYTES
    {
        return Err(invalid_data("recovery record exceeds limits"));
    }
    let total = 36usize
        .checked_add(identifier_len)
        .and_then(|value| value.checked_add(path_len))
        .and_then(|value| value.checked_add(content_type_len))
        .and_then(|value| value.checked_add(contents_len))
        .ok_or_else(|| invalid_data("invalid recovery length"))?;
    if bytes.len() != total {
        return Err(invalid_data("truncated recovery record"));
    }
    let mut offset = 36;
    let identifier = read_string(bytes, &mut offset, identifier_len)?;
    validate_identifier(&identifier)?;
    let path = read_string(bytes, &mut offset, path_len)?;
    let content_type = read_string(bytes, &mut offset, content_type_len)?;
    let contents = bytes[offset..].to_vec();
    Ok(RecoveryRecord {
        identifier,
        original_path: (!path.is_empty()).then(|| PathBuf::from(path)),
        content_type,
        revision,
        contents,
    })
}

fn read_u32(bytes: &[u8], offset: usize) -> std::io::Result<u32> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| invalid_data("truncated recovery record"))?;
    Ok(u32::from_le_bytes(value.try_into().unwrap()))
}

fn read_u64(bytes: &[u8], offset: usize) -> std::io::Result<u64> {
    let value = bytes
        .get(offset..offset + 8)
        .ok_or_else(|| invalid_data("truncated recovery record"))?;
    Ok(u64::from_le_bytes(value.try_into().unwrap()))
}

fn read_string(bytes: &[u8], offset: &mut usize, length: usize) -> std::io::Result<String> {
    let end = offset
        .checked_add(length)
        .ok_or_else(|| invalid_data("invalid recovery length"))?;
    let value = std::str::from_utf8(
        bytes
            .get(*offset..end)
            .ok_or_else(|| invalid_data("truncated recovery record"))?,
    )
    .map_err(|_| invalid_data("recovery metadata is not UTF-8"))?
    .to_owned();
    *offset = end;
    Ok(value)
}

fn invalid_data(message: &'static str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message)
}

fn sync_directory(directory: &Path) {
    if let Ok(file) = File::open(directory) {
        let _ = file.sync_all();
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn temporary_directory() -> PathBuf {
        std::env::temp_dir().join(format!(
            "mochios-recovery-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn round_trips_and_removes_a_snapshot() {
        let directory = temporary_directory();
        let store = RecoveryStore::new(&directory);
        let record = RecoveryRecord {
            identifier: "window-2".into(),
            original_path: Some(PathBuf::from("/home/user/note.txt")),
            content_type: "text/plain".into(),
            revision: 42,
            contents: b"unsaved".to_vec(),
        };
        store.save(&record).unwrap();
        assert_eq!(store.load("window-2").unwrap(), Some(record));
        assert_eq!(store.identifiers().unwrap(), ["window-2"]);
        assert!(store.remove("window-2").unwrap());
        assert_eq!(store.load("window-2").unwrap(), None);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rejects_path_like_identifiers() {
        let store = RecoveryStore::new(temporary_directory());
        assert!(store.load("../escape").is_err());
    }
}
