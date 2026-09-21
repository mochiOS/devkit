use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use ed25519_dalek::VerifyingKey;
use sha2::{Digest, Sha256};

use crate::{
    cli::{ElfSignArgs, ElfVerifyArgs},
    crypto,
    elf_signature::{self, PublicKeyResolver, SignatureKind},
};

struct SinglePublicKey {
    key: VerifyingKey,
    key_id: [u8; 32],
}

impl SinglePublicKey {
    fn new(key: VerifyingKey) -> Self {
        let key_id = Sha256::digest(key.to_bytes()).into();
        Self { key, key_id }
    }
}

impl PublicKeyResolver for SinglePublicKey {
    fn resolve(&self, key_id: &[u8; 32]) -> Result<Option<VerifyingKey>> {
        Ok((self.key_id == *key_id).then_some(self.key))
    }
}

pub fn sign(args: ElfSignArgs) -> Result<()> {
    let output = args.output.as_deref().unwrap_or(&args.elf);
    if let Some(key_path) = args.key.as_deref() {
        reject_same_file(output, key_path)?;
    }
    let input = read_regular_file(&args.elf)?;
    let signing_key = if args.adhoc {
        None
    } else {
        let key_path = args
            .key
            .as_deref()
            .context("either --adhoc or --key is required")?;
        Some(crypto::read_private_key(key_path)?)
    };
    let signed = match &signing_key {
        None => elf_signature::sign_adhoc(&input)?,
        Some(key) => elf_signature::sign_ed25519(&input, key)?,
    };

    // Verify the exact bytes that will be persisted. This catches accidental
    // encoder/serializer regressions before replacing the caller's file.
    if let Some(key) = &signing_key {
        let resolver = SinglePublicKey::new(key.verifying_key());
        elf_signature::verify(&signed, Some(&resolver))?;
    } else {
        elf_signature::verify(&signed, None)?;
    }

    atomic_write(output, &signed, &args.elf)?;
    println!("signed ELF: {}", output.display());
    Ok(())
}

pub fn verify(args: ElfVerifyArgs) -> Result<()> {
    let bytes = read_regular_file(&args.elf)?;
    let decoded = elf_signature::decode(&bytes)?;
    if decoded.kind == SignatureKind::AdHoc && args.public_key.is_some() {
        bail!("--public-key was provided, but the ELF contains an AdHoc signature");
    }
    let public_key = args
        .public_key
        .as_deref()
        .map(crypto::read_public_key)
        .transpose()?
        .map(SinglePublicKey::new);
    let verified = elf_signature::verify(
        &bytes,
        public_key
            .as_ref()
            .map(|resolver| resolver as &dyn PublicKeyResolver),
    )?;

    match verified.kind {
        SignatureKind::AdHoc => {
            println!("valid AdHoc ELF signature");
        }
        SignatureKind::Ed25519 => {
            let key_id = verified
                .key_id
                .context("verified keyed signature is missing key_id")?;
            println!(
                "valid Ed25519 ELF signature (key_id={})",
                encode_hex(&key_id)
            );
        }
    }
    println!("digest={}", encode_hex(&verified.digest));
    Ok(())
}

