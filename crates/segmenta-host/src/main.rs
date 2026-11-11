//! `segmenta-host`: Native Messaging host for the Segmenta browser extension.
//!
//! Thin shell around [`segmenta_host::serve`]; the framing and the proxy live in the
//! library so the tests can drive them without a browser.

use std::io::{self, BufWriter};
use std::process::ExitCode;

use segmenta_host::{serve, socket_path, HostOptions, SystemctlActivator};

fn main() -> ExitCode {
    let options = HostOptions::new(socket_path(), Box::new(SystemctlActivator));
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut input = stdin.lock();
    let mut output = BufWriter::new(stdout.lock());
    match serve(&mut input, &mut output, &options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("segmenta-host: {err}");
            ExitCode::FAILURE
        }
    }
}
