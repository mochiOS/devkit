use std::{
    env,
    io::Read,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use mochios_certificate::is_valid_developer_id;
use rand_core::{OsRng, RngCore};
use reqwest::{
    blocking::{Client, Response},
    StatusCode,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;
use zeroize::Zeroize;

use crate::credential::{CredentialPersistence, StoredCredential};

pub const DEFAULT_ACCOUNTS_API_BASE: &str = "https://accounts.mochios.org/v1/cli";
const CLIENT_ID: &str = "kome-cli";
const RESPONSE_LIMIT: u64 = 1024 * 1024;
const SLOW_DOWN_SECONDS: u64 = 5;

#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub(crate) fn new(value: String) -> Self {
        Self(value)
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Deserialize)]
pub struct DeviceAuthorization {
    #[serde(skip)]
    pub device_code: Secret,
    #[serde(rename = "device_code")]
    device_code_wire: String,
    pub user_code: String,
    pub verification_uri: String,
    #[serde(default)]
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
    pub interval: u64,
}

impl std::fmt::Debug for DeviceAuthorization {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DeviceAuthorization")
            .field("device_code", &"[REDACTED]")
            .field("user_code", &self.user_code)
            .field("verification_uri", &self.verification_uri)
            .field("verification_uri_complete", &"[PUBLIC CODE URL]")
            .field("expires_in", &self.expires_in)
            .field("interval", &self.interval)
            .finish()
    }
}