fn read_regular_file(path: &Path) -> Result<Vec<u8>> {
    let path_metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect ELF: {}", path.display()))?;
    if path_metadata.file_type().is_symlink() {
        bail!(
            "refusing to operate on an ELF through a symbolic link: {}",
            path.display()
        );
    }
    if !path_metadata.is_file() {
        bail!("ELF input is not a regular file: {}", path.display());
    }
    if path_metadata.len() > elf_signature::MAX_ELF_LEN as u64 {
        bail!("ELF exceeds the v1 size limit");
    }
    let mut file =
        fs::File::open(path).with_context(|| format!("failed to open ELF: {}", path.display()))?;
    let opened_metadata = file
        .metadata()
        .with_context(|| format!("failed to inspect opened ELF: {}", path.display()))?;
    if !opened_metadata.is_file() {
        bail!("opened ELF is not a regular file: {}", path.display());
    }
    if opened_metadata.len() > elf_signature::MAX_ELF_LEN as u64 {
        bail!("ELF exceeds the v1 size limit");
    }
    #[cfg(unix)]
    if !metadata_identifies_same_file(&path_metadata, &opened_metadata) {
        bail!(
            "ELF input changed while it was being opened: {}",
            path.display()
        );
    }

    let expected_len =
        usize::try_from(opened_metadata.len()).context("ELF size does not fit usize")?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(expected_len)
        .context("unable to allocate memory for ELF")?;
    Read::by_ref(&mut file)
        .take(opened_metadata.len())
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to read ELF: {}", path.display()))?;
    let mut extra = [0u8; 1];
    if bytes.len() != expected_len
        || file
            .read(&mut extra)
            .context("failed to finish reading ELF")?
            != 0
    {
        bail!(
            "ELF input size changed while it was being read: {}",
            path.display()
        );
    }
    let final_metadata = file
        .metadata()
        .with_context(|| format!("failed to re-inspect opened ELF: {}", path.display()))?;
    if final_metadata.len() != opened_metadata.len()
        || final_metadata.modified().ok() != opened_metadata.modified().ok()
    {
        bail!(
            "ELF input changed while it was being read: {}",
            path.display()
        );
    }
    Ok(bytes)
}

fn reject_same_file(left: &Path, right: &Path) -> Result<()> {
    if left == right {
        bail!("ELF output must not replace the private key");
    }
    let (Ok(left_metadata), Ok(right_metadata)) = (fs::metadata(left), fs::metadata(right)) else {
        return Ok(());
    };
    if metadata_identifies_same_file(&left_metadata, &right_metadata) {
        bail!("ELF output must not replace the private key");
    }
    Ok(())
}

#[cfg(unix)]
fn metadata_identifies_same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(not(unix))]
fn metadata_identifies_same_file(_left: &fs::Metadata, _right: &fs::Metadata) -> bool {
    false
}

fn atomic_write(path: &Path, bytes: &[u8], mode_source: &Path) -> Result<()> {
    let parent = normalized_parent(path);
    let parent_metadata = fs::metadata(&parent)
        .with_context(|| format!("failed to inspect output directory: {}", parent.display()))?;
    if !parent_metadata.is_dir() {
        bail!("ELF output parent is not a directory: {}", parent.display());
    }
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() {
            bail!(
                "refusing to replace an ELF through a symbolic link: {}",
                path.display()
            );
        }
        if !metadata.is_file() {
            bail!("ELF output is not a regular file: {}", path.display());
        }
    }

    let source_metadata = fs::metadata(mode_source).with_context(|| {
        format!(
            "failed to inspect input permissions: {}",
            mode_source.display()
        )
    })?;
    let mut temporary = tempfile::NamedTempFile::new_in(&parent)
        .with_context(|| format!("failed to create temporary ELF in {}", parent.display()))?;
    temporary
        .as_file_mut()
        .set_permissions(source_metadata.permissions())
        .context("failed to preserve ELF permissions")?;
    temporary
        .write_all(bytes)
        .context("failed to write signed ELF")?;
    temporary
        .as_file_mut()
        .sync_all()
        .context("failed to sync signed ELF")?;
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to atomically replace ELF: {}", path.display()))?;
    sync_directory(&parent)?;
    Ok(())
}

