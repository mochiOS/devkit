use anyhow::Result;
use url::Url;

use crate::{
    auth::{
        device_login_and_persist, Browser, HttpAccountsApi, InterruptibleWaiter, LoginUi,
        SystemBrowser,
    },
    cli::LoginArgs,
    credential::CredentialStore,
};

struct DisabledBrowser;

impl Browser for DisabledBrowser {
    fn open(&self, _url: &Url) -> bool {
        false
    }
}

struct ConsoleUi;

impl LoginUi for ConsoleUi {
    fn present(&self, verification_url: &Url, user_code: &str, browser_opened: bool) {
        if browser_opened {
            println!("Opening browser for mochiOS Account login...");
        } else {
            println!("Open the following URL to log in to your mochiOS Account.");
        }
        println!();
        println!("URL:");
        println!("{verification_url}");
        println!();
        println!("Code:");
        println!("{user_code}");
        println!();
    }

    fn waiting(&self) {
        println!("Waiting for authorization...");
    }
}

pub fn run(args: LoginArgs) -> Result<()> {
    let api = HttpAccountsApi::new(&args.accounts_api_base)?;
    let waiter = InterruptibleWaiter::install()?;
    let browser: &dyn Browser = if args.no_browser {
        &DisabledBrowser
    } else {
        &SystemBrowser
    };
    let store = CredentialStore::system()?;
    let result = device_login_and_persist(&api, browser, &waiter, &ConsoleUi, &store)?;
    println!(
        "Logged in as {}.",
        result.authenticated.account.account_name
    );
    Ok(())
}
