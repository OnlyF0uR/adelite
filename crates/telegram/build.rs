//! Seals the app's Telegram API id and hash into the build.
//!
//! They come from `ADELITE_TELEGRAM_API_ID` and `ADELITE_TELEGRAM_API_HASH`:
//! in the environment (CI's secrets), or in the `.env` file at the
//! workspace's root, which git ignores. They are written into the crate
//! sealed (`src/seal.rs`), never as text, under seeds drawn for this build.
//! A build given neither still works and asks for a pair when it runs; a
//! build given half a pair, or one that cannot be right, fails here rather
//! than ship without.

use std::fmt::Write as _;
use std::hash::{BuildHasher, Hasher, RandomState};
use std::path::PathBuf;
use std::{env, fs};

#[expect(dead_code, reason = "the build only seals; opening is the app's part")]
#[path = "src/seal.rs"]
mod seal;

const ID: &str = "ADELITE_TELEGRAM_API_ID";
const HASH: &str = "ADELITE_TELEGRAM_API_HASH";

/// A number nobody can guess: std seeds every `RandomState` from the
/// system's randomness.
fn random() -> u64 {
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(0x5EA1);
    hasher.finish()
}

fn main() {
    let env_file =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR")).join("../../.env");
    println!("cargo:rerun-if-changed={}", env_file.display());
    println!("cargo:rerun-if-changed=src/seal.rs");
    println!("cargo:rerun-if-env-changed={ID}");
    println!("cargo:rerun-if-env-changed={HASH}");
    let file = fs::read_to_string(&env_file).unwrap_or_default();
    // The environment wins over the file.
    let given = |key: &str| {
        let set = env::var(key).ok().map(|value| value.trim().to_owned()).filter(|value| !value.is_empty());
        set.or_else(|| seal::value_of(&file, key).map(str::to_owned))
    };

    let mut code = String::from("// Made by build.rs: the API id and hash, sealed, and the seeds they open with.\n");
    match (given(ID), given(HASH)) {
        (None, None) => {
            println!("cargo:warning=built without a Telegram API id ({ID}, {HASH} in .env): the app will ask for one");
            code.push_str("pub const APP: Option<([u8; super::seal::SEALED], [u64; 2])> = None;\n");
        }
        (Some(id), Some(hash)) => {
            let id: i32 = id.parse().unwrap_or_else(|_| panic!("{ID} must be a number, as my.telegram.org shows it"));
            let seeds = [random(), random()];
            let mut filler = [0; seal::SEALED];
            for chunk in filler.chunks_mut(8) {
                chunk.copy_from_slice(&random().to_le_bytes()[..chunk.len()]);
            }
            let sealed = seal::seal(id, &hash, seeds, filler)
                .unwrap_or_else(|| panic!("{ID} must be positive and {HASH} at most 59 bytes"));
            let _ = writeln!(
                code,
                "pub const APP: Option<([u8; super::seal::SEALED], [u64; 2])> = Some(({sealed:?}, {seeds:?}));"
            );
        }
        _ => panic!("set both {ID} and {HASH}, or neither"),
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR")).join("sealed.rs");
    fs::write(out, code).expect("OUT_DIR is writable");
}