impl DeviceAuthorization {
    fn normalize(mut self) -> Result<Self> {
        if self.device_code_wire.is_empty()
            || self.user_code.is_empty()
            || self.verification_uri.is_empty()
            || self.expires_in == 0
            || self.interval == 0
        {
            bail!("Accounts returned an incomplete Device Authorization response");
        }
        self.device_code = Secret::new(std::mem::take(&mut self.device_code_wire));
        Ok(self)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct AccountMetadata {
    #[serde(alias = "id")]
    pub account_id: String,
    #[serde(alias = "name")]
    pub account_name: String,
    #[serde(default = "default_device_name")]
    pub device_name: String,
}

fn default_device_name() -> String {
    "Kome CLI".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct DeveloperMembership {
    pub id: String,
    pub display_name: String,
    pub status: String,
    pub verification_status: String,
    pub role: String,
    pub can_issue: bool,
}

impl DeveloperMembership {
    pub fn is_issuable(&self) -> bool {
        is_valid_developer_id(&self.id)
            && self.status == "active"
            && self.verification_status == "verified"
            && self.can_issue
    }
}

#[derive(Debug)]
pub struct AccessSession {
    pub access_token: Secret,
    pub refresh_token: Secret,
}

#[derive(Debug)]
pub struct AuthenticatedAccount {
    pub session: AccessSession,
    pub account: AccountMetadata,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PollResult {
    Granted(DeviceTokenGrant),
    AuthorizationPending,
    SlowDown,
    AccessDenied,
    ExpiredToken,
    InvalidGrant,
}

#[derive(Deserialize, PartialEq, Eq)]
pub struct DeviceTokenGrant {
    token_type: String,
    access_token: String,
    expires_in: u64,
    refresh_token: String,
    account: AccountMetadata,
}

impl std::fmt::Debug for DeviceTokenGrant {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DeviceTokenGrant")
            .field("token_type", &self.token_type)
            .field("access_token", &"[REDACTED]")
            .field("expires_in", &self.expires_in)
            .field("refresh_token", &"[REDACTED]")
            .field("account", &self.account)
            .finish()
    }
}

impl DeviceTokenGrant {
    fn into_session(mut self) -> Result<(AccessSession, AccountMetadata)> {
        if self.account.account_id.is_empty() || self.account.account_name.is_empty() {
            bail!("Accounts returned an incomplete CLI session");
        }
        let session = access_session(
            &self.token_type,
            self.expires_in,
            &mut self.access_token,
            &mut self.refresh_token,
        )?;
        Ok((session, self.account))
    }
}

#[derive(Deserialize, PartialEq, Eq)]
struct RefreshTokenGrant {
    token_type: String,
    access_token: String,
    expires_in: u64,
    refresh_token: String,
}

impl std::fmt::Debug for RefreshTokenGrant {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RefreshTokenGrant")
            .field("token_type", &self.token_type)
            .field("access_token", &"[REDACTED]")
            .field("expires_in", &self.expires_in)
            .field("refresh_token", &"[REDACTED]")
            .finish()
    }
}

impl RefreshTokenGrant {
    fn into_session(mut self) -> Result<AccessSession> {
        access_session(
            &self.token_type,
            self.expires_in,
            &mut self.access_token,
            &mut self.refresh_token,
        )
    }
}

fn access_session(
    token_type: &str,
    expires_in: u64,
    access_token: &mut String,
    refresh_token: &mut String,
) -> Result<AccessSession> {
    if token_type != "Bearer"
        || access_token.is_empty()
        || expires_in == 0
        || refresh_token.is_empty()
    {
        bail!("Accounts returned an incomplete CLI session");
    }
    Ok(AccessSession {
        access_token: Secret::new(std::mem::take(access_token)),
        refresh_token: Secret::new(std::mem::take(refresh_token)),
    })
}

pub trait AccountsApi {
    fn start_device_authorization(
        &self,
        code_challenge: &str,
        device_name: &str,
    ) -> Result<DeviceAuthorization>;
    fn poll_device_token(&self, device_code: &str, code_verifier: &str) -> Result<PollResult>;
    fn refresh(&self, refresh_token: &str) -> Result<AccessSession>;
    fn revoke(&self, access_token: &str) -> Result<()>;
}

pub trait DeveloperApi {
    fn developers(&self, access_token: &str) -> Result<Vec<DeveloperMembership>>;
}

pub trait Browser {
    fn open(&self, url: &Url) -> bool;
}

pub trait Waiter {
    fn wait(&self, duration: Duration) -> bool;
}

pub trait LoginUi {
    fn present(&self, verification_url: &Url, user_code: &str, browser_opened: bool);
    fn waiting(&self);
}

pub struct SystemBrowser;

impl Browser for SystemBrowser {
    fn open(&self, url: &Url) -> bool {
        open::that_detached(url.as_str()).is_ok()
    }
}

pub struct InterruptibleWaiter {
    cancelled: Arc<AtomicBool>,
}

impl InterruptibleWaiter {
    pub fn install() -> Result<Self> {
        let cancelled = Arc::new(AtomicBool::new(false));
        let signal = cancelled.clone();
        ctrlc::set_handler(move || signal.store(true, Ordering::SeqCst))
            .context("failed to install the Ctrl+C handler")?;
        Ok(Self { cancelled })
    }
}

impl Waiter for InterruptibleWaiter {
    fn wait(&self, duration: Duration) -> bool {
        let mut remaining = duration;
        let slice = Duration::from_millis(100);
        while !remaining.is_zero() {
            if self.cancelled.load(Ordering::SeqCst) {
                return false;
            }
            let current = remaining.min(slice);
            thread::sleep(current);
            remaining = remaining.saturating_sub(current);
        }
        !self.cancelled.load(Ordering::SeqCst)
    }
}

pub struct HttpAccountsApi {
    client: Client,
    base: Url,
}

impl HttpAccountsApi {
    pub fn new(base: &str) -> Result<Self> {
        let mut base = Url::parse(base).context("Accounts API base URL is invalid")?;
        if base.scheme() != "https" && !is_loopback_http(&base) {
            bail!("Accounts API must use HTTPS");
        }
        if !base.path().ends_with('/') {
            let path = format!("{}/", base.path());
            base.set_path(&path);
        }
        Ok(Self {
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .context("failed to create the Accounts HTTP client")?,
            base,
        })
    }

    fn endpoint(&self, path: &str) -> Result<Url> {
        self.base
            .join(path)
            .context("failed to construct an Accounts endpoint")
    }
}

pub struct HttpDeveloperApi {
    client: Client,
    base: Url,
}

impl HttpDeveloperApi {
    pub fn new(base: &str) -> Result<Self> {
        let mut base = Url::parse(base).context("DeveloperCA API base URL is invalid")?;
        if base.scheme() != "https" && !is_loopback_http(&base) {
            bail!("DeveloperCA API must use HTTPS");
        }
        if !base.path().ends_with('/') {
            let path = format!("{}/", base.path());
            base.set_path(&path);
        }
        Ok(Self {
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .context("failed to create the DeveloperCA HTTP client")?,
            base,
        })
    }

    fn endpoint(&self, path: &str) -> Result<Url> {
        self.base
            .join(path)
            .context("failed to construct a DeveloperCA endpoint")
    }
}

#[derive(Serialize)]
struct DeviceAuthorizationRequest<'a> {
    client_id: &'static str,
    code_challenge: &'a str,
    code_challenge_method: &'static str,
    device_name: &'a str,
}

#[derive(Serialize)]
struct DeviceTokenRequest<'a> {
    device_code: &'a str,
    code_verifier: &'a str,
}

