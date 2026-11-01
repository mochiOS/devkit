use crate::{Error, Options, sdk};
use mochios_file_publication::PrivateDirectory;
use std::os::unix::{
    fs::{MetadataExt, PermissionsExt},
    process::ExitStatusExt,
};
use std::{
    ffi::OsString,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
};

fn tool(explicit: &Option<PathBuf>, variable: &str, name: &str, executable: &Path) -> PathBuf {
    let path = explicit
        .clone()
        .or_else(|| std::env::var_os(variable).map(PathBuf::from))
        .or_else(|| {
            executable
                .parent()
                .map(|parent| parent.join(name))
                .filter(|path| path.is_file())
        })
        .unwrap_or_else(|| name.into());
    if path.components().count() == 1 && !path.is_absolute() {
        if let Some(search) = std::env::var_os("PATH") {
            for directory in std::env::split_paths(&search) {
                let candidate = directory.join(&path);
                if fs::metadata(&candidate).is_ok_and(|metadata| {
                    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
                }) {
                    return candidate;
                }
            }
        }
    }
    path
}

fn execute(program: &Path, arguments: &[OsString], options: &Options) -> Result<(), Error> {
    let own = std::env::current_exe().and_then(fs::metadata);
    if let (Ok(own), Ok(tool)) = (own, fs::metadata(program)) {
        if own.dev() == tool.dev() && own.ino() == tool.ino() {
            return Err(Error::diagnostic(
                "child tool resolves to this linker driver",
            ));
        }
    }
    if options.verbose || options.dry_run {
        eprintln!("{program:?} {arguments:?}");
    }
    if options.dry_run {
        return Ok(());
    }
    let status = Command::new(program)
        .args(arguments)
        .status()
        .map_err(|error| {
            Error::diagnostic(format!("cannot execute {}: {error}", program.display()))
        })?;
    if !status.success() {
        return Err(child_failure(program, status));
    }
    Ok(())
}

fn child_failure(program: &Path, status: ExitStatus) -> Error {
    Error {
        message: format!("{} failed: {status}", program.display()),
        exit_code: status
            .code()
            .unwrap_or_else(|| 128 + status.signal().unwrap_or(1)),
    }
}

fn check_output(options: &Options) -> Result<(), Error> {
    match fs::symlink_metadata(&options.output) {
        Ok(metadata) => {
            if !metadata.is_file() {
                return Err(Error::diagnostic(
                    "output must be a regular file, not a symlink or directory",
                ));
            }
            for input in [&options.key, &options.public_key, &options.manifest].into_iter().flatten() {
                let key_metadata = fs::metadata(input)
                    .map_err(|error| Error::diagnostic(format!("{}: {error}", input.display())))?;
                if metadata.dev() == key_metadata.dev() && metadata.ino() == key_metadata.ino() {
                    return Err(Error::diagnostic(
                        "output must not replace a signing key, verification key or manifest",
                    ));
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(Error::diagnostic(format!(
                "{}: {error}",
                options.output.display()
            )));
        }
    }
    Ok(())
}

fn check_elf(path: &Path, relocatable: bool) -> Result<(), Error> {
    let mut header = [0; 64];
    fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut header))
        .map_err(|error| Error::diagnostic(format!("read linked ELF: {error}")))?;
    if &header[..4] != b"\x7fELF"
        || header[4..7] != [2, 1, 1]
        || u16::from_le_bytes([header[16], header[17]]) != if relocatable { 1 } else { 2 }
        || u16::from_le_bytes([header[18], header[19]]) != 62
    {
        return Err(Error::diagnostic(
            "ld.lld did not produce the expected x86-64 ELF type",
        ));
    }
    Ok(())
}

pub fn run(options: Options) -> Result<(), Error> {
    if options.help {
        print!("{}", crate::HELP);
        return Ok(());
    }
    let executable =
        std::env::current_exe().map_err(|error| Error::diagnostic(error.to_string()))?;
    let lld = tool(&options.lld, "MOCHIOS_LLD", "ld.lld", &executable);
    if options.version {
        return execute(&lld, &["--version".into()], &options);
    }
    if options.arguments.is_empty() {
        return Err(Error::diagnostic("no input files"));
    }
    check_output(&options)?;
    let mut arguments = sdk::arguments(&options, &executable)?;
    let parent = options
        .output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let temporary = if options.dry_run {
        None
    } else {
        Some(
            PrivateDirectory::new_in(parent, "mochios-ld")
                .map_err(|error| Error::diagnostic(format!("create link output: {error}")))?,
        )
    };
    let linked = temporary
        .as_ref()
        .map(|temp| temp.path().join("linked.elf"))
        .unwrap_or_else(|| parent.join(".mochios-ld-DRY-RUN/linked.elf"));
    arguments.push("-o".into());
    arguments.push(linked.clone().into());
    execute(&lld, &arguments, &options)?;
    if !options.dry_run {
        check_elf(&linked, options.relocatable)?;
    }
    if options.sign && !options.relocatable {
        let msign = tool(&options.msign, "MOCHIOS_MSIGN", "msign", &executable);
        let mut sign = vec![OsString::from("elf"), "sign".into(), linked.clone().into()];
        if let Some(key) = &options.key {
            sign.push("--key".into());
            sign.push(key.clone().into());
        } else {
            sign.push("--adhoc".into());
        }
        if let Some(manifest) = &options.manifest {
            sign.push("--manifest".into());
            sign.push(manifest.clone().into());
        }
        execute(&msign, &sign, &options)?;
        let mut verify = vec![
            OsString::from("elf"),
            "verify".into(),
            linked.clone().into(),
        ];
        if let Some(key) = &options.public_key {
            verify.push("--public-key".into());
            verify.push(key.clone().into());
        }
        execute(&msign, &verify, &options)?;
    }
    if !options.dry_run {
        check_output(&options)?;
        let file = fs::File::open(&linked)
            .map_err(|error| Error::diagnostic(format!("open final output: {error}")))?;
        temporary
            .as_ref()
            .expect("live link directory")
            .publish("linked.elf", &file, &options.output)
            .map_err(|error| {
                Error::diagnostic(format!("publish {}: {error}", options.output.display()))
            })?;
    }
    Ok(())
}
