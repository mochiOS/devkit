use std::{
    fs,
    io::{Read, Write},
    path::Path,
    time::Duration,
};

use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use mochios_certificate::{is_valid_developer_id, key_id, DeveloperCertificate};
use rand_core::{OsRng, RngCore};
use reqwest::{blocking::Client, Url};
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

pub const DEFAULT_DEVELOPER_CA_API_BASE: &str = "https://ca.mochios.org/v1";
const MAX_REQUEST_BYTES: usize = 16 * 1024;
const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;
const MAX_CAPABILITIES: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertificateRequirements {
    pub developer_id: String,
    pub subject_public_key: [u8; 32],
    pub package_id: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug)]
pub struct IssuedCertificate {
    pub certificate_bytes: Vec<u8>,
    pub issuer_public_key: [u8; 32],
}

pub trait CertificateIssuer {
    fn issue(
        &self,
        access_token: &str,
        requirements: &CertificateRequirements,
    ) -> Result<IssuedCertificate>;
}

pub struct HttpCertificateIssuer {
    client: Client,
    base: Url,
}

impl HttpCertificateIssuer {
    pub fn new(base: &str) -> Result<Self> {
        let base = Url::parse(base).context("DeveloperCA API base URL is invalid")?;
        if base.scheme() != "https" && !is_loopback_http(&base) {
            bail!("DeveloperCA API must use HTTPS");
        }
        Ok(Self {
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .context("failed to create the DeveloperCA HTTP client")?,
            base,
        })
    }

    fn issue_url(&self, developer_id: &str) -> Result<Url> {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .map_err(|_| anyhow!("DeveloperCA API base URL cannot contain path segments"))?
            .pop_if_empty()
            .extend(["developers", developer_id, "certificates", "issue"]);
        Ok(url)
    }
}

#[derive(Serialize)]
struct IssueRequest<'a> {
    subject_public_key: String,
    package_id: &'a str,
    capabilities: &'a [String],
}

#[derive(Deserialize)]
struct IssueResponse {
    #[serde(alias = "certificate")]
    certificate_base64: String,
    #[serde(alias = "root_public_key")]
    issuer_public_key: String,
    developer_id: String,
}

#[derive(Deserialize)]
struct ErrorResponse {
    #[serde(default)]
    error: String,
    #[serde(default, alias = "error_description")]
    message: String,
}

impl CertificateIssuer for HttpCertificateIssuer {
    fn issue(
        &self,
        access_token: &str,
        requirements: &CertificateRequirements,
    ) -> Result<IssuedCertificate> {
        validate_requirements(requirements)?;
        let request = IssueRequest {
            subject_public_key: STANDARD.encode(requirements.subject_public_key),
            package_id: &requirements.package_id,
            capabilities: &requirements.capabilities,
        };
        let body = serde_json::to_vec(&request).context("failed to encode certificate request")?;
        if body.len() > MAX_REQUEST_BYTES {
            bail!("certificate request body exceeds 16 KiB");
        }
        let response = self
            .client
            .post(self.issue_url(&requirements.developer_id)?)
            .bearer_auth(access_token)
            .header("X-Idempotency-Key", idempotency_key())
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .context("Developer Certificate request failed")?;
        let successful = response.status().is_success();
        let response_body = read_limited(response)?;
        if !successful {
            let response: ErrorResponse = serde_json::from_slice(&response_body)
                .context("DeveloperCA returned an invalid error response")?;
            let message = if response.message.is_empty() {
                response.error
            } else {
                response.message
            };
            bail!("Developer Certificateの発行に失敗しました: {message}");
        }
        let response: IssueResponse = serde_json::from_slice(&response_body)
            .context("DeveloperCA returned an invalid certificate response")?;
        if response.developer_id != requirements.developer_id {
            bail!("DeveloperCA response Developer ID does not match the request");
        }
        let certificate_bytes = STANDARD
            .decode(response.certificate_base64.trim())
            .context("DeveloperCA certificate is not valid Base64")?;
        let issuer_public_key: [u8; 32] = STANDARD
            .decode(response.issuer_public_key.trim())
            .context("DeveloperCA issuer public key is not valid Base64")?
            .try_into()
            .map_err(|_| anyhow!("DeveloperCA issuer public key must contain 32 bytes"))?;
        validate_certificate(
            &certificate_bytes,
            &issuer_public_key,
            requirements,
            current_unix_time()?,
        )?;
        Ok(IssuedCertificate {
            certificate_bytes,
            issuer_public_key,
        })
    }
}

