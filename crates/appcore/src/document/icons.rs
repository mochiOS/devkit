//! Declarative document artwork. No image rendering or filesystem privilege
//! escalation: assets are read with the caller's ordinary filesystem authority.
use super::AssociationRoles;
use crate::{Error, Result};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

const MAX_MANIFEST: usize = 64 * 1024;
const MAX_ICON: usize = 1024 * 1024;

/// 宣言されたファイルアイコンの画像形式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocumentIconFormat {
    /// ベクター形式のSVG。
    Svg,
    /// ラスター形式のPNG。
    Png,
}

/// 関連付け先アプリが提供する静的なファイルアイコン。
#[derive(Clone, Debug)]
pub struct DocumentIcon {
    /// 提供元アプリの識別子。
    pub bundle_id: String,
    /// アプリ内で宣言されたリソースの相対パス。
    pub resource: String,
    /// 画像データの形式。
    pub format: DocumentIconFormat,
    bytes: Vec<u8>,
}
impl DocumentIcon {
    /// サイズ上限付きで読み取った画像データを返します。
    pub fn data(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Clone)]
struct Rule {
    extensions: Vec<String>,
    types: Vec<String>,
    resource: String,
}
#[derive(Clone)]
struct Application {
    root: PathBuf,
    id: String,
    name: String,
    roles: u16,
    extensions: Vec<String>,
    types: Vec<String>,
    rules: Vec<Rule>,
    fallback: Option<String>,
    entry: PathBuf,
    thumbnail: bool,
}

/// インストール済みアプリの宣言を一度読み取るアイコン解決器。
/// ディレクトリの再読込み時などに作り直すと、アプリの追加・更新が反映されます。
pub struct DocumentIconResolver {
    applications: Vec<Application>,
}

impl DocumentIconResolver {
    /// アプリカタログを読み取ります。mochiOSでは通常のファイル読取り権限が必要です。
    /// Linuxプレビューではセッションの `MOCHIOS_PREVIEW_APPS` を使用します。
    ///
    /// manifestの `[application]` に `document_icon = "icons/text.svg"` と
    /// `resources = ["icons/text.svg", "icons/rust.svg"]` を宣言できます。
    /// `[[application.document_icons]]` の `extensions = ["rs"]` と
    /// `resource = "icons/rust.svg"` で種別ごとの画像を指定します。
    /// 画像ルールはアプリ自身が処理対象として宣言した拡張子・内容型に限定されます。
    /// これは静的な画像の取得APIであり、アプリの実行やサムネイル生成は行いません。
    pub fn new() -> Result<Self> {
        #[cfg(target_os = "mochios")]
        let roots = vec![
            PathBuf::from("/system/applications"),
            PathBuf::from("/applications"),
        ];
        #[cfg(target_os = "linux")]
        let roots = vec![
            std::env::var_os("MOCHIOS_PREVIEW_APPS")
                .map(PathBuf::from)
                .ok_or(Error::UnsupportedPlatform)?,
        ];
        #[cfg(not(any(target_os = "mochios", target_os = "linux")))]
        return Err(Error::UnsupportedPlatform);
        #[cfg(any(target_os = "mochios", target_os = "linux"))]
        Self::from_roots(&roots)
    }

    fn from_roots(roots: &[PathBuf]) -> Result<Self> {
        let mut applications = Vec::new();
        let mut seen = BTreeSet::new();
        for root in roots {
            let entries = match fs::read_dir(root) {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(io_error(error)),
            };
            let mut local = BTreeMap::<String, Vec<Application>>::new();
            for entry in entries.take(1024).flatten() {
                if let Some(app) = parse_application(&entry.path()) {
                    local.entry(app.id.clone()).or_default().push(app);
                }
            }
            for (id, mut entries) in local {
                if entries.len() == 1 && seen.insert(id) {
                    applications.push(entries.remove(0));
                }
            }
        }
        applications.sort_by_key(|app| (app.name.to_lowercase(), app.id.clone()));
        Ok(Self { applications })
    }

