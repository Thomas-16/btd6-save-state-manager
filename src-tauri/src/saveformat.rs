//! Parsing and encoding of the BTD6 save container (see SPEC.md section 2.5).
//!
//! Both `Profile.Save` and `MapSaveData` share one container. The profile payload is
//! AES-128-CBC encrypted (PBKDF2-HMAC-SHA1 over the password "11") around a zlib stream;
//! `MapSaveData` is a bare zlib stream. [`encode_profile`] rebuilds an encrypted profile from
//! JSON for the selective restore; everything else is read-only.

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use flate2::read::ZlibDecoder;
use flate2::{write::ZlibEncoder, Compression};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use std::io::{Read, Write};

type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;
type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;

/// Ticks between 0001-01-01 and 1970-01-01 in .NET DateTime units (100 ns).
const DOTNET_EPOCH_TICKS: u64 = 621_355_968_000_000_000;

/// Length of the PBKDF2 salt that precedes the ciphertext.
pub const SALT_LEN: usize = 24;
/// The game's `passwordIndex`; selects the password "11".
const PASSWORD_INDEX: u64 = 2;
/// zlib level the game uses (header bytes `78 5E`).
const ZLIB_LEVEL: u32 = 2;

#[derive(Debug)]
pub enum SaveError {
    TooShort,
    BadVersion(u32),
    BadHeader,
    Decrypt,
    Inflate(std::io::Error),
    Utf8,
    Json(serde_json::Error),
    Random(String),
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SaveError::TooShort => write!(f, "file is too short to be a BTD6 save"),
            SaveError::BadVersion(v) => write!(f, "unexpected container version {v}"),
            SaveError::BadHeader => write!(f, "malformed save header"),
            SaveError::Decrypt => write!(f, "decryption failed (bad padding)"),
            SaveError::Inflate(e) => write!(f, "zlib inflate failed: {e}"),
            SaveError::Utf8 => write!(f, "payload is not valid UTF-8"),
            SaveError::Json(e) => write!(f, "payload is not valid JSON: {e}"),
            SaveError::Random(e) => write!(f, "random salt unavailable: {e}"),
        }
    }
}

impl std::error::Error for SaveError {}

/// The parsed fixed header plus a slice pointing at the payload.
#[derive(Debug)]
pub struct Container<'a> {
    pub version: u32,
    pub header_len: u32,
    pub save_count: u32,
    pub guid: [u8; 16],
    /// .NET DateTime binary as stored.
    pub timestamp_raw: u64,
    pub header_string: String,
    /// Everything before the payload, verbatim, so an encoder can reuse it.
    pub header_bytes: &'a [u8],
    pub payload: &'a [u8],
}

impl Container<'_> {
    /// Unix milliseconds derived from the .NET DateTime binary, if it is in range.
    pub fn timestamp_unix_ms(&self) -> Option<i64> {
        let ticks = self.timestamp_raw & 0x3FFF_FFFF_FFFF_FFFF; // strip the two Kind bits
        if ticks < DOTNET_EPOCH_TICKS {
            return None;
        }
        Some(((ticks - DOTNET_EPOCH_TICKS) / 10_000) as i64)
    }
}

