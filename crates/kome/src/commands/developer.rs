use anyhow::{bail, Result};
use mochios_certificate::is_valid_developer_id;

use crate::{
    auth::{refresh_login, DeveloperApi, DeveloperMembership, HttpAccountsApi, HttpDeveloperApi},
    cli::{DeveloperListArgs, DeveloperUseArgs},
    commands::account::print_login_required,
    credential::CredentialStore,
    preferences::Preferences,
};

pub fn list(args: DeveloperListArgs) -> Result<()> {
    let store = CredentialStore::system()?;
    if store.load()?.is_none() {
        print_login_required();
        return Ok(());
    }
    let api = HttpAccountsApi::new(&args.accounts_api_base)?;
    let account = refresh_login(&api, &store)?;
    let developers = HttpDeveloperApi::new(&args.developer_ca_api_base)?
        .developers(account.session.access_token.expose())?;
    print_memberships(&developers);
    Ok(())
}

pub fn use_developer(args: DeveloperUseArgs) -> Result<()> {
    if !is_valid_developer_id(&args.developer_id) {
        bail!("Developer ID must be a 32-character lowercase hexadecimal identifier");
    }
    let store = CredentialStore::system()?;
    if store.load()?.is_none() {
        print_login_required();
        return Ok(());
    }
    let api = HttpAccountsApi::new(&args.api.accounts_api_base)?;
    let account = refresh_login(&api, &store)?;
    let developers = HttpDeveloperApi::new(&args.api.developer_ca_api_base)?
        .developers(account.session.access_token.expose())?;
    let membership = developers
        .iter()
        .find(|membership| membership.id == args.developer_id)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "現在のAccountは指定されたDeveloperのMemberではありません。\n\n確認:\n  kome account\n  kome developer list"
            )
        })?;
    ensure_usable_membership(membership)?;

    let mut preferences = Preferences::load()?;
    preferences.default_developer = Some(args.developer_id.clone());
    preferences.save()?;
    println!("Default Developer: {}", args.developer_id);
    Ok(())
}

fn print_memberships(memberships: &[DeveloperMembership]) {
    if memberships.is_empty() {
        println!("利用可能なDeveloperがありません。");
        println!();
        println!("ConsoleでDeveloperを作成してから、もう一度実行してください。");
        return;
    }
    for membership in memberships {
        println!(
            "{}\t{}\tmembership={}\tstatus={}\tcertificate={}",
            membership.id,
            membership.display_name,
            membership.status,
            membership.verification_status,
            if membership.can_issue {
                "available"
            } else {
                "unavailable"
            }
        );
    }
}

fn ensure_usable_membership(membership: &DeveloperMembership) -> Result<()> {
    if membership.status != "active" {
        bail!(
            "現在のAccountは指定されたDeveloperのactive Memberではありません。\n\n確認:\n  kome account\n  kome developer list"
        );
    }
    if membership.verification_status != "verified" || !membership.can_issue {
        bail!("Developerはまだ確認されていません。\nConsoleで状態を確認してください。");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn membership(status: &str, membership: &str, issuable: bool) -> DeveloperMembership {
        DeveloperMembership {
            id: "019f9e5ac6687902b0e72fe53abfbef1".to_string(),
            display_name: "Example".to_string(),
            status: membership.to_string(),
            verification_status: status.to_string(),
            role: "owner".to_string(),
            can_issue: issuable,
        }
    }

    #[test]
    fn only_active_verified_membership_is_usable() {
        assert!(ensure_usable_membership(&membership("verified", "active", true)).is_ok());
        assert!(ensure_usable_membership(&membership("verified", "invited", true)).is_err());
        assert!(ensure_usable_membership(&membership("pending", "active", true)).is_err());
        assert!(ensure_usable_membership(&membership("verified", "active", false)).is_err());
    }
}
