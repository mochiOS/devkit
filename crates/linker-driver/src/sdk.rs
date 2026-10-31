use crate::{Error, Options};
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

fn required(path: &Path) -> Result<PathBuf, Error> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() && metadata.len() != 0 => Ok(path.to_owned()),
        _ => Err(Error::diagnostic(format!(
            "required SDK file is missing or empty: {}",
            path.display()
        ))),
    }
}

pub(super) fn arguments(options: &Options, executable: &Path) -> Result<Vec<OsString>, Error> {
    if options.relocatable {
        return Ok(options.arguments.clone());
    }
    let root = options
        .sdk
        .clone()
        .or_else(|| std::env::var_os("MOCHIOS_SDK").map(PathBuf::from))
        .or_else(|| {
            options
                .sysroot
                .as_ref()
                .filter(|path| path.file_name() == Some("sysroot".as_ref()))
                .and_then(|path| path.parent().map(Path::to_path_buf))
        })
        .or_else(|| {
            executable
                .parent()
                .and_then(Path::parent)
                .map(Path::to_path_buf)
        })
        .ok_or_else(|| Error::diagnostic("cannot locate SDK; set MOCHIOS_SDK or --mochios-sdk"))?;
    let sysroot = options
        .sysroot
        .clone()
        .unwrap_or_else(|| root.join("sysroot"));
    let mut result = Vec::new();
    let mut root_arg = OsString::from("--sysroot=");
    root_arg.push(&sysroot);
    result.push(root_arg);
    if !options.script {
        result.push("-T".into());
        result.push(required(&root.join("lib/linker.ld"))?.into());
    }
    if options.startup {
        result.push(required(&root.join("lib/crt0.o"))?.into());
    }
    result.extend(options.arguments.iter().cloned());
    // These are the current executable profile, including when a caller's
    // flags precede them. Relocatable links returned above have no profile.
    result.extend(["-static", "-no-pie", "-z", "noexecstack"].map(OsString::from));
    if options.libraries {
        let manifest_path = root.join("manifest.toml");
        let manifest = match fs::read_to_string(&manifest_path) {
            Ok(text) => Some(text.parse::<toml::Table>().map_err(|error| {
                Error::diagnostic(format!("{}: {error}", manifest_path.display()))
            })?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(Error::diagnostic(format!(
                    "{}: {error}",
                    manifest_path.display()
                )));
            }
        };
        if let Some(architecture) = manifest.as_ref().and_then(|m| m.get("architecture")) {
            if architecture.as_str() != Some("x86_64") {
                return Err(Error::diagnostic("SDK architecture must be x86_64"));
            }
        }
        let manifest_runtime = manifest
            .as_ref()
            .and_then(|m| m.get("compiler_runtime"))
            .map(|value| {
                value
                    .as_str()
                    .ok_or_else(|| Error::diagnostic("SDK compiler_runtime must be a string"))
            })
            .transpose()?;
        let runtime = options
            .compiler_runtime
            .as_deref()
            .or(manifest_runtime)
            .unwrap_or("libgcc");
        let runtime = match runtime {
            "libgcc" => "lib/libgcc.a",
            "compiler-rt" => "lib/libclang_rt.builtins-x86_64.a",
            _ => {
                return Err(Error::diagnostic(
                    "compiler runtime must be libgcc or compiler-rt",
                ));
            }
        };
        for path in [root.join("lib"), sysroot.join("lib")] {
            let mut argument = OsString::from("-L");
            argument.push(path);
            result.push(argument);
        }
        result.push("--start-group".into());
        result.push(required(&root.join("lib/libmochi_user_newlib_runtime.a"))?.into());
        result.push(required(&sysroot.join("lib/libc.a"))?.into());
        result.push(required(&sysroot.join("lib/libm.a"))?.into());
        result.push(required(&root.join(runtime))?.into());
        result.push("--end-group".into());
    }
    Ok(result)
}
