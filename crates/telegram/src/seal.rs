//! Keeping the app's API id and hash out of plain sight in the binary.
//!
//! The build script seals them ([`seal`]) and the app opens them when
//! TDLib asks ([`open`]); this file is compiled into both, so the two
//! cannot drift apart. Sealed, they are a block of bytes that looks like
//! noise, the same size whatever the hash: masked with a keystream grown
//! from two numbers drawn anew for every build.
//!
//! This hides them from a search of the file for text or for anything
//! shaped like a hash. It cannot hide them from someone who runs the app
//! under a debugger: Telegram's library needs them in the clear.

/// Bytes a sealed API id and hash take, whatever the hash's length.
pub const SEALED: usize = 64;
/// Where the hash starts: after the id (four bytes) and the hash's length (one).
const HASH_AT: usize = 5;

/// Masks `bytes`: each is combined (exclusive or) with a keystream grown
/// from `seed` by `SplitMix64`. Doing it twice gives the bytes back.
fn mask(bytes: &mut [u8], seed: u64) {
    let mut state = seed;
    for chunk in bytes.chunks_mut(8) {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut key = state;
        key = (key ^ (key >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        key = (key ^ (key >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        key ^= key >> 31;
        for (byte, key) in chunk.iter_mut().zip(key.to_le_bytes()) {
            *byte ^= key;
        }
    }
}

/// The keystream's seed from the two numbers the build keeps apart.
fn seed(seeds: [u64; 2]) -> u64 {
    seeds[0] ^ seeds[1].rotate_left(23)
}

/// Seals `id` and `hash` with `seeds`. What the hash does not fill is
/// `filler` (random bytes), so the block gives away no length. `None` for
/// an id that is not positive, or a hash that is empty or too long.
pub fn seal(id: i32, hash: &str, seeds: [u64; 2], filler: [u8; SEALED]) -> Option<[u8; SEALED]> {
    let length = u8::try_from(hash.len()).ok().filter(|length| *length > 0)?;
    let mut sealed = filler;
    sealed.get_mut(..4)?.copy_from_slice(&id.to_le_bytes());
    *sealed.get_mut(4)? = length;
    sealed.get_mut(HASH_AT..HASH_AT + hash.len())?.copy_from_slice(hash.as_bytes());
    mask(&mut sealed, seed(seeds));
    (id > 0).then_some(sealed)
}

/// Opens what [`seal`] sealed with the same `seeds`: the id and the hash.
/// `None` when the block is not one of ours.
pub fn open(mut sealed: [u8; SEALED], seeds: [u64; 2]) -> Option<(i32, String)> {
    mask(&mut sealed, seed(seeds));
    let id = i32::from_le_bytes(sealed.first_chunk::<4>().copied()?);
    let length = usize::from(*sealed.get(4)?);
    let hash = sealed.get(HASH_AT..HASH_AT + length).filter(|hash| !hash.is_empty())?;
    let hash = std::str::from_utf8(hash).ok()?;
    (id > 0).then(|| (id, hash.to_owned()))
}

/// The value of `key` in the text of an env file: lines of `KEY=value`,
/// with `#` comments, an optional `export` before the key and optional
/// quotes around the value. `None` when the key is absent or its value empty.
pub fn value_of<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().find_map(|line| {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line);
        let (name, value) = line.split_once('=')?;
        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .or_else(|| value.strip_prefix('\'').and_then(|value| value.strip_suffix('\'')))
            .unwrap_or(value);
        (name.trim() == key && !value.is_empty()).then_some(value)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn what_is_sealed_opens_and_shows_nothing() {
        let seeds = [0x1234_5678_9ABC_DEF0, 0x0FED_CBA9_8765_4321];
        let sealed = seal(1_234_567, HASH, seeds, [0xAA; SEALED]).expect("sealed");
        assert_eq!(open(sealed, seeds), Some((1_234_567, HASH.to_owned())));
        // Nothing of the hash, the id or the filler is left standing.
        assert!(!sealed.windows(4).any(|window| HASH.as_bytes().windows(4).any(|part| part == window)));
        assert!(!sealed.windows(4).any(|window| window == 1_234_567_i32.to_le_bytes() || window == [0xAA; 4]));
        // Other seeds, another block; the wrong ones open nothing that fits.
        let other = seal(1_234_567, HASH, [1, 2], [0xAA; SEALED]).expect("sealed");
        assert_ne!(sealed, other);
        assert_ne!(open(sealed, [1, 2]), Some((1_234_567, HASH.to_owned())));
        // Noise is no reason to panic.
        let _ = (open([0; SEALED], [0, 0]), open([0xFF; SEALED], [u64::MAX; 2]));
    }

    #[test]
    fn only_a_real_id_and_hash_are_sealed() {
        let seeds = [7, 9];
        assert!(seal(0, HASH, seeds, [0; SEALED]).is_none(), "an id is positive");
        assert!(seal(-5, HASH, seeds, [0; SEALED]).is_none());
        assert!(seal(5, "", seeds, [0; SEALED]).is_none(), "a hash is not empty");
        assert!(seal(5, &"a".repeat(SEALED - HASH_AT + 1), seeds, [0; SEALED]).is_none(), "nor longer than its block");
        let longest = "é".repeat((SEALED - HASH_AT) / 2);
        let sealed = seal(5, &longest, seeds, [0; SEALED]).expect("it just fits");
        assert_eq!(open(sealed, seeds), Some((5, longest)));
    }

    #[test]
    fn env_files_are_read_as_they_are_written() {
        let text = "# Telegram\nADELITE_TELEGRAM_API_ID=1234567\nexport ADELITE_TELEGRAM_API_HASH = \"abc def\"\nEMPTY=\nQUOTED='x'\nnot a line\n";
        assert_eq!(value_of(text, "ADELITE_TELEGRAM_API_ID"), Some("1234567"));
        assert_eq!(value_of(text, "ADELITE_TELEGRAM_API_HASH"), Some("abc def"));
        assert_eq!(value_of(text, "QUOTED"), Some("x"));
        assert_eq!(value_of(text, "EMPTY"), None, "an empty value is no value");
        assert_eq!(value_of(text, "MISSING"), None);
        assert_eq!(value_of(text, "# Telegram"), None);
    }
}
