//! Serde wire encoding for the `Config::scan_dirs` list.
//!
//! TOML strings are Unicode, so the default `PathBuf` encoding fails on any
//! path holding non-UTF-8 bytes (`path contains invalid UTF-8 characters`)
//! and the whole save is rejected. Entries are therefore encoded one by one:
//!
//! * **plain string** — every UTF-8 path, byte-for-byte the pre-existing
//!   format, so config files written by earlier releases keep loading;
//! * **inline table** `{ path_hex = "<hex>" }` — the raw path bytes as
//!   lowercase hex, so an arbitrary `OsStr` survives a save/load round trip.
//!
//! The table form is an additive extension of the original format: a table
//! entry can never be mistaken for a string, so decoding is unambiguous, and
//! future revisions may only add marker keys to the table, never rewrite the
//! string form.

use std::path::{Path, PathBuf};

use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::ser::SerializeSeq;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Marker key carrying the hex-encoded bytes of a non-UTF-8 path.
const PATH_HEX_KEY: &str = "path_hex";

/// Serialize `Config::scan_dirs` (see the [module docs](self)).
pub(super) fn serialize<S>(dirs: &[PathBuf], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let mut seq = serializer.serialize_seq(Some(dirs.len()))?;
    for dir in dirs {
        seq.serialize_element(&WireDirRef(dir))?;
    }
    seq.end()
}

/// Deserialize `Config::scan_dirs` (see the [module docs](self)).
///
/// Accepts the legacy plain-string entries, the `{ path_hex = .. }` marker
/// form, or any mix of the two.
pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<Vec<PathBuf>, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_seq(ScanDirsVisitor)
}

/// One borrowed `scan_dirs` entry on the way out: plain string when the path
/// is UTF-8, marker table otherwise.
struct WireDirRef<'a>(&'a Path);

impl Serialize for WireDirRef<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self.0.to_str() {
            Some(utf8) => serializer.serialize_str(utf8),
            None => encode_non_utf8(self.0, serializer),
        }
    }
}

/// One owned `scan_dirs` entry read back from either wire form.
struct WireDir(PathBuf);

impl<'de> Deserialize<'de> for WireDir {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(WireDirVisitor)
    }
}

/// Write the `{ path_hex = .. }` marker form for a path that is not UTF-8.
#[cfg(unix)]
fn encode_non_utf8<S>(path: &Path, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    use serde::ser::SerializeMap;
    let mut map = serializer.serialize_map(Some(1))?;
    map.serialize_entry(PATH_HEX_KEY, &hex_encode(&path_bytes(path)))?;
    map.end()
}

/// Other platforms build paths from UTF-8, so a non-UTF-8 path there is only
/// a lone WTF-8 surrogate: refuse to save it instead of mangling it.
#[cfg(not(unix))]
fn encode_non_utf8<S>(path: &Path, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    Err(serde::ser::Error::custom(format!(
        "cannot store path {} in the config",
        path.display()
    )))
}

/// Reads one entry from either wire form (plain string or marker table).
struct WireDirVisitor;

impl<'de> Visitor<'de> for WireDirVisitor {
    type Value = WireDir;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a path string or an inline table with a `path_hex` key")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(WireDir(PathBuf::from(value)))
    }

    fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        let mut hex: Option<String> = None;
        while let Some(key) = map.next_key::<String>()? {
            if key != PATH_HEX_KEY {
                return Err(de::Error::custom(format!(
                    "unknown scan_dirs entry key: {key}"
                )));
            }
            hex = Some(map.next_value()?);
        }
        let Some(hex) = hex else {
            return Err(de::Error::custom(format!("missing `{PATH_HEX_KEY}` key")));
        };
        let bytes = hex_decode(&hex)
            .ok_or_else(|| de::Error::custom(format!("`{PATH_HEX_KEY}` is not valid hex")))?;
        let path = path_from_bytes(bytes).map_err(de::Error::custom)?;
        Ok(WireDir(path))
    }
}

/// Reads the whole `scan_dirs` array.
struct ScanDirsVisitor;

impl<'de> Visitor<'de> for ScanDirsVisitor {
    type Value = Vec<PathBuf>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a list of scan directories")
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut dirs = Vec::new();
        while let Some(WireDir(dir)) = seq.next_element::<WireDir>()? {
            dirs.push(dir);
        }
        Ok(dirs)
    }
}

/// The raw bytes of `path`, as stored in the `path_hex` marker.
#[cfg(unix)]
fn path_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt as _;
    path.as_os_str().as_bytes().to_vec()
}

/// Rebuild a path from the bytes written by [`path_bytes`].
///
/// Unix keeps arbitrary bytes as-is; other platforms only form paths from
/// UTF-8, so non-UTF-8 bytes there can only come from a hand-edited file and
/// are rejected rather than silently mangled.
#[cfg(unix)]
fn path_from_bytes(bytes: Vec<u8>) -> Result<PathBuf, &'static str> {
    use std::os::unix::ffi::OsStringExt as _;
    Ok(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
}

