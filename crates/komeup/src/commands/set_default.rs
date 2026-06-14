use crate::config::{kome_home, load_config, save_config};
use crate::commands::install;
use anyhow::{bail, Result};

pub fn run(toolchain: &str) -> Result<()> {
    let home = kome_home()?;
    let mut config = load_config(&home)?;

    if !config.toolchains.contains_key(toolchain) {
        bail!("toolchain '{}' is not installed", toolchain);
    }

    config.default_toolchain = toolchain.to_string();

    save_config(&home, &config)?;
    install::refresh_default_shims(&home, toolchain)?;

    println!("default toolchain set to '{}'", toolchain);

    Ok(())
}