    /// ファイルの拡張子・内容型・操作ロールから、関連付け先のアイコンを取得します。
    /// mochiOSでは既存の関連付けAPIを使うため `file-association.read` も必要です。
    /// 拡張子別の画像、内容型別の画像、汎用文書画像、アプリアイコンの順に選びます。
    /// 画像はアプリの宣言済みリソース内に限定し、SVG/PNGを最大1MiBまで読み取ります。PNGの寸法上限は1024pxです。
    /// 未登録の場合は `Ok(None)`、権限やI/Oの失敗はエラーを返します。
    pub fn resolve(
        &self,
        extension: &str,
        content_type: &str,
        roles: AssociationRoles,
    ) -> Result<Option<DocumentIcon>> {
        let extension = extension.trim_start_matches('.').to_ascii_lowercase();
        let content_type = content_type.to_ascii_lowercase();
        if (extension.is_empty() && content_type.is_empty())
            || extension.len() > 63
            || !extension
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
            || (!content_type.is_empty()
                && crate::content_type::ContentType::parse(&content_type).is_err())
        {
            return Err(Error::InvalidArgument);
        }
        let provider = self.provider(&extension, &content_type, roles)?;
        let Some(app) = provider else {
            return Ok(None);
        };
        let resource = app
            .rules
            .iter()
            .find(|rule| !extension.is_empty() && rule.extensions.contains(&extension))
            .or_else(|| {
                app.rules
                    .iter()
                    .find(|rule| !content_type.is_empty() && rule.types.contains(&content_type))
            })
            .map(|rule| &rule.resource)
            .or(app.fallback.as_ref());
        let Some(resource) = resource else {
            return Ok(None);
        };
        let format = match Path::new(resource)
            .extension()
            .and_then(|value| value.to_str())
        {
            Some("svg") => DocumentIconFormat::Svg,
            Some("png") => DocumentIconFormat::Png,
            _ => return Ok(None),
        };
        let root = fs::canonicalize(&app.root).map_err(io_error)?;
        let path = fs::canonicalize(root.join(resource)).map_err(io_error)?;
        if !path.starts_with(&root) {
            return Err(Error::InvalidArgument);
        }
        let bytes = read_limited(&path, MAX_ICON).map_err(io_error)?;
        if format == DocumentIconFormat::Png {
            if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR"
            {
                return Err(Error::InvalidArgument);
            }
            let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
            let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
            if width == 0 || height == 0 || width > 1024 || height > 1024 {
                return Err(Error::InvalidArgument);
            }
        }
        Ok(Some(DocumentIcon {
            bundle_id: app.id.clone(),
            resource: resource.clone(),
            format,
            bytes,
        }))
    }
    fn provider(
        &self,
        extension: &str,
        content_type: &str,
        roles: AssociationRoles,
    ) -> Result<Option<&Application>> {
        #[cfg(target_os = "mochios")]
        let provider = match super::resolve_default(extension, content_type, roles) {
            Ok(id) => self.applications.iter().find(|app| app.id == id),
            Err(Error::System(2)) => return Ok(None),
            Err(error) => return Err(error),
        };
        #[cfg(not(target_os = "mochios"))]
        let provider = [true, false].into_iter().find_map(|mime| {
            self.applications.iter().find(|app| {
                app.roles & roles.bits() != 0
                    && if mime {
                        !content_type.is_empty()
                            && app.types.iter().any(|value| value == content_type)
                    } else {
                        !extension.is_empty()
                            && app.extensions.iter().any(|value| value == extension)
                    }
            })
        });
        Ok(provider)
    }

    /// 関連付け先が明示的に `thumbnail = true` を宣言した場合、生成担当を返します。
    /// mochiOSでは既存の関連付け権限を検証します。未対応なら `Ok(None)` です。
    /// この関数ではアプリを起動しません。生成にはバックグラウンドで
    /// [`super::thumbnail::ThumbnailProvider::generate`] を呼び出してください。
    #[cfg(any(target_os = "linux", target_os = "mochios"))]
    pub fn thumbnail_provider(
        &self,
        extension: &str,
        content_type: &str,
    ) -> Result<Option<super::thumbnail::ThumbnailProvider>> {
        super::validate_key(extension, content_type)?;
        if extension.len() > 63
            || !extension
                .trim_start_matches('.')
                .bytes()
                .all(|b| b.is_ascii_alphanumeric())
            || (!content_type.is_empty()
                && crate::content_type::ContentType::parse(content_type).is_err())
        {
            return Err(Error::InvalidArgument);
        }
        let extension = extension.trim_start_matches('.').to_ascii_lowercase();
        let content_type = content_type.to_ascii_lowercase();
        Ok(self
            .provider(&extension, &content_type, AssociationRoles::VIEW)?
            .filter(|app| app.thumbnail)
            .map(|app| super::thumbnail::ThumbnailProvider::new(app.id.clone(), app.entry.clone())))
    }
}

fn io_error(error: std::io::Error) -> Error {
    Error::System(error.raw_os_error().unwrap_or(5) as i64)
}
fn read_limited(path: &Path, maximum: usize) -> std::io::Result<Vec<u8>> {
    let file = fs::File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::other("resource is not a file"));
    }
    let mut bytes = Vec::new();
    file.take(maximum as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > maximum {
        return Err(std::io::Error::other("resource exceeds size limit"));
    }
    Ok(bytes)
}
fn relative(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4095
        && !Path::new(value).is_absolute()
        && Path::new(value)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}