/// See the Unix variant: decode hex bytes as UTF-8 on platforms whose paths
/// are UTF-8.
#[cfg(not(unix))]
fn path_from_bytes(bytes: Vec<u8>) -> Result<PathBuf, &'static str> {
    String::from_utf8(bytes)
        .map(PathBuf::from)
        .map_err(|_| "`path_hex` bytes are not valid UTF-8 on this platform")
}

/// Encode `bytes` as lowercase hex, two digits per byte.
fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(out, "{byte:02x}").expect("writing to a String never fails");
    }
    out
}

/// Decode hex written by [`hex_encode`]; `None` on an odd length or on any
/// non-hex digit, so a malformed marker is rejected instead of guessed at.
fn hex_decode(text: &str) -> Option<Vec<u8>> {
    let raw = text.as_bytes();
    if !raw.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(raw.len() / 2);
    for pair in raw.chunks(2) {
        out.push((hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?);
    }
    Some(out)
}

/// Value of a single hex digit, `None` when `byte` is not one.
fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    /// A path whose bytes are not valid UTF-8 (Unix only).
    #[cfg(unix)]
    fn non_utf8_path() -> PathBuf {
        use std::os::unix::ffi::OsStrExt as _;
        PathBuf::from(std::ffi::OsStr::from_bytes(b"/home/user/\xff\xfe_not-utf8"))
    }

    #[test]
    fn utf8_paths_serialize_as_plain_strings() {
        let config = Config {
            scan_dirs: vec![PathBuf::from("/home/user/repos")],
        };
        let text = toml::to_string_pretty(&config).expect("serialize");
        assert!(
            text.contains("/home/user/repos"),
            "UTF-8 path must stay a plain string: {text}"
        );
        assert!(
            !text.contains(PATH_HEX_KEY),
            "UTF-8 path must not take the marker form: {text}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_paths_serialize_as_marker_table() {
        let config = Config {
            scan_dirs: vec![non_utf8_path()],
        };
        let text = toml::to_string_pretty(&config).expect("serialize");
        assert!(
            text.contains(PATH_HEX_KEY),
            "non-UTF-8 path must take the marker form: {text}"
        );
        assert!(
            !text.contains('\u{fffd}'),
            "path bytes must not be rendered lossily: {text}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn mixed_list_round_trips_through_the_module() {
        let config = Config {
            scan_dirs: vec![
                PathBuf::from("/home/user/repos"),
                non_utf8_path(),
                PathBuf::from("/opt/projects"),
            ],
        };
        let text = toml::to_string_pretty(&config).expect("serialize");
        let back: Config = toml::from_str(&text).expect("deserialize");
        assert_eq!(back, config);
    }

    #[cfg(unix)]
    #[test]
    fn marker_bytes_rebuild_the_same_path() {
        let path = non_utf8_path();
        let encoded = hex_encode(&path_bytes(&path));
        let bytes = hex_decode(&encoded).expect("decodable");
        assert_eq!(path_from_bytes(bytes).expect("unix keeps bytes"), path);
    }

    #[test]
    fn hex_round_trips_arbitrary_bytes() {
        for bytes in [vec![], vec![0u8], b"/tmp/\xff\xfe".to_vec(), vec![0xff; 33]] {
            let encoded = hex_encode(&bytes);
            assert_eq!(hex_decode(&encoded), Some(bytes), "{encoded}");
        }
    }

    #[test]
    fn hex_decode_rejects_malformed_input() {
        assert_eq!(hex_decode("abc"), None, "odd length");
        assert_eq!(hex_decode("zz"), None, "non-hex digit");
        assert_eq!(hex_decode("2f 1"), None, "embedded space");
    }

    #[test]
    fn rejects_unknown_marker_keys() {
        let err = toml::from_str::<Config>(r#"scan_dirs = [{ other = "ff" }]"#)
            .expect_err("unknown key must not load");
        assert!(
            err.to_string().contains("unknown scan_dirs entry key"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_missing_or_malformed_marker() {
        let missing = toml::from_str::<Config>("scan_dirs = [{}]")
            .expect_err("missing path_hex must not load");
        assert!(
            missing.to_string().contains("missing `path_hex`"),
            "{missing}"
        );

        let bad = toml::from_str::<Config>(r#"scan_dirs = [{ path_hex = "zz" }]"#)
            .expect_err("non-hex path_hex must not load");
        assert!(bad.to_string().contains("not valid hex"), "{bad}");
    }

    #[test]
    fn rejects_entries_that_are_neither_string_nor_marker() {
        let err = toml::from_str::<Config>("scan_dirs = [42]").expect_err("integer is not a path");
        assert!(
            err.to_string().contains("a path string"),
            "unexpected error: {err}"
        );
    }
}