fn read_u32(b: &[u8], off: usize) -> Result<u32, SaveError> {
    b.get(off..off + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or(SaveError::TooShort)
}

fn read_u64(b: &[u8], off: usize) -> Result<u64, SaveError> {
    b.get(off..off + 8)
        .map(|s| u64::from_le_bytes(s.try_into().unwrap()))
        .ok_or(SaveError::TooShort)
}

/// Reads a .NET BinaryWriter 7-bit length-prefixed string at `off`.
fn read_prefixed_string(b: &[u8], off: usize) -> Result<(String, usize), SaveError> {
    let mut len: usize = 0;
    let mut shift = 0;
    let mut pos = off;
    loop {
        let byte = *b.get(pos).ok_or(SaveError::TooShort)?;
        pos += 1;
        len |= ((byte & 0x7F) as usize) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift > 28 {
            return Err(SaveError::BadHeader);
        }
    }
    let s = b.get(pos..pos + len).ok_or(SaveError::TooShort)?;
    Ok((String::from_utf8_lossy(s).into_owned(), pos + len))
}

/// Parses the shared container header. Does not touch the payload.
pub fn parse_container(bytes: &[u8]) -> Result<Container<'_>, SaveError> {
    let version = read_u32(bytes, 0)?;
    if version != 2 {
        return Err(SaveError::BadVersion(version));
    }
    let header_len = read_u32(bytes, 4)?;
    let save_count = read_u32(bytes, 8)?;
    let guid: [u8; 16] = bytes
        .get(0x0C..0x1C)
        .ok_or(SaveError::TooShort)?
        .try_into()
        .unwrap();
    let timestamp_raw = read_u64(bytes, 0x24)?;
    let (header_string, string_end) = read_prefixed_string(bytes, 0x2C)?;
    let payload_start = 8 + header_len as usize;
    if payload_start != string_end {
        return Err(SaveError::BadHeader);
    }
    let payload = bytes.get(payload_start..).ok_or(SaveError::TooShort)?;
    Ok(Container {
        version,
        header_len,
        save_count,
        guid,
        timestamp_raw,
        header_string,
        header_bytes: &bytes[..payload_start],
        payload,
    })
}

fn inflate_to_json(zlib_bytes: &[u8]) -> Result<String, SaveError> {
    let mut out = Vec::new();
    ZlibDecoder::new(zlib_bytes)
        .read_to_end(&mut out)
        .map_err(SaveError::Inflate)?;
    let s = String::from_utf8(out).map_err(|_| SaveError::Utf8)?;
    Ok(s.strip_prefix('\u{FEFF}').map(str::to_owned).unwrap_or(s))
}

/// Derives (iv, key) exactly as the game does: PBKDF2-HMAC-SHA1("11", salt, 10, 32).
fn derive_iv_key(salt: &[u8]) -> ([u8; 16], [u8; 16]) {
    let mut kiv = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(b"11", salt, 10, &mut kiv);
    let mut iv = [0u8; 16];
    let mut key = [0u8; 16];
    iv.copy_from_slice(&kiv[..16]);
    key.copy_from_slice(&kiv[16..]);
    (iv, key)
}

/// Decrypts an encrypted payload (`u64 passwordIndex`, 24-byte salt, AES-CBC body) to zlib bytes.
fn decrypt_payload(payload: &[u8]) -> Result<Vec<u8>, SaveError> {
    let _password_index = read_u64(payload, 0)?;
    let salt = payload.get(8..32).ok_or(SaveError::TooShort)?;
    let body = payload.get(32..).ok_or(SaveError::TooShort)?;
    let (iv, key) = derive_iv_key(salt);
    Aes128CbcDec::new(&key.into(), &iv.into())
        .decrypt_padded_vec_mut::<Pkcs7>(body)
        .map_err(|_| SaveError::Decrypt)
}

/// Returns the JSON text of either file kind. Encrypted payloads are detected by the
/// absence of the zlib magic (`78 xx`) at the payload start.
pub fn decode_json(bytes: &[u8]) -> Result<String, SaveError> {
    let c = parse_container(bytes)?;
    if c.payload.first() == Some(&0x78) {
        inflate_to_json(c.payload)
    } else {
        let zlib = decrypt_payload(c.payload)?;
        inflate_to_json(&zlib)
    }
}

/// `decode_json` parsed into a `serde_json::Value` (number text preserved verbatim).
pub fn decode_value(bytes: &[u8]) -> Result<Value, SaveError> {
    serde_json::from_str(&decode_json(bytes)?).map_err(SaveError::Json)
}

// ---- Encoding --------------------------------------------------------------------------

/// BOM + JSON text -> zlib stream at the game's compression level.
pub fn deflate_json(json: &str) -> Vec<u8> {
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::new(ZLIB_LEVEL));
    enc.write_all("\u{FEFF}".as_bytes()).expect("in-memory write");
    enc.write_all(json.as_bytes()).expect("in-memory write");
    enc.finish().expect("in-memory finish")
}

