use anyhow::{Context, Result};

use crate::{
    cli::SignArgs,
    crypto,
    package,
    signature::{PackageSignature, SIGNATURE_ALGORITHM, SIGNATURE_VERSION},
};

pub fn run(args: SignArgs) -> Result<()> {
    let signing_key = crypto::read_private_key(&args.key)?;
    let public_key = signing_key.verifying_key();
    let public_key_b64 = crypto::public_key_to_base64(&public_key);

    let package_hash = package::calculate_package_hash(&args.package)?;

    let message =
        PackageSignature::signing_message(&package_hash, &args.key_id, &public_key_b64);

    let signature_b64 = crypto::sign(&signing_key, &message);

    let signature = PackageSignature {
        version: SIGNATURE_VERSION,
        algorithm: SIGNATURE_ALGORITHM.to_string(),
        key_id: args.key_id,
        public_key: public_key_b64,
        package_hash,
        signature: signature_b64,
    };

    let output = args.output.unwrap_or_else(|| args.package.clone());

    package::write_signature(&args.package, &output, &signature)
        .with_context(|| format!("failed to sign package: {}", args.package.display()))?;

    println!("signed: {}", output.display());

    Ok(())
}