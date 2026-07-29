use anyhow::Result;

use crate::{
    auth::{AccountsApi, HttpAccountsApi},
    cli::LogoutArgs,
    credential::{CredentialPersistence, CredentialStore},
};

pub fn run(args: LogoutArgs) -> Result<()> {
    let store = CredentialStore::system()?;
    let Some(stored) = store.load_credential()? else {
        store.delete_credential()?;
        println!("Kome CLI is already logged out.");
        return Ok(());
    };

    let revoke_result = match HttpAccountsApi::new(&args.accounts_api_base) {
        Ok(api) => revoke_and_delete(&api, &store, &stored),
        Err(error) => {
            store.delete_credential()?;
            Err(error)
        }
    };
    if let Err(error) = revoke_result {
        eprintln!("warning: Cloud側のCLI sessionを失効できませんでした: {error:#}");
    }
    println!("Logged out from Kome CLI.");
    Ok(())
}

fn revoke_and_delete(
    api: &dyn AccountsApi,
    store: &dyn CredentialPersistence,
    stored: &crate::credential::StoredCredential,
) -> Result<()> {
    let revoke_result = (|| -> Result<()> {
        let (session, _) = api.refresh(&stored.refresh_token)?;
        api.revoke(session.access_token.expose())
    })();
    store.delete_credential()?;
    revoke_result
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use anyhow::bail;

    use super::*;
    use crate::{
        auth::{AccessSession, AccountMetadata, DeviceAuthorization, PollResult, Secret},
        credential::StoredCredential,
    };

    struct FailingApi;

    impl AccountsApi for FailingApi {
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

        fn refresh(&self, _refresh_token: &str) -> Result<(AccessSession, AccountMetadata)> {
            Ok((
                AccessSession {
                    access_token: Secret::new("access".to_string()),
                    refresh_token: Secret::new("refresh".to_string()),
                },
                AccountMetadata {
                    account_id: "account".to_string(),
                    account_name: "jine".to_string(),
                    device_name: "Kome CLI".to_string(),
                },
            ))
        }

        fn revoke(&self, _access_token: &str) -> Result<()> {
            bail!("offline")
        }
    }

    struct MockStore(RefCell<Option<StoredCredential>>);

    impl CredentialPersistence for MockStore {
        fn load_credential(&self) -> Result<Option<StoredCredential>> {
            Ok(self.0.borrow().clone())
        }

        fn save_credential(&self, credential: &StoredCredential) -> Result<()> {
            *self.0.borrow_mut() = Some(credential.clone());
            Ok(())
        }

        fn delete_credential(&self) -> Result<()> {
            *self.0.borrow_mut() = None;
            Ok(())
        }
    }

    #[test]
    fn cloud_failure_still_deletes_local_credential() {
        let stored = StoredCredential {
            refresh_token: "old-refresh".to_string(),
            account_id: "account".to_string(),
            account_name: "jine".to_string(),
            device_name: "Kome CLI".to_string(),
        };
        let store = MockStore(RefCell::new(Some(stored.clone())));
        assert!(revoke_and_delete(&FailingApi, &store, &stored).is_err());
        assert!(store.0.borrow().is_none());
    }
}