#[derive(Serialize)]
struct RefreshRequest<'a> {
    refresh_token: &'a str,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ErrorResponse {
    OAuth {
        error: String,
        #[serde(default)]
        error_description: String,
    },
    General {
        error: GeneralError,
    },
}

#[derive(Debug, Deserialize)]
struct GeneralError {
    code: String,
    message: String,
}

impl AccountsApi for HttpAccountsApi {
    fn start_device_authorization(
        &self,
        code_challenge: &str,
        device_name: &str,
    ) -> Result<DeviceAuthorization> {
        let response = self
            .client
            .post(self.endpoint("device/authorize")?)
            .json(&DeviceAuthorizationRequest {
                client_id: CLIENT_ID,
                code_challenge,
                code_challenge_method: "S256",
                device_name,
            })
            .send()
            .context("Device Authorization request failed")?;
        decode_success::<DeviceAuthorization>(response, "Accounts", "Device Authorization")?
            .normalize()
    }

    fn poll_device_token(&self, device_code: &str, code_verifier: &str) -> Result<PollResult> {
        let response = self
            .client
            .post(self.endpoint("device/token")?)
            .json(&DeviceTokenRequest {
                device_code,
                code_verifier,
            })
            .send()
            .context("Device Authorization polling failed")?;
        if response.status().is_success() {
            return Ok(PollResult::Granted(decode_json(response, "Accounts")?));
        }
        let error = decode_error(response, "Accounts")?;
        match error_code(&error) {
            "authorization_pending" => Ok(PollResult::AuthorizationPending),
            "slow_down" => Ok(PollResult::SlowDown),
            "access_denied" => Ok(PollResult::AccessDenied),
            "expired_token" => Ok(PollResult::ExpiredToken),
            "invalid_grant" => Ok(PollResult::InvalidGrant),
            _ => bail!(
                "Accounts rejected Device Authorization: {}",
                human_api_error(&error)
            ),
        }
    }

    fn refresh(&self, refresh_token: &str) -> Result<AccessSession> {
        let response = self
            .client
            .post(self.endpoint("token/refresh")?)
            .json(&RefreshRequest { refresh_token })
            .send()
            .context("failed to refresh the Kome CLI session")?;
        decode_success::<RefreshTokenGrant>(response, "Accounts", "CLI session refresh")?
            .into_session()
    }

    fn revoke(&self, access_token: &str) -> Result<()> {
        let response = self
            .client
            .post(self.endpoint("session/revoke-current")?)
            .bearer_auth(access_token)
            .send()
            .context("failed to revoke the Kome CLI session")?;
        if response.status().is_success() {
            Ok(())
        } else {
            let error = decode_error(response, "Accounts")?;
            bail!("CLI session revocation failed: {}", human_api_error(&error));
        }
    }
}

impl DeveloperApi for HttpDeveloperApi {
    fn developers(&self, access_token: &str) -> Result<Vec<DeveloperMembership>> {
        #[derive(Deserialize)]
        struct DeveloperList {
            developers: Vec<DeveloperMembership>,
        }

        let response = self
            .client
            .get(self.endpoint("cli/developers")?)
            .bearer_auth(access_token)
            .send()
            .context("failed to obtain the Developer list")?;
        let developers =
            decode_success::<DeveloperList>(response, "DeveloperCA", "Developer list")?.developers;
        if developers
            .iter()
            .any(|membership| !is_valid_developer_id(&membership.id))
        {
            bail!("DeveloperCA returned an invalid Developer ID");
        }
        Ok(developers)
    }
}

#[derive(Debug)]
pub struct LoginResult {
    pub authenticated: AuthenticatedAccount,
}

pub fn device_login(
    api: &dyn AccountsApi,
    browser: &dyn Browser,
    waiter: &dyn Waiter,
    ui: &dyn LoginUi,
) -> Result<LoginResult> {
    let (mut verifier, challenge) = generate_pkce();
    let device_name = current_device_name();
    let authorization = api.start_device_authorization(&challenge, &device_name)?;
    let verification_url = canonical_verification_url(&authorization)?;
    let browser_opened = browser.open(&verification_url);
    ui.present(&verification_url, &authorization.user_code, browser_opened);
    ui.waiting();
    let (session, mut account) = poll_until_authorized(api, waiter, &authorization, &verifier)?;
    verifier.zeroize();
    account.device_name = device_name;
    Ok(LoginResult {
        authenticated: AuthenticatedAccount { session, account },
    })
}

