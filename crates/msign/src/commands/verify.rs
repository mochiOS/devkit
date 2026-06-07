use anyhow::{bail, Context, Result};

use crate::{
    cli::VerifyArgs,
    crypto,
    package,
    signature::{PackageSignature, SIGNATURE_ALGORITHM, SIGNATURE_VERSION},
};

pub fn run(args: VerifyArgs) -> Result<()> {
    let signature = package::read_signature(&args.package)
        .with_context(|| format!("failed to read signature from {}", args.package.display()))?;

    if signature.version != SIGNATURE_VERSION {
        bail!("unsupported signature version: {}", signature.version);
    }

    if signature.algorithm != SIGNATURE_ALGORITHM {
        bail!("unsupported signature algorithm: {}", signature.algorithm);
    }

    let actual_hash = package::calculate_package_hash(&args.package)?;

    if actual_hash != signature.package_hash {
        bail!("package hash mismatch");
    }

    let public_key = match args.pubkey {
        Some(path) => crypto::read_public_key(&path)?,
        None => crypto::public_key_from_base64(&signature.public_key)?,
    };

    let message = PackageSignature::signing_message(
        &signature.package_hash,
        &signature.key_id,
        &signature.public_key,
    );

    crypto::verify(&public_key, &message, &signature.signature)?;

    println!("verified: {}", args.package.display());
    println!("key_id: {}", signature.key_id);

    Ok(())
}