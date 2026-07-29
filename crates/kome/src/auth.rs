use std::{
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
use reqwest::blocking::{Client, Response};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;
use zeroize::Zeroize;

use crate::credential::{CredentialPersistence, StoredCredential};

pub const DEFAULT_ACCOUNTS_API_BASE: &str = "https://accounts.mochios.org/v1";
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
    #[serde(alias = "id")]
    pub developer_id: String,
    #[serde(default)]
    pub name: String,
    pub membership_status: String,
    pub developer_status: String,
    pub certificate_issuable: bool,
}

impl DeveloperMembership {
    pub fn can_issue(&self) -> bool {
        is_valid_developer_id(&self.developer_id)
            && self.membership_status == "active"
            && self.developer_status == "verified"
            && self.certificate_issuable
    }
}

#[derive(Debug)]
pub struct AccessSession {
    pub access_token: Secret,
    pub refresh_credential: Secret,
    pub session_id: String,
}

#[derive(Debug)]
pub struct AuthenticatedAccount {
    pub session: AccessSession,
    pub account: AccountMetadata,
    pub developers: Vec<DeveloperMembership>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PollResult {
    Granted(TokenGrant),
    AuthorizationPending,
    SlowDown,
    AccessDenied,
    ExpiredToken,
    InvalidGrant,
}

#[derive(Deserialize, PartialEq, Eq)]
pub struct TokenGrant {
    access_token: String,
    refresh_credential: String,
    session_id: String,
}

impl std::fmt::Debug for TokenGrant {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TokenGrant")
            .field("access_token", &"[REDACTED]")
            .field("refresh_credential", &"[REDACTED]")
            .field("session_id", &self.session_id)
            .finish()
    }
}

impl TokenGrant {
    fn into_session(mut self) -> Result<AccessSession> {
        if self.access_token.is_empty()
            || self.refresh_credential.is_empty()
            || self.session_id.is_empty()
        {
            bail!("Accounts returned an incomplete CLI session");
        }
        Ok(AccessSession {
            access_token: Secret::new(std::mem::take(&mut self.access_token)),
            refresh_credential: Secret::new(std::mem::take(&mut self.refresh_credential)),
            session_id: self.session_id,
        })
    }
}

pub trait AccountsApi {
    fn start_device_authorization(&self, code_challenge: &str) -> Result<DeviceAuthorization>;
    fn poll_device_token(&self, device_code: &str, code_verifier: &str) -> Result<PollResult>;
    fn refresh(&self, refresh_credential: &str) -> Result<AccessSession>;
    fn account(&self, access_token: &str) -> Result<AccountMetadata>;
    fn developers(&self, access_token: &str) -> Result<Vec<DeveloperMembership>>;
    fn revoke(&self, access_token: &str, session_id: &str) -> Result<()>;
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

    fn session_revoke_endpoint(&self, session_id: &str) -> Result<Url> {
        let mut endpoint = self.endpoint("sessions")?;
        endpoint
            .path_segments_mut()
            .map_err(|_| anyhow!("Accounts API base URL cannot contain path segments"))?
            .push(session_id)
            .push("revoke");
        Ok(endpoint)
    }
}

#[derive(Serialize)]
struct DeviceAuthorizationRequest<'a> {
    client_id: &'static str,
    code_challenge: &'a str,
    code_challenge_method: &'static str,
}

#[derive(Serialize)]
struct DeviceTokenRequest<'a> {
    client_id: &'static str,
    device_code: &'a str,
    code_verifier: &'a str,
}

#[derive(Serialize)]
struct RefreshRequest<'a> {
    client_id: &'static str,
    refresh_credential: &'a str,
}

#[derive(Debug, Deserialize)]
struct ErrorResponse {
    error: String,
    #[serde(default)]
    error_description: String,
}

impl AccountsApi for HttpAccountsApi {
    fn start_device_authorization(&self, code_challenge: &str) -> Result<DeviceAuthorization> {
        let response = self
            .client
            .post(self.endpoint("device/authorization")?)
            .json(&DeviceAuthorizationRequest {
                client_id: CLIENT_ID,
                code_challenge,
                code_challenge_method: "S256",
            })
            .send()
            .context("Device Authorization request failed")?;
        decode_success::<DeviceAuthorization>(response, "Device Authorization")?.normalize()
    }