pub fn persist_login(
    store: &dyn CredentialPersistence,
    account: &AuthenticatedAccount,
) -> Result<()> {
    store.save_credential(&StoredCredential {
        refresh_token: account.session.refresh_token.expose().to_string(),
        account_id: account.account.account_id.clone(),
        account_name: account.account.account_name.clone(),
        device_name: account.account.device_name.clone(),
    })
}

pub fn refresh_login(
    api: &dyn AccountsApi,
    store: &dyn CredentialPersistence,
) -> Result<AuthenticatedAccount> {
    let stored = store
        .load_credential()?
        .ok_or_else(|| anyhow!("login required"))?;
    let session = api.refresh(&stored.refresh_token)?;
    let authenticated = AuthenticatedAccount {
        session,
        account: AccountMetadata {
            account_id: stored.account_id.clone(),
            account_name: stored.account_name.clone(),
            device_name: stored.device_name.clone(),
        },
    };
    persist_login(store, &authenticated)?;
    Ok(authenticated)
}

pub fn device_login_and_persist(
    api: &dyn AccountsApi,
    browser: &dyn Browser,
    waiter: &dyn Waiter,
    ui: &dyn LoginUi,
    store: &dyn CredentialPersistence,
) -> Result<LoginResult> {
    let result = device_login(api, browser, waiter, ui)?;
    persist_login(store, &result.authenticated)?;
    Ok(result)
}

fn poll_until_authorized(
    api: &dyn AccountsApi,
    waiter: &dyn Waiter,
    authorization: &DeviceAuthorization,
    verifier: &str,
) -> Result<(AccessSession, AccountMetadata)> {
    let mut interval = authorization.interval;
    let mut elapsed = 0u64;
    loop {
        if elapsed >= authorization.expires_in {
            bail!("Device Authorization expired before approval");
        }
        if interval > authorization.expires_in.saturating_sub(elapsed) {
            bail!("Device Authorization expired before approval");
        }
        if !waiter.wait(Duration::from_secs(interval)) {
            bail!("Device Authorization was cancelled");
        }
        elapsed = elapsed.saturating_add(interval);
        match api.poll_device_token(authorization.device_code.expose(), verifier)? {
            PollResult::Granted(grant) => return grant.into_session(),
            PollResult::AuthorizationPending => {}
            PollResult::SlowDown => {
                interval = interval.saturating_add(SLOW_DOWN_SECONDS);
            }
            PollResult::AccessDenied => bail!("Device Authorization was denied"),
            PollResult::ExpiredToken => bail!("Device Authorization expired"),
            PollResult::InvalidGrant => bail!("Device Authorization grant is invalid"),
        }
    }
}

fn current_device_name() -> String {
    ["COMPUTERNAME", "HOSTNAME"]
        .into_iter()
        .find_map(|name| env::var(name).ok())
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(default_device_name)
}

fn generate_pkce() -> (String, String) {
    let mut random = [0u8; 32];
    OsRng.fill_bytes(&mut random);
    let verifier = URL_SAFE_NO_PAD.encode(random);
    random.zeroize();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

fn canonical_verification_url(authorization: &DeviceAuthorization) -> Result<Url> {
    let mut base = Url::parse(&authorization.verification_uri)
        .context("Accounts returned an invalid verification URI")?;
    if base.scheme() != "https" && !is_loopback_http(&base) {
        bail!("verification URI must use HTTPS");
    }
    if !base.username().is_empty() || base.password().is_some() || base.fragment().is_some() {
        bail!("verification URI contains forbidden URL components");
    }
    base.set_query(None);
    base.query_pairs_mut()
        .append_pair("code", &authorization.user_code);

    if let Some(complete) = &authorization.verification_uri_complete {
        let supplied = Url::parse(complete)
            .context("Accounts returned an invalid complete verification URI")?;
        if supplied != base {
            bail!("complete verification URI must contain only the public user code");
        }
    }
    Ok(base)
}

fn decode_success<T: DeserializeOwned>(
    response: Response,
    service: &str,
    operation: &str,
) -> Result<T> {
    if response.status().is_success() {
        decode_json(response, service)
    } else {
        let error = decode_error(response, service)?;
        bail!("{operation} failed: {}", human_api_error(&error));
    }
}

fn decode_error(response: Response, service: &str) -> Result<ErrorResponse> {
    let status = response.status();
    let body = read_response(response, service)?;
    parse_error(status, &body, service)
}

fn parse_error(status: StatusCode, body: &[u8], service: &str) -> Result<ErrorResponse> {
    serde_json::from_slice(body).map_err(|_| {
        anyhow!(
            "{service} returned HTTP {status} with a non-JSON error body: {}",
            short_body(body)
        )
    })
}

fn decode_json<T: DeserializeOwned>(response: Response, service: &str) -> Result<T> {
    let status = response.status();
    let body = read_response(response, service)?;
    serde_json::from_slice(&body)
        .with_context(|| format!("{service} returned an invalid JSON response (HTTP {status})"))
}

fn read_response(mut response: Response, service: &str) -> Result<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|size| size > RESPONSE_LIMIT)
    {
        bail!("{service} response is too large");
    }
    let mut body = Vec::new();
    response
        .by_ref()
        .take(RESPONSE_LIMIT + 1)
        .read_to_end(&mut body)
        .with_context(|| format!("failed to read the {service} response"))?;
    if body.len() as u64 > RESPONSE_LIMIT {
        bail!("{service} response is too large");
    }
    Ok(body)
}

