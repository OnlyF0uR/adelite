//! Signs release files for Adelite's updater.
//!
//! `adelite-sign <file>...` writes `<file>.sig` beside each file: the
//! signature, in hex, of the file under its name (so sign files under their
//! final names). `adelite-sign --public` prints the key's public half.
//!
//! The key is the 32-byte seed, in hex, in `ADELITE_UPDATE_KEY`. The release
//! workflow takes it from the `release` environment's secrets. It must be
//! the key whose public half the app carries (`signature::PUBLIC_KEY`), or
//! nothing is signed: the app would refuse what it signed.

use std::path::PathBuf;
use std::process::ExitCode;

use adelite_update::signature::{self, PUBLIC_KEY};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("adelite-sign: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let seed: [u8; 32] = std::env::var("ADELITE_UPDATE_KEY")
        .ok()
        .and_then(|key| signature::decode(key.trim()))
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or("ADELITE_UPDATE_KEY must hold the signing key: 64 hex digits")?;
    let public = signature::encode(&signature::public_key(&seed).map_err(|e| e.to_string())?);
    let files: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if files.len() == 1 && files[0].as_os_str() == "--public" {
        println!("{public}");
        return Ok(());
    }
    if public != PUBLIC_KEY {
        return Err(format!("this is not the key the app trusts: its public half is {public}, the app's {PUBLIC_KEY}"));
    }
    if files.is_empty() {
        return Err("usage: adelite-sign <file>... (or --public)".to_owned());
    }
    for path in files {
        let name = path.file_name().and_then(|n| n.to_str()).ok_or(format!("{} has no usable name", path.display()))?;
        let contents = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let signed = signature::encode(&signature::sign(&seed, name, &contents).map_err(|e| e.to_string())?);
        // Checked the way the app will check it.
        signature::verify(&signature::app_key(), name, &path, &signed).map_err(|e| format!("{name}: {e}"))?;
        let mut out = path.clone().into_os_string();
        out.push(".sig");
        std::fs::write(&out, format!("{signed}\n")).map_err(|e| format!("{}: {e}", PathBuf::from(&out).display()))?;
        println!("signed {name}");
    }
    Ok(())
}
