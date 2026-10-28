//! Local device-wide legal receipts and independent optional-data permissions.
//! Missing, malformed, incomplete or outdated permission is always denied.
//! No network, telemetry collection, account identifiers or upload queue lives here.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const SCHEMA_VERSION: u32 = 1;
pub const PENDING_RETENTION_SECONDS: u64 = 7 * 24 * 60 * 60;
const MAX_RECORD_BYTES: u64 = 64 * 1024;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

pub fn document_hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentReceipt {
    pub version: String,
    pub sha256: String,
    pub acknowledged_at: u64,
}
impl DocumentReceipt {
    pub fn new(version: &str, bytes: &[u8], acknowledged_at: u64) -> Self {
        Self {
            version: version.into(),
            sha256: document_hash(bytes),
            acknowledged_at,
        }
    }
    pub fn matches(&self, version: &str, bytes: &[u8]) -> bool {
        self.version == version && self.sha256 == document_hash(bytes)
    }
    fn valid(&self) -> bool {
        valid_version(&self.version)
            && self.sha256.len() == 64
            && self
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataCategory {
    ActiveUsage,
    CrashReports,
    PerformanceDiagnostics,
}
impl DataCategory {
    pub const ALL: [Self; 3] = [
        Self::ActiveUsage,
        Self::CrashReports,
        Self::PerformanceDiagnostics,
    ];
    pub const fn index(self) -> usize {
        match self {
            Self::ActiveUsage => 0,
            Self::CrashReports => 1,
            Self::PerformanceDiagnostics => 2,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataPermission {
    pub allowed: bool,
    pub policy_version: String,
    pub decided_at: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsentRecord {
    pub schema_version: u32,
    pub terms: Option<DocumentReceipt>,
    pub privacy: Option<DocumentReceipt>,
    pub open_source_licenses: Option<DocumentReceipt>,
    pub active_usage: Option<DataPermission>,
    pub crash_reports: Option<DataPermission>,
    pub performance_diagnostics: Option<DataPermission>,
}
impl Default for ConsentRecord {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            terms: None,
            privacy: None,
            open_source_licenses: None,
            active_usage: None,
            crash_reports: None,
            performance_diagnostics: None,
        }
    }
}
impl ConsentRecord {
    pub fn permission(&self, category: DataCategory) -> Option<&DataPermission> {
        match category {
            DataCategory::ActiveUsage => &self.active_usage,
            DataCategory::CrashReports => &self.crash_reports,
            DataCategory::PerformanceDiagnostics => &self.performance_diagnostics,
        }
        .as_ref()
    }
    pub fn set_permission(
        &mut self,
        category: DataCategory,
        allowed: bool,
        policy_version: &str,
        now: u64,
    ) {
        let slot = match category {
            DataCategory::ActiveUsage => &mut self.active_usage,
            DataCategory::CrashReports => &mut self.crash_reports,
            DataCategory::PerformanceDiagnostics => &mut self.performance_diagnostics,
        };
        *slot = Some(DataPermission {
            allowed,
            policy_version: policy_version.into(),
            decided_at: now,
        });
    }
    /// Shared gate for both acquisition and transmission. Callers supply the
    /// current category-specific policy version; a changed purpose fails closed.
    pub fn allows(&self, category: DataCategory, current_policy_version: &str) -> bool {
        self.complete()
            && valid_version(current_policy_version)
            && self
                .permission(category)
                .is_some_and(|p| p.allowed && p.policy_version == current_policy_version)
    }
    pub fn complete(&self) -> bool {
        self.valid()
            && self.terms.is_some()
            && self.privacy.is_some()
            && self.open_source_licenses.is_some()
            && DataCategory::ALL
                .into_iter()
                .all(|category| self.permission(category).is_some())
    }
    pub fn valid(&self) -> bool {
        self.schema_version == SCHEMA_VERSION
            && [&self.terms, &self.privacy, &self.open_source_licenses]
                .into_iter()
                .flatten()
                .all(DocumentReceipt::valid)
            && DataCategory::ALL
                .into_iter()
                .filter_map(|c| self.permission(c))
                .all(|p| valid_version(&p.policy_version))
    }
}
fn valid_version(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}

/// A caller-authorized machine directory, never a user's home/preferences.
/// Installer must construct this only from a verified mounted target after the
/// backend's persistence-ready stage. Settings uses /var/config/privacy.
#[derive(Clone, Debug)]
pub struct ConsentStore {
    directory: PathBuf,
}
impl ConsentStore {
    pub fn at_directory(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }
    #[cfg(target_os = "mochios")]
    pub fn system() -> Self {
        Self::at_directory("/var/config/privacy")
    }
    pub fn load(&self) -> io::Result<ConsentRecord> {
        let path = self.directory.join("consent.json");
        reject_symlink(&self.directory)?;
        reject_symlink(&path)?;
        let file = match fs::File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(ConsentRecord::default()),
            Err(e) => return Err(e),
        };
        let mut bytes = Vec::new();
        file.take(MAX_RECORD_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_RECORD_BYTES {
            return Err(invalid("Consent record exceeds its size limit"));
        }
        let record: ConsentRecord =
            serde_json::from_slice(&bytes).map_err(|_| invalid("Invalid consent record"))?;
        if !record.valid() {
            return Err(invalid("Unsupported or invalid consent record"));
        }
        Ok(record)
    }
    pub fn allows(&self, category: DataCategory, policy: &str) -> bool {
        self.load()
            .is_ok_and(|record| record.allows(category, policy))
    }
    /// Atomic local replacement; incomplete initial records are not published.
    /// Permissions are enforced by the existing filesystem/capability policy.
    pub fn save(&self, record: &ConsentRecord) -> io::Result<()> {
        if !record.complete() {
            return Err(invalid("Consent record is incomplete"));
        }
        reject_symlink(&self.directory)?;
        let created = !self.directory.exists();
        fs::create_dir_all(&self.directory)?;
        #[cfg(unix)]
        if created {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.directory, fs::Permissions::from_mode(0o755))?;
        }
        reject_symlink(&self.directory.join("consent.json"))?;
        let temp = self.directory.join(format!(
            ".consent-{}-{}.tmp",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        let mut created_temp = false;
        let result = (|| {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o644);
            }
            let mut file = options.open(&temp)?;
            created_temp = true;
            // Device receipts contain no credentials and are shared by local
            // users; only the owner/system authority may replace them.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(fs::Permissions::from_mode(0o644))?;
            }
            let bytes = serde_json::to_vec_pretty(record)
                .map_err(|_| invalid("Cannot encode consent record"))?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temp, self.directory.join("consent.json"))?;
            fs::File::open(&self.directory)?.sync_all()?;
            Ok(())
        })();
        if result.is_err() && created_temp {
            let _ = fs::remove_file(&temp);
        }
        result
    }
}
fn reject_symlink(path: &Path) -> io::Result<()> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(invalid("Consent path contains a symbolic link"));
            }
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn complete() -> ConsentRecord {
        let mut record = ConsentRecord::default();
        record.terms = Some(DocumentReceipt::new("terms-v1", b"abcdef", 1));
        record.privacy = Some(DocumentReceipt::new("privacy-v1", b"abcdef", 2));
        record.open_source_licenses = Some(DocumentReceipt::new("licenses-v1", b"abcdef", 3));
        for category in DataCategory::ALL {
            record.set_permission(category, false, "policy-v1", 4);
        }
        record
    }
    #[test]
    fn unknown_incomplete_declined_and_outdated_permissions_fail_closed() {
        let mut record = ConsentRecord::default();
        for category in DataCategory::ALL {
            assert!(!record.allows(category, "policy-v1"));
        }
        record.set_permission(DataCategory::ActiveUsage, true, "policy-v1", 5);
        assert!(!record.allows(DataCategory::ActiveUsage, "policy-v1"));
        let mut record = complete();
        record.set_permission(DataCategory::CrashReports, true, "policy-v1", 6);
        assert!(record.allows(DataCategory::CrashReports, "policy-v1"));
        assert!(!record.allows(DataCategory::ActiveUsage, "policy-v1"));
        assert!(!record.allows(DataCategory::PerformanceDiagnostics, "policy-v1"));
        assert!(!record.allows(DataCategory::CrashReports, "policy-v2"));
        record.set_permission(DataCategory::CrashReports, false, "policy-v1", 7);
        assert!(!record.allows(DataCategory::CrashReports, "policy-v1"));
        assert_eq!(record.terms.as_ref().unwrap().acknowledged_at, 1);
    }
    #[test]
    fn receipts_bind_version_and_exact_bytes() {
        let receipt = DocumentReceipt::new("v1", b"abcdef", 4);
        assert!(receipt.matches("v1", b"abcdef"));
        assert!(!receipt.matches("v2", b"abcdef"));
        assert!(!receipt.matches("v1", b"abcdef\n"));
        assert_eq!(
            document_hash(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
    #[test]
    fn local_store_round_trips_revocation_and_rejects_malformed_or_partial_records() {
        let root = std::env::temp_dir().join(format!(
            "mochios-consent-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let store = ConsentStore::at_directory(root.join("device"));
        assert_eq!(store.load().unwrap(), ConsentRecord::default());
        assert!(!store.allows(DataCategory::CrashReports, "policy-v1"));
        assert!(store.save(&ConsentRecord::default()).is_err());
        let mut record = complete();
        record.set_permission(DataCategory::CrashReports, true, "policy-v1", 9);
        store.save(&record).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(root.join("device/consent.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o644
            );
        }
        assert_eq!(store.load().unwrap(), record);
        assert!(store.allows(DataCategory::CrashReports, "policy-v1"));
        record.set_permission(DataCategory::CrashReports, false, "policy-v1", 10);
        store.save(&record).unwrap();
        assert!(!store.allows(DataCategory::CrashReports, "policy-v1"));
        fs::write(root.join("device/consent.json"), b"{broken}").unwrap();
        assert!(store.load().is_err());
        assert!(!store.allows(DataCategory::CrashReports, "policy-v1"));
        let _ = fs::remove_dir_all(root);
    }
    #[cfg(unix)]
    #[test]
    fn symbolic_links_cannot_redirect_device_consent() {
        let root = std::env::temp_dir().join(format!(
            "mochios-consent-links-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("real")).unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("link")).unwrap();
        let store = ConsentStore::at_directory(root.join("link"));
        assert!(store.save(&complete()).is_err());
        assert!(store.load().is_err());
        assert!(!root.join("real/consent.json").exists());
        let _ = fs::remove_dir_all(root);
    }
}
