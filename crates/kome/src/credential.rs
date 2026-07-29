use std::{
    cell::Cell,
    env, fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;
use zeroize::Zeroize;

const KEYRING_SERVICE: &str = "org.mochios.kome";
const KEYRING_USER: &str = "cli-session";
const CREDENTIAL_FILE: &str = "credentials.json";

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredCredential {
    pub refresh_credential: String,
    pub session_id: String,
    pub account_id: String,
    pub account_name: String,
    pub device_name: String,
}

impl std::fmt::Debug for StoredCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StoredCredential")
            .field("refresh_credential", &"[REDACTED]")
            .field("session_id", &self.session_id)
            .field("account_id", &self.account_id)
            .field("account_name", &self.account_name)
            .field("device_name", &self.device_name)
            .finish()
    }
}

impl Drop for StoredCredential {
    fn drop(&mut self) {
        self.refresh_credential.zeroize();
    }
}

trait SecretBackend {
    fn load(&self) -> Result<Option<String>>;
    fn save(&self, secret: &str) -> Result<()>;
    fn delete(&self) -> Result<()>;
}

struct OsCredentialBackend;

impl SecretBackend for OsCredentialBackend {
    fn load(&self) -> Result<Option<String>> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
            .context("failed to open the OS credential store")?;
        match entry.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(anyhow!(error)).context("failed to read the OS credential store"),
        }
    }

    fn save(&self, secret: &str) -> Result<()> {
        keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
            .context("failed to open the OS credential store")?
            .set_password(secret)
            .context("failed to write the OS credential store")
    }

    fn delete(&self) -> Result<()> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
            .context("failed to open the OS credential store")?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(anyhow!(error)).context("failed to clear the OS credential store"),
        }
    }
}

#[derive(Debug, Clone)]
struct FileCredentialBackend {
    path: PathBuf,
}

impl SecretBackend for FileCredentialBackend {
    fn load(&self) -> Result<Option<String>> {
        match fs::read_to_string(&self.path) {
            Ok(secret) => Ok(Some(secret)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => {
                Err(error).with_context(|| format!("failed to read {}", self.path.display()))
            }
        }
    }

    fn save(&self, secret: &str) -> Result<()> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| anyhow!("credential path has no parent"))?;
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
        protect_directory(parent)?;
        reject_project_storage(&self.path)?;
        let mut temporary = NamedTempFile::new_in(parent)
            .with_context(|| format!("failed to create a file in {}", parent.display()))?;
        set_owner_only(temporary.path())?;
        temporary
            .write_all(secret.as_bytes())
            .context("failed to write fallback credentials")?;
        temporary
            .as_file()
            .sync_all()
            .context("failed to flush fallback credentials")?;
        temporary
            .persist(&self.path)
            .map_err(|error| error.error)
            .with_context(|| format!("failed to replace {}", self.path.display()))?;
        set_owner_only(&self.path)
    }

    fn delete(&self) -> Result<()> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => {
                Err(error).with_context(|| format!("failed to remove {}", self.path.display()))
            }
        }
    }
}

pub struct CredentialStore {
    os: Box<dyn SecretBackend>,
    fallback: Box<dyn SecretBackend>,
    active_backend: Cell<Option<BackendKind>>,
}

#[derive(Clone, Copy)]
enum BackendKind {
    Os,
    Fallback,
}

pub trait CredentialPersistence {
    fn load_credential(&self) -> Result<Option<StoredCredential>>;
    fn save_credential(&self, credential: &StoredCredential) -> Result<()>;
    fn delete_credential(&self) -> Result<()>;
}

impl CredentialStore {
    pub fn system() -> Result<Self> {
        Ok(Self {
            os: Box::new(OsCredentialBackend),
            fallback: Box::new(FileCredentialBackend {
                path: config_dir()?.join(CREDENTIAL_FILE),
            }),
            active_backend: Cell::new(None),
        })
    }

