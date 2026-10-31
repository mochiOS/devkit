use crate::Error;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::{
    ffi::{OsStr, OsString},
    path::PathBuf,
};

#[derive(Debug)]
pub struct Options {
    pub output: PathBuf,
    pub arguments: Vec<OsString>,
    pub sdk: Option<PathBuf>,
    pub sysroot: Option<PathBuf>,
    pub lld: Option<PathBuf>,
    pub msign: Option<PathBuf>,
    pub compiler_runtime: Option<String>,
    pub key: Option<PathBuf>,
    pub public_key: Option<PathBuf>,
    pub sign: bool,
    pub relocatable: bool,
    pub startup: bool,
    pub libraries: bool,
    pub script: bool,
    pub verbose: bool,
    pub dry_run: bool,
    pub version: bool,
    pub help: bool,
}

impl Options {
    pub fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Self, Error> {
        let mut result = Self {
            output: "a.out".into(),
            arguments: Vec::new(),
            sdk: None,
            sysroot: None,
            lld: None,
            msign: None,
            compiler_runtime: None,
            key: None,
            public_key: None,
            sign: true,
            relocatable: false,
            startup: true,
            libraries: true,
            script: false,
            verbose: false,
            dry_run: false,
            version: false,
            help: false,
        };
        let arguments: Vec<_> = arguments.into_iter().collect();
        if arguments
            .iter()
            .any(|argument| argument.as_bytes().starts_with(b"@"))
        {
            return Err(Error::diagnostic(
                "response files are not supported by mochiOS ld yet",
            ));
        }
        let mut arguments = arguments.into_iter();
        let mut literal = false;
        while let Some(argument) = arguments.next() {
            let bytes = argument.as_bytes();
            // LLD expands these before parsing options: forwarding one could
            // replace our private -o and write an unverified public output.
            if bytes.starts_with(b"@") {
                return Err(Error::diagnostic(
                    "response files are not supported by mochiOS ld yet",
                ));
            }
            if literal {
                let mut path = OsString::from("./");
                path.push(&argument);
                result.arguments.push(if bytes.starts_with(b"-") {
                    path
                } else {
                    argument
                });
                continue;
            }
            if bytes == b"--" {
                literal = true;
                continue;
            }
            let (name, joined) = bytes
                .iter()
                .position(|byte| *byte == b'=')
                .map_or((bytes, None), |index| {
                    (&bytes[..index], Some(&bytes[index + 1..]))
                });
            let mut value = || -> Result<OsString, Error> {
                let value = joined
                    .map(|bytes| OsString::from_vec(bytes.to_vec()))
                    .or_else(|| arguments.next())
                    .ok_or_else(|| {
                        Error::diagnostic(format!(
                            "{} requires a value",
                            argument.to_string_lossy()
                        ))
                    })?;
                if value.is_empty() {
                    return Err(Error::diagnostic("empty option value"));
                }
                Ok(value)
            };
            match name {
                b"--mochios-sdk" => result.sdk = Some(value()?.into()),
                b"--mochios-lld" => result.lld = Some(value()?.into()),
                b"--mochios-msign" => result.msign = Some(value()?.into()),
                b"--sysroot" => result.sysroot = Some(value()?.into()),
                b"--compiler-runtime" => {
                    result.compiler_runtime = Some(value()?.into_string().map_err(|_| {
                        Error::diagnostic("compiler runtime must be libgcc or compiler-rt")
                    })?)
                }
                b"--sign-key" => result.key = Some(value()?.into()),
                b"--public-key" => result.public_key = Some(value()?.into()),
                b"--output" | b"-o" if joined.is_some() || bytes == name => {
                    result.output = value()?.into()
                }
                _ if bytes.starts_with(b"-o") && bytes.len() > 2 => {
                    result.output = OsString::from_vec(bytes[2..].to_vec()).into();
                }
                b"--no-sign" if joined.is_none() => result.sign = false,
                b"-r" | b"--relocatable" if joined.is_none() => {
                    result.relocatable = true;
                    result.arguments.push(argument);
                }
                b"-nostdlib" if joined.is_none() => {
                    result.startup = false;
                    result.libraries = false;
                }
                b"-nostartfiles" if joined.is_none() => result.startup = false,
                b"-nodefaultlibs" if joined.is_none() => result.libraries = false,
                b"-v" | b"--verbose" if joined.is_none() => result.verbose = true,
                b"--dry-run" if joined.is_none() => result.dry_run = true,
                b"--version" if joined.is_none() => result.version = true,
                b"--help" if joined.is_none() => result.help = true,
                b"-shared" | b"--shared" | b"-pie" | b"--pie" => {
                    return Err(Error::diagnostic(
                        "shared/PIE output is not supported by mochiOS ld yet",
                    ));
                }
                b"-T" | b"--script" => {
                    result.script = true;
                    result.arguments.push("-T".into());
                    result.arguments.push(value()?);
                }
                _ if bytes.starts_with(b"-T")
                    && bytes.len() > 2
                    && !bytes.starts_with(b"-Ttext")
                    && !bytes.starts_with(b"-Tdata")
                    && !bytes.starts_with(b"-Tbss") =>
                {
                    result.script = true;
                    result.arguments.push(argument);
                }
                b"-L" | b"-l" | b"-z" | b"-e" | b"--entry" | b"-m" if joined.is_none() => {
                    result.arguments.push(argument.clone());
                    result.arguments.push(value()?);
                }
                _ => result.arguments.push(argument),
            }
        }
        if result.output.as_os_str() == OsStr::new("-") || result.output.as_os_str().is_empty() {
            return Err(Error::diagnostic("output must be a filesystem path"));
        }
        if result.key.is_some() != result.public_key.is_some() {
            return Err(Error::diagnostic(
                "--sign-key and --public-key must be specified together",
            ));
        }
        if (!result.sign || result.relocatable) && result.key.is_some() {
            return Err(Error::diagnostic(
                "keyed signing conflicts with --no-sign or -r",
            ));
        }
        Ok(result)
    }
}
