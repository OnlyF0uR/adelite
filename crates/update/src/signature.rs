//! Every download is signed (Ed25519) by the release workflow, and checked
//! here before anything of it runs. A release that someone else put on
//! GitHub, or a file swapped on the way, is refused: the app holds an
//! account's session, and the update replaces the app.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use ed25519_compact::{KeyPair, PublicKey, Seed, Signature};

use crate::Error;

/// The public half of the key the releases are signed with, in hex. The
/// private half is the secret `ADELITE_UPDATE_KEY` of the repository's
/// `release` environment (and its owner's backup); `adelite-sign` refuses
/// to sign with any other.
pub const PUBLIC_KEY: &str = "5f341d9a9f79ba409134aabfad8336a4a54973ea7092cf71d43798bb41f198a2";

/// What is signed: a label and the file's name, then its bytes. The name
/// carries the version and the system, so a signed file cannot be passed
/// off as another release's.
fn preamble(name: &str) -> Vec<u8> {
    format!("adelite-update\n{name}\n").into_bytes()
}

/// Signs the file `name` holding `contents` with the key made from `seed`.
///
/// # Errors
/// The seed is all zeros, which no key comes from.
pub fn sign(seed: &[u8; 32], name: &str, contents: &[u8]) -> Result<[u8; 64], Error> {
    let pair = KeyPair::try_from_seed(Seed::new(*seed)).map_err(|_| Error::new("that signing key is all zeros"))?;
    let mut message = preamble(name);
    message.extend_from_slice(contents);
    Ok(*pair.sk.sign(&message, None))
}

/// The public key of the key made from `seed`.
///
/// # Errors
/// The seed is all zeros.
pub fn public_key(seed: &[u8; 32]) -> Result<[u8; 32], Error> {
    let pair = KeyPair::try_from_seed(Seed::new(*seed)).map_err(|_| Error::new("that signing key is all zeros"))?;
    Ok(*pair.pk)
}

/// Checks that `file`, published as `name`, is signed by `key` with
/// `signature` (hex, as the `.sig` file holds it). Reads the file in pieces.
///
/// # Errors
/// The signature is malformed or does not match, or the file cannot be read.
pub fn verify(key: &[u8; 32], name: &str, file: &Path, signature: &str) -> Result<(), Error> {
    let refused = || Error::new("The download did not pass its signature check, so it was not installed.");
    let signature =
        decode(signature.trim()).and_then(|bytes| Signature::from_slice(&bytes).ok()).ok_or_else(refused)?;
    let mut state = PublicKey::new(*key).verify_incremental(&signature).map_err(|_| refused())?;
    state.absorb(preamble(name));
    let mut file = File::open(file).map_err(|e| Error::new(format!("The download could not be read: {e}")))?;
    let mut buffer = vec![0; 1 << 16];
    loop {
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => state.absorb(&buffer[..read]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(Error::new(format!("The download could not be read: {e}"))),
        }
    }
    state.verify().map_err(|_| refused())
}

/// The app's own [`PUBLIC_KEY`] as bytes.
#[must_use]
pub fn app_key() -> [u8; 32] {
    decode(PUBLIC_KEY).and_then(|bytes| bytes.try_into().ok()).unwrap_or([0; 32])
}

/// `bytes` as lowercase hex.
#[must_use]
pub fn encode(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    bytes.iter().flat_map(|b| [DIGITS[usize::from(b >> 4)], DIGITS[usize::from(b & 15)]]).map(char::from).collect()
}

/// Hex (either case) as bytes; `None` if it isn't hex.
#[must_use]
pub fn decode(text: &str) -> Option<Vec<u8>> {
    let digit = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let text = text.as_bytes();
    if !text.len().is_multiple_of(2) {
        return None;
    }
    text.chunks(2).map(|pair| Some((digit(pair[0])? << 4) | digit(pair[1])?)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: [u8; 32] = [7; 32];

    fn temp(name: &str, contents: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("adelite-update-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp folder");
        let path = dir.join(name);
        std::fs::write(&path, contents).expect("temp file");
        path
    }

    #[test]
    fn a_signed_file_passes_and_anything_else_is_refused() {
        let contents: Vec<u8> = (0..200_000u32).map(|i| (i * 31 % 251) as u8).collect();
        let name = "Adelite-0.2.0-linux-x64.tar.gz";
        let signature = encode(&sign(&SEED, name, &contents).expect("signs"));
        let key = public_key(&SEED).expect("key");
        let file = temp("signed", &contents);
        assert!(verify(&key, name, &file, &signature).is_ok());
        assert!(verify(&key, name, &file, &format!("{}\n", signature.to_uppercase())).is_ok());
        // Another release's name, another key, a changed byte, a broken signature: all refused.
        assert!(verify(&key, "Adelite-0.1.0-linux-x64.tar.gz", &file, &signature).is_err());
        assert!(verify(&public_key(&[8; 32]).expect("key"), name, &file, &signature).is_err());
        let mut changed = contents.clone();
        changed[100_000] ^= 1;
        assert!(verify(&key, name, &temp("changed", &changed), &signature).is_err());
        assert!(verify(&key, name, &file, &signature[2..]).is_err());
        assert!(verify(&key, name, &file, "not hex at all").is_err());
        assert!(sign(&[0; 32], name, &contents).is_err());
    }

    #[test]
    fn the_app_key_is_a_valid_public_key() {
        let key = app_key();
        assert_ne!(key, [0; 32]);
        assert_eq!(encode(&key), PUBLIC_KEY);
        // A signature from some other key is not taken for one of ours.
        let file = temp("other", b"something");
        let other = encode(&sign(&SEED, "x", b"something").expect("signs"));
        assert!(verify(&key, "x", &file, &other).is_err());
    }

    #[test]
    fn hex_goes_both_ways() {
        assert_eq!(encode(&[0, 1, 0xab, 0xff]), "0001abff");
        assert_eq!(decode("0001ABff"), Some(vec![0, 1, 0xab, 0xff]));
        assert_eq!(decode("abc"), None);
        assert_eq!(decode("zz"), None);
    }
}