/// `u64 passwordIndex` + salt + AES-128-CBC/PKCS7 of the zlib bytes.
pub fn encrypt_payload(zlib: &[u8], salt: &[u8; SALT_LEN]) -> Vec<u8> {
    let (iv, key) = derive_iv_key(salt);
    let body = Aes128CbcEnc::new(&key.into(), &iv.into()).encrypt_padded_vec_mut::<Pkcs7>(zlib);
    let mut payload = Vec::with_capacity(8 + SALT_LEN + body.len());
    payload.extend_from_slice(&PASSWORD_INDEX.to_le_bytes());
    payload.extend_from_slice(salt);
    payload.extend_from_slice(&body);
    payload
}

pub fn random_salt() -> Result<[u8; SALT_LEN], SaveError> {
    let mut salt = [0u8; SALT_LEN];
    getrandom::fill(&mut salt).map_err(|e| SaveError::Random(e.to_string()))?;
    Ok(salt)
}

/// Builds a complete encrypted profile file from a container header (copied verbatim from
/// the live file) and JSON text.
pub fn encode_profile_with_salt(header: &[u8], json: &str, salt: &[u8; SALT_LEN]) -> Vec<u8> {
    let payload = encrypt_payload(&deflate_json(json), salt);
    let mut out = Vec::with_capacity(header.len() + payload.len());
    out.extend_from_slice(header);
    out.extend_from_slice(&payload);
    out
}

pub fn encode_profile(header: &[u8], json: &str) -> Result<Vec<u8>, SaveError> {
    Ok(encode_profile_with_salt(header, json, &random_salt()?))
}

/// Auto-info about one entry in `savedMaps` (or a standalone `MapSaveData`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SavedMapInfo {
    pub key: String,
    pub map: String,
    pub mode: String,
    #[serde(default)]
    pub difficulty: String,
    pub game_type: i64,
    pub round: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boss: Option<String>,
    #[serde(default)]
    pub elite: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub challenge_id: Option<String>,
    #[serde(default)]
    pub game_id: i64,
    #[serde(default)]
    pub attempt_id: i64,
    #[serde(default)]
    pub match_won: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_version: Option<String>,
}

/// Auto-info extracted from a decrypted profile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProfileInfo {
    pub game_version: Option<String>,
    pub rank: Option<i64>,
    pub monkey_money: Option<i64>,
    pub saved_maps: Vec<SavedMapInfo>,
    /// Key of the most recently played `savedMaps` entry (highest gameId, then attemptId).
    pub last_played_key: Option<String>,
    /// Human summary of the last played entry, e.g. `Lych Elite T1 · Frozen Over · R59`.
    pub summary: Option<String>,
}

fn as_i64(v: &Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_f64().map(|f| f as i64))
        .or_else(|| v.as_str()?.parse().ok())
}

fn as_str(v: &Value) -> Option<&str> {
    v.as_str().filter(|s| !s.is_empty())
}

/// Leading alphabetic run of a challenge id: `Lych61_mtw1v6de` -> `Lych`.
pub fn boss_name_from_challenge_id(id: &str) -> Option<String> {
    let name: String = id.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    (!name.is_empty()).then_some(name)
}

/// Builds a [`SavedMapInfo`] from one game-state object (a `savedMaps` value or MapSaveData).
pub fn saved_map_info(key: &str, v: &Value) -> SavedMapInfo {
    let meta = v.get("metaData").and_then(Value::as_object);
    let challenge_id = v.get("playerChallengeId").and_then(as_str).map(str::to_owned);
    let is_boss =
        challenge_id.is_some() && meta.map_or(false, |m| m.contains_key("BossEventIsElite"));
    let boss = if is_boss {
        challenge_id.as_deref().and_then(boss_name_from_challenge_id)
    } else {
        None
    };
    let elite = is_boss
        && meta
            .and_then(|m| m.get("BossEventIsElite"))
            .and_then(Value::as_str)
            .map_or(false, |s| s.eq_ignore_ascii_case("true"));
    let tier = if is_boss {
        meta.and_then(|m| m.get("BossEventTier")).and_then(as_i64)
    } else {
        None
    };
    SavedMapInfo {
        key: key.to_owned(),
        map: v.get("mapName").and_then(as_str).unwrap_or("?").to_owned(),
        mode: v.get("modeName").and_then(as_str).unwrap_or("?").to_owned(),
        difficulty: v.get("mapDifficulty").and_then(as_str).unwrap_or("").to_owned(),
        game_type: v.get("gameType").and_then(as_i64).unwrap_or(0),
        round: v.get("round").and_then(as_i64).unwrap_or(0),
        boss,
        elite,
        tier,
        challenge_id,
        game_id: v.get("gameId").and_then(as_i64).unwrap_or(0),
        attempt_id: v.get("attemptId").and_then(as_i64).unwrap_or(0),
        match_won: v.get("matchWon").and_then(Value::as_bool).unwrap_or(false),
        game_version: v.get("gameVersion").and_then(as_str).map(str::to_owned),
    }
}

