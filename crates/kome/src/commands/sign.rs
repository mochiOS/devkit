use std::{
    fs,
    io::{self, BufReader},
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context, Result};
use tempfile::NamedTempFile;

use crate::{
    auth::{refresh_login, DeveloperApi, HttpAccountsApi, HttpDeveloperApi},
    certificate_client::{
        read_public_key, validate_certificate, write_issuer_public_key, CertificateIssuer,
        CertificateRequirements, HttpCertificateIssuer, IssuedCertificate,
    },
    cli::{BuildArgs, KeygenArgs, LoginArgs, PackArgs, SignArgs},
    commands::{build, keygen, login, pack},
    credential::CredentialStore,
    developer_selection,
    manifest::KomeManifest,
    preferences::Preferences,
    project,
};

pub fn run(args: SignArgs) -> Result<()> {
    let project_dir = absolute_project_path(&args.project)?;
    let manifest = project::read_manifest(&project_dir)?;
    let paths = SignPaths::new(&project_dir, &manifest, &args);

    ensure_built(&project_dir, &manifest, &paths, args.release)?;
    ensure_packed(&project_dir, &manifest, &paths, args.release)?;
    keygen::run_in_project(
        KeygenArgs {
            private_key: paths.private_key.clone(),
            public_key: paths.public_key.clone(),
        },
        &project_dir,
    )?;

    let store = CredentialStore::system()?;
    if store.load()?.is_none() {
        if args.login {
            login::run(LoginArgs {
                accounts_api_base: args.accounts_api_base.clone(),
                no_browser: false,
            })?;
        } else {
            bail!("Developer Certificateを取得するにはログインが必要です。\n\n実行:\n  kome login");
        }
    }
    let accounts = HttpAccountsApi::new(&args.accounts_api_base)?;
    let authenticated = refresh_login(&accounts, &store).map_err(|error| {
        anyhow::anyhow!(
            "ログイン状態の有効期限が切れています。\n\n再ログイン:\n  kome login\n\n原因: {error:#}"
        )
    })?;
    let developers = HttpDeveloperApi::new(&args.developer_ca_api_base)?
        .developers(authenticated.session.access_token.expose())?;
    let preferences = Preferences::load()?;
    let stdin = io::stdin();
    let mut input = BufReader::new(stdin.lock());
    let mut output = io::stdout();
    let developer_id = developer_selection::resolve(
        &manifest,
        &preferences,
        &developers,
        &mut input,
        &mut output,
    )?;

    let requirements = CertificateRequirements {
        developer_id: developer_id.clone(),
        subject_public_key: read_public_key(&paths.public_key)?,
        package_id: manifest.package.id.clone(),
        capabilities: required_capabilities(&manifest),
    };
    let now = args.unix_time.unwrap_or(current_unix_time()?);
    let certificate_reused = existing_certificate_is_valid(&paths, &requirements, now);
    if !certificate_reused {
        let issuer = HttpCertificateIssuer::new(&args.developer_ca_api_base)?;
        let issued = issuer.issue(authenticated.session.access_token.expose(), &requirements)?;
        save_issued_certificate(&paths, &issued)?;
    }

    sign_and_verify(&paths, now)?;

    println!("Account:     {}", authenticated.account.account_name);
    println!("Developer:   {developer_id}");
    println!("Certificate: {}", paths.certificate.display());
    println!("Signed:      {}", paths.signed_package.display());
    println!("Verified:    OK");
    Ok(())
}

struct SignPaths {
    unsigned_package: PathBuf,
    signed_package: PathBuf,
    private_key: PathBuf,
    public_key: PathBuf,
    certificate: PathBuf,
    issuer_public_key: PathBuf,
    build_entry: PathBuf,
}

impl SignPaths {
    fn new(project_dir: &Path, manifest: &KomeManifest, args: &SignArgs) -> Self {
        let profile = if args.release { "release" } else { "debug" };
        Self {
            unsigned_package: args.package.as_ref().map_or_else(
                || {
                    project_dir
                        .join("dist")
                        .join(format!("{}-unsigned.mpkg", manifest.package.name))
                },
                |path| project_path(project_dir, path),
            ),
            signed_package: args.output.as_ref().map_or_else(
                || {
                    project_dir
                        .join("dist")
                        .join(format!("{}.mpkg", manifest.package.name))
                },
                |path| project_path(project_dir, path),
            ),
            private_key: project_path(project_dir, &args.key),
            public_key: project_path(project_dir, &args.public_key),
            certificate: project_path(project_dir, &args.certificate),
            issuer_public_key: project_path(project_dir, &args.issuer_public_key),
            build_entry: project_dir
                .join("target")
                .join(profile)
                .join(&manifest.app.entry),
        }
    }
}

