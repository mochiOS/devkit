fn main() -> std::process::ExitCode {
    let result = mochios_linker_driver::Options::parse(std::env::args_os().skip(1))
        .and_then(mochios_linker_driver::run);
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ld: {error}");
            std::process::ExitCode::from(error.exit_code.clamp(1, 255) as u8)
        }
    }
}
