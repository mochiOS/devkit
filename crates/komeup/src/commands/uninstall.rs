use crate::config::kome_home;
use anyhow::{bail, Context, Result};
use std::fs;

pub fn run(yes: bool) -> Result<()> {
    let home = kome_home()?;

    if !yes {
        bail!(
            "this removes {}. Re-run with `komeup uninstall --yes`.",
            home.display()
        );
    }

    if home.exists() {
        fs::remove_dir_all(&home)
            .with_context(|| format!("failed to remove {}", home.display()))?;
    }

    println!("removed {}", home.display());

    Ok(())
}