fn ensure_built(
    project_dir: &Path,
    _manifest: &KomeManifest,
    paths: &SignPaths,
    release: bool,
) -> Result<()> {
    let inputs = [project_dir.join("Kome.toml"), project_dir.join("src")];
    if !is_fresh(&paths.build_entry, &inputs)? {
        build::run(BuildArgs {
            project_dir: project_dir.to_path_buf(),
            release,
        })?;
    }
    Ok(())
}

fn ensure_packed(
    project_dir: &Path,
    manifest: &KomeManifest,
    paths: &SignPaths,
    release: bool,
) -> Result<()> {
    let mut inputs = vec![project_dir.join("Kome.toml"), paths.build_entry.clone()];
    inputs.push(project_dir.join(&manifest.app.icon));
    inputs.extend(
        manifest
            .resources
            .files
            .iter()
            .map(|path| project_dir.join(path)),
    );
    if !is_fresh(&paths.unsigned_package, &inputs)? {
        pack::run(PackArgs {
            project_dir: project_dir.to_path_buf(),
            output: Some(paths.unsigned_package.clone()),
            release,
            force: true,
        })?;
    }
    Ok(())
}

fn existing_certificate_is_valid(
    paths: &SignPaths,
    requirements: &CertificateRequirements,
    now: u64,
) -> bool {
    let certificate = match fs::read(&paths.certificate) {
        Ok(value) => value,
        Err(_) => return false,
    };
    let issuer = match read_public_key(&paths.issuer_public_key) {
        Ok(value) => value,
        Err(_) => return false,
    };
    validate_certificate(&certificate, &issuer, requirements, now).is_ok()
}

fn save_issued_certificate(paths: &SignPaths, issued: &IssuedCertificate) -> Result<()> {
    if let Some(parent) = paths.certificate.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    write_issuer_public_key(&paths.issuer_public_key, &issued.issuer_public_key)?;
    let parent = paths
        .certificate
        .parent()
        .ok_or_else(|| anyhow::anyhow!("certificate path has no parent"))?;
    let mut temporary = NamedTempFile::new_in(parent)
        .context("failed to create temporary Developer Certificate")?;
    use std::io::Write;
    temporary
        .write_all(&issued.certificate_bytes)
        .context("failed to write temporary Developer Certificate")?;
    temporary
        .persist(&paths.certificate)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {}", paths.certificate.display()))?;
    Ok(())
}

fn sign_and_verify(paths: &SignPaths, now: u64) -> Result<()> {
    if let Some(parent) = paths.signed_package.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let temporary = NamedTempFile::new_in(
        paths
            .signed_package
            .parent()
            .unwrap_or_else(|| Path::new(".")),
    )?;
    let temporary = temporary.into_temp_path();
    fs::remove_file(&temporary).context("failed to prepare temporary signed MPKG path")?;
    run_tool(
        Command::new("msign")
            .arg("package")
            .arg("sign")
            .arg(&paths.unsigned_package)
            .arg("--certificate")
            .arg(&paths.certificate)
            .arg("--key")
            .arg(&paths.private_key)
            .arg("--output")
            .arg(&temporary)
            .arg("--unix-time")
            .arg(now.to_string()),
        "msign package sign",
    )?;
    run_tool(
        Command::new("msign")
            .arg("package")
            .arg("verify")
            .arg(&temporary)
            .arg("--root-public-key")
            .arg(&paths.issuer_public_key)
            .arg("--unix-time")
            .arg(now.to_string()),
        "msign package verify",
    )?;
    temporary
        .persist(&paths.signed_package)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {}", paths.signed_package.display()))?;
    Ok(())
}

fn run_tool(command: &mut Command, name: &str) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("failed to execute {name}"))?;
    if !status.success() {
        bail!("{name} failed");
    }
    Ok(())
}

fn required_capabilities(manifest: &KomeManifest) -> Vec<String> {
    let mut capabilities = manifest.capabilities.required.clone();
    capabilities.sort();
    capabilities.dedup();
    capabilities
}

fn is_fresh(output: &Path, inputs: &[PathBuf]) -> Result<bool> {
    let output_time = match fs::metadata(output).and_then(|metadata| metadata.modified()) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error).context("failed to inspect generated output"),
    };
    for input in inputs {
        if !input.exists() {
            continue;
        }
        if newest_modified(input)? > output_time {
            return Ok(false);
        }
    }
    Ok(true)
}

fn newest_modified(path: &Path) -> Result<SystemTime> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        bail!("build input cannot be a symlink: {}", path.display());
    }
    let mut newest = metadata.modified()?;
    if metadata.is_dir() {
        for entry in
            fs::read_dir(path).with_context(|| format!("failed to read {}", path.display()))?
        {
            newest = newest.max(newest_modified(&entry?.path())?);
        }
    }
    Ok(newest)
}

