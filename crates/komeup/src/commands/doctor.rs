use crate::config::{kome_home, load_config};
use anyhow::{bail, Result};
use std::env;
use std::path::{Path, PathBuf};

const BINARIES: &[&str] = &["kome", "komec", "msign", "mpack"];

pub fn run() -> Result<()> {
    let home = kome_home()?;
    let config = load_config(&home)?;

    let mut ok = true;

    println!("checking komeup installation");
    println!("home: {}", home.display());

    if home.exists() {
        println!("OK: home directory exists");
    } else {
        println!("NG: home directory does not exist");
        ok = false;
    }

    let Some(toolchain) = config.toolchains.get(&config.default_toolchain) else {
        println!(
            "NG: default toolchain '{}' is not installed",
            config.default_toolchain
        );
        bail!("doctor failed");
    };

    let toolchain_path = PathBuf::from(&toolchain.path);
    let bin_dir = toolchain_path.join("bin");
    let std_dir = toolchain_path.join("lib").join("std");

    if bin_dir.exists() {
        println!("OK: toolchain bin exists");
    } else {
        println!("NG: missing {}", bin_dir.display());
        ok = false;
    }

    for binary in BINARIES {
        let path = bin_dir.join(binary);

        if path.exists() {
            println!("OK: {} exists", binary);
        } else {
            println!("NG: missing {}", path.display());
            ok = false;
        }
    }

    if std_dir.exists() {
        println!("OK: std exists");
    } else {
        println!("NG: missing {}", std_dir.display());
        ok = false;
    }

    let shim_dir = home.join("bin");

    if path_contains(&shim_dir) {
        println!("OK: {} is in PATH", shim_dir.display());
    } else {
        println!("NG: {} is not in PATH", shim_dir.display());
        println!("add this to your shell profile:");
        println!("  export PATH=\"{}:$PATH\"", shim_dir.display());
        ok = false;
    }

    if ok {
        println!("doctor: ok");
        Ok(())
    } else {
        bail!("doctor failed")
    }
}

fn path_contains(target: &Path) -> bool {
    let Some(paths) = env::var_os("PATH") else {
        return false;
    };

    env::split_paths(&paths).any(|path| path == target)
}
