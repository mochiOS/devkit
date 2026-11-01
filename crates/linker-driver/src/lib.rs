//! mochiOS link policy. LLD owns ELF layout; msign owns signing and verification.
mod options;
mod sdk;
mod transaction;

pub use options::Options;
pub use transaction::run;

#[derive(Debug)]
pub struct Error {
    pub message: String,
    pub exit_code: i32,
}

impl Error {
    fn diagnostic(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            exit_code: 1,
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        output.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

pub const HELP: &str = "mochiOS ld: ld.lld with mochiOS runtime and msign ELF signing
usage: ld [OPTIONS] FILE...
  -o FILE                   Output (default: a.out)
  -L PATH, -l NAME, -T FILE  Forward library and script options to ld.lld
  --mochios-sdk PATH         SDK root (otherwise MOCHIOS_SDK or ../ relative to ld)
  --mochios-lld PATH         ld.lld executable (otherwise sibling or MOCHIOS_LLD)
  --mochios-msign PATH       msign executable (otherwise sibling or MOCHIOS_MSIGN)
  --compiler-runtime NAME   libgcc or compiler-rt (otherwise SDK manifest)
  --no-sign                 Publish unsigned output explicitly
  --sign-key FILE           msign private key; requires --public-key
  --public-key FILE         Verify keyed output with msign
  --manifest FILE           Embed manifest.toml into the ELF before signing
  -nostdlib                 Omit startup and default libraries
  -nostartfiles             Omit crt0.o
  -nodefaultlibs            Omit default libraries
  -r                        Relocatable output: no SDK injection or signing
  -v                        Print executed commands
  --dry-run                 Print commands without launching or writing files
  --version                 Print ld.lld version
The initial driver supports x86-64 static ELF and -r. Shared/PIE output and
response files are rejected explicitly. AdHoc signing never grants trust or capabilities.
";