fn human_api_error(error: &ErrorResponse) -> &str {
    match error {
        ErrorResponse::OAuth {
            error,
            error_description,
        } => {
            if error_description.is_empty() {
                error
            } else {
                error_description
            }
        }
        ErrorResponse::General { error } => {
            if error.message.is_empty() {
                &error.code
            } else {
                &error.message
            }
        }
    }
}

fn error_code(error: &ErrorResponse) -> &str {
    match error {
        ErrorResponse::OAuth { error, .. } => error,
        ErrorResponse::General { error } => &error.code,
    }
}

fn short_body(body: &[u8]) -> String {
    const LIMIT: usize = 256;
    let end = body.len().min(LIMIT);
    let mut text = String::from_utf8_lossy(&body[..end])
        .replace(['\r', '\n', '\t'], " ")
        .trim()
        .to_string();
    if body.len() > LIMIT {
        text.push_str("...");
    }
    if text.is_empty() {
        "<empty>".to_string()
    } else {
        text
    }
}

fn is_loopback_http(url: &Url) -> bool {
    url.scheme() == "http" && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1"))
}

#[cfg(test)]
mod tests {
    use std::{
        cell::{Cell, RefCell},
        collections::VecDeque,
    };

    use super::*;

    struct MockApi {
        challenge: RefCell<Option<String>>,
        device_name: RefCell<Option<String>>,
        verifier: RefCell<Option<String>>,
        polls: RefCell<VecDeque<PollResult>>,
    }

    #[derive(Default)]
    struct MockStore {
        credential: RefCell<Option<StoredCredential>>,
    }

    impl CredentialPersistence for MockStore {
        fn load_credential(&self) -> Result<Option<StoredCredential>> {
            Ok(self.credential.borrow().clone())
        }

        fn save_credential(&self, credential: &StoredCredential) -> Result<()> {
            *self.credential.borrow_mut() = Some(credential.clone());
            Ok(())
        }

        fn delete_credential(&self) -> Result<()> {
            *self.credential.borrow_mut() = None;
            Ok(())
        }
    }

    impl MockApi {
        fn with_polls(polls: Vec<PollResult>) -> Self {
            Self {
                challenge: RefCell::new(None),
                device_name: RefCell::new(None),
                verifier: RefCell::new(None),
                polls: RefCell::new(polls.into()),
            }
        }
    }

    impl AccountsApi for MockApi {
        fn start_device_authorization(
            &self,
            code_challenge: &str,
            device_name: &str,
        ) -> Result<DeviceAuthorization> {
            *self.challenge.borrow_mut() = Some(code_challenge.to_string());
            *self.device_name.borrow_mut() = Some(device_name.to_string());
            DeviceAuthorization {
                device_code: Secret::default(),
                device_code_wire: "device-secret".to_string(),
                user_code: "ABCD-EFGH".to_string(),
                verification_uri: "https://accounts.mochios.org/device".to_string(),
                verification_uri_complete: Some(
                    "https://accounts.mochios.org/device?code=ABCD-EFGH".to_string(),
                ),
                expires_in: 60,
                interval: 2,
            }
            .normalize()
        }

        fn poll_device_token(&self, device_code: &str, code_verifier: &str) -> Result<PollResult> {
            assert_eq!(device_code, "device-secret");
            *self.verifier.borrow_mut() = Some(code_verifier.to_string());
            self.polls
                .borrow_mut()
                .pop_front()
                .ok_or_else(|| anyhow!("unexpected poll"))
        }

        fn refresh(&self, _refresh_token: &str) -> Result<AccessSession> {
            unreachable!()
        }

