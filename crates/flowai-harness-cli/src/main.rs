use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    match flowai_harness_cli::run(std::env::args_os()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            // CliError::Parse already carries clap's full "error: ..." text.
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}
