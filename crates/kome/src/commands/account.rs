use anyhow::{Context, Result};

use crate::{
    auth::{refresh_login, HttpAccountsApi},
    cli::AccountArgs,
    credential::CredentialStore,
    preferences::Preferences,
};

pub fn run(args: AccountArgs) -> Result<()> {
    let store = CredentialStore::system()?;
    if store.load()?.is_none() {
        print_login_required();
        return Ok(());
    }
    let api = HttpAccountsApi::new(&args.accounts_api_base)?;
    let authenticated = refresh_login(&api, &store).map_err(|error| {
        anyhow::anyhow!(
            "ログイン状態の有効期限が切れているか、確認できませんでした。\n\n再ログイン:\n  kome login\n\n原因: {error:#}"
        )
    })?;
    let preferences = Preferences::load().context("failed to load Kome settings")?;

    println!("Account: {}", authenticated.account.account_name);
    println!("Account ID: {}", authenticated.account.account_id);
    println!("Session: active");
    println!("Device: {}", authenticated.account.device_name);
    println!(
        "Default Developer: {}",
        preferences
            .default_developer
            .as_deref()
            .unwrap_or("not selected")
    );
    Ok(())
}

pub fn print_login_required() {
    println!("Developer Certificateを取得するにはログインが必要です。");
    println!();
    println!("実行:");
    println!("  kome login");
}
