use std::{fs, path::Path};

use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use mochios_mpkg_reviewer::{Expectations, inspect_mpkg};

fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    ensure!(
        args.len() == 7,
        "invalid AppStore Reviewer fixture arguments"
    );
    let package = Path::new(&args[0]);
    let issuer_public_key = read_public_key(Path::new(&args[1]))?;
    let developer_public_key =
        fs::read_to_string(&args[2]).with_context(|| format!("failed to read {}", args[2]))?;
    let expected_file_size = fs::metadata(package)?.len();

    let report = inspect_mpkg(
        package,
        &Expectations {
            package_id: "org.example.application",
            version: "0.1.0",
            certificate_id: "devkit-e2e-certificate",
            certificate_serial: &args[3],
            certificate_subject_key_id: &args[4],
            certificate_developer_id: &args[5],
            certificate_issuer_key_id: &args[6],
            minimum_mochios_version: "0.1.0",
            public_key: developer_public_key.trim(),
            issuer_public_key: &issuer_public_key,
            expected_file_size,
            unix_time: 1_800_000_000,
        },
    )?;
    ensure!(report.package_id == "org.example.application");
    ensure!(report.version == "0.1.0");
    ensure!(report.file_size == expected_file_size);
    println!("AppStore Reviewer accepted {}", package.display());
    Ok(())
}

fn read_public_key(path: &Path) -> Result<[u8; 32]> {
    let text =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    STANDARD
        .decode(text.trim())
        .context("issuer public key is not Base64")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("issuer public key must contain 32 bytes"))
}
