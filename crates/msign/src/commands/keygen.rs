use anyhow::Result;

use crate::{cli::KeygenArgs, crypto};

pub fn run(args: KeygenArgs) -> Result<()> {
    let (private_key, public_key) = crypto::generate_keypair();

    crypto::write_private_key(&args.private_key, &private_key)?;
    crypto::write_public_key(&args.public_key, &public_key)?;

    println!("generated private key: {}", args.private_key.display());
    println!("generated public key: {}", args.public_key.display());

    Ok(())
}