        fn revoke(&self, _access_token: &str) -> Result<()> {
            unreachable!()
        }
    }

    struct ClosedBrowser;

    impl Browser for ClosedBrowser {
        fn open(&self, _url: &Url) -> bool {
            false
        }
    }

    struct SilentUi;

    impl LoginUi for SilentUi {
        fn present(&self, _verification_url: &Url, _user_code: &str, _browser_opened: bool) {}

        fn waiting(&self) {}
    }

    #[derive(Default)]
    struct RecordingUi {
        browser_opened: Cell<Option<bool>>,
    }

    impl LoginUi for RecordingUi {
        fn present(&self, _verification_url: &Url, _user_code: &str, browser_opened: bool) {
            self.browser_opened.set(Some(browser_opened));
        }

        fn waiting(&self) {}
    }

    #[derive(Default)]
    struct RecordingWaiter {
        waits: RefCell<Vec<u64>>,
        cancel: bool,
    }

    impl Waiter for RecordingWaiter {
        fn wait(&self, duration: Duration) -> bool {
            self.waits.borrow_mut().push(duration.as_secs());
            !self.cancel
        }
    }

    fn grant() -> PollResult {
        PollResult::Granted(DeviceTokenGrant {
            token_type: "Bearer".to_string(),
            access_token: "access-secret".to_string(),
            expires_in: 600,
            refresh_token: "refresh-secret".to_string(),
            account: AccountMetadata {
                account_id: "account-1".to_string(),
                account_name: "jine".to_string(),
                device_name: default_device_name(),
            },
        })
    }

    #[test]
    fn production_api_paths_match_the_cli_contract() -> Result<()> {
        let api = HttpAccountsApi::new(DEFAULT_ACCOUNTS_API_BASE)?;
        assert_eq!(
            api.endpoint("device/authorize")?.as_str(),
            "https://accounts.mochios.org/v1/cli/device/authorize"
        );
        assert_eq!(
            api.endpoint("session/revoke-current")?.as_str(),
            "https://accounts.mochios.org/v1/cli/session/revoke-current"
        );
        let developers = HttpDeveloperApi::new("https://ca.mochios.org/v1")?;
        assert_eq!(
            developers.endpoint("cli/developers")?.as_str(),
            "https://ca.mochios.org/v1/cli/developers"
        );
        Ok(())
    }

    #[test]
    fn device_token_response_requires_account_metadata() -> Result<()> {
        let grant: DeviceTokenGrant = serde_json::from_str(
            r#"{"token_type":"Bearer","access_token":"access","expires_in":600,"refresh_token":"refresh","account":{"id":"account-1","name":"jine"}}"#,
        )?;
        let (session, account) = grant.into_session()?;
        assert_eq!(session.access_token.expose(), "access");
        assert_eq!(session.refresh_token.expose(), "refresh");
        assert_eq!(account.account_id, "account-1");
        assert_eq!(account.account_name, "jine");
        Ok(())
    }

    #[test]
    fn refresh_token_response_does_not_require_account_metadata() -> Result<()> {
        let grant: RefreshTokenGrant = serde_json::from_str(
            r#"{"token_type":"Bearer","access_token":"access","expires_in":600,"refresh_token":"refresh"}"#,
        )?;
        let session = grant.into_session()?;
        assert_eq!(session.access_token.expose(), "access");
        assert_eq!(session.refresh_token.expose(), "refresh");

        let without_account = r#"{"token_type":"Bearer","access_token":"access","expires_in":600,"refresh_token":"refresh"}"#;
        assert!(serde_json::from_str::<DeviceTokenGrant>(without_account).is_err());
        Ok(())
    }

    #[test]
    fn legacy_token_response_fields_are_rejected() {
        let legacy = r#"{"token_type":"Bearer","access_token":"access","expires_in":600,"refresh_credential":"refresh","session_id":"session","account":{"id":"account-1","name":"jine"}}"#;
        assert!(serde_json::from_str::<DeviceTokenGrant>(legacy).is_err());
    }

    #[test]
    fn accounts_request_bodies_contain_only_supported_fields() -> Result<()> {
        assert_eq!(
            serde_json::to_value(DeviceAuthorizationRequest {
                client_id: CLIENT_ID,
                code_challenge: "challenge",
                code_challenge_method: "S256",
                device_name: "workstation",
            })?,
            serde_json::json!({
                "client_id": "kome-cli",
                "code_challenge": "challenge",
                "code_challenge_method": "S256",
                "device_name": "workstation",
            })
        );
        assert_eq!(
            serde_json::to_value(DeviceTokenRequest {
                device_code: "device-code",
                code_verifier: "verifier",
            })?,
            serde_json::json!({
                "device_code": "device-code",
                "code_verifier": "verifier",
            })
        );
        assert_eq!(
            serde_json::to_value(RefreshRequest {
                refresh_token: "refresh",
            })?,
            serde_json::json!({"refresh_token": "refresh"})
        );
        Ok(())
    }