fn normalized_parent(path: &Path) -> PathBuf {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<()> {
    fs::File::open(path)
        .with_context(|| format!("failed to open output directory: {}", path.display()))?
        .sync_all()
        .with_context(|| format!("failed to sync output directory: {}", path.display()))
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<()> {
    Ok(())
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn elf_fixture() -> Vec<u8> {
        let shstr = b"\0.shstrtab\0.text\0";
        let mut elf = vec![0u8; 0x100 + 3 * 64];
        elf[..4].copy_from_slice(b"\x7fELF");
        elf[4] = 2;
        elf[5] = 1;
        elf[6] = 1;
        put_u16(&mut elf, 16, 2);
        put_u16(&mut elf, 18, 62);
        put_u32(&mut elf, 20, 1);
        put_u64(&mut elf, 24, 0x80);
        put_u64(&mut elf, 40, 0x100);
        put_u16(&mut elf, 52, 64);
        put_u16(&mut elf, 58, 64);
        put_u16(&mut elf, 60, 3);
        put_u16(&mut elf, 62, 1);
        elf[0x80..0x84].copy_from_slice(&[0x90, 0x90, 0xc3, 0]);
        elf[0x90..0x90 + shstr.len()].copy_from_slice(shstr);
        put_u32(&mut elf, 0x100 + 64, 1);
        put_u32(&mut elf, 0x100 + 68, 3);
        put_u64(&mut elf, 0x100 + 64 + 24, 0x90);
        put_u64(&mut elf, 0x100 + 64 + 32, shstr.len() as u64);
        put_u64(&mut elf, 0x100 + 64 + 48, 1);
        put_u32(&mut elf, 0x100 + 128, 11);
        put_u32(&mut elf, 0x100 + 132, 1);
        put_u64(&mut elf, 0x100 + 128 + 24, 0x80);
        put_u64(&mut elf, 0x100 + 128 + 32, 4);
        put_u64(&mut elf, 0x100 + 128 + 48, 16);
        elf
    }

    #[test]
    fn commands_sign_and_verify_adhoc_and_keyed_elfs() {
        let temporary = tempfile::tempdir().unwrap();
        let input = temporary.path().join("input.elf");
        let adhoc = temporary.path().join("adhoc.elf");
        let keyed = temporary.path().join("keyed.elf");
        let private_key = temporary.path().join("developer.key");
        let public_key = temporary.path().join("developer.pub");
        fs::write(&input, elf_fixture()).unwrap();

        sign(ElfSignArgs {
            elf: input.clone(),
            adhoc: true,
            key: None,
            output: Some(adhoc.clone()),
        })
        .unwrap();
        verify(ElfVerifyArgs {
            elf: adhoc,
            public_key: None,
        })
        .unwrap();

        let (signing_key, verifying_key) = crypto::generate_keypair();
        crypto::write_private_key(&private_key, &signing_key).unwrap();
        crypto::write_public_key(&public_key, &verifying_key).unwrap();
        sign(ElfSignArgs {
            elf: input,
            adhoc: false,
            key: Some(private_key),
            output: Some(keyed.clone()),
        })
        .unwrap();
        assert!(verify(ElfVerifyArgs {
            elf: keyed.clone(),
            public_key: None,
        })
        .is_err());
        verify(ElfVerifyArgs {
            elf: keyed,
            public_key: Some(public_key),
        })
        .unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn command_rejects_symlink_input_and_private_key_output_alias() {
        use std::os::unix::fs::symlink;

        let temporary = tempfile::tempdir().unwrap();
        let input = temporary.path().join("input.elf");
        let input_link = temporary.path().join("input-link.elf");
        let private_key = temporary.path().join("developer.key");
        fs::write(&input, elf_fixture()).unwrap();
        symlink(&input, &input_link).unwrap();
        assert!(sign(ElfSignArgs {
            elf: input_link,
            adhoc: true,
            key: None,
            output: None,
        })
        .is_err());

        let (signing_key, _) = crypto::generate_keypair();
        crypto::write_private_key(&private_key, &signing_key).unwrap();
        let original_key = fs::read(&private_key).unwrap();
        assert!(sign(ElfSignArgs {
            elf: input,
            adhoc: false,
            key: Some(private_key.clone()),
            output: Some(private_key.clone()),
        })
        .is_err());
        assert_eq!(fs::read(private_key).unwrap(), original_key);
    }
}
