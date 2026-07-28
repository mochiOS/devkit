use anyhow::{bail, Result};

use crate::{cli::KeygenArgs, crypto};

pub fn run(args: KeygenArgs) -> Result<()> {
    let (private_key, public_key) = crypto::generate_keypair();

    crypto::write_private_key(&args.private_key, &private_key)?;
    crypto::write_public_key(&args.public_key, &public_key)?;
    verify_written_keypair(&args)?;

    println!("generated private key: {}", args.private_key.display());
    println!("generated public key: {}", args.public_key.display());

    Ok(())
}

fn verify_written_keypair(args: &KeygenArgs) -> Result<()> {
    let private_key = crypto::read_private_key(&args.private_key)?;
    let public_key = crypto::read_public_key(&args.public_key)?;
    if private_key.verifying_key().to_bytes() != public_key.to_bytes() {
        bail!("generated public key does not match private key");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keygen_writes_matching_keypair_and_refuses_overwrite() {
        let temporary = tempfile::tempdir().unwrap();
        let private_key = temporary.path().join("application.key");
        let public_key = temporary.path().join("application.pub");

        run(KeygenArgs {
            private_key: private_key.clone(),
            public_key: public_key.clone(),
        })
        .unwrap();

        let private = crypto::read_private_key(&private_key).unwrap();
        let public = crypto::read_public_key(&public_key).unwrap();
        assert_eq!(private.verifying_key().to_bytes(), public.to_bytes());
        assert!(run(KeygenArgs {
            private_key,
            public_key,
        })
        .is_err());
    }
}