fn strings(value: Option<&toml::Value>) -> Vec<String> {
    value
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_str)
        .take(128)
        .map(str::to_owned)
        .collect()
}
fn parse_application(root: &Path) -> Option<Application> {
    let bytes = read_limited(&root.join("manifest.toml"), MAX_MANIFEST).ok()?;
    let manifest: toml::Value = toml::from_str(std::str::from_utf8(&bytes).ok()?).ok()?;
    let package = manifest.get("package")?;
    let table = manifest.get("application")?;
    let id = package.get("id")?.as_str()?.to_owned();
    if id.is_empty()
        || id.len() > 255
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
    {
        return None;
    }
    let name = package.get("name")?.as_str()?.to_owned();
    if name.trim().is_empty() || name.len() > 255 || name.chars().any(char::is_control) {
        return None;
    }
    let entry = table.get("entry")?.as_str()?;
    if !relative(entry) || !root.join(entry).is_file() {
        return None;
    }
    let resources = strings(table.get("resources"));
    let allowed =
        |resource: &str| relative(resource) && resources.iter().any(|path| path == resource);
    let extensions = strings(table.get("document_extensions"))
        .into_iter()
        .map(|value| value.trim_start_matches('.').to_ascii_lowercase())
        .collect::<Vec<_>>();
    let types = strings(table.get("document_content_types"))
        .into_iter()
        .map(|value| value.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let roles = strings(table.get("document_roles"))
        .iter()
        .fold(0, |value, role| {
            value
                | match role.as_str() {
                    "view" => 1,
                    "edit" => 2,
                    _ => 0,
                }
        });
    let mut rules = Vec::new();
    for value in table
        .get("document_icons")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .take(64)
    {
        let Some(resource) = value
            .get("resource")
            .and_then(toml::Value::as_str)
            .filter(|resource| allowed(resource))
        else {
            continue;
        };
        let rule_extensions = strings(value.get("extensions"))
            .into_iter()
            .map(|value| value.trim_start_matches('.').to_ascii_lowercase())
            .filter(|value| extensions.contains(value))
            .collect::<Vec<_>>();
        let rule_types = strings(value.get("content_types"))
            .into_iter()
            .map(|value| value.to_ascii_lowercase())
            .filter(|value| types.contains(value))
            .collect::<Vec<_>>();
        if !rule_extensions.is_empty() || !rule_types.is_empty() {
            rules.push(Rule {
                extensions: rule_extensions,
                types: rule_types,
                resource: resource.into(),
            });
        }
    }
    let fallback = table
        .get("document_icon")
        .and_then(toml::Value::as_str)
        .filter(|value| allowed(value))
        .or_else(|| {
            table
                .get("icon")
                .and_then(toml::Value::as_str)
                .filter(|value| allowed(value))
        })
        .map(str::to_owned);
    Some(Application {
        root: root.to_owned(),
        id,
        name,
        roles,
        extensions,
        types,
        rules,
        fallback,
        entry: root.join(entry),
        thumbnail: table
            .get("thumbnail")
            .and_then(toml::Value::as_bool)
            .unwrap_or(false),
    })
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "appcore-icons-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                SERIAL.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn app(&self, directory: &str, extra: &str) -> PathBuf {
            let root = self.0.join(directory);
            fs::create_dir_all(root.join("icons")).unwrap();
            fs::write(root.join("entry.elf"), b"entry").unwrap();
            fs::write(
                root.join("manifest.toml"),
                format!(
                    r#"
[package]
id = "org.example.editor"
name = "Editor"
[application]
entry = "entry.elf"
resources = ["icons/default.svg", "icons/rust.svg", "icons/mime.svg", "icons/image.png"]
document_extensions = ["rs", "txt", "png"]
document_content_types = ["text/plain", "image/png"]
document_roles = ["view", "edit"]
{extra}
"#
                ),
            )
            .unwrap();
            for name in ["default", "rust", "mime"] {
                fs::write(root.join(format!("icons/{name}.svg")), name).unwrap();
            }
            root
        }
        fn resolver(&self) -> DocumentIconResolver {
            DocumentIconResolver::from_roots(&[self.0.clone()]).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn thumbnail_generation_requires_explicit_provider_opt_in() {
        let fixture = Fixture::new();
        fixture.app("Editor.app", "document_icon = \"icons/default.svg\"");
        assert!(
            fixture
                .resolver()
                .thumbnail_provider("txt", "text/plain")
                .unwrap()
                .is_none()
        );
        let root = fixture.0.join("Editor.app/manifest.toml");
        let manifest = fs::read_to_string(&root).unwrap();
        fs::write(&root, format!("{manifest}\nthumbnail = true\n")).unwrap();
        let resolver = fixture.resolver();
        assert_eq!(
            resolver
                .thumbnail_provider("txt", "text/plain")
                .unwrap()
                .unwrap()
                .bundle_id,
            "org.example.editor"
        );
        assert!(resolver.thumbnail_provider("png", "not mime").is_err());
    }

    #[test]
    fn extension_rule_precedes_mime_and_fallback() {
        let fixture = Fixture::new();
        fixture.app(
            "Editor.app",
            r#"
document_icon = "icons/default.svg"
[[application.document_icons]]
content_types = ["text/plain"]
resource = "icons/mime.svg"
[[application.document_icons]]
extensions = ["rs"]
resource = "icons/rust.svg"
"#,
        );
        let resolver = fixture.resolver();
        assert_eq!(
            resolver
                .resolve(".RS", "text/plain", AssociationRoles::VIEW)
                .unwrap()
                .unwrap()
                .data(),
            b"rust"
        );
        assert_eq!(
            resolver
                .resolve("txt", "text/plain", AssociationRoles::EDIT)
                .unwrap()
                .unwrap()
                .data(),
            b"mime"
        );
        assert_eq!(
            resolver
                .resolve("txt", "", AssociationRoles::VIEW)
                .unwrap()
                .unwrap()
                .data(),
            b"default"
        );
        assert!(
            resolver
                .resolve("unknown", "", AssociationRoles::VIEW)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn unhandled_rules_and_undeclared_resources_are_ignored() {
        let fixture = Fixture::new();
        fixture.app(
            "Editor.app",
            r#"
icon = "icons/default.svg"
document_icon = "../external.svg"
[[application.document_icons]]
extensions = ["txt"]
resource = "undeclared.svg"
[[application.document_icons]]
content_types = ["image/jpeg"]
resource = "icons/rust.svg"
"#,
        );
        let resolver = fixture.resolver();
        assert_eq!(
            resolver
                .resolve("txt", "image/jpeg", AssociationRoles::VIEW)
                .unwrap()
                .unwrap()
                .data(),
            b"default"
        );
        assert!(
            resolver
                .resolve("../txt", "", AssociationRoles::VIEW)
                .is_err()
        );
        assert!(
            resolver
                .resolve("txt", "bad mime", AssociationRoles::VIEW)
                .is_err()
        );
    }

    #[test]
    fn rejects_external_symlinks_and_oversized_assets() {
        let fixture = Fixture::new();
        let root = fixture.app("Editor.app", "document_icon = \"icons/default.svg\"");
        let asset = root.join("icons/default.svg");
        fs::remove_file(&asset).unwrap();
        fs::write(fixture.0.join("external.svg"), "external").unwrap();
        std::os::unix::fs::symlink(fixture.0.join("external.svg"), &asset).unwrap();
        let resolver = fixture.resolver();
        assert!(resolver.resolve("txt", "", AssociationRoles::VIEW).is_err());
        fs::remove_file(&asset).unwrap();
        fs::write(&asset, vec![0; MAX_ICON + 1]).unwrap();
        assert!(resolver.resolve("txt", "", AssociationRoles::VIEW).is_err());
    }

    #[test]
    fn duplicate_ids_are_not_selected_and_catalogue_is_a_snapshot() {
        let fixture = Fixture::new();
        fixture.app("A.app", "document_icon = \"icons/default.svg\"");
        let resolver = fixture.resolver();
        fixture.app("B.app", "document_icon = \"icons/default.svg\"");
        assert!(
            fixture
                .resolver()
                .resolve("txt", "", AssociationRoles::VIEW)
                .unwrap()
                .is_none()
        );
        assert!(
            resolver
                .resolve("txt", "", AssociationRoles::VIEW)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn bounds_png_dimensions_before_decode() {
        let fixture = Fixture::new();
        let root = fixture.app("Editor.app", "document_icon = \"icons/image.png\"");
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&1025u32.to_be_bytes());
        png.extend_from_slice(&64u32.to_be_bytes());
        fs::write(root.join("icons/image.png"), &png).unwrap();
        let resolver = fixture.resolver();
        assert!(
            resolver
                .resolve("png", "image/png", AssociationRoles::VIEW)
                .is_err()
        );
        png[16..20].copy_from_slice(&64u32.to_be_bytes());
        fs::write(root.join("icons/image.png"), &png).unwrap();
        assert_eq!(
            resolver
                .resolve("png", "image/png", AssociationRoles::VIEW)
                .unwrap()
                .unwrap()
                .format,
            DocumentIconFormat::Png
        );
    }
}
