//! `segmenta-cli` binary: the same CLI as `segmenta`, under the explicit name.

fn main() -> std::process::ExitCode {
    segmenta_cli::cli::main()
}