pub fn validate_certificate(
    bytes: &[u8],
    issuer_public_key: &[u8; 32],
    requirements: &CertificateRequirements,
    unix_time: u64,
) -> Result<DeveloperCertificate> {
    validate_requirements(requirements)?;
    let certificate = DeveloperCertificate::decode(bytes).map_err(|error| anyhow!(error))?;
    let mut canonical = vec![0; certificate.encoded_len().map_err(|error| anyhow!(error))?];
    certificate
        .encode(&mut canonical)
        .map_err(|error| anyhow!(error))?;
    if canonical != bytes {
        bail!("Developer Certificate is not canonical MCER encoding");
    }
    if certificate.subject_public_key != requirements.subject_public_key
        || certificate.subject_key_id != key_id(&requirements.subject_public_key)
    {
        bail!("Developer Certificate Subject does not match application.pub");
    }
    if certificate.developer_id != requirements.developer_id {
        bail!("Developer Certificate Developer ID does not match");
    }
    let verified = certificate
        .verify(issuer_public_key, unix_time, &requirements.package_id)
        .map_err(|error| anyhow!(error))?;
    for capability in &requirements.capabilities {
        if !verified.allows_capability(capability) {
            bail!("Developer Certificate does not allow capability: {capability}");
        }
    }
    Ok(certificate)
}

pub fn read_public_key(path: &Path) -> Result<[u8; 32]> {
    let text =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    STANDARD
        .decode(text.trim())
        .context("application.pub is not valid Base64")?
        .try_into()
        .map_err(|_| anyhow!("application.pub must contain 32 raw Ed25519 bytes"))
}

pub fn write_issuer_public_key(path: &Path, key: &[u8; 32]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("issuer public key path has no parent"))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let mut temporary =
        NamedTempFile::new_in(parent).context("failed to create temporary issuer public key")?;
    temporary
        .write_all(STANDARD.encode(key).as_bytes())
        .context("failed to write temporary issuer public key")?;
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    Ok(())
}

fn validate_requirements(requirements: &CertificateRequirements) -> Result<()> {
    if !is_valid_developer_id(&requirements.developer_id) {
        bail!("Developer ID is invalid");
    }
    if requirements.capabilities.len() > MAX_CAPABILITIES {
        bail!("certificate request contains more than {MAX_CAPABILITIES} capabilities");
    }
    Ok(())
}

fn read_limited(mut response: reqwest::blocking::Response) -> Result<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES)
    {
        bail!("DeveloperCA response is too large");
    }
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take(MAX_RESPONSE_BYTES + 1)
        .read_to_end(&mut bytes)
        .context("failed to read DeveloperCA response")?;
    if bytes.len() as u64 > MAX_RESPONSE_BYTES {
        bail!("DeveloperCA response is too large");
    }
    Ok(bytes)
}

fn idempotency_key() -> String {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn current_unix_time() -> Result<u64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("system time is before UNIX_EPOCH")?
        .as_secs())
}