    pub fn load(&self) -> Result<Option<StoredCredential>> {
        let serialized = match self.os.load() {
            Ok(Some(value)) => {
                self.active_backend.set(Some(BackendKind::Os));
                Some(value)
            }
            Ok(None) => match self.fallback.load()? {
                Some(value) => {
                    self.active_backend.set(Some(BackendKind::Fallback));
                    Some(value)
                }
                None => {
                    self.active_backend.set(None);
                    None
                }
            },
            Err(_) => {
                self.active_backend.set(Some(BackendKind::Fallback));
                self.fallback.load()?
            }
        };
        serialized
            .map(|value| {
                serde_json::from_str(&value).context("stored Kome credentials are invalid")
            })
            .transpose()
    }

    pub fn save(&self, credential: &StoredCredential) -> Result<()> {
        let mut serialized =
            serde_json::to_string(credential).context("failed to serialize Kome credentials")?;
        let result = match self.os.save(&serialized) {
            Ok(()) => {
                self.active_backend.set(Some(BackendKind::Os));
                self.fallback.delete()
            }
            Err(_) => {
                self.fallback.save(&serialized)?;
                self.active_backend.set(Some(BackendKind::Fallback));
                Ok(())
            }
        };
        serialized.zeroize();
        result
    }

    pub fn delete(&self) -> Result<()> {
        let os_result = self.os.delete();
        let fallback_result = self.fallback.delete();
        let result = match (self.active_backend.get(), os_result, fallback_result) {
            (_, _, Err(error)) => Err(error),
            (Some(BackendKind::Os), Err(error), Ok(())) => Err(error),
            (_, _, Ok(())) => Ok(()),
        };
        if result.is_ok() {
            self.active_backend.set(None);
        }
        result
    }
}

impl CredentialPersistence for CredentialStore {
    fn load_credential(&self) -> Result<Option<StoredCredential>> {
        self.load()
    }

    fn save_credential(&self, credential: &StoredCredential) -> Result<()> {
        self.save(credential)
    }

    fn delete_credential(&self) -> Result<()> {
        self.delete()
    }
}

pub fn config_dir() -> Result<PathBuf> {
    if let Some(path) = env::var_os("KOME_CONFIG_HOME") {
        return Ok(PathBuf::from(path));
    }

    #[cfg(target_os = "windows")]
    let base = env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join("Library/Application Support"));
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let base = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));

    base.map(|path| path.join("mochios/kome"))
        .ok_or_else(|| anyhow!("cannot determine the Kome configuration directory"))
}

fn reject_project_storage(path: &Path) -> Result<()> {
    let current = env::current_dir().context("failed to determine the current directory")?;
    reject_project_storage_from(path, &current)
}

fn reject_project_storage_from(path: &Path, current: &Path) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("credential path has no parent"))?;
    let credential_parent = fs::canonicalize(parent)
        .with_context(|| format!("failed to resolve {}", parent.display()))?;
    for ancestor in current.ancestors() {
        if !ancestor.join("Kome.toml").is_file() {
            continue;
        }
        let project = fs::canonicalize(ancestor)
            .with_context(|| format!("failed to resolve {}", ancestor.display()))?;
        if credential_parent.starts_with(project) {
            bail!("refusing to store CLI credentials inside the project directory");
        }
    }
    Ok(())
}

#[cfg(unix)]
fn set_owner_only(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("failed to protect {}", path.display()))
}

#[cfg(unix)]
fn protect_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("failed to protect {}", path.display()))
}

#[cfg(windows)]
fn set_owner_only(path: &Path) -> Result<()> {
    let user = env::var_os("USERNAME").ok_or_else(|| anyhow!("USERNAME is not set"))?;
    let status = std::process::Command::new("icacls")
        .arg(path)
        .arg("/inheritance:r")
        .arg("/grant:r")
        .arg(format!("{}:F", user.to_string_lossy()))
        .status()
        .context("failed to execute icacls")?;
    if !status.success() {
        bail!("icacls failed to protect fallback credentials");
    }
    Ok(())
}

