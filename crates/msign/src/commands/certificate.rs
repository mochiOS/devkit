use std::{fs, io::Read};

use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use ed25519_dalek::Signer;
use mochios_certificate::{
    key_id, DeveloperCertificate, PackageIdScope, KEY_USAGE_PACKAGE_SIGNING, SIGNATURE_LEN,
};
use reqwest::blocking::{Client, Response};
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};

use crate::cli::{CertificateInspectArgs, CertificateIssueArgs, CertificateObtainArgs};
use crate::commands::mpkg;
use crate::crypto;

const MAX_CERTIFICATE_RESPONSE_BYTES: u64 = 1024 * 1024;
const MAX_ERROR_BODY_BYTES: u64 = 8 * 1024;

pub fn issue(args: CertificateIssueArgs) -> Result<()> {
    let issuer_key_path = args
        .issuer_key
        .or(args.root_key)
        .ok_or_else(|| anyhow!("--issuer-key is required"))?;
    let issuer_key = crypto::read_private_key(&issuer_key_path)?;
    let root_public_key = issuer_key.verifying_key().to_bytes();
    let subject_public_key = match (args.subject_public_key, args.developer_key) {
        (Some(path), _) => crypto::read_public_key(&path)?.to_bytes(),
        (None, Some(path)) => {
            eprintln!("warning: --developer-key for certificate issue is deprecated; use --subject-public-key");
            crypto::read_private_key(&path)?.verifying_key().to_bytes()
        }
        (None, None) => bail!("--subject-public-key is required"),
    };
    let mut package_id_scopes = args
        .scopes
        .iter()
        .map(|value| parse_scope(value))
        .collect::<Result<Vec<_>>>()?;
    package_id_scopes.sort();
    let mut allowed_capabilities = args.capabilities;
    allowed_capabilities.sort();

    let mut certificate = DeveloperCertificate {
        serial_number: args.serial,
        issuer_key_id: key_id(&root_public_key),
        developer_id: args.developer_id,
        subject_key_id: key_id(&subject_public_key),
        subject_public_key,
        not_before: args.not_before,
        not_after: args.not_after,
        key_usage: KEY_USAGE_PACKAGE_SIGNING,
        package_id_scopes,
        allowed_capabilities,
        signature: [0; SIGNATURE_LEN],
    };
    certificate.validate().map_err(|error| anyhow!(error))?;
    let message = certificate
        .signing_message()
        .map_err(|error| anyhow!(error))?;
    certificate.signature = issuer_key.sign(&message).to_bytes();
    let length = certificate.encoded_len().map_err(|error| anyhow!(error))?;
    let mut encoded = vec![0; length];
    certificate
        .encode(&mut encoded)
        .map_err(|error| anyhow!(error))?;
    if let Some(parent) = args.output.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(&args.output, encoded)
        .with_context(|| format!("failed to write {}", args.output.display()))?;
    println!("issued: {}", args.output.display());
    println!("developer_id: {}", certificate.developer_id);
    println!("serial_number: {}", certificate.serial_number);
    println!("subject_key_id: {}", hex(&certificate.subject_key_id));
    Ok(())
}

