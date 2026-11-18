//! Builds the two files of a release from the release executable and the
//! taskbar helper library, with the layout of `bv_core::package`. Used by
//! `tools/package.ps1`.
//!
//! ```text
//! package <setup|portable> <program> <payload> <output>
//! package verify <file>
//! ```
//!
//! `verify` prints the kind and the lengths read back from a file's trailer, and fails
//! when it is not a release file.

use std::io::Write;
use std::process::ExitCode;

use bv_core::package::{build, Kind, Trailer, TRAILER_LEN};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["verify", file] => verify(file),
        [kind @ ("setup" | "portable"), program, payload, output] => {
            let kind = if *kind == "setup" { Kind::Setup } else { Kind::Portable };
            make(kind, program, payload, output)
        }
        _ => Err("usage: package <setup|portable> <program> <payload> <output> | verify <file>"
            .to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            let _ = writeln!(std::io::stderr(), "{message}");
            ExitCode::FAILURE
        }
    }
}

fn make(kind: Kind, program: &str, payload: &str, output: &str) -> Result<(), String> {
    let program = std::fs::read(program).map_err(|e| format!("{program}: {e}"))?;
    let payload = std::fs::read(payload).map_err(|e| format!("{payload}: {e}"))?;
    if !program.starts_with(b"MZ") || !payload.starts_with(b"MZ") {
        return Err("the program and the payload must be Windows executables".to_owned());
    }
    std::fs::write(output, build(kind, &program, &payload)).map_err(|e| format!("{output}: {e}"))
}

fn verify(file: &str) -> Result<(), String> {
    let bytes = std::fs::read(file).map_err(|e| format!("{file}: {e}"))?;
    let tail = bytes.len().checked_sub(TRAILER_LEN).map(|start| &bytes[start..]).unwrap_or(&[]);
    let trailer = Trailer::parse(tail, bytes.len() as u64)
        .ok_or_else(|| format!("{file}: not a release file"))?;
    let program_len = usize::try_from(trailer.program_len).map_err(|e| e.to_string())?;
    let payload = &bytes[program_len..program_len + trailer.payload_len as usize];
    if !bytes.starts_with(b"MZ") || !payload.starts_with(b"MZ") {
        return Err(format!("{file}: the parts are not Windows executables"));
    }
    let kind = match trailer.kind {
        Kind::Setup => "setup",
        Kind::Portable => "portable",
    };
    writeln!(
        std::io::stdout(),
        "{kind} program={} payload={}",
        trailer.program_len,
        trailer.payload_len
    )
    .map_err(|e| e.to_string())
}
