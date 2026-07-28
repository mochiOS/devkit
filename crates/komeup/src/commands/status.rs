use crate::config::{kome_home, load_config};
use anyhow::Result;

pub fn run() -> Result<()> {
    let home = kome_home()?;
    let config = load_config(&home)?;

    println!("komeup home: {}", config.home);
    println!("default toolchain: {}", config.default_toolchain);

    if config.toolchains.is_empty() {
        println!("no toolchains installed");
        return Ok(());
    }

    for (name, toolchain) in &config.toolchains {
        println!();
        println!("toolchain: {}", name);
        println!("  channel: {}", toolchain.channel);
        println!("  version: {}", toolchain.version);
        println!("  path: {}", toolchain.path);
        println!("  installed_at: {}", toolchain.installed_at);

        if toolchain.components.is_empty() {
            println!("  components: none");
            continue;
        }

        println!("  components:");

        for component in toolchain.components.values() {
            println!("    {} {}", component.name, component.version);
        }
    }

    Ok(())
}