#[derive(Debug, Serialize)]
struct ObtainRequest {
    developer_id: String,
    subject_public_key: String,
    package_id: String,
    capabilities: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ObtainResponse {
    certificate: Option<String>,
    certificate_base64: Option<String>,
}

pub fn obtain(args: CertificateObtainArgs) -> Result<()> {
    let public_key = crypto::read_public_key(&args.public_key)?;
    let request = mpkg::certificate_request(&args.package, &args.developer, &public_key)?;
    let response = request_certificate(&args.api_base, args.bearer_token.as_deref(), &request)?;
    let certificate_bytes = decode_certificate_response(&response)?;
    let certificate =
        DeveloperCertificate::decode(&certificate_bytes).map_err(|error| anyhow!(error))?;

    validate_obtained_certificate(&certificate, &public_key.to_bytes(), &request)?;
    if let Some(parent) = args.output.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(&args.output, certificate_bytes)
        .with_context(|| format!("failed to write {}", args.output.display()))?;

    println!("obtained: {}", args.output.display());
    println!("developer_id: {}", certificate.developer_id);
    println!("serial_number: {}", certificate.serial_number);
    println!("subject_key_id: {}", hex(&certificate.subject_key_id));

    Ok(())
}

pub(crate) struct CertificateRequest {
    pub developer_id: String,
    pub subject_public_key: String,
    pub package_id: String,
    pub capabilities: Vec<String>,
}

fn request_certificate(
    api_base: &str,
    bearer_token: Option<&str>,
    request: &CertificateRequest,
) -> Result<ObtainResponse> {
    let url = format!("{}/developer-certificates", api_base.trim_end_matches('/'));
    let body = ObtainRequest {
        developer_id: request.developer_id.clone(),
        subject_public_key: request.subject_public_key.clone(),
        package_id: request.package_id.clone(),
        capabilities: request.capabilities.clone(),
    };
    let client = Client::new();
    let mut http = client
        .post(&url)
        .header(ACCEPT, "application/json")
        .header(CONTENT_TYPE, "application/json")
        .json(&body);
    if let Some(token) = bearer_token {
        http = http.header(AUTHORIZATION, format!("Bearer {token}"));
    }
    let response = http
        .send()
        .with_context(|| format!("failed to request certificate from {url}"))?;
    let status = response.status();
    if !status.is_success() {
        let text = read_response_body_limited(response, MAX_ERROR_BODY_BYTES)
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default();
        bail!("certificate issuance failed: HTTP {status}: {text}");
    }
    let body = read_response_body_limited(response, MAX_CERTIFICATE_RESPONSE_BYTES)?;
    serde_json::from_slice(&body).context("failed to parse certificate response")
}

fn read_response_body_limited(response: Response, limit: u64) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    response
        .take(limit + 1)
        .read_to_end(&mut body)
        .context("failed to read certificate response")?;
    if body.len() > usize::try_from(limit).unwrap_or(usize::MAX) {
        bail!("certificate response is too large");
    }
    Ok(body)
}

fn decode_certificate_response(response: &ObtainResponse) -> Result<Vec<u8>> {
    let encoded = response
        .certificate_base64
        .as_deref()
        .or(response.certificate.as_deref())
        .ok_or_else(|| anyhow!("certificate response is missing certificate bytes"))?;
    STANDARD
        .decode(encoded.trim())
        .context("certificate response is not valid base64")
}

fn validate_obtained_certificate(
    certificate: &DeveloperCertificate,
    public_key: &[u8; 32],
    request: &CertificateRequest,
) -> Result<()> {
    if &certificate.subject_public_key != public_key {
        bail!("certificate subject public key does not match requested public key");
    }
    if certificate.subject_key_id != key_id(public_key) {
        bail!("certificate subject key id does not match requested public key");
    }
    if certificate.developer_id != request.developer_id {
        bail!("certificate developer id does not match request");
    }
    if !certificate
        .package_id_scopes
        .iter()
        .any(|scope| scope.matches(&request.package_id))
    {
        bail!("certificate package scope does not cover requested package");
    }
    let now = current_unix_time()?;
    if now < certificate.not_before {
        bail!("certificate is not yet valid");
    }
    if now >= certificate.not_after {
        bail!("certificate is expired");
    }
    for capability in &request.capabilities {
        if !certificate
            .allowed_capabilities
            .iter()
            .any(|allowed| allowed == capability)
        {
            bail!("certificate does not allow requested capability: {capability}");
        }
    }
    Ok(())
}

fn current_unix_time() -> Result<u64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("system time is before UNIX_EPOCH")?
        .as_secs())
}

pub fn inspect(args: CertificateInspectArgs) -> Result<()> {
    let bytes = fs::read(&args.certificate)
        .with_context(|| format!("failed to read {}", args.certificate.display()))?;
    let certificate = DeveloperCertificate::decode(&bytes).map_err(|error| anyhow!(error))?;
    println!("format_version: 1");
    println!("serial_number: {}", certificate.serial_number);
    println!("issuer_key_id: {}", hex(&certificate.issuer_key_id));
    println!("developer_id: {}", certificate.developer_id);
    println!("subject_key_id: {}", hex(&certificate.subject_key_id));
    println!(
        "subject_public_key: {}",
        hex(&certificate.subject_public_key)
    );
    println!("not_before: {}", certificate.not_before);
    println!("not_after: {}", certificate.not_after);
    println!("key_usage: {:#x}", certificate.key_usage);
    for scope in certificate.package_id_scopes {
        println!("package_scope: {:?}:{}", scope.kind, scope.package_id);
    }
    for capability in certificate.allowed_capabilities {
        println!("allowed_capability: {capability}");
    }
    println!("signature: {}", hex(&certificate.signature));
    Ok(())
}

fn parse_scope(value: &str) -> Result<PackageIdScope> {
    if let Some(package_id) = value.strip_prefix("exact:") {
        return Ok(PackageIdScope::exact(package_id));
    }
    if let Some(package_id) = value.strip_prefix("prefix:") {
        return Ok(PackageIdScope::prefix(package_id));
    }
    bail!("scope must be exact:PACKAGE_ID or prefix:PACKAGE_ID")
}

fn hex(bytes: &[u8]) -> String {
    const TABLE: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(TABLE[(byte >> 4) as usize] as char);
        output.push(TABLE[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
    };

    #[test]
    fn issue_accepts_subject_public_key_without_developer_private_key() {
        let temporary = tempfile::tempdir().unwrap();
        let root_key_path = temporary.path().join("root.key");
        let developer_public_path = temporary.path().join("application.pub");
        let certificate_path = temporary.path().join("developer.cert");
        let (root_key, _) = crypto::generate_keypair();
        let (_, developer_public) = crypto::generate_keypair();
        crypto::write_private_key(&root_key_path, &root_key).unwrap();
        crypto::write_public_key(&developer_public_path, &developer_public).unwrap();

        issue(CertificateIssueArgs {
            root_key: None,
            issuer_key: Some(root_key_path),
            developer_key: None,
            subject_public_key: Some(developer_public_path),
            output: certificate_path.clone(),
            serial: 7,
            developer_id: "org.example.developer".to_string(),
            not_before: 1_700_000_000,
            not_after: 1_900_000_000,
            scopes: vec!["exact:org.example.application".to_string()],
            capabilities: vec!["window.create".to_string()],
        })
        .unwrap();

        let bytes = fs::read(certificate_path).unwrap();
        let certificate = DeveloperCertificate::decode(&bytes).unwrap();
        assert_eq!(certificate.subject_public_key, developer_public.to_bytes());
        assert_eq!(
            certificate.subject_key_id,
            key_id(&developer_public.to_bytes())
        );
    }

    #[test]
    fn obtained_certificate_must_match_requested_public_key() {
        let (_, requested_public) = crypto::generate_keypair();
        let (_, other_public) = crypto::generate_keypair();
        let request = CertificateRequest {
            developer_id: "org.example.developer".to_string(),
            subject_public_key: crypto::public_key_to_base64(&requested_public),
            package_id: "org.example.application".to_string(),
            capabilities: vec!["window.create".to_string()],
        };
        let other_public_bytes = other_public.to_bytes();
        let certificate = DeveloperCertificate {
            serial_number: 1,
            issuer_key_id: [0; 32],
            developer_id: "org.example.developer".to_string(),
            subject_key_id: key_id(&other_public_bytes),
            subject_public_key: other_public_bytes,
            not_before: 1_700_000_000,
            not_after: 1_900_000_000,
            key_usage: KEY_USAGE_PACKAGE_SIGNING,
            package_id_scopes: vec![PackageIdScope::exact("org.example.application")],
            allowed_capabilities: vec!["window.create".to_string()],
            signature: [0; SIGNATURE_LEN],
        };

        assert!(validate_obtained_certificate(
            &certificate,
            &requested_public.to_bytes(),
            &request
        )
        .is_err());
    }

    #[test]
    fn obtained_certificate_must_be_currently_valid() {
        let (_, requested_public) = crypto::generate_keypair();
        let request = CertificateRequest {
            developer_id: "org.example.developer".to_string(),
            subject_public_key: crypto::public_key_to_base64(&requested_public),
            package_id: "org.example.application".to_string(),
            capabilities: Vec::new(),
        };
        let public_key = requested_public.to_bytes();
        let mut certificate = DeveloperCertificate {
            serial_number: 1,
            issuer_key_id: [0; 32],
            developer_id: "org.example.developer".to_string(),
            subject_key_id: key_id(&public_key),
            subject_public_key: public_key,
            not_before: 1,
            not_after: 2,
            key_usage: KEY_USAGE_PACKAGE_SIGNING,
            package_id_scopes: vec![PackageIdScope::exact("org.example.application")],
            allowed_capabilities: Vec::new(),
            signature: [0; SIGNATURE_LEN],
        };
        assert!(validate_obtained_certificate(&certificate, &public_key, &request).is_err());

        let now = current_unix_time().unwrap();
        certificate.not_before = now + 60;
        certificate.not_after = now + 120;
        assert!(validate_obtained_certificate(&certificate, &public_key, &request).is_err());
    }

    #[test]
    fn obtain_request_contains_only_public_certificate_inputs() {
        let (api_base, received, server) =
            serve_once(200, r#"{"certificate_base64":"AA=="}"#.to_string());
        let request = CertificateRequest {
            developer_id: "org.example.developer".to_string(),
            subject_public_key: "PUBLIC_KEY".to_string(),
            package_id: "org.example.application".to_string(),
            capabilities: vec!["window.create".to_string(), "process.spawn".to_string()],
        };

        let response = request_certificate(&api_base, Some("test-token"), &request).unwrap();
        server.join().unwrap();
        let http = received.recv().unwrap();

        assert_eq!(response.certificate_base64.as_deref(), Some("AA=="));
        assert!(http.starts_with("POST /developer-certificates HTTP/1.1"));
        assert!(http.contains("authorization: Bearer test-token"));
        assert!(http.contains(r#""developer_id":"org.example.developer""#));
        assert!(http.contains(r#""subject_public_key":"PUBLIC_KEY""#));
        assert!(http.contains(r#""package_id":"org.example.application""#));
        assert!(http.contains(r#""capabilities":["window.create","process.spawn"]"#));
        assert!(!http.contains("application.key"));
        assert!(!http.contains("PRIVATE"));
        assert!(!http.contains("entry.elf"));
        assert!(!http.contains("payload"));
    }

    #[test]
    fn certificate_api_error_is_not_success() {
        let (api_base, _received, server) =
            serve_once(403, "Developer is not verified".to_string());
        let request = CertificateRequest {
            developer_id: "org.example.developer".to_string(),
            subject_public_key: "PUBLIC_KEY".to_string(),
            package_id: "org.example.application".to_string(),
            capabilities: Vec::new(),
        };

        let error = request_certificate(&api_base, None, &request)
            .unwrap_err()
            .to_string();
        server.join().unwrap();

        assert!(error.contains("certificate issuance failed: HTTP 403"));
        assert!(error.contains("Developer is not verified"));
    }

    #[test]
    fn certificate_response_size_is_limited() {
        let oversized = "x".repeat(MAX_CERTIFICATE_RESPONSE_BYTES as usize + 1);
        let (api_base, _received, server) = serve_once(200, oversized);
        let request = CertificateRequest {
            developer_id: "org.example.developer".to_string(),
            subject_public_key: "PUBLIC_KEY".to_string(),
            package_id: "org.example.application".to_string(),
            capabilities: Vec::new(),
        };

        let error = request_certificate(&api_base, None, &request)
            .unwrap_err()
            .to_string();
        server.join().unwrap();

        assert!(error.contains("certificate response is too large"));
    }

    fn serve_once(
        status: u16,
        response_body: String,
    ) -> (String, mpsc::Receiver<String>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let api_base = format!("http://{}", listener.local_addr().unwrap());
        let (sender, receiver) = mpsc::channel();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            sender.send(request).unwrap();
            let status_text = match status {
                200 => "OK",
                403 => "Forbidden",
                _ => "Error",
            };
            let response = format!(
                "HTTP/1.1 {status} {status_text}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        (api_base, receiver, handle)
    }

    fn read_http_request(stream: &mut std::net::TcpStream) -> String {
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 1024];
        let mut content_length = None;
        loop {
            let count = stream.read(&mut buffer).unwrap();
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..count]);
            if let Some(header_end) = find_header_end(&bytes) {
                if content_length.is_none() {
                    let headers = String::from_utf8_lossy(&bytes[..header_end]);
                    content_length = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .and_then(|value| value.trim().parse::<usize>().ok());
                }
                let expected = header_end + 4 + content_length.unwrap_or(0);
                if bytes.len() >= expected {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }

    fn find_header_end(bytes: &[u8]) -> Option<usize> {
        bytes.windows(4).position(|window| window == b"\r\n\r\n")
    }
}