/// `FrozenOver` -> `Frozen Over`, `LotusIsland` -> `Lotus Island`, `X` -> `X`.
pub fn humanize_map_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    let chars: Vec<char> = name.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if i > 0 && c.is_uppercase() {
            let prev = chars[i - 1];
            let next_lower = chars.get(i + 1).map_or(false, |n| n.is_lowercase());
            if prev.is_lowercase() || prev.is_ascii_digit() || (prev.is_uppercase() && next_lower)
            {
                out.push(' ');
            }
        }
        out.push(c);
    }
    out
}

/// [`summarize`] without the round: `Lych Elite T1 · Frozen Over` / `Cubism · Easy Standard`.
pub fn describe(m: &SavedMapInfo) -> String {
    let map = humanize_map_name(&m.map);
    if let Some(boss) = &m.boss {
        let mut head = boss.clone();
        if m.elite {
            head.push_str(" Elite");
        }
        if let Some(t) = m.tier {
            head.push_str(&format!(" T{t}"));
        }
        return format!("{head} · {map}");
    }
    let kind = if m.game_type == 3 {
        "Odyssey".to_owned()
    } else if m.mode.eq_ignore_ascii_case("Standard") && !m.difficulty.is_empty() {
        format!("{} {}", m.difficulty, m.mode)
    } else {
        m.mode.clone()
    };
    format!("{map} · {kind}")
}

/// `Lych Elite T1 · Frozen Over · R59` / `Lotus Island · Odyssey · R20` / `Cubism · Easy Standard · R297`.
pub fn summarize(m: &SavedMapInfo) -> String {
    format!("{} · R{}", describe(m), m.round)
}

/// Picks the most recently played entry: highest `gameId`, then highest `attemptId`.
pub fn most_recent(maps: &[SavedMapInfo]) -> Option<&SavedMapInfo> {
    maps.iter().max_by_key(|m| (m.game_id, m.attempt_id))
}

/// Extracts auto-info from a decrypted profile JSON document.
pub fn extract_profile_info(json: &str) -> Result<ProfileInfo, SaveError> {
    let root: Value = serde_json::from_str(json).map_err(SaveError::Json)?;
    let saved_maps: Vec<SavedMapInfo> = root
        .get("savedMaps")
        .and_then(Value::as_object)
        .map(|o| o.iter().map(|(k, v)| saved_map_info(k, v)).collect())
        .unwrap_or_default();
    let last = most_recent(&saved_maps);
    Ok(ProfileInfo {
        game_version: root
            .get("savedByGameVersion")
            .and_then(as_str)
            .map(str::to_owned),
        rank: root.get("rank").and_then(as_i64),
        monkey_money: root.get("monkeyMoney").and_then(as_i64),
        last_played_key: last.map(|m| m.key.clone()),
        summary: last.map(summarize),
        saved_maps,
    })
}

/// Full pipeline for a profile file: decode + extract.
pub fn read_profile_info(bytes: &[u8]) -> Result<ProfileInfo, SaveError> {
    extract_profile_info(&decode_json(bytes)?)
}

/// True when a MapSaveData file holds a real game state (not the literal `null`).
pub fn map_save_has_game(bytes: &[u8]) -> Result<bool, SaveError> {
    let json = decode_json(bytes)?;
    Ok(json.trim() != "null")
}

/// Builders for synthetic save files, shared by the unit tests of several modules.
#[cfg(test)]
pub(crate) mod testutil {
    use super::*;