    #[test]
    fn developer_ca_membership_matches_the_production_shape() -> Result<()> {
        let membership: DeveloperMembership = serde_json::from_str(
            r#"{"id":"019f9e5ac6687902b0e72fe53abfbef1","display_name":"Example","status":"active","verification_status":"verified","role":"owner","can_issue":true}"#,
        )?;
        assert!(membership.is_issuable());
        assert_eq!(membership.role, "owner");
        Ok(())
    }

    #[test]
    fn oauth_and_nested_api_errors_are_both_supported() -> Result<()> {
        let oauth: ErrorResponse = serde_json::from_str(
            r#"{"error":"authorization_pending","error_description":"Authorization is pending"}"#,
        )?;
        assert_eq!(error_code(&oauth), "authorization_pending");
        assert_eq!(human_api_error(&oauth), "Authorization is pending");

        let general: ErrorResponse = serde_json::from_str(
            r#"{"error":{"code":"DEVICE_REQUEST_INVALID","message":"Device authorization request is invalid"}}"#,
        )?;
        assert_eq!(error_code(&general), "DEVICE_REQUEST_INVALID");
        assert_eq!(
            human_api_error(&general),
            "Device authorization request is invalid"
        );
        Ok(())
    }

    #[test]
    fn non_json_error_body_is_short_and_single_line() {
        let body = vec![b'x'; 300];
        assert_eq!(short_body(b"Not Found\n"), "Not Found");
        let shortened = short_body(&body);
        assert_eq!(shortened.len(), 259);
        assert!(shortened.ends_with("..."));
    }

    #[test]
    fn non_json_error_reports_http_status_and_body() {
        let error = parse_error(StatusCode::NOT_FOUND, b"Not Found\n", "Accounts")
            .unwrap_err()
            .to_string();
        assert!(error.contains("404 Not Found"));
        assert!(error.contains("Not Found"));
    }

    #[test]
    fn device_flow_uses_pkce_and_honors_pending_and_slow_down() {
        let api = MockApi::with_polls(vec![
            PollResult::AuthorizationPending,
            PollResult::SlowDown,
            grant(),
        ]);
        let waiter = RecordingWaiter::default();
        let ui = RecordingUi::default();
        let result = device_login(&api, &ClosedBrowser, &waiter, &ui).unwrap();
        assert_eq!(result.authenticated.account.account_name, "jine");
        assert_eq!(
            api.device_name.borrow().as_deref(),
            Some(current_device_name().as_str())
        );
        assert_eq!(ui.browser_opened.get(), Some(false));
        assert_eq!(waiter.waits.borrow().as_slice(), &[2, 2, 7]);

        let verifier = api.verifier.borrow();
        let verifier = verifier.as_ref().unwrap();
        let expected = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        assert_eq!(api.challenge.borrow().as_deref(), Some(expected.as_str()));
    }

    #[test]
    fn verification_url_contains_only_public_code() {
        let authorization = DeviceAuthorization {
            device_code: Secret::default(),
            device_code_wire: "device-secret".to_string(),
            user_code: "ABCD-EFGH".to_string(),
            verification_uri: "https://accounts.mochios.org/device".to_string(),
            verification_uri_complete: Some(
                "https://accounts.mochios.org/device?code=ABCD-EFGH".to_string(),
            ),
            expires_in: 60,
            interval: 2,
        }
        .normalize()
        .unwrap();
        let verification_url = canonical_verification_url(&authorization).unwrap();
        assert_eq!(
            verification_url.as_str(),
            "https://accounts.mochios.org/device?code=ABCD-EFGH"
        );
        assert!(!verification_url.as_str().contains("device-secret"));
        assert!(!verification_url.as_str().contains("access-secret"));
    }

    #[test]
    fn malformed_complete_url_is_rejected() {
        let authorization = DeviceAuthorization {
            device_code: Secret::default(),
            device_code_wire: "device-secret".to_string(),
            user_code: "ABCD-EFGH".to_string(),
            verification_uri: "https://accounts.mochios.org/device".to_string(),
            verification_uri_complete: Some(
                "https://accounts.mochios.org/device?code=ABCD-EFGH&device_code=secret".to_string(),
            ),
            expires_in: 60,
            interval: 2,
        };
        assert!(canonical_verification_url(&authorization).is_err());
    }