fn is_loopback_http(url: &Url) -> bool {
    url.scheme() == "http" && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1"))
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
    };

    use ed25519_dalek::{Signer, SigningKey};
    use mochios_certificate::{PackageIdScope, KEY_USAGE_PACKAGE_SIGNING, SIGNATURE_LEN};

    use super::*;

    fn signed_certificate(requirements: &CertificateRequirements) -> (Vec<u8>, [u8; 32]) {
        let root = SigningKey::from_bytes(&[7; 32]);
        let root_public = root.verifying_key().to_bytes();
        let mut certificate = DeveloperCertificate {
            serial_number: 1,
            issuer_key_id: key_id(&root_public),
            developer_id: requirements.developer_id.clone(),
            subject_key_id: key_id(&requirements.subject_public_key),
            subject_public_key: requirements.subject_public_key,
            not_before: 1_700_000_000,
            not_after: 1_900_000_000,
            key_usage: KEY_USAGE_PACKAGE_SIGNING,
            package_id_scopes: vec![PackageIdScope::exact(&requirements.package_id)],
            allowed_capabilities: requirements.capabilities.clone(),
            signature: [0; SIGNATURE_LEN],
        };
        certificate.signature = root
            .sign(&certificate.signing_message().unwrap())
            .to_bytes();
        let mut bytes = vec![0; certificate.encoded_len().unwrap()];
        certificate.encode(&mut bytes).unwrap();
        (bytes, root_public)
    }

    fn requirements() -> CertificateRequirements {
        CertificateRequirements {
            developer_id: "019f9e5ac6687902b0e72fe53abfbef1".to_string(),
            subject_public_key: SigningKey::from_bytes(&[9; 32]).verifying_key().to_bytes(),
            package_id: "com.example.app".to_string(),
            capabilities: vec!["window.create".to_string()],
        }
    }

    #[test]
    fn certificate_validation_checks_issuer_subject_scope_and_capabilities() {
        let requirements = requirements();
        let (bytes, root) = signed_certificate(&requirements);
        assert!(validate_certificate(&bytes, &root, &requirements, 1_800_000_000).is_ok());

        let mut wrong_subject = requirements.clone();
        wrong_subject.subject_public_key = [3; 32];
        assert!(validate_certificate(&bytes, &root, &wrong_subject, 1_800_000_000).is_err());

        let mut wrong_scope = requirements.clone();
        wrong_scope.package_id = "com.example.other".to_string();
        assert!(validate_certificate(&bytes, &root, &wrong_scope, 1_800_000_000).is_err());

        let mut extra_capability = requirements.clone();
        extra_capability
            .capabilities
            .push("process.spawn".to_string());
        assert!(validate_certificate(&bytes, &root, &extra_capability, 1_800_000_000).is_err());

        let other_root = SigningKey::from_bytes(&[11; 32]).verifying_key().to_bytes();
        assert!(validate_certificate(&bytes, &other_root, &requirements, 1_800_000_000).is_err());
    }

    #[test]
    fn idempotency_keys_are_random_and_safe() {
        let first = idempotency_key();
        let second = idempotency_key();
        assert_eq!(first.len(), 32);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(first, second);
    }

    #[test]
    fn issue_request_contains_only_public_certificate_inputs() {
        let requirements = requirements();
        let (certificate, root) = signed_certificate(&requirements);
        let response = serde_json::json!({
            "certificate_base64": STANDARD.encode(certificate),
            "issuer_public_key": STANDARD.encode(root),
            "developer_id": requirements.developer_id,
        })
        .to_string();
        let (base, request, server) = serve_once(response);
        let issuer = HttpCertificateIssuer::new(&base).unwrap();
        issuer.issue("access-secret", &requirements).unwrap();
        server.join().unwrap();
        let request = request.recv().unwrap();
        let (_, body) = request.split_once("\r\n\r\n").unwrap();

        assert!(request.starts_with(
            "POST /v1/developers/019f9e5ac6687902b0e72fe53abfbef1/certificates/issue HTTP/1.1"
        ));
        assert!(request.contains("authorization: Bearer access-secret"));
        assert!(body.contains("subject_public_key"));
        assert!(body.contains("com.example.app"));
        assert!(body.contains("window.create"));
        assert!(!body.contains("access-secret"));
        assert!(!body.contains("refresh"));
        assert!(!body.contains("application.key"));
        assert!(!body.contains("entry.elf"));
        assert!(!body.contains("payload"));
    }

    fn serve_once(
        response_body: String,
    ) -> (String, mpsc::Receiver<String>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let (sender, receiver) = mpsc::channel();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            sender.send(request).unwrap();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        (base, receiver, handle)
    }

    fn read_request(stream: &mut std::net::TcpStream) -> String {
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 1024];
        loop {
            let count = stream.read(&mut buffer).unwrap();
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..count]);
            if let Some(header_end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..header_end]);
                let length = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length: "))
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(0);
                if bytes.len() >= header_end + 4 + length {
                    break;
                }
            }
        }
        String::from_utf8(bytes).unwrap()
    }
}