fn project_path(project_dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_dir.join(path)
    }
}

fn absolute_project_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn current_unix_time() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system time is before UNIX_EPOCH")?
        .as_secs())
}

pub fn default_signed_package_path(project_dir: &Path) -> Result<PathBuf> {
    let manifest = project::read_manifest(project_dir)?;
    Ok(project_dir
        .join("dist")
        .join(format!("{}.mpkg", manifest.package.name)))
}

#[cfg(test)]
mod tests {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use ed25519_dalek::{Signer, SigningKey};
    use mochios_certificate::{
        key_id, DeveloperCertificate, PackageIdScope, KEY_USAGE_PACKAGE_SIGNING, SIGNATURE_LEN,
    };

    use super::*;

    #[test]
    fn required_capabilities_are_a_sorted_union() {
        let mut manifest = KomeManifest::new_app(
            "Example".to_string(),
            "com.example.app".to_string(),
            "Example Developer".to_string(),
        );
        manifest.capabilities.required = vec![
            "window.create".to_string(),
            "fs.read.all".to_string(),
            "window.create".to_string(),
        ];
        assert_eq!(
            required_capabilities(&manifest),
            vec!["fs.read.all".to_string(), "window.create".to_string()]
        );
    }

    #[test]
    fn freshness_detects_newer_nested_source() {
        let temporary = tempfile::tempdir().unwrap();
        let source_dir = temporary.path().join("src");
        fs::create_dir(&source_dir).unwrap();
        let output = temporary.path().join("output");
        fs::write(&output, b"old").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(source_dir.join("main.kome"), b"new").unwrap();
        assert!(!is_fresh(&output, &[source_dir]).unwrap());
    }

    #[test]
    fn certificate_reuse_tracks_key_developer_package_capability_and_expiry() {
        let temporary = tempfile::tempdir().unwrap();
        let paths = SignPaths {
            unsigned_package: temporary.path().join("unsigned.mpkg"),
            signed_package: temporary.path().join("signed.mpkg"),
            private_key: temporary.path().join("application.key"),
            public_key: temporary.path().join("application.pub"),
            certificate: temporary.path().join("developer.cert"),
            issuer_public_key: temporary.path().join("issuer.pub"),
            build_entry: temporary.path().join("entry.elf"),
        };
        let root = SigningKey::from_bytes(&[7; 32]);
        let root_public = root.verifying_key().to_bytes();
        let subject = SigningKey::from_bytes(&[9; 32]).verifying_key().to_bytes();
        let requirements = CertificateRequirements {
            developer_id: "019f9e5ac6687902b0e72fe53abfbef1".to_string(),
            subject_public_key: subject,
            package_id: "com.example.app".to_string(),
            capabilities: vec!["window.create".to_string()],
        };
        let mut certificate = DeveloperCertificate {
            serial_number: 1,
            issuer_key_id: key_id(&root_public),
            developer_id: requirements.developer_id.clone(),
            subject_key_id: key_id(&subject),
            subject_public_key: subject,
            not_before: 100,
            not_after: 200,
            key_usage: KEY_USAGE_PACKAGE_SIGNING,
            package_id_scopes: vec![PackageIdScope::exact("com.example.app")],
            allowed_capabilities: vec!["window.create".to_string()],
            signature: [0; SIGNATURE_LEN],
        };
        certificate.signature = root
            .sign(&certificate.signing_message().unwrap())
            .to_bytes();
        let mut bytes = vec![0; certificate.encoded_len().unwrap()];
        certificate.encode(&mut bytes).unwrap();
        fs::write(&paths.certificate, bytes).unwrap();
        fs::write(&paths.issuer_public_key, STANDARD.encode(root_public)).unwrap();

        assert!(existing_certificate_is_valid(&paths, &requirements, 150));
        assert!(!existing_certificate_is_valid(&paths, &requirements, 200));

        let mut changed = requirements.clone();
        changed.subject_public_key = [3; 32];
        assert!(!existing_certificate_is_valid(&paths, &changed, 150));
        let mut changed = requirements.clone();
        changed.developer_id = "019f9e5ac6687902b0e72fe53abfbef2".to_string();
        assert!(!existing_certificate_is_valid(&paths, &changed, 150));
        let mut changed = requirements.clone();
        changed.package_id = "com.example.other".to_string();
        assert!(!existing_certificate_is_valid(&paths, &changed, 150));
        let mut changed = requirements;
        changed.capabilities.push("process.spawn".to_string());
        assert!(!existing_certificate_is_valid(&paths, &changed, 150));
    }
}