    fn poll_device_token(&self, device_code: &str, code_verifier: &str) -> Result<PollResult> {
        let response = self
            .client
            .post(self.endpoint("device/token")?)
            .json(&DeviceTokenRequest {
                client_id: CLIENT_ID,
                device_code,
                code_verifier,
            })
            .send()
            .context("Device Authorization polling failed")?;
        if response.status().is_success() {
            return Ok(PollResult::Granted(decode_json(response)?));
        }
        let error: ErrorResponse = decode_json(response)?;
        match error.error.as_str() {
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

    fn refresh(&self, refresh_credential: &str) -> Result<AccessSession> {
        let response = self
            .client
            .post(self.endpoint("token/refresh")?)
            .json(&RefreshRequest {
                client_id: CLIENT_ID,
                refresh_credential,
            })
            .send()
            .context("failed to refresh the Kome CLI session")?;
        decode_success::<TokenGrant>(response, "CLI session refresh")?.into_session()
    }

    fn account(&self, access_token: &str) -> Result<AccountMetadata> {
        let response = self
            .client
            .get(self.endpoint("account")?)
            .bearer_auth(access_token)
            .send()
            .context("failed to obtain Account information")?;
        decode_success(response, "Account information")
    }

    fn developers(&self, access_token: &str) -> Result<Vec<DeveloperMembership>> {
        #[derive(Deserialize)]
        struct DeveloperList {
            developers: Vec<DeveloperMembership>,
        }

        let response = self
            .client
            .get(self.endpoint("developers")?)
            .bearer_auth(access_token)
            .send()
            .context("failed to obtain the Developer list")?;
        let developers = decode_success::<DeveloperList>(response, "Developer list")?.developers;
        if developers
            .iter()
            .any(|membership| !is_valid_developer_id(&membership.developer_id))
        {
            bail!("Accounts returned an invalid Developer ID");
        }
        Ok(developers)
    }

    fn revoke(&self, access_token: &str, session_id: &str) -> Result<()> {
        let response = self
            .client
            .post(self.session_revoke_endpoint(session_id)?)
            .bearer_auth(access_token)
            .send()
            .context("failed to revoke the Kome CLI session")?;
        if response.status().is_success() {
            Ok(())
        } else {
            let error = decode_error(response)?;
            bail!("CLI session revocation failed: {}", human_api_error(&error));
        }
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
    let authorization = api.start_device_authorization(&challenge)?;
    let verification_url = canonical_verification_url(&authorization)?;
    let browser_opened = browser.open(&verification_url);
    ui.present(&verification_url, &authorization.user_code, browser_opened);
    ui.waiting();
    let session = poll_until_authorized(api, waiter, &authorization, &verifier)?;
    verifier.zeroize();
    let account = api.account(session.access_token.expose())?;
    let developers = api.developers(session.access_token.expose())?;
    Ok(LoginResult {
        authenticated: AuthenticatedAccount {
            session,
            account,
            developers,
        },
    })
}

pub fn persist_login(
    store: &dyn CredentialPersistence,
    account: &AuthenticatedAccount,
) -> Result<()> {
    store.save_credential(&StoredCredential {
        refresh_credential: account.session.refresh_credential.expose().to_string(),
        session_id: account.session.session_id.clone(),
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
    let session = api.refresh(&stored.refresh_credential)?;
    let account = api.account(session.access_token.expose())?;
    let developers = api.developers(session.access_token.expose())?;
    let authenticated = AuthenticatedAccount {
        session,
        account,
        developers,
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
) -> Result<AccessSession> {
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

fn decode_success<T: DeserializeOwned>(response: Response, operation: &str) -> Result<T> {
    if response.status().is_success() {
        decode_json(response)
    } else {
        let error = decode_error(response)?;
        bail!("{operation} failed: {}", human_api_error(&error));
    }
}

fn decode_error(response: Response) -> Result<ErrorResponse> {
    decode_json(response).context("Accounts returned an invalid error response")
}

fn decode_json<T: DeserializeOwned>(mut response: Response) -> Result<T> {
    if response
        .content_length()
        .is_some_and(|size| size > RESPONSE_LIMIT)
    {
        bail!("Accounts response is too large");
    }
    let mut body = Vec::new();
    response
        .by_ref()
        .take(RESPONSE_LIMIT + 1)
        .read_to_end(&mut body)
        .context("failed to read the Accounts response")?;
    if body.len() as u64 > RESPONSE_LIMIT {
        bail!("Accounts response is too large");
    }
    serde_json::from_slice(&body).context("Accounts returned an invalid response")
}

fn human_api_error(error: &ErrorResponse) -> &str {
    if error.error_description.is_empty() {
        &error.error
    } else {
        &error.error_description
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
                verifier: RefCell::new(None),
                polls: RefCell::new(polls.into()),
            }
        }
    }

    impl AccountsApi for MockApi {
        fn start_device_authorization(&self, code_challenge: &str) -> Result<DeviceAuthorization> {
            *self.challenge.borrow_mut() = Some(code_challenge.to_string());
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

        fn refresh(&self, _refresh_credential: &str) -> Result<AccessSession> {
            unreachable!()
        }

        fn account(&self, access_token: &str) -> Result<AccountMetadata> {
            assert_eq!(access_token, "access-secret");
            Ok(AccountMetadata {
                account_id: "account-1".to_string(),
                account_name: "jine".to_string(),
                device_name: "Kome CLI test".to_string(),
            })
        }

        fn developers(&self, access_token: &str) -> Result<Vec<DeveloperMembership>> {
            assert_eq!(access_token, "access-secret");
            Ok(Vec::new())
        }

        fn revoke(&self, _access_token: &str, _session_id: &str) -> Result<()> {
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
        PollResult::Granted(TokenGrant {
            access_token: "access-secret".to_string(),
            refresh_credential: "refresh-secret".to_string(),
            session_id: "session-1".to_string(),
        })
    }

    #[test]
    fn session_revoke_endpoint_has_no_empty_path_segment() -> Result<()> {
        let api = HttpAccountsApi::new("http://127.0.0.1:1234/v1")?;
        assert_eq!(
            api.session_revoke_endpoint("session-1")?.as_str(),
            "http://127.0.0.1:1234/v1/sessions/session-1/revoke"
        );
        Ok(())
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
            developer_id: "019f9e5ac6687902b0e72fe53abfbef1".to_string(),
            name: "Example".to_string(),
            membership_status: "active".to_string(),
            developer_status: "verified".to_string(),
            certificate_issuable: true,
        };
        assert!(membership.can_issue());
        membership.membership_status = "invited".to_string();
        assert!(!membership.can_issue());
        membership.membership_status = "active".to_string();
        membership.developer_id = "org.mochios.developer.invalid".to_string();
        assert!(!membership.can_issue());
    }

    #[test]
    fn secret_debug_output_is_redacted() {
        let grant = TokenGrant {
            access_token: "access-secret".to_string(),
            refresh_credential: "refresh-secret".to_string(),
            session_id: "session-1".to_string(),
        };
        let output = format!("{grant:?}");
        assert!(!output.contains("access-secret"));
        assert!(!output.contains("refresh-secret"));
    }

    #[test]
    fn successful_login_is_persisted_after_account_fetch() {
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
        assert_eq!(stored.refresh_credential, "refresh-secret");
        assert_eq!(stored.account_name, "jine");
    }

    #[test]
    fn refresh_rotation_replaces_stored_credential() {
        struct RefreshApi;

        impl AccountsApi for RefreshApi {
            fn start_device_authorization(
                &self,
                _code_challenge: &str,
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

            fn refresh(&self, refresh_credential: &str) -> Result<AccessSession> {
                assert_eq!(refresh_credential, "old-refresh");
                Ok(AccessSession {
                    access_token: Secret::new("new-access".to_string()),
                    refresh_credential: Secret::new("new-refresh".to_string()),
                    session_id: "new-session".to_string(),
                })
            }

            fn account(&self, access_token: &str) -> Result<AccountMetadata> {
                assert_eq!(access_token, "new-access");
                Ok(AccountMetadata {
                    account_id: "account-1".to_string(),
                    account_name: "jine".to_string(),
                    device_name: "Kome CLI test".to_string(),
                })
            }

            fn developers(&self, access_token: &str) -> Result<Vec<DeveloperMembership>> {
                assert_eq!(access_token, "new-access");
                Ok(Vec::new())
            }

            fn revoke(&self, _access_token: &str, _session_id: &str) -> Result<()> {
                unreachable!()
            }
        }

        let store = MockStore::default();
        *store.credential.borrow_mut() = Some(StoredCredential {
            refresh_credential: "old-refresh".to_string(),
            session_id: "old-session".to_string(),
            account_id: "account-1".to_string(),
            account_name: "jine".to_string(),
            device_name: "Kome CLI test".to_string(),
        });
        refresh_login(&RefreshApi, &store).unwrap();
        let stored = store.credential.borrow();
        let stored = stored.as_ref().unwrap();
        assert_eq!(stored.refresh_credential, "new-refresh");
        assert_eq!(stored.session_id, "new-session");
    }
}