    #[test]
    fn cancellation_stops_before_polling() {
        let api = MockApi::with_polls(vec![grant()]);
        let store = MockStore::default();
        let waiter = RecordingWaiter {
            waits: RefCell::new(Vec::new()),
            cancel: true,
        };
        let error = device_login_and_persist(&api, &ClosedBrowser, &waiter, &SilentUi, &store)
            .unwrap_err()
            .to_string();
        assert!(error.contains("cancelled"));
        assert!(api.polls.borrow().len() == 1);
        assert!(store.credential.borrow().is_none());
    }

    #[test]
    fn denial_expiry_and_invalid_grant_are_not_success() {
        for result in [
            PollResult::AccessDenied,
            PollResult::ExpiredToken,
            PollResult::InvalidGrant,
        ] {
            let api = MockApi::with_polls(vec![result]);
            assert!(
                device_login(&api, &ClosedBrowser, &RecordingWaiter::default(), &SilentUi,)
                    .is_err()
            );
        }
    }

    #[test]
    fn developer_membership_requires_all_active_states() {
        let mut membership = DeveloperMembership {
            id: "019f9e5ac6687902b0e72fe53abfbef1".to_string(),
            display_name: "Example".to_string(),
            status: "active".to_string(),
            verification_status: "verified".to_string(),
            role: "owner".to_string(),
            can_issue: true,
        };
        assert!(membership.is_issuable());
        membership.status = "invited".to_string();
        assert!(!membership.is_issuable());
        membership.status = "active".to_string();
        membership.id = "org.mochios.developer.invalid".to_string();
        assert!(!membership.is_issuable());
    }

    #[test]
    fn secret_debug_output_is_redacted() {
        let grant = DeviceTokenGrant {
            token_type: "Bearer".to_string(),
            access_token: "access-secret".to_string(),
            expires_in: 600,
            refresh_token: "refresh-secret".to_string(),
            account: AccountMetadata {
                account_id: "account-1".to_string(),
                account_name: "jine".to_string(),
                device_name: default_device_name(),
            },
        };
        let output = format!("{grant:?}");
        assert!(!output.contains("access-secret"));
        assert!(!output.contains("refresh-secret"));
    }

    #[test]
    fn successful_login_persists_account_from_token_grant() {
        let api = MockApi::with_polls(vec![grant()]);
        let store = MockStore::default();
        device_login_and_persist(
            &api,
            &ClosedBrowser,
            &RecordingWaiter::default(),
            &SilentUi,
            &store,
        )
        .unwrap();
        let stored = store.credential.borrow();
        let stored = stored.as_ref().unwrap();
        assert_eq!(stored.refresh_token, "refresh-secret");
        assert_eq!(stored.account_name, "jine");
    }

    #[test]
    fn refresh_rotation_replaces_stored_credential() {
        struct RefreshApi;

        impl AccountsApi for RefreshApi {
            fn start_device_authorization(
                &self,
                _code_challenge: &str,
                _device_name: &str,
            ) -> Result<DeviceAuthorization> {
                unreachable!()
            }

            fn poll_device_token(
                &self,
                _device_code: &str,
                _code_verifier: &str,
            ) -> Result<PollResult> {
                unreachable!()
            }

            fn refresh(&self, refresh_token: &str) -> Result<AccessSession> {
                assert_eq!(refresh_token, "old-refresh");
                Ok(AccessSession {
                    access_token: Secret::new("new-access".to_string()),
                    refresh_token: Secret::new("new-refresh".to_string()),
                })
            }

            fn revoke(&self, _access_token: &str) -> Result<()> {
                unreachable!()
            }
        }

        let store = MockStore::default();
        *store.credential.borrow_mut() = Some(StoredCredential {
            refresh_token: "old-refresh".to_string(),
            account_id: "account-1".to_string(),
            account_name: "jine".to_string(),
            device_name: "Kome CLI test".to_string(),
        });
        let authenticated = refresh_login(&RefreshApi, &store).unwrap();
        assert_eq!(authenticated.account.account_id, "account-1");
        assert_eq!(authenticated.account.account_name, "jine");
        assert_eq!(authenticated.account.device_name, "Kome CLI test");
        let stored = store.credential.borrow();
        let stored = stored.as_ref().unwrap();
        assert_eq!(stored.refresh_token, "new-refresh");
    }
}