#[cfg(windows)]
fn protect_directory(path: &Path) -> Result<()> {
    set_owner_only(path)
}

#[cfg(not(any(unix, windows)))]
fn set_owner_only(_path: &Path) -> Result<()> {
    bail!("no owner-only permission implementation is available on this platform")
}

#[cfg(not(any(unix, windows)))]
fn protect_directory(_path: &Path) -> Result<()> {
    bail!("no owner-only permission implementation is available on this platform")
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use super::*;

    #[derive(Clone)]
    struct MemoryBackend {
        value: Rc<RefCell<Option<String>>>,
        fail: bool,
    }

    impl SecretBackend for MemoryBackend {
        fn load(&self) -> Result<Option<String>> {
            if self.fail {
                bail!("unavailable");
            }
            Ok(self.value.borrow().clone())
        }

        fn save(&self, secret: &str) -> Result<()> {
            if self.fail {
                bail!("unavailable");
            }
            *self.value.borrow_mut() = Some(secret.to_string());
            Ok(())
        }

        fn delete(&self) -> Result<()> {
            if self.fail {
                bail!("unavailable");
            }
            *self.value.borrow_mut() = None;
            Ok(())
        }
    }

    fn credential() -> StoredCredential {
        StoredCredential {
            refresh_credential: "refresh-secret".to_string(),
            session_id: "session-1".to_string(),
            account_id: "account-1".to_string(),
            account_name: "jine".to_string(),
            device_name: "Kome CLI test".to_string(),
        }
    }

    #[test]
    fn os_store_is_preferred_and_refresh_rotation_is_persisted() {
        let os = Rc::new(RefCell::new(None));
        let fallback = Rc::new(RefCell::new(None));
        let store = CredentialStore {
            os: Box::new(MemoryBackend {
                value: os.clone(),
                fail: false,
            }),
            fallback: Box::new(MemoryBackend {
                value: fallback.clone(),
                fail: false,
            }),
            active_backend: Cell::new(None),
        };
        let mut value = credential();
        store.save(&value).unwrap();
        assert!(os.borrow().as_ref().unwrap().contains("refresh-secret"));
        assert!(fallback.borrow().is_none());

        value.refresh_credential = "rotated-secret".to_string();
        store.save(&value).unwrap();
        assert_eq!(store.load().unwrap(), Some(value));
    }

    #[test]
    fn unavailable_os_store_uses_fallback() {
        let fallback = Rc::new(RefCell::new(None));
        let store = CredentialStore {
            os: Box::new(MemoryBackend {
                value: Rc::new(RefCell::new(None)),
                fail: true,
            }),
            fallback: Box::new(MemoryBackend {
                value: fallback.clone(),
                fail: false,
            }),
            active_backend: Cell::new(None),
        };
        store.save(&credential()).unwrap();
        assert!(fallback.borrow().is_some());
        assert_eq!(store.load().unwrap(), Some(credential()));
        store.delete().unwrap();
        assert!(fallback.borrow().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn fallback_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("credentials.json");
        let backend = FileCredentialBackend { path: path.clone() };
        backend.save("secret").unwrap();
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn debug_output_redacts_refresh_credential() {
        let output = format!("{:?}", credential());
        assert!(!output.contains("refresh-secret"));
        assert!(output.contains("[REDACTED]"));
    }

    #[test]
    fn fallback_rejects_project_storage_from_nested_directory() {
        let temporary = tempfile::tempdir().unwrap();
        let project = temporary.path().join("project");
        let nested = project.join("src/nested");
        let credential_dir = project.join("private-config");
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir_all(&credential_dir).unwrap();
        fs::write(project.join("Kome.toml"), "[package]\n").unwrap();

        assert!(
            reject_project_storage_from(&credential_dir.join("credentials.json"), &nested,)
                .is_err()
        );
    }
}