    fn write_prefixed_string(out: &mut Vec<u8>, s: &str) {
        let mut len = s.len();
        loop {
            let byte = (len & 0x7F) as u8;
            len >>= 7;
            if len == 0 {
                out.push(byte);
                break;
            }
            out.push(byte | 0x80);
        }
        out.extend_from_slice(s.as_bytes());
    }

    pub(crate) fn container(header_string: &str, payload: &[u8]) -> Vec<u8> {
        let mut header_tail = Vec::new();
        header_tail.extend_from_slice(&7u32.to_le_bytes()); // saveCount
        header_tail.extend_from_slice(&[0xAB; 16]); // guid
        header_tail.extend_from_slice(&[0; 8]); // unknown
        let ticks = DOTNET_EPOCH_TICKS + 1_700_000_000_000 * 10_000; // 2023-11-14T22:13:20Z
        header_tail.extend_from_slice(&ticks.to_le_bytes());
        write_prefixed_string(&mut header_tail, header_string);
        let mut out = Vec::new();
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&(header_tail.len() as u32).to_le_bytes());
        out.extend_from_slice(&header_tail);
        out.extend_from_slice(payload);
        out
    }

    pub(crate) fn zlib(json: &str) -> Vec<u8> {
        deflate_json(json)
    }

    pub(crate) fn encrypted_profile(json: &str) -> Vec<u8> {
        container(
            "\"0123456789abcdef0123456789abcdef\"",
            &encrypt_payload(&zlib(json), &[0x5A; SALT_LEN]),
        )
    }

    /// A MapSaveData-shaped file wrapping `json` (use "null" for the idle state).
    pub(crate) fn map_save(json: &str) -> Vec<u8> {
        container("", &zlib(json))
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;
    use std::path::PathBuf;

    const PROFILE_JSON: &str = r#"{
      "savedByGameVersion": "56.3", "rank": 143, "monkeyMoney": 8042,
      "savedMaps": {
        "Cubism": {"mapName":"Cubism","modeName":"Standard","mapDifficulty":"Easy","round":297,"gameType":0,"playerChallengeId":null,"gameId":1254,"attemptId":13,"matchWon":true,"gameVersion":"56.1","metaData":{}},
        "OD": {"mapName":"LotusIsland","modeName":"Standard","mapDifficulty":"Medium","round":20,"gameType":3,"gameId":2144,"attemptId":0,"metaData":{}},
        "BB": {"mapName":"FrozenOver","modeName":"Standard","mapDifficulty":"Medium","round":59,"gameType":4,"playerChallengeId":"Lych61_mtw1v6de","gameId":2152,"attemptId":10,"gameVersion":"56.3","metaData":{"BossEventIsElite":"True","BossEventTier":"1"}},
        "BBCONT": {"mapName":"FrozenOver","modeName":"Standard","mapDifficulty":"Medium","round":40,"gameType":4,"playerChallengeId":"Lych61_mtw1v6de","gameId":2152,"attemptId":5,"gameVersion":"56.3","metaData":{"BossEventIsElite":"True","BossEventTier":"0"}}
      }
    }"#;

    #[test]
    fn header_round_trip() {
        let bytes = container("\"abc\"", &[0x78, 0x9C]);
        let c = parse_container(&bytes).unwrap();
        assert_eq!(c.version, 2);
        assert_eq!(c.header_len, 42);
        assert_eq!(c.save_count, 7);
        assert_eq!(c.guid, [0xAB; 16]);
        assert_eq!(c.header_string, "\"abc\"");
        assert_eq!(c.payload, &[0x78, 0x9C]);
        assert_eq!(c.timestamp_unix_ms(), Some(1_700_000_000_000));
    }

    #[test]
    fn header_len_matches_spec_for_real_files() {
        // Profile: 36 fixed + 1 length byte + 34 chars = 71. MapSaveData: 36 + 1 + 0 = 37.
        let p = container("\"0123456789abcdef0123456789abcdef\"", &[0x78]);
        assert_eq!(parse_container(&p).unwrap().header_len, 71);
        let m = container("", &[0x78]);
        assert_eq!(parse_container(&m).unwrap().header_len, 37);
    }

    #[test]
    fn rejects_garbage() {
        assert!(matches!(parse_container(&[1, 2, 3]), Err(SaveError::TooShort)));
        assert!(matches!(
            parse_container(&[9, 0, 0, 0, 1, 0, 0, 0]),
            Err(SaveError::BadVersion(9))
        ));
        let mut bad = container("", &[0x78]);
        bad[4] = 99; // header_len no longer matches the string end
        assert!(matches!(parse_container(&bad), Err(SaveError::BadHeader)));
    }

    #[test]
    fn decodes_unencrypted_map_save() {
        let bytes = container("", &zlib("null"));
        assert_eq!(decode_json(&bytes).unwrap(), "null");
        assert!(!map_save_has_game(&bytes).unwrap());
        let bytes = container("", &zlib(r#"{"round":58}"#));
        assert!(map_save_has_game(&bytes).unwrap());
    }

    #[test]
    fn decrypts_synthetic_profile_and_extracts_info() {
        let bytes = encrypted_profile(PROFILE_JSON);
        let info = read_profile_info(&bytes).unwrap();
        assert_eq!(info.game_version.as_deref(), Some("56.3"));
        assert_eq!(info.rank, Some(143));
        assert_eq!(info.monkey_money, Some(8042));
        assert_eq!(info.saved_maps.len(), 4);
        assert_eq!(info.last_played_key.as_deref(), Some("BB"));
        assert_eq!(
            info.summary.as_deref(),
            Some("Lych Elite T1 · Frozen Over · R59")
        );
        let bb = info.saved_maps.iter().find(|m| m.key == "BB").unwrap();
        assert_eq!(bb.boss.as_deref(), Some("Lych"));
        assert!(bb.elite);
        assert_eq!(bb.tier, Some(1));
        let cubism = info.saved_maps.iter().find(|m| m.key == "Cubism").unwrap();
        assert_eq!(cubism.boss, None);
        assert!(cubism.match_won);
    }

    #[test]
    fn encode_decode_round_trip() {
        let original = encrypted_profile(PROFILE_JSON);
        let c = parse_container(&original).unwrap();
        assert_eq!(c.header_bytes.len(), 8 + c.header_len as usize);
        let json = decode_json(&original).unwrap();

        let rebuilt = encode_profile(c.header_bytes, &json).unwrap();
        let rc = parse_container(&rebuilt).unwrap();
        assert_eq!(rc.header_bytes, c.header_bytes);
        assert_eq!(rc.save_count, c.save_count);
        assert_eq!(rc.header_string, c.header_string);
        assert_eq!(decode_json(&rebuilt).unwrap(), json);
        assert_eq!(read_profile_info(&rebuilt).unwrap(), read_profile_info(&original).unwrap());

        // Fresh salts each time; same salt reproduces the bytes exactly.
        let again = encode_profile(c.header_bytes, &json).unwrap();
        assert_ne!(again, rebuilt);
        let fixed = encode_profile_with_salt(c.header_bytes, &json, &[0x5A; SALT_LEN]);
        assert_eq!(fixed, original);
        assert_eq!(&deflate_json("x")[..2], &[0x78, 0x5E], "game's zlib level");
    }

    #[test]
    fn number_text_survives_round_trip() {
        // The game writes C# "R" floats; arbitrary_precision keeps them byte-for-byte.
        let json = r#"{"a":1.0,"b":506399.83894175291,"c":12345678901234567,"d":-0.0,"e":8042}"#;
        let v: Value = serde_json::from_str(json).unwrap();
        assert_eq!(serde_json::to_string(&v).unwrap(), json);
        assert_eq!(as_i64(&v["a"]), Some(1));
        assert_eq!(as_i64(&v["e"]), Some(8042));
        let bytes = encode_profile_with_salt(&container("", &[]), json, &[1; SALT_LEN]);
        assert_eq!(decode_json(&bytes).unwrap(), json);
    }

    #[test]
    fn corrupted_ciphertext_fails_cleanly() {
        let mut bytes = encrypted_profile(PROFILE_JSON);
        let n = bytes.len();
        bytes[n - 1] ^= 0xFF;
        assert!(read_profile_info(&bytes).is_err());
    }

    #[test]
    fn summaries() {
        let mk = |map: &str, mode: &str, diff: &str, gt: i64, round: i64| SavedMapInfo {
            key: "k".into(),
            map: map.into(),
            mode: mode.into(),
            difficulty: diff.into(),
            game_type: gt,
            round,
            boss: None,
            elite: false,
            tier: None,
            challenge_id: None,
            game_id: 0,
            attempt_id: 0,
            match_won: false,
            game_version: None,
        };
        assert_eq!(
            summarize(&mk("Cubism", "Standard", "Easy", 0, 297)),
            "Cubism · Easy Standard · R297"
        );
        assert_eq!(
            summarize(&mk("Peninsula", "Clicks", "Hard", 0, 100)),
            "Peninsula · Clicks · R100"
        );
        assert_eq!(
            summarize(&mk("LotusIsland", "Standard", "Medium", 3, 20)),
            "Lotus Island · Odyssey · R20"
        );
        let mut boss = mk("FrozenOver", "Standard", "Medium", 4, 59);
        boss.boss = Some("Lych".into());
        assert_eq!(summarize(&boss), "Lych · Frozen Over · R59");
        boss.elite = true;
        boss.tier = Some(1);
        assert_eq!(summarize(&boss), "Lych Elite T1 · Frozen Over · R59");
    }

    #[test]
    fn humanizes_map_names() {
        assert_eq!(humanize_map_name("FrozenOver"), "Frozen Over");
        assert_eq!(humanize_map_name("LotusIsland"), "Lotus Island");
        assert_eq!(humanize_map_name("Cubism"), "Cubism");
        assert_eq!(humanize_map_name("XFactor"), "X Factor");
        assert_eq!(humanize_map_name("DarkCastle"), "Dark Castle");
        assert_eq!(humanize_map_name(""), "");
    }

    #[test]
    fn boss_name_parsing() {
        assert_eq!(
            boss_name_from_challenge_id("Lych61_mtw1v6de").as_deref(),
            Some("Lych")
        );
        assert_eq!(
            boss_name_from_challenge_id("Bloonarius3_x").as_deref(),
            Some("Bloonarius")
        );
        assert_eq!(boss_name_from_challenge_id("12abc"), None);
    }

    /// Optional check against a copy of the live files. Put `Profile.Save` and `MapSaveData`
    /// in `src-tauri/fixtures/` (gitignored); the test is skipped when they are absent.
    #[test]
    fn live_fixtures_if_present() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures");
        if let Ok(bytes) = std::fs::read(dir.join("Profile.Save")) {
            let c = parse_container(&bytes).unwrap();
            assert_eq!(c.header_len, 71);
            assert!(c.timestamp_unix_ms().is_some());
            let info = read_profile_info(&bytes).expect("live profile decrypts");
            assert!(info.game_version.is_some());
            assert!(info.rank.is_some());
            assert!(info.summary.is_some());
            // Parse -> serialize -> encode -> decode must reproduce the game's text exactly.
            let json = decode_json(&bytes).unwrap();
            let v: Value = serde_json::from_str(&json).unwrap();
            // Text is preserved except that C# writes exponents as `1E+08` and serde_json
            // as `1e+08`; values are identical.
            let out = serde_json::to_string(&v).unwrap();
            assert_eq!(out.len(), json.len());
            let bad = out.bytes().zip(json.bytes()).find(|(a, b)| a != b && !(*a == b'e' && *b == b'E'));
            assert!(bad.is_none(), "unexpected round-trip difference: {bad:?}");
            assert_eq!(serde_json::from_str::<Value>(&out).unwrap(), v);
            let rebuilt = encode_profile(c.header_bytes, &json).unwrap();
            assert_eq!(decode_json(&rebuilt).unwrap(), json);
        }
        if let Ok(bytes) = std::fs::read(dir.join("MapSaveData")) {
            assert_eq!(parse_container(&bytes).unwrap().header_len, 37);
            map_save_has_game(&bytes).expect("live MapSaveData inflates");
        }
    }
}
