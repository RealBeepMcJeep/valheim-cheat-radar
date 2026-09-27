use flate2::read::GzDecoder;
use std::collections::HashMap;
use std::fmt;
#[cfg(not(target_arch = "wasm32"))]
use std::io::BufReader;
use std::io::{self, Read};
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::process::{Command, Stdio};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

const FLAG_CONNECTION: u16 = 0x001;
const FLAG_FLOATS: u16 = 0x002;
const FLAG_VEC3: u16 = 0x004;
const FLAG_QUATS: u16 = 0x008;
const FLAG_INTS: u16 = 0x010;
const FLAG_LONGS: u16 = 0x020;
const FLAG_STRINGS: u16 = 0x040;
const FLAG_BYTE_ARRAYS: u16 = 0x080;
const FLAG_ROTATION: u16 = 0x1000;
const FLAG_SMALL_POSITION: u16 = 0x2000;

pub const CHEATED: i32 = -153_476_768;
pub const CHEATED_QUEUED: i32 = 554_813_359;
pub const ITEM_DATA: i32 = 949_524_933;
pub const ITEMS: i32 = -938_864_442;
/// `ItemStand` keeps the hash of the item it holds in this int (0 when empty) next to `itemData`.
const ITEM_INT: i32 = -1_126_421_579;
const TYPE_INT: i32 = 1_421_151_742;
pub const QUEUED: i32 = -2_086_149_575;

#[derive(Debug, Clone)]
pub struct ScanError(pub String);

impl fmt::Display for ScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ScanError {}

impl From<io::Error> for ScanError {
    fn from(value: io::Error) -> Self {
        Self(value.to_string())
    }
}

fn error(message: impl Into<String>) -> ScanError {
    ScanError(message.into())
}

fn checked_len(value: i32, what: &str) -> Result<usize, ScanError> {
    if !(0..=128 * 1024 * 1024).contains(&value) {
        return Err(error(format!("invalid {what} length {value}")));
    }
    Ok(value as usize)
}

fn read_exact<R: Read>(reader: &mut R, buf: &mut [u8]) -> Result<(), ScanError> {
    reader.read_exact(buf).map_err(ScanError::from)
}

fn read_u8<R: Read>(reader: &mut R) -> Result<u8, ScanError> {
    let mut b = [0; 1];
    read_exact(reader, &mut b)?;
    Ok(b[0])
}

fn read_i16<R: Read>(reader: &mut R) -> Result<i16, ScanError> {
    let mut b = [0; 2];
    read_exact(reader, &mut b)?;
    Ok(i16::from_le_bytes(b))
}

fn read_u16<R: Read>(reader: &mut R) -> Result<u16, ScanError> {
    let mut b = [0; 2];
    read_exact(reader, &mut b)?;
    Ok(u16::from_le_bytes(b))
}

fn read_i32<R: Read>(reader: &mut R) -> Result<i32, ScanError> {
    let mut b = [0; 4];
    read_exact(reader, &mut b)?;
    Ok(i32::from_le_bytes(b))
}

fn read_u32<R: Read>(reader: &mut R) -> Result<u32, ScanError> {
    let mut b = [0; 4];
    read_exact(reader, &mut b)?;
    Ok(u32::from_le_bytes(b))
}

fn read_i64<R: Read>(reader: &mut R) -> Result<i64, ScanError> {
    let mut b = [0; 8];
    read_exact(reader, &mut b)?;
    Ok(i64::from_le_bytes(b))
}

fn read_f32<R: Read>(reader: &mut R) -> Result<f32, ScanError> {
    let mut b = [0; 4];
    read_exact(reader, &mut b)?;
    Ok(f32::from_le_bytes(b))
}

fn read_f64<R: Read>(reader: &mut R) -> Result<f64, ScanError> {
    let mut b = [0; 8];
    read_exact(reader, &mut b)?;
    Ok(f64::from_le_bytes(b))
}

fn read_7bit<R: Read>(reader: &mut R) -> Result<usize, ScanError> {
    let mut value = 0usize;
    for shift in (0..35).step_by(7) {
        let byte = read_u8(reader)?;
        value |= ((byte & 0x7f) as usize) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(error("invalid .NET 7-bit integer"))
}

/// Transparent `Read` pass-through that counts bytes consumed, so a chunk-file offset can be
/// recovered for a scrub patch without changing what or how any byte is read. Wrapping an existing
/// reader in this changes no parsing behaviour: every scan still reads the exact same bytes in the
/// exact same order.
struct CountingReader<R: Read> {
    inner: R,
    pos: u64,
}

impl<R: Read> CountingReader<R> {
    fn new(inner: R) -> Self {
        Self { inner, pos: 0 }
    }

    fn pos(&self) -> u64 {
        self.pos
    }
}

impl<R: Read> Read for CountingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buf)?;
        self.pos += read as u64;
        Ok(read)
    }
}

fn read_dotnet_bytes<R: Read>(reader: &mut R, output: &mut Vec<u8>) -> Result<(), ScanError> {
    let length = read_7bit(reader)?;
    if length > 128 * 1024 * 1024 {
        return Err(error(format!("string is too large: {length} bytes")));
    }
    output.resize(length, 0);
    read_exact(reader, output)
}

fn skip_bytes<R: Read>(reader: &mut R, mut count: u64) -> Result<(), ScanError> {
    let mut scratch = [0u8; 8192];
    while count != 0 {
        let wanted = count.min(scratch.len() as u64) as usize;
        read_exact(reader, &mut scratch[..wanted])?;
        count -= wanted as u64;
    }
    Ok(())
}

fn skip_dotnet_string<R: Read>(reader: &mut R) -> Result<(), ScanError> {
    let length = read_7bit(reader)?;
    if length > 128 * 1024 * 1024 {
        return Err(error(format!("string is too large: {length} bytes")));
    }
    skip_bytes(reader, length as u64)
}

fn read_num_items<R: Read>(reader: &mut R) -> Result<usize, ScanError> {
    let first = read_u8(reader)?;
    let value = if first & 0x80 == 0 {
        first as usize
    } else {
        (((first & 0x7f) as usize) << 8) | read_u8(reader)? as usize
    };
    if value > 1_000_000 {
        return Err(error(format!("unreasonable item count {value}")));
    }
    Ok(value)
}

fn read_legacy_count<R: Read>(reader: &mut R) -> Result<usize, ScanError> {
    let first = read_u8(reader)?;
    let width = if first < 0x80 {
        1
    } else if first & 0xe0 == 0xc0 {
        2
    } else if first & 0xf0 == 0xe0 {
        3
    } else {
        return Err(error("invalid legacy UTF-8 count"));
    };
    let mut codepoint = (first & (0x7f >> width)) as u32;
    for _ in 1..width {
        let byte = read_u8(reader)?;
        if byte & 0xc0 != 0x80 {
            return Err(error("invalid legacy UTF-8 count continuation"));
        }
        codepoint = (codepoint << 6) | (byte & 0x3f) as u32;
    }
    if codepoint > 1_000_000 {
        return Err(error(format!("unreasonable legacy item count {codepoint}")));
    }
    Ok(codepoint as usize)
}

/// Valheim's GetStableHashCode, including signed 32-bit overflow semantics.
pub fn stable_hash(value: &str) -> i32 {
    let units: Vec<u16> = value.encode_utf16().collect();
    stable_hash_units(&units)
}

fn stable_hash_units(units: &[u16]) -> i32 {
    let mut first = 5381i32;
    let mut second = 5381i32;
    let mut index = 0;
    while index < units.len() && units[index] != 0 {
        first = first.wrapping_mul(33) ^ units[index] as i32;
        if index == units.len() - 1 || units[index + 1] == 0 {
            break;
        }
        second = second.wrapping_mul(33) ^ units[index + 1] as i32;
        index += 2;
    }
    first.wrapping_add(second.wrapping_mul(1_566_083_941))
}

fn stable_hash_index_item_data(index: usize) -> i32 {
    let text = index.to_string();
    stable_hash(&format!("{text}_itemData"))
}

#[derive(Debug, Clone, Default)]
pub struct PrefabNames {
    names: Vec<(i32, String)>,
    indexed_item_data: HashMap<i32, usize>,
    /// Prefab hash -> biome bitmask, from the game's own data (see `prefab_biomes.txt`).
    biomes: HashMap<i32, u16>,
    /// Hashes whose biome mask came from a name-prefix heuristic (the "hint" column), not from a
    /// biome-tagged component the game itself carries. See `biome_verdict_preferring_real`.
    hinted: std::collections::HashSet<i32>,
}

impl PrefabNames {
    pub fn from_text(text: &str) -> Self {
        let names = text
            .lines()
            .map(str::trim)
            .filter(|name| !name.is_empty() && !name.starts_with('#'))
            .map(|name| (stable_hash(name), name.to_string()))
            .collect();
        let indexed_item_data = (0..=1024)
            .map(|index| (stable_hash_index_item_data(index), index))
            .collect();
        Self {
            names,
            indexed_item_data,
            biomes: HashMap::new(),
            hinted: std::collections::HashSet::new(),
        }
    }

    /// Attach the generated prefab -> biome table:
    /// `<prefab name><TAB><biome>[,<biome>...][<TAB>hint]`. Unknown biome names are ignored rather
    /// than guessed, so a stale table degrades to "no colour". A trailing `hint` column marks a name
    /// the game's own data never tagged, filled in by a prefix heuristic (see
    /// `tools/extract-prefab-data.py`); those votes are trusted only when game-tagged evidence in the
    /// same cell is too thin to stand on its own (`biome_verdict_preferring_real`).
    pub fn with_biome_text(mut self, text: &str) -> Self {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut fields = line.split('\t');
            let Some(name) = fields.next() else {
                continue;
            };
            let Some(biomes) = fields.next() else {
                continue;
            };
            let hinted = fields.next().map(str::trim) == Some("hint");
            let mut mask = 0u16;
            for biome in biomes.split(',') {
                if let Some((bit, _)) = BIOMES.iter().find(|(_, known)| *known == biome.trim()) {
                    mask |= bit;
                }
            }
            if mask != 0 {
                let hash = stable_hash(name.trim());
                self.biomes.insert(hash, mask);
                if hinted {
                    self.hinted.insert(hash);
                }
            }
        }
        self
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn load(path: &Path) -> Result<Self, ScanError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| error(format!("cannot read {}: {e}", path.display())))?;
        Ok(Self::from_text(&text))
    }

    pub fn name(&self, hash: i32) -> Option<&str> {
        self.names
            .iter()
            .find_map(|(known_hash, name)| (*known_hash == hash).then_some(name.as_str()))
    }

    /// Biome bitmask for a prefab hash; 0 when the prefab is unknown or carries no biome.
    pub fn biome_mask(&self, hash: i32) -> u16 {
        self.biomes.get(&hash).copied().unwrap_or(0)
    }

    /// Same as `biome_mask`, but 0 for a hash whose mask came from the flora-hint heuristic rather
    /// than a biome-tagged component the game itself carries.
    pub fn real_biome_mask(&self, hash: i32) -> u16 {
        if self.hinted.contains(&hash) {
            0
        } else {
            self.biome_mask(hash)
        }
    }

    /// Number of prefabs in the biome table, for diagnostics.
    pub fn biome_count(&self) -> usize {
        self.biomes.len()
    }

    fn indexed_item_data(&self, hash: i32) -> Option<usize> {
        self.indexed_item_data.get(&hash).copied()
    }
}

#[derive(Debug, Clone, Default)]
pub struct ItemEvidence {
    pub item_hash: Option<i32>,
    pub grid: Option<(u8, u8)>,
    pub quality: u16,
    pub stack: u16,
    pub variant: i32,
    pub world_level: u8,
    pub crafter_id: Option<i64>,
    pub crafter_name: Option<String>,
    pub custom_keys: Vec<String>,
    /// Byte offset of this item's trailing cheated-flag byte, relative to the start of the byte
    /// slice this item was decoded from. `None` only when the item's format has no such byte
    /// (never the case here: an `ItemEvidence` only exists because that byte's bit 0 was set).
    pub cheated_byte_offset: Option<usize>,
}

/// One in-place, same-length byte patch the "Mode A" scrub applies to clear a cheat flag: a ZDO
/// int flag's 4-byte value zeroed, or a single item byte's bit 0 cleared. See `scrub_world_tar_bytes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrubPatchKind {
    /// A `cheated`/`cheatedQueued[+slot]` ZDO int value, zeroed in place.
    ZdoIntFlag,
    /// An item's trailing flags byte (`m_cheated`, bit 0), cleared in place.
    ItemCheatedBit,
}

/// A single recorded patch location, found while parsing a v41 chunk file. `offset` is absolute
/// within `file_path`. `old_value`/`new_value` are recorded from the parser's own read of the
/// bytes at the moment it found the flag, and `apply_patch` re-reads the buffer at `offset` and
/// refuses to write unless it still matches `old_value`, so a mismatch fails closed instead of
/// silently patching the wrong byte.
#[derive(Debug, Clone)]
pub struct ScrubPatch {
    pub file_path: String,
    pub offset: u64,
    pub kind: ScrubPatchKind,
    pub key_name: String,
    pub owner_prefab_name: Option<String>,
    pub position: Position,
    pub old_value: i64,
    pub new_value: i64,
}

/// What the parser records for a scrub while it walks a chunk: the flag patches (always; they are
/// few) and, only when `layouts` is `Some`, every ZDO record's byte layout, which the delete modes
/// need and an ordinary scan of ~900k ZDOs must not pay for.
#[derive(Debug, Default)]
struct ScrubSink {
    patches: Vec<ScrubPatch>,
    layouts: Option<Vec<ZdoLayout>>,
}

impl ScrubSink {
    fn with_layouts() -> Self {
        Self {
            patches: Vec::new(),
            layouts: Some(Vec::new()),
        }
    }
}

/// One ZDO record inside a chunk file, as much as a delete scrub needs. Offsets are absolute
/// within the chunk file.
#[derive(Debug, Clone)]
struct ZdoLayout {
    start: u64,
    end: u64,
    owner_prefab_hash: i32,
    position: Position,
    /// Nonzero `cheated`/`cheatedQueued[+slot]` values: the object itself is flagged.
    object_flags: usize,
    /// Value offset and value of the `item`/`type` ints an `ItemStand` keeps. Their presence is
    /// what tells a stand holding an item apart from an item lying in the world.
    item_int: Option<(u64, i32)>,
    type_int: Option<(u64, i32)>,
    /// `items`, `itemData` and `N_itemData` byte arrays; other arrays are not recorded.
    arrays: Vec<ArrayLayout>,
}

#[derive(Debug, Clone)]
struct ArrayLayout {
    key: i32,
    /// Offset of the array's i32 length prefix; the data follows it.
    len_offset: u64,
    len: u64,
    /// Item ranges relative to the data start (a single item for `itemData`).
    items: Vec<ItemLayout>,
}

#[derive(Debug, Clone)]
struct ItemLayout {
    start: u64,
    end: u64,
    cheated: bool,
    /// Absolute offset of the flags byte holding the cheated bit, for cheated items.
    cheated_byte: Option<u64>,
    item_hash: Option<i32>,
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], ScanError> {
        let end = self
            .position
            .checked_add(count)
            .ok_or_else(|| error("cursor overflow"))?;
        if end > self.bytes.len() {
            return Err(error("truncated payload"));
        }
        let value = &self.bytes[self.position..end];
        self.position = end;
        Ok(value)
    }

    fn u8(&mut self) -> Result<u8, ScanError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, ScanError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn i32(&mut self) -> Result<i32, ScanError> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn i64(&mut self) -> Result<i64, ScanError> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn f32(&mut self) -> Result<f32, ScanError> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn bool(&mut self) -> Result<bool, ScanError> {
        Ok(self.u8()? != 0)
    }

    fn byte_array(&mut self) -> Result<&'a [u8], ScanError> {
        let length = checked_len(self.i32()?, "character byte array")?;
        self.take(length)
    }

    fn num_items(&mut self) -> Result<usize, ScanError> {
        let first = self.u8()?;
        let value = if first & 0x80 == 0 {
            first as usize
        } else {
            (((first & 0x7f) as usize) << 8) | self.u8()? as usize
        };
        if value > 1_000_000 {
            return Err(error(format!("unreasonable item count {value}")));
        }
        Ok(value)
    }

    fn string(&mut self) -> Result<String, ScanError> {
        let length = self.num_7bit()?;
        let bytes = self.take(length)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| error("invalid UTF-8 item string"))
    }

    fn skip_string(&mut self) -> Result<(), ScanError> {
        let length = self.num_7bit()?;
        self.take(length).map(|_| ())
    }

    fn num_7bit(&mut self) -> Result<usize, ScanError> {
        let mut value = 0usize;
        for shift in (0..35).step_by(7) {
            let byte = self.u8()?;
            value |= ((byte & 0x7f) as usize) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(error("invalid .NET 7-bit item string length"))
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct ItemSummary {
    /// Position, within the slice the enclosing `Cursor` was built over, of the trailing cheated
    /// flags byte. `None` when this item's version has no such byte (106).
    cheated_byte_pos: Option<usize>,
    /// This item's byte range within that same slice, so a delete scrub can cut it out.
    start: usize,
    end: usize,
}

fn validate_item_version(version: i32, _allow_legacy_106: bool) -> Result<u8, ScanError> {
    match version {
        106 | 107 | 109 => Ok(version as u8),
        _ => Err(error(format!(
            "unsupported item/inventory version {version}"
        ))),
    }
}

fn scan_item(
    cursor: &mut Cursor<'_>,
    version: u8,
    allow_legacy_106: bool,
) -> Result<(ItemSummary, bool), ScanError> {
    validate_item_version(version as i32, allow_legacy_106)?;
    let start = cursor.position;
    let _durability = cursor.i32()?;
    let _grid_x = cursor.u8()?;
    let _grid_y = cursor.u8()?;
    let _world_level = cursor.u8()?;
    let flags = cursor.u8()?;
    let _quality = if flags & 0x04 != 0 { cursor.u16()? } else { 1 };
    let _stack = if flags & 0x08 != 0 { cursor.u16()? } else { 1 };
    let _variant = if flags & 0x10 != 0 { cursor.i32()? } else { 0 };
    if flags & 0x20 != 0 {
        cursor.i64()?;
        cursor.skip_string()?;
    }
    if flags & 0x40 != 0 {
        cursor.i32()?;
    }
    if flags & 0x80 != 0 {
        let count = cursor.num_items()?;
        for _ in 0..count {
            cursor.skip_string()?;
            cursor.skip_string()?;
        }
    }
    let (cheated_byte_pos, cheated) = if version >= 109 || version == 107 {
        let pos = cursor.position;
        let bit_set = cursor.u8()? & 1 != 0;
        (Some(pos), bit_set)
    } else {
        (None, false)
    };
    Ok((
        ItemSummary {
            cheated_byte_pos,
            start,
            end: cursor.position,
        },
        cheated,
    ))
}

fn decode_item(
    bytes: &[u8],
    version: u8,
    allow_legacy_106: bool,
) -> Result<Option<ItemEvidence>, ScanError> {
    let mut first_pass = Cursor::new(bytes);
    let (summary, cheated) = scan_item(&mut first_pass, version, allow_legacy_106)?;
    if !cheated {
        return Ok(None);
    }
    let mut cursor = Cursor::new(bytes);
    cursor.i32()?;
    let grid = Some((cursor.u8()?, cursor.u8()?));
    let world_level = cursor.u8()?;
    let flags = cursor.u8()?;
    let quality = if flags & 0x04 != 0 { cursor.u16()? } else { 1 };
    let stack = if flags & 0x08 != 0 { cursor.u16()? } else { 1 };
    let variant = if flags & 0x10 != 0 { cursor.i32()? } else { 0 };
    let (crafter_id, crafter_name) = if flags & 0x20 != 0 {
        (Some(cursor.i64()?), Some(cursor.string()?))
    } else {
        (None, None)
    };
    let item_hash = if flags & 0x40 != 0 {
        Some(cursor.i32()?)
    } else {
        None
    };
    let mut custom_keys = Vec::new();
    if flags & 0x80 != 0 {
        let count = cursor.num_items()?;
        for _ in 0..count {
            custom_keys.push(cursor.string()?);
            cursor.skip_string()?;
        }
    }
    if version >= 109 || version == 107 {
        cursor.u8()?;
    }
    Ok(Some(ItemEvidence {
        item_hash,
        grid,
        quality,
        stack,
        variant,
        world_level,
        crafter_id,
        crafter_name,
        custom_keys,
        cheated_byte_offset: summary.cheated_byte_pos,
    }))
}

pub fn parse_direct_item(bytes: &[u8]) -> Result<Option<ItemEvidence>, ScanError> {
    if bytes.is_empty() {
        return Err(error("empty direct item payload"));
    }
    decode_item(&bytes[1..], bytes[0], false)
}

fn parse_inventory_internal<F>(
    bytes: &[u8],
    allow_legacy_106: bool,
    mut observe: F,
) -> Result<(Vec<ItemEvidence>, usize), ScanError>
where
    F: FnMut(ItemSummary, bool),
{
    let mut cursor = Cursor::new(bytes);
    let version = validate_item_version(cursor.i32()?, false)?;
    let count = cursor.u16()? as usize;
    let mut hits = Vec::new();
    for _ in 0..count {
        let start = cursor.position;
        let (summary, cheated) = scan_item(&mut cursor, version, allow_legacy_106)?;
        observe(summary, cheated);
        if cheated {
            if let Some(mut item) =
                decode_item(&bytes[start..cursor.position], version, allow_legacy_106)?
            {
                // `decode_item` measured the offset from its own slice (this item only); make it
                // absolute within `bytes` (the whole container array) by adding the item's start.
                item.cheated_byte_offset = item.cheated_byte_offset.map(|pos| pos + start);
                hits.push(item);
            }
        }
    }
    Ok((hits, count))
}

pub fn parse_inventory(bytes: &[u8]) -> Result<Vec<ItemEvidence>, ScanError> {
    parse_inventory_internal(bytes, false, |_, _| {}).map(|(hits, _)| hits)
}

fn character_count(cursor: &mut Cursor<'_>, what: &str) -> Result<usize, ScanError> {
    let count = cursor.i32()?;
    if !(0..=1_000_000).contains(&count) {
        return Err(error(format!("invalid {what} count {count}")));
    }
    Ok(count as usize)
}

fn skip_float_dictionary(cursor: &mut Cursor<'_>, what: &str) -> Result<(), ScanError> {
    for _ in 0..character_count(cursor, what)? {
        cursor.skip_string()?;
        cursor.f32()?;
    }
    Ok(())
}

fn parse_character_profile(payload: &[u8]) -> Result<CharacterScan, ScanError> {
    let mut cursor = Cursor::new(payload);
    let profile_version = cursor.i32()?;
    let stat_slots = cursor.i32()?;
    let stat_profiles = cursor.i32()?;
    if profile_version != 46 || stat_slots != 205 || stat_profiles != 10 {
        return Err(error(format!(
            "unsupported character profile header {profile_version}/{stat_slots}/{stat_profiles}"
        )));
    }
    let mut cheat_stat_nonzero_count = 0u32;
    let mut known_command_hits = 0u32;
    for _ in 0..stat_profiles {
        for slot in 0..stat_slots {
            let value = cursor.f32()?;
            if slot == 4 && value != 0.0 {
                cheat_stat_nonzero_count += 1;
            }
        }
        skip_float_dictionary(&mut cursor, "known worlds")?;
        skip_float_dictionary(&mut cursor, "known world keys")?;
        for _ in 0..character_count(&mut cursor, "known commands")? {
            let command = cursor.string()?.to_ascii_lowercase();
            cursor.f32()?;
            if command == "devcommands" || command == "clearcheats" {
                known_command_hits += 1;
            }
        }
        let enemy_groups = character_count(&mut cursor, "enemy stat groups")?;
        for _ in 0..enemy_groups {
            skip_float_dictionary(&mut cursor, "enemy stats")?;
        }
        skip_float_dictionary(&mut cursor, "item pickup stats")?;
        skip_float_dictionary(&mut cursor, "item craft stats")?;
        skip_float_dictionary(&mut cursor, "pickable stats")?;
        skip_float_dictionary(&mut cursor, "food eaten stats")?;
        skip_float_dictionary(&mut cursor, "pieces placed stats")?;
    }
    cursor.bool()?;
    for _ in 0..character_count(&mut cursor, "world data")? {
        cursor.i64()?;
        cursor.bool()?;
        for _ in 0..3 {
            cursor.f32()?;
        }
        cursor.bool()?;
        for _ in 0..3 {
            cursor.f32()?;
        }
        cursor.bool()?;
        for _ in 0..3 {
            cursor.f32()?;
        }
        for _ in 0..3 {
            cursor.f32()?;
        }
        if cursor.bool()? {
            cursor.byte_array()?;
        }
    }
    let player_name = cursor.string()?;
    let player_id = cursor.i64()?;
    cursor.skip_string()?;
    let used_cheats = cursor.bool()?;
    cursor.i64()?;
    let has_player_data = cursor.bool()?;
    let player_data = if has_player_data {
        Some(cursor.byte_array()?.to_vec())
    } else {
        None
    };
    if cursor.position != payload.len() {
        return Err(error(format!(
            "character profile has {} trailing bytes",
            payload.len() - cursor.position
        )));
    }
    let player_data_summary = if let Some(data) = player_data.as_deref() {
        parse_character_player_data(data)?
    } else {
        PlayerDataSummary::default()
    };
    Ok(CharacterScan {
        source: String::new(),
        canonical: false,
        canonical_eligible: true,
        input_id: 0,
        modified_unix: None,
        file_bytes: 0,
        payload_bytes: payload.len() as u64,
        hash_bytes: 0,
        hash_valid: false,
        trusted: false,
        supported: true,
        parse_error: None,
        profile_version,
        stat_slots,
        stat_profiles,
        player_name,
        player_id: Some(player_id),
        used_cheats,
        cheat_stat_nonzero_count,
        known_command_hits,
        player_data_version: player_data_summary.version,
        inventory_version: player_data_summary.inventory_version,
        inventory_item_count: player_data_summary.inventory_item_count,
        cheated_inventory_count: player_data_summary.cheated_inventory_count,
        bypass_cheat_checks: player_data_summary.bypass_cheat_checks,
        player_data_complete: player_data_summary.complete,
    })
}

#[derive(Debug, Default)]
struct PlayerDataSummary {
    version: Option<i32>,
    inventory_version: Option<i32>,
    inventory_item_count: u32,
    cheated_inventory_count: u32,
    bypass_cheat_checks: bool,
    complete: bool,
}

fn parse_character_player_data(data: &[u8]) -> Result<PlayerDataSummary, ScanError> {
    let mut cursor = Cursor::new(data);
    let version = cursor.i32()?;
    if version != 33 {
        return Err(error(format!("unsupported playerData version {version}")));
    }
    for _ in 0..4 {
        cursor.f32()?;
    }
    cursor.skip_string()?;
    cursor.f32()?;
    let item_version = validate_item_version(cursor.i32()?, false)?;
    let item_count = cursor.u16()? as usize;
    let mut cheated_items = 0u32;
    for _ in 0..item_count {
        let (_, cheated) = scan_item(&mut cursor, item_version, false)?;
        cheated_items += u32::from(cheated);
    }
    for _ in 0..character_count(&mut cursor, "known recipes")? {
        cursor.skip_string()?;
    }
    for _ in 0..character_count(&mut cursor, "known stations")? {
        cursor.skip_string()?;
        cursor.i32()?;
    }
    for _ in 0..character_count(&mut cursor, "known materials")? {
        cursor.skip_string()?;
    }
    for _ in 0..character_count(&mut cursor, "shown tutorials")? {
        cursor.skip_string()?;
    }
    for _ in 0..character_count(&mut cursor, "unique keys")? {
        cursor.skip_string()?;
    }
    for _ in 0..character_count(&mut cursor, "trophies")? {
        cursor.skip_string()?;
    }
    for _ in 0..character_count(&mut cursor, "known biomes")? {
        cursor.skip_string()?;
    }
    for _ in 0..character_count(&mut cursor, "known texts")? {
        cursor.skip_string()?;
        cursor.skip_string()?;
    }
    cursor.skip_string()?;
    cursor.skip_string()?;
    for _ in 0..6 {
        cursor.f32()?;
    }
    cursor.i32()?;
    for _ in 0..character_count(&mut cursor, "foods")? {
        cursor.skip_string()?;
        cursor.f32()?;
    }
    let skill_version = cursor.i32()?;
    if !(0..=10).contains(&skill_version) {
        return Err(error("invalid skills version"));
    }
    for _ in 0..character_count(&mut cursor, "skills")? {
        cursor.i32()?;
        cursor.f32()?;
        if skill_version >= 2 {
            cursor.f32()?;
        }
    }
    let mut bypass = false;
    for _ in 0..character_count(&mut cursor, "player custom data")? {
        let key = cursor.string()?;
        let value = cursor.string()?;
        if key == "bypasscheatchecks" && value == "1" {
            bypass = true;
        }
    }
    cursor.f32()?;
    cursor.f32()?;
    cursor.f32()?;
    cursor.byte_array()?;
    if cursor.position != data.len() {
        return Err(error(format!(
            "playerData has {} trailing bytes",
            data.len() - cursor.position
        )));
    }
    Ok(PlayerDataSummary {
        version: Some(version),
        inventory_version: Some(item_version as i32),
        inventory_item_count: item_count as u32,
        cheated_inventory_count: cheated_items,
        bypass_cheat_checks: bypass,
        complete: true,
    })
}

const SHA512_K: [u64; 80] = [
    0x428a2f98d728ae22,
    0x7137449123ef65cd,
    0xb5c0fbcfec4d3b2f,
    0xe9b5dba58189dbbc,
    0x3956c25bf348b538,
    0x59f111f1b605d019,
    0x923f82a4af194f9b,
    0xab1c5ed5da6d8118,
    0xd807aa98a3030242,
    0x12835b0145706fbe,
    0x243185be4ee4b28c,
    0x550c7dc3d5ffb4e2,
    0x72be5d74f27b896f,
    0x80deb1fe3b1696b1,
    0x9bdc06a725c71235,
    0xc19bf174cf692694,
    0xe49b69c19ef14ad2,
    0xefbe4786384f25e3,
    0x0fc19dc68b8cd5b5,
    0x240ca1cc77ac9c65,
    0x2de92c6f592b0275,
    0x4a7484aa6ea6e483,
    0x5cb0a9dcbd41fbd4,
    0x76f988da831153b5,
    0x983e5152ee66dfab,
    0xa831c66d2db43210,
    0xb00327c898fb213f,
    0xbf597fc7beef0ee4,
    0xc6e00bf33da88fc2,
    0xd5a79147930aa725,
    0x06ca6351e003826f,
    0x142929670a0e6e70,
    0x27b70a8546d22ffc,
    0x2e1b21385c26c926,
    0x4d2c6dfc5ac42aed,
    0x53380d139d95b3df,
    0x650a73548baf63de,
    0x766a0abb3c77b2a8,
    0x81c2c92e47edaee6,
    0x92722c851482353b,
    0xa2bfe8a14cf10364,
    0xa81a664bbc423001,
    0xc24b8b70d0f89791,
    0xc76c51a30654be30,
    0xd192e819d6ef5218,
    0xd69906245565a910,
    0xf40e35855771202a,
    0x106aa07032bbd1b8,
    0x19a4c116b8d2d0c8,
    0x1e376c085141ab53,
    0x2748774cdf8eeb99,
    0x34b0bcb5e19b48a8,
    0x391c0cb3c5c95a63,
    0x4ed8aa4ae3418acb,
    0x5b9cca4f7763e373,
    0x682e6ff3d6b2b8a3,
    0x748f82ee5defb2fc,
    0x78a5636f43172f60,
    0x84c87814a1f0ab72,
    0x8cc702081a6439ec,
    0x90befffa23631e28,
    0xa4506cebde82bde9,
    0xbef9a3f7b2c67915,
    0xc67178f2e372532b,
    0xca273eceea26619c,
    0xd186b8c721c0c207,
    0xeada7dd6cde0eb1e,
    0xf57d4f7fee6ed178,
    0x06f067aa72176fba,
    0x0a637dc5a2c898a6,
    0x113f9804bef90dae,
    0x1b710b35131c471b,
    0x28db77f523047d84,
    0x32caab7b40c72493,
    0x3c9ebe0a15c9bebc,
    0x431d67c49c100d4c,
    0x4cc5d4becb3e42b6,
    0x597f299cfc657e2a,
    0x5fcb6fab3ad6faec,
    0x6c44198c4a475817,
];

fn sha512(bytes: &[u8]) -> [u8; 64] {
    let mut h: [u64; 8] = [
        0x6a09e667f3bcc908,
        0xbb67ae8584caa73b,
        0x3c6ef372fe94f82b,
        0xa54ff53a5f1d36f1,
        0x510e527fade682d1,
        0x9b05688c2b3e6c1f,
        0x1f83d9abfb41bd6b,
        0x5be0cd19137e2179,
    ];
    let bit_len = (bytes.len() as u128) * 8;
    let padded_len = (bytes.len() + 17).div_ceil(128) * 128;
    let mut data = vec![0u8; padded_len];
    data[..bytes.len()].copy_from_slice(bytes);
    data[bytes.len()] = 0x80;
    data[padded_len - 16..].copy_from_slice(&bit_len.to_be_bytes());
    for block in data.as_chunks::<128>().0 {
        let mut w = [0u64; 80];
        for (i, word) in w[..16].iter_mut().enumerate() {
            *word = u64::from_be_bytes(block[i * 8..i * 8 + 8].try_into().unwrap());
        }
        for i in 16..80 {
            let x = w[i - 15];
            let y = w[i - 2];
            let s0 = x.rotate_right(1) ^ x.rotate_right(8) ^ (x >> 7);
            let s1 = y.rotate_right(19) ^ y.rotate_right(61) ^ (y >> 6);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut a = h[0];
        let mut b = h[1];
        let mut c = h[2];
        let mut d = h[3];
        let mut e = h[4];
        let mut f = h[5];
        let mut g = h[6];
        let mut hh = h[7];
        for i in 0..80 {
            let s1 = e.rotate_right(14) ^ e.rotate_right(18) ^ e.rotate_right(41);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(SHA512_K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(28) ^ a.rotate_right(34) ^ a.rotate_right(39);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    let mut output = [0u8; 64];
    for (i, word) in h.iter().enumerate() {
        output[i * 8..i * 8 + 8].copy_from_slice(&word.to_be_bytes());
    }
    output
}

#[derive(Debug, Clone, Default)]
pub struct CharacterScan {
    pub source: String,
    pub canonical: bool,
    pub canonical_eligible: bool,
    pub input_id: u32,
    pub modified_unix: Option<i64>,
    pub file_bytes: u64,
    pub payload_bytes: u64,
    pub hash_bytes: u64,
    pub hash_valid: bool,
    pub trusted: bool,
    pub supported: bool,
    pub parse_error: Option<String>,
    pub profile_version: i32,
    pub stat_slots: i32,
    pub stat_profiles: i32,
    pub player_name: String,
    pub player_id: Option<i64>,
    pub used_cheats: bool,
    pub cheat_stat_nonzero_count: u32,
    pub known_command_hits: u32,
    pub player_data_version: Option<i32>,
    pub inventory_version: Option<i32>,
    pub inventory_item_count: u32,
    pub cheated_inventory_count: u32,
    pub bypass_cheat_checks: bool,
    pub player_data_complete: bool,
}

pub fn scan_character_bytes(bytes: &[u8], source: &str) -> Result<CharacterScan, ScanError> {
    let mut cursor = Cursor::new(bytes);
    let payload_len = checked_len(cursor.i32()?, "character payload")?;
    let payload = cursor.take(payload_len)?.to_vec();
    let hash_len = checked_len(cursor.i32()?, "character hash")?;
    let stored_hash = cursor.take(hash_len)?.to_vec();
    if cursor.position != bytes.len() {
        return Err(error("character wrapper has trailing bytes"));
    }
    let hash_valid = hash_len == 64 && stored_hash == sha512(&payload);
    let mut result = if !hash_valid {
        CharacterScan {
            source: String::new(),
            canonical: false,
            canonical_eligible: true,
            input_id: 0,
            modified_unix: None,
            file_bytes: 0,
            payload_bytes: payload.len() as u64,
            hash_bytes: 0,
            hash_valid: false,
            trusted: false,
            supported: false,
            parse_error: Some("SHA-512 verification failed".to_string()),
            profile_version: 0,
            stat_slots: 0,
            stat_profiles: 0,
            player_name: String::new(),
            player_id: None,
            used_cheats: false,
            cheat_stat_nonzero_count: 0,
            known_command_hits: 0,
            player_data_version: None,
            inventory_version: None,
            inventory_item_count: 0,
            cheated_inventory_count: 0,
            bypass_cheat_checks: false,
            player_data_complete: false,
        }
    } else {
        match parse_character_profile(&payload) {
            Ok(result) => result,
            Err(parse_error) => CharacterScan {
                source: String::new(),
                canonical: false,
                canonical_eligible: true,
                input_id: 0,
                modified_unix: None,
                file_bytes: 0,
                payload_bytes: payload.len() as u64,
                hash_bytes: 0,
                hash_valid: true,
                trusted: false,
                supported: false,
                parse_error: Some(parse_error.to_string()),
                profile_version: payload
                    .get(..4)
                    .and_then(|bytes| bytes.try_into().ok())
                    .map(i32::from_le_bytes)
                    .unwrap_or_default(),
                stat_slots: 0,
                stat_profiles: 0,
                player_name: String::new(),
                player_id: None,
                used_cheats: false,
                cheat_stat_nonzero_count: 0,
                known_command_hits: 0,
                player_data_version: None,
                inventory_version: None,
                inventory_item_count: 0,
                cheated_inventory_count: 0,
                bypass_cheat_checks: false,
                player_data_complete: false,
            },
        }
    };
    if hash_valid && result.parse_error.is_none() {
        result.trusted = true;
    }
    result.source = source.to_string();
    result.file_bytes = bytes.len() as u64;
    result.hash_bytes = hash_len as u64;
    result.hash_valid = hash_valid;
    Ok(result)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn scan_character(path: &Path) -> Result<CharacterScan, ScanError> {
    let bytes = std::fs::read(path)
        .map_err(|e| error(format!("cannot read character {}: {e}", path.display())))?;
    let mut result = scan_character_bytes(&bytes, &path.display().to_string())?;
    result.modified_unix = std::fs::metadata(path)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs() as i64);
    Ok(result)
}

#[cfg(not(target_arch = "wasm32"))]
fn same_path(left: &Path, right: &Path) -> bool {
    std::fs::canonicalize(left)
        .ok()
        .zip(std::fs::canonicalize(right).ok())
        .map(|(left, right)| left == right)
        .unwrap_or_else(|| left == right)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn scan_character_timeline(
    canonical: &Path,
    history_dir: &Path,
) -> Result<Vec<CharacterScan>, ScanError> {
    let mut paths = match std::fs::read_dir(history_dir) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.contains(".fch"))
            })
            .collect::<Vec<_>>(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(io_error) => {
            return Err(error(format!(
                "cannot read character history {}: {io_error}",
                history_dir.display()
            )))
        }
    };
    if canonical.exists() && !paths.iter().any(|path| same_path(path, canonical)) {
        paths.push(canonical.to_path_buf());
    }
    paths.sort_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()));
    let history_prefix = if canonical
        .parent()
        .is_some_and(|parent| same_path(parent, history_dir))
    {
        "character-saves"
    } else {
        "steam-cloud"
    };
    let mut scans = paths
        .iter()
        .map(|path| {
            let mut scan = scan_character(path)?;
            scan.canonical = same_path(path, canonical);
            let basename = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| error("character filename is not valid UTF-8"))?;
            scan.source = if scan.canonical {
                format!("character-saves/{basename}")
            } else {
                format!("{history_prefix}/{basename}")
            };
            Ok(scan)
        })
        .collect::<Result<Vec<_>, ScanError>>()?;
    scans.sort_by_key(|scan| (scan.modified_unix.unwrap_or_default(), scan.source.clone()));
    Ok(scans)
}

fn parse_direct_item_internal(
    bytes: &[u8],
) -> Result<(Option<ItemEvidence>, ItemSummary), ScanError> {
    if bytes.is_empty() {
        return Err(error("empty direct item payload"));
    }
    let mut cursor = Cursor::new(&bytes[1..]);
    let (summary, cheated) = scan_item(&mut cursor, bytes[0], false)?;
    let item = if cheated {
        // The version tag at bytes[0] is not part of the slice `decode_item` measured from, so its
        // offset is relative to `&bytes[1..]`; make it absolute within `bytes` by adding 1 back.
        decode_item(&bytes[1..cursor.position + 1], bytes[0], false)?.map(|mut item| {
            item.cheated_byte_offset = item.cheated_byte_offset.map(|pos| pos + 1);
            item
        })
    } else {
        None
    };
    Ok((item, summary))
}

fn base64_value(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

fn decode_base64(input: &[u8], output: &mut Vec<u8>) -> Result<(), ScanError> {
    output.clear();
    let mut quartet = [0u8; 4];
    let mut count = 0;
    for &byte in input.iter().filter(|byte| !byte.is_ascii_whitespace()) {
        quartet[count] = byte;
        count += 1;
        if count != 4 {
            continue;
        }
        let a = base64_value(quartet[0]).ok_or_else(|| error("invalid base64"))? as u32;
        let b = base64_value(quartet[1]).ok_or_else(|| error("invalid base64"))? as u32;
        output.push(((a << 2) | (b >> 4)) as u8);
        if quartet[2] != b'=' {
            let c = base64_value(quartet[2]).ok_or_else(|| error("invalid base64"))? as u32;
            output.push(((b << 4) | (c >> 2)) as u8);
            if quartet[3] != b'=' {
                let d = base64_value(quartet[3]).ok_or_else(|| error("invalid base64"))? as u32;
                output.push(((c << 6) | d) as u8);
            }
        }
        count = 0;
    }
    if count != 0 {
        return Err(error("invalid base64 length"));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Position {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Debug, Clone)]
pub struct Evidence {
    pub archive: String,
    pub snapshot: String,
    pub internal_path: String,
    pub format: String,
    pub chunk_filename: Option<String>,
    pub chunk_version: Option<i16>,
    pub chunk_size: Option<u8>,
    pub chunk_revision: Option<u32>,
    pub zdo_ordinal: u32,
    pub owner_prefab_hash: i32,
    pub owner_prefab_name: Option<String>,
    pub position: Position,
    pub legacy_sector: Option<(i16, i16)>,
    pub key_hash: i32,
    pub key_name: String,
    pub kind: String,
    pub item_hash: Option<i32>,
    pub item_name: Option<String>,
    pub grid: Option<(u8, u8)>,
    pub quality: Option<u16>,
    pub stack: Option<u16>,
    pub variant: Option<i32>,
    pub crafter_id: Option<i64>,
    pub crafter_name: Option<String>,
    pub world_level: Option<u8>,
    pub custom_keys: Vec<String>,
}

#[derive(Debug, Default)]
struct EvidenceCounts {
    decoded_item_count: u64,
    item_count: u64,
    direct_item_count: u64,
    container_item_count: u64,
    indexed_item_count: u64,
    zdo_cheated_count: u64,
    station_queued_cheated_count: u64,
}

#[derive(Debug, Default)]
pub struct ParseOutput {
    pub zdo_count: u64,
    pub evidence: Vec<Evidence>,
    counts: EvidenceCounts,
    /// ZDO count per world-map cell, keyed by (floor(x / MAP_CELL_METERS), floor(z / MAP_CELL_METERS)).
    pub grid: HashMap<(i32, i32), u32>,
    /// ZDO count per owner prefab hash, for habitat/prefab context.
    pub prefabs: HashMap<i32, u32>,
    /// Biome votes per world-map cell, from prefabs the game tags with a biome (real tags plus
    /// flora-hint heuristics).
    pub biomes: HashMap<(i32, i32), BiomeTally>,
    /// Same tally restricted to real, game-tagged votes only (no flora hints). See
    /// `biome_verdict_preferring_real`.
    pub biomes_real: HashMap<(i32, i32), BiomeTally>,
}

/// Biome flag bits, matching the game's `Heightmap.Biome` enum (see FORMAT.md). The order fixes the
/// index used by the map layer and its legend. The game's bits run to `0x200`, so this is a `u16`.
const BIOMES: [(u16, &str); 9] = [
    (0x01, "meadows"),
    (0x02, "swamp"),
    (0x04, "mountain"),
    (0x08, "blackforest"),
    (0x10, "plains"),
    (0x20, "ashlands"),
    (0x40, "deepnorth"),
    (0x100, "ocean"),
    (0x200, "mistlands"),
];
const BIOME_COUNT: usize = BIOMES.len();
/// Per-cell biome votes, with the total weight in the last slot.
type BiomeTally = [u32; BIOME_COUNT + 1];
const BIOME_TOTAL: usize = BIOME_COUNT;

/// A prefab the game tags with several biomes carries proportionally less information, so weight it
/// `12 / biomes` (a single-biome object is worth 12, a two-biome object 6, nine biomes 1).
fn biome_weight(mask: u16) -> u32 {
    12u32
        .checked_div(mask.count_ones())
        .map_or(0, |weight| weight.max(1))
}

fn add_biome_votes(tally: &mut BiomeTally, mask: u16, weight: u32) {
    tally[BIOME_TOTAL] = tally[BIOME_TOTAL].saturating_add(weight);
    for (index, (bit, _)) in BIOMES.iter().enumerate() {
        if mask & bit != 0 {
            tally[index] = tally[index].saturating_add(weight);
        }
    }
}

/// A cell is coloured only with three single-biome objects' worth of weight and a 60% share of it;
/// anything weaker stays uncoloured rather than guessed (undeveloped, ocean and unexplored cells).
const BIOME_MIN_WEIGHT: u32 = 36;
const BIOME_MIN_SHARE_PERCENT: u32 = 60;

/// Dominant biome for a cell and its share of the votes, or None when the evidence is too thin.
fn biome_verdict(tally: &BiomeTally) -> Option<(usize, u32)> {
    let total = tally[BIOME_TOTAL];
    if total < BIOME_MIN_WEIGHT {
        return None;
    }
    let (index, votes) = tally[..BIOME_COUNT]
        .iter()
        .enumerate()
        .max_by_key(|(_, votes)| **votes)?;
    if *votes == 0 {
        return None;
    }
    let share = votes * 100 / total;
    (share >= BIOME_MIN_SHARE_PERCENT).then_some((index, share))
}

/// Prefer a verdict reached from real, game-tagged evidence alone; fall back to the combined tally
/// (real plus flora-hint votes) only when the real evidence in that cell is too thin to decide on its
/// own. Without this, dense flora hints (every tree in a forested cell casts a vote) can outvote the
/// naturally sparse single-biome markers a real biome like mountain relies on (ore, named monsters) in
/// a cell that borders both.
fn biome_verdict_preferring_real(real: &BiomeTally, combined: &BiomeTally) -> Option<(usize, u32)> {
    biome_verdict(real).or_else(|| biome_verdict(combined))
}

/// World-map aggregation cell size in metres. 64 m matches a Valheim zone.
pub const MAP_CELL_METERS: f32 = 64.0;

/// Positions beyond this are not places. A real save carried 82 such ZDOs out of
/// 809,753 (sentinel/garbage coordinates); they still count toward `zdo_count`
/// but are kept off the map so one outlier cannot stretch the whole view.
pub const MAP_WORLD_LIMIT_METERS: f32 = 16384.0;

/// Bin every ZDO by position and prefab. This runs for all ZDOs, not just
/// evidence, so the result shows where the world actually has content.
fn record_zdo_spatial(output: &mut ParseOutput, info: &ZdoInfo<'_>) {
    let p = info.position;
    if p.x.is_finite()
        && p.z.is_finite()
        && p.x.abs() <= MAP_WORLD_LIMIT_METERS
        && p.z.abs() <= MAP_WORLD_LIMIT_METERS
    {
        let cell = (
            (p.x / MAP_CELL_METERS).floor() as i32,
            (p.z / MAP_CELL_METERS).floor() as i32,
        );
        *output.grid.entry(cell).or_default() += 1;
        let mask = info.prefabs.biome_mask(info.owner_prefab_hash);
        let weight = biome_weight(mask);
        if weight > 0 {
            add_biome_votes(output.biomes.entry(cell).or_default(), mask, weight);
        }
        let real_mask = info.prefabs.real_biome_mask(info.owner_prefab_hash);
        let real_weight = biome_weight(real_mask);
        if real_weight > 0 {
            add_biome_votes(
                output.biomes_real.entry(cell).or_default(),
                real_mask,
                real_weight,
            );
        }
    }
    *output.prefabs.entry(info.owner_prefab_hash).or_default() += 1;
}

fn push_evidence(output: &mut ParseOutput, evidence: Evidence) {
    match evidence.kind.as_str() {
        "direct_item_data" => {
            output.counts.item_count += 1;
            output.counts.direct_item_count += 1;
        }
        "container_inventory" => {
            output.counts.item_count += 1;
            output.counts.container_item_count += 1;
        }
        "indexed_item_data" => {
            output.counts.item_count += 1;
            output.counts.indexed_item_count += 1;
        }
        "zdo_cheated" => output.counts.zdo_cheated_count += 1,
        "station_queued_cheated" => output.counts.station_queued_cheated_count += 1,
        _ => {}
    }
    output.evidence.push(evidence);
}

struct ZdoInfo<'a> {
    archive: &'a str,
    snapshot: &'a str,
    internal_path: &'a str,
    format: &'a str,
    chunk_filename: Option<&'a str>,
    chunk_version: Option<i16>,
    chunk_size: Option<u8>,
    chunk_revision: Option<u32>,
    ordinal: u32,
    prefabs: &'a PrefabNames,
    position: Position,
    legacy_sector: Option<(i16, i16)>,
    owner_prefab_hash: i32,
}

fn make_evidence(info: &ZdoInfo<'_>, key_hash: i32, key_name: String, kind: &str) -> Evidence {
    Evidence {
        archive: info.archive.to_string(),
        snapshot: info.snapshot.to_string(),
        internal_path: info.internal_path.to_string(),
        format: info.format.to_string(),
        chunk_filename: info.chunk_filename.map(str::to_string),
        chunk_version: info.chunk_version,
        chunk_size: info.chunk_size,
        chunk_revision: info.chunk_revision,
        zdo_ordinal: info.ordinal,
        owner_prefab_hash: info.owner_prefab_hash,
        owner_prefab_name: info
            .prefabs
            .name(info.owner_prefab_hash)
            .map(str::to_string),
        position: info.position,
        legacy_sector: info.legacy_sector,
        key_hash,
        key_name,
        kind: kind.to_string(),
        item_hash: None,
        item_name: None,
        grid: None,
        quality: None,
        stack: None,
        variant: None,
        crafter_id: None,
        crafter_name: None,
        world_level: None,
        custom_keys: Vec::new(),
    }
}

fn add_item_hits(
    output: &mut ParseOutput,
    info: &ZdoInfo<'_>,
    key_hash: i32,
    key_name: String,
    kind: &str,
    items: Vec<ItemEvidence>,
) {
    for item in items {
        let mut evidence = make_evidence(info, key_hash, key_name.clone(), kind);
        evidence.item_hash = item.item_hash;
        evidence.item_name = item
            .item_hash
            .and_then(|hash| info.prefabs.name(hash).map(str::to_string));
        evidence.grid = item.grid;
        evidence.quality = Some(item.quality);
        evidence.stack = Some(item.stack);
        evidence.variant = Some(item.variant);
        evidence.crafter_id = item.crafter_id;
        evidence.crafter_name = item.crafter_name;
        evidence.world_level = Some(item.world_level);
        evidence.custom_keys = item.custom_keys;
        push_evidence(output, evidence);
    }
}

/// Records the scrub patch for one cheated item's trailing flags byte (container `items` slot or a
/// direct `itemData`/`N_itemData` entry): `array_start` is this byte array's offset within the
/// chunk file, `array` is its already-read bytes, and `hit` is the decoded cheated item. Fails
/// closed rather than guessing if the parser didn't record where that byte is, or if bit 0 of the
/// byte the offset points at is not actually set (both would mean the parser and the raw bytes
/// disagree, which must never be silently patched over).
fn push_item_cheated_bit_patch(
    patches: &mut Vec<ScrubPatch>,
    info: &ZdoInfo<'_>,
    key_name: &str,
    array_start: u64,
    array: &[u8],
    hit: &ItemEvidence,
) -> Result<(), ScanError> {
    let rel = hit
        .cheated_byte_offset
        .ok_or_else(|| error("cheated item hit is missing its recorded byte offset"))?;
    let old_value = *array
        .get(rel)
        .ok_or_else(|| error("cheated item byte offset is out of range"))?;
    if old_value & 1 == 0 {
        return Err(error(
            "cheated item byte offset does not point at a set cheated bit",
        ));
    }
    patches.push(ScrubPatch {
        file_path: String::new(),
        offset: array_start + rel as u64,
        kind: ScrubPatchKind::ItemCheatedBit,
        key_name: key_name.to_string(),
        owner_prefab_name: info
            .prefabs
            .name(info.owner_prefab_hash)
            .map(str::to_string),
        position: info.position,
        old_value: old_value as i64,
        new_value: (old_value & 0xfe) as i64,
    });
    Ok(())
}

fn read_small_rotation<R: Read>(reader: &mut R) -> Result<(), ScanError> {
    let first = read_u16(reader)?;
    if first & 0x8000 == 0 {
        read_u16(reader)?;
    }
    Ok(())
}

fn read_vec3<R: Read>(reader: &mut R) -> Result<Position, ScanError> {
    Ok(Position {
        x: read_f32(reader)?,
        y: read_f32(reader)?,
        z: read_f32(reader)?,
    })
}

// One more parameter (`patches`) than clippy's default limit; splitting the reusable scratch
// buffers, per-ZDO context and scrub sink into a struct would be pure ceremony for a function with
// exactly one call site's worth of real structure.
#[allow(clippy::too_many_arguments)]
fn parse_zdo<R: Read>(
    reader: &mut CountingReader<R>,
    current: bool,
    legacy_count: bool,
    info_without_position: &ZdoInfo<'_>,
    output: &mut ParseOutput,
    scratch: &mut Vec<u8>,
    base64_scratch: &mut Vec<u8>,
    sink: &mut ScrubSink,
) -> Result<(), ScanError> {
    let record_start = reader.pos();
    let mut object_flags = 0usize;
    let mut item_int = None;
    let mut type_int = None;
    let mut arrays = Vec::new();
    let flags = read_u16(reader)?;
    let (position, legacy_sector) = if current {
        let position = if flags & FLAG_SMALL_POSITION != 0 {
            Position {
                x: read_i16(reader)? as f32,
                y: 0.0,
                z: read_i16(reader)? as f32,
            }
        } else {
            read_vec3(reader)?
        };
        (position, None)
    } else {
        let sector = (read_i16(reader)?, read_i16(reader)?);
        (read_vec3(reader)?, Some(sector))
    };
    let owner_prefab_hash = read_i32(reader)?;
    if flags & FLAG_ROTATION != 0 {
        if current {
            read_small_rotation(reader)?;
        } else {
            read_vec3(reader)?;
        }
    }
    let info = ZdoInfo {
        position,
        legacy_sector,
        owner_prefab_hash,
        ..*info_without_position
    };
    record_zdo_spatial(output, &info);
    if flags & FLAG_CONNECTION != 0 {
        read_u8(reader)?;
        read_i32(reader)?;
    }
    if flags & FLAG_FLOATS != 0 {
        let count = if legacy_count {
            read_legacy_count(reader)?
        } else {
            read_num_items(reader)?
        };
        for _ in 0..count {
            read_i32(reader)?;
            read_f32(reader)?;
        }
    }
    if flags & FLAG_VEC3 != 0 {
        let count = if legacy_count {
            read_legacy_count(reader)?
        } else {
            read_num_items(reader)?
        };
        for _ in 0..count {
            read_i32(reader)?;
            read_vec3(reader)?;
        }
    }
    if flags & FLAG_QUATS != 0 {
        let count = if legacy_count {
            read_legacy_count(reader)?
        } else {
            read_num_items(reader)?
        };
        for _ in 0..count {
            read_i32(reader)?;
            for _ in 0..4 {
                read_f32(reader)?;
            }
        }
    }
    if flags & FLAG_INTS != 0 {
        let count = if legacy_count {
            read_legacy_count(reader)?
        } else {
            read_num_items(reader)?
        };
        for _ in 0..count {
            let key = read_i32(reader)?;
            let value_offset = reader.pos();
            let value = read_i32(reader)?;
            if key == ITEM_INT {
                item_int = Some((value_offset, value));
            } else if key == TYPE_INT {
                type_int = Some((value_offset, value));
            }
            if value == 0 {
                continue;
            }
            if key == CHEATED {
                object_flags += 1;
                sink.patches.push(ScrubPatch {
                    file_path: String::new(),
                    offset: value_offset,
                    kind: ScrubPatchKind::ZdoIntFlag,
                    key_name: "cheated".to_string(),
                    owner_prefab_name: info
                        .prefabs
                        .name(info.owner_prefab_hash)
                        .map(str::to_string),
                    position: info.position,
                    old_value: value as i64,
                    new_value: 0,
                });
                push_evidence(
                    output,
                    make_evidence(&info, key, "cheated".to_string(), "zdo_cheated"),
                );
            } else if key == CHEATED_QUEUED || (key > CHEATED_QUEUED && key <= CHEATED_QUEUED + 32)
            {
                let key_name = if key == CHEATED_QUEUED {
                    "cheatedQueued".to_string()
                } else {
                    format!("cheatedQueued+{}", key - CHEATED_QUEUED)
                };
                object_flags += 1;
                sink.patches.push(ScrubPatch {
                    file_path: String::new(),
                    offset: value_offset,
                    kind: ScrubPatchKind::ZdoIntFlag,
                    key_name: key_name.clone(),
                    owner_prefab_name: info
                        .prefabs
                        .name(info.owner_prefab_hash)
                        .map(str::to_string),
                    position: info.position,
                    old_value: value as i64,
                    new_value: 0,
                });
                push_evidence(
                    output,
                    make_evidence(&info, key, key_name, "station_queued_cheated"),
                );
            }
        }
    }
    if flags & FLAG_LONGS != 0 {
        let count = if legacy_count {
            read_legacy_count(reader)?
        } else {
            read_num_items(reader)?
        };
        for _ in 0..count {
            read_i32(reader)?;
            read_i64(reader)?;
        }
    }
    if flags & FLAG_STRINGS != 0 {
        let count = if legacy_count {
            read_legacy_count(reader)?
        } else {
            read_num_items(reader)?
        };
        for _ in 0..count {
            let key = read_i32(reader)?;
            if !current && key == ITEMS {
                read_dotnet_bytes(reader, scratch)?;
                decode_base64(scratch, base64_scratch)?;
                let (hits, _decoded) = parse_inventory_internal(base64_scratch, true, |_, _| {})?;
                output.counts.decoded_item_count += 1;
                add_item_hits(
                    output,
                    &info,
                    key,
                    "items".to_string(),
                    "container_inventory",
                    hits,
                );
            } else {
                skip_dotnet_string(reader)?;
            }
        }
    }
    if flags & FLAG_BYTE_ARRAYS != 0 {
        let count = if legacy_count {
            read_legacy_count(reader)?
        } else {
            read_num_items(reader)?
        };
        for _ in 0..count {
            let key = read_i32(reader)?;
            let relevant =
                key == ITEMS || key == ITEM_DATA || info.prefabs.indexed_item_data(key).is_some();
            if !relevant {
                let length = read_i32(reader)?;
                let length = checked_len(length, "byte array")?;
                skip_bytes(reader, length as u64)?;
                continue;
            }
            let length = checked_len(read_i32(reader)?, "byte array")?;
            let array_start = reader.pos();
            scratch.resize(length, 0);
            read_exact(reader, scratch)?;
            let mut items = Vec::new();
            if key == ITEMS {
                let (hits, _decoded) =
                    parse_inventory_internal(scratch, legacy_count, |summary, cheated| {
                        items.push(ItemLayout {
                            start: summary.start as u64,
                            end: summary.end as u64,
                            cheated,
                            cheated_byte: None,
                            item_hash: None,
                        });
                    })?;
                output.counts.decoded_item_count += 1;
                // `hits` holds the cheated items in order; attach each to its layout.
                for (layout, hit) in items.iter_mut().filter(|item| item.cheated).zip(&hits) {
                    layout.cheated_byte =
                        hit.cheated_byte_offset.map(|pos| array_start + pos as u64);
                    layout.item_hash = hit.item_hash;
                }
                for hit in &hits {
                    push_item_cheated_bit_patch(
                        &mut sink.patches,
                        &info,
                        "items",
                        array_start,
                        scratch,
                        hit,
                    )?;
                }
                add_item_hits(
                    output,
                    &info,
                    key,
                    "items".to_string(),
                    "container_inventory",
                    hits,
                );
            } else {
                let indexed = info.prefabs.indexed_item_data(key);
                let kind = if indexed.is_some() {
                    "indexed_item_data"
                } else {
                    "direct_item_data"
                };
                let key_name = if let Some(index) = indexed {
                    format!("{index}_itemData")
                } else {
                    "itemData".to_string()
                };
                let (item, summary) = parse_direct_item_internal(scratch)?;
                // The summary's range is relative to the data after the version byte.
                items.push(ItemLayout {
                    start: summary.start as u64 + 1,
                    end: summary.end as u64 + 1,
                    cheated: item.is_some(),
                    cheated_byte: item
                        .as_ref()
                        .and_then(|item| item.cheated_byte_offset)
                        .map(|pos| array_start + pos as u64),
                    item_hash: item.as_ref().and_then(|item| item.item_hash),
                });
                if let Some(item) = item {
                    push_item_cheated_bit_patch(
                        &mut sink.patches,
                        &info,
                        &key_name,
                        array_start,
                        scratch,
                        &item,
                    )?;
                    add_item_hits(output, &info, key, key_name, kind, vec![item]);
                }
            }
            arrays.push(ArrayLayout {
                key,
                len_offset: array_start - 4,
                len: length as u64,
                items,
            });
        }
    }
    if let Some(layouts) = sink.layouts.as_mut() {
        layouts.push(ZdoLayout {
            start: record_start,
            end: reader.pos(),
            owner_prefab_hash,
            position,
            object_flags,
            item_int,
            type_int,
            arrays,
        });
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn parse_chunk_reader<R: Read>(
    reader: &mut R,
    entry_bytes: u64,
    archive: &str,
    snapshot: &str,
    internal_path: &str,
    prefabs: &PrefabNames,
    metadata: &[ChunkMeta],
    sink: &mut ScrubSink,
) -> Result<ParseOutput, ScanError> {
    let mut reader = CountingReader::new(reader);
    let reader = &mut reader;
    let version = read_i16(reader)?;
    if version != 41 {
        return Err(error(format!(
            "expected current chunk version 41, found {version}"
        )));
    }
    let count = read_i32(reader)?;
    if count < 0 {
        return Err(error(format!("negative chunk ZDO count {count}")));
    }
    let path_name = internal_path.rsplit('/').next().unwrap_or(internal_path);
    let (chunk_size, _file_version, chunk_index) = parse_chunk_name(path_name)?;
    let revision = metadata
        .iter()
        .find(|meta| meta.chunk_index == chunk_index && meta.chunk_size == chunk_size)
        .map(|meta| meta.revision);
    let mut output = ParseOutput::default();
    let mut scratch = Vec::new();
    let mut base64_scratch = Vec::new();
    let info = ZdoInfo {
        archive,
        snapshot,
        internal_path,
        format: "chunked_v41",
        chunk_filename: Some(path_name),
        chunk_version: Some(version),
        chunk_size: Some(chunk_size),
        chunk_revision: revision,
        ordinal: 0,
        prefabs,
        position: Position::default(),
        legacy_sector: None,
        owner_prefab_hash: 0,
    };
    for ordinal in 1..=count as u32 {
        let per_zdo = info.clone_for_ordinal(ordinal);
        parse_zdo(
            reader,
            true,
            false,
            &per_zdo,
            &mut output,
            &mut scratch,
            &mut base64_scratch,
            sink,
        )?;
        output.zdo_count += 1;
    }
    let _ = entry_bytes;
    Ok(output)
}

impl<'a> ZdoInfo<'a> {
    fn clone_for_ordinal(&self, ordinal: u32) -> ZdoInfo<'a> {
        ZdoInfo { ordinal, ..*self }
    }
}

fn parse_legacy_reader<R: Read>(
    reader: &mut R,
    archive: &str,
    snapshot: &str,
    internal_path: &str,
    prefabs: &PrefabNames,
) -> Result<ParseOutput, ScanError> {
    let mut reader = CountingReader::new(reader);
    let reader = &mut reader;
    // Legacy v37 worlds are not a scrub target (see `scrub_world_tar_bytes`); patches found here
    // are always discarded.
    let mut sink = ScrubSink::default();
    let version = read_i32(reader)?;
    if version != 37 {
        return Err(error(format!(
            "expected legacy version 37, found {version}"
        )));
    }
    let _net_time = read_f64(reader)?;
    let _old_session_id = read_i64(reader)?;
    let _next_uid = read_u32(reader)?;
    let count = read_i32(reader)?;
    if count < 0 {
        return Err(error(format!("negative legacy ZDO count {count}")));
    }
    let mut output = ParseOutput::default();
    let mut scratch = Vec::new();
    let mut base64_scratch = Vec::new();
    let info = ZdoInfo {
        archive,
        snapshot,
        internal_path,
        format: "legacy_v37",
        chunk_filename: None,
        chunk_version: None,
        chunk_size: None,
        chunk_revision: None,
        ordinal: 0,
        prefabs,
        position: Position::default(),
        legacy_sector: None,
        owner_prefab_hash: 0,
    };
    for ordinal in 1..=count as u32 {
        let per_zdo = info.clone_for_ordinal(ordinal);
        parse_zdo(
            reader,
            false,
            true,
            &per_zdo,
            &mut output,
            &mut scratch,
            &mut base64_scratch,
            &mut sink,
        )?;
        output.zdo_count += 1;
    }
    Ok(output)
}

fn parse_chunk_name(name: &str) -> Result<(u8, u32, u16), ScanError> {
    let stem = name
        .strip_suffix(".chunk")
        .ok_or_else(|| error(format!("not a chunk filename: {name}")))?;
    let (coords, version) = stem
        .split_once("__")
        .ok_or_else(|| error(format!("invalid chunk filename: {name}")))?;
    let (y, x) = coords
        .split_once('_')
        .ok_or_else(|| error(format!("invalid chunk coordinates: {name}")))?;
    let (size, file_version) = version
        .split_once('_')
        .ok_or_else(|| error(format!("invalid chunk version: {name}")))?;
    let y = u16::from_str_radix(y, 16).map_err(|_| error(format!("invalid chunk y: {name}")))?;
    let x = u16::from_str_radix(x, 16).map_err(|_| error(format!("invalid chunk x: {name}")))?;
    let size = size
        .parse::<u8>()
        .map_err(|_| error(format!("invalid chunk size: {name}")))?;
    let file_version = file_version
        .parse::<u32>()
        .map_err(|_| error(format!("invalid chunk file version: {name}")))?;
    Ok((size, file_version, (y << 8) | x))
}

#[derive(Debug, Clone)]
struct ChunkMeta {
    chunk_index: u16,
    chunk_size: u8,
    revision: u32,
    _count: i32,
}

fn parse_chunks_metadata<R: Read>(reader: &mut R) -> Result<(i16, i32, Vec<ChunkMeta>), ScanError> {
    let version = read_i16(reader)?;
    let total = read_i32(reader)?;
    let entries = read_i32(reader)?;
    if !(0..=100_000).contains(&entries) {
        return Err(error(format!(
            "invalid chunk metadata entry count {entries}"
        )));
    }
    let mut metadata = Vec::with_capacity(entries as usize);
    for _ in 0..entries {
        metadata.push(ChunkMeta {
            chunk_index: read_u16(reader)?,
            chunk_size: read_u8(reader)?,
            revision: read_u32(reader)?,
            _count: read_i32(reader)?,
        });
    }
    Ok((version, total, metadata))
}

struct EntryHeader {
    path: [u8; 100],
    size: u64,
    kind: u8,
}

impl EntryHeader {
    fn path(&self) -> &[u8] {
        let end = self
            .path
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(self.path.len());
        &self.path[..end]
    }
}

struct LimitedReader<'a, R: Read> {
    reader: &'a mut R,
    remaining: u64,
}

impl<R: Read> Read for LimitedReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.remaining == 0 || buf.is_empty() {
            return Ok(0);
        }
        let wanted = self.remaining.min(buf.len() as u64) as usize;
        let read = self.reader.read(&mut buf[..wanted])?;
        self.remaining -= read as u64;
        Ok(read)
    }
}

impl<R: Read> LimitedReader<'_, R> {
    fn drain(&mut self) -> Result<(), ScanError> {
        skip_bytes(self, self.remaining)
    }
}

struct TarStream<R: Read> {
    reader: R,
    ended: bool,
}

impl<R: Read> TarStream<R> {
    fn new(reader: R) -> Self {
        Self {
            reader,
            ended: false,
        }
    }

    fn next_header(&mut self) -> Result<Option<EntryHeader>, ScanError> {
        if self.ended {
            return Ok(None);
        }
        let mut block = [0u8; 512];
        self.reader
            .read_exact(&mut block)
            .map_err(|io_error| error(format!("truncated tar header: {io_error}")))?;
        if block.iter().all(|byte| *byte == 0) {
            let mut second = [0u8; 512];
            self.reader
                .read_exact(&mut second)
                .map_err(|io_error| error(format!("truncated tar end marker: {io_error}")))?;
            if !second.iter().all(|byte| *byte == 0) {
                return Err(error("tar requires two zero end blocks"));
            }
            self.ended = true;
            return Ok(None);
        }
        let stored_checksum = parse_tar_octal(&block[148..156])?;
        let actual_checksum = block
            .iter()
            .enumerate()
            .map(|(index, byte)| {
                if (148..156).contains(&index) {
                    b' ' as u64
                } else {
                    *byte as u64
                }
            })
            .sum::<u64>();
        if stored_checksum != actual_checksum {
            return Err(error(format!(
                "tar header checksum mismatch: expected {stored_checksum}, calculated {actual_checksum}"
            )));
        }
        let mut path = [0u8; 100];
        path.copy_from_slice(&block[..100]);
        let size = parse_tar_octal(&block[124..136])?;
        Ok(Some(EntryHeader {
            path,
            size,
            kind: block[156],
        }))
    }

    fn for_each_entry<F>(&mut self, mut callback: F) -> Result<(), ScanError>
    where
        F: FnMut(&EntryHeader, &mut LimitedReader<'_, R>) -> Result<(), ScanError>,
    {
        while let Some(header) = self.next_header()? {
            let mut body = LimitedReader {
                reader: &mut self.reader,
                remaining: header.size,
            };
            if header.kind == 0 || header.kind == b'0' {
                callback(&header, &mut body)?;
            }
            body.drain()?;
            let padding = (512 - (header.size % 512)) % 512;
            skip_bytes(&mut self.reader, padding)?;
        }
        Ok(())
    }

    fn reject_nonzero_trailing(&mut self) -> Result<(), ScanError> {
        let mut buffer = [0u8; 8192];
        loop {
            let read = self
                .reader
                .read(&mut buffer)
                .map_err(|io_error| error(format!("cannot read tar padding: {io_error}")))?;
            if read == 0 {
                return Ok(());
            }
            if buffer[..read].iter().any(|byte| *byte != 0) {
                return Err(error("tar contains nonzero trailing data"));
            }
        }
    }
}

fn parse_tar_octal(bytes: &[u8]) -> Result<u64, ScanError> {
    let mut value = 0u64;
    let mut saw_digit = false;
    for &byte in bytes {
        if byte == 0 || byte == b' ' {
            continue;
        }
        if !(b'0'..=b'7').contains(&byte) {
            return Err(error("invalid tar size"));
        }
        saw_digit = true;
        value = value
            .checked_mul(8)
            .and_then(|value| value.checked_add((byte - b'0') as u64))
            .ok_or_else(|| error("tar size overflow"))?;
    }
    if !saw_digit {
        Ok(0)
    } else {
        Ok(value)
    }
}

#[derive(Debug, Default)]
pub struct ArchiveScan {
    pub archive: String,
    pub snapshot: String,
    pub format: String,
    pub zdo_count: u64,
    pub decoded_item_count: u64,
    pub evidence: Vec<Evidence>,
    pub item_count: u64,
    pub direct_item_count: u64,
    pub container_item_count: u64,
    pub indexed_item_count: u64,
    pub zdo_cheated_count: u64,
    pub station_queued_cheated_count: u64,
    pub metadata_total: Option<i32>,
    pub metadata_entries: usize,
    pub player_profiles_present: bool,
    /// ZDO density per 64 m world cell, and prefab histogram, for the map view.
    pub grid: HashMap<(i32, i32), u32>,
    pub prefabs: HashMap<i32, u32>,
    /// Biome votes per 64 m world cell, tallied while parsing (see `record_zdo_spatial`).
    pub biomes: HashMap<(i32, i32), BiomeTally>,
    /// Same tally restricted to real, game-tagged votes (no flora hints).
    pub biomes_real: HashMap<(i32, i32), BiomeTally>,
    /// `.fwl2`/`.db2` world metadata. Player ids and character names are never recorded here.
    pub world: WorldMeta,
    pub world_metadata_error: Option<String>,
}

fn merge_parse(archive: &mut ArchiveScan, output: ParseOutput) {
    archive.zdo_count += output.zdo_count;
    archive.decoded_item_count += output.counts.decoded_item_count;
    archive.item_count += output.counts.item_count;
    archive.direct_item_count += output.counts.direct_item_count;
    archive.container_item_count += output.counts.container_item_count;
    archive.indexed_item_count += output.counts.indexed_item_count;
    archive.zdo_cheated_count += output.counts.zdo_cheated_count;
    archive.station_queued_cheated_count += output.counts.station_queued_cheated_count;
    archive.evidence.extend(output.evidence);
    for (cell, count) in output.grid {
        *archive.grid.entry(cell).or_default() += count;
    }
    for (cell, tally) in output.biomes {
        let target = archive.biomes.entry(cell).or_default();
        for (slot, value) in tally.iter().enumerate() {
            target[slot] = target[slot].saturating_add(*value);
        }
    }
    for (cell, tally) in output.biomes_real {
        let target = archive.biomes_real.entry(cell).or_default();
        for (slot, value) in tally.iter().enumerate() {
            target[slot] = target[slot].saturating_add(*value);
        }
    }
    for (hash, count) in output.prefabs {
        *archive.prefabs.entry(hash).or_default() += count;
    }
}

/// A world-progression flag read from the `.db2` global keys (`defeated_*`, `killed*`, `activebosses`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalKey {
    pub key: String,
    pub value: Option<i64>,
}

/// World-level metadata stored beside the chunks: `.fwl2` carries the world name, seed and player
/// count; `.db2` carries the progression flags. Player ids and character names are read only far
/// enough to count them and are deliberately never kept.
#[derive(Debug, Clone, Default)]
pub struct WorldMeta {
    pub version: Option<u32>,
    pub name: Option<String>,
    pub seed: Option<String>,
    pub player_count: Option<usize>,
    pub global_keys: Vec<GlobalKey>,
}

const MAX_WORLD_META_BYTES: u64 = 1024 * 1024;
const MAX_DB2_ENTRY_BYTES: u64 = 16 * 1024 * 1024;
const MAX_DB2_PLAIN_BYTES: usize = 64 * 1024 * 1024;
const MAX_GLOBAL_KEYS: usize = 64;

/// Global-key namespaces worth reporting. Anything else in the payload is ignored, which keeps
/// unrelated strings (including player names) out of reports.
const GLOBAL_KEY_PREFIXES: [&str; 6] = [
    "defeated_",
    "killed",
    "activebosses",
    "event_",
    "hildir",
    "bosshildir",
];

fn read_bounded(reader: &mut impl Read, limit: u64) -> Result<Vec<u8>, ScanError> {
    let mut bytes = Vec::new();
    reader.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(error(format!("entry exceeds the {limit} byte limit")));
    }
    Ok(bytes)
}

fn meta_u32(bytes: &[u8], offset: usize) -> Result<u32, ScanError> {
    let slice = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| error("truncated world metadata"))?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

/// Read one `[u8 length][utf-8 bytes]` string, advancing `offset`.
fn read_pstring(bytes: &[u8], offset: &mut usize) -> Result<String, ScanError> {
    let slice = read_pstring_bytes(bytes, offset)?;
    std::str::from_utf8(slice)
        .map(str::to_string)
        .map_err(|_| error("world metadata string is not utf-8"))
}

/// Same, but only printable ASCII is accepted. The `.db2` payload interleaves binary sections, so a
/// length byte read from a binary run must not be allowed to consume (and hide) the real keys.
fn read_ascii_pstring(bytes: &[u8], offset: &mut usize) -> Result<String, ScanError> {
    let slice = read_pstring_bytes(bytes, offset)?;
    if !slice.iter().all(|byte| (0x20..=0x7e).contains(byte)) {
        return Err(error("world metadata string is not printable ascii"));
    }
    Ok(String::from_utf8_lossy(slice).into_owned())
}

fn read_pstring_bytes<'a>(bytes: &'a [u8], offset: &mut usize) -> Result<&'a [u8], ScanError> {
    let length = *bytes
        .get(*offset)
        .ok_or_else(|| error("truncated world metadata string"))? as usize;
    *offset += 1;
    let slice = bytes
        .get(*offset..*offset + length)
        .ok_or_else(|| error("truncated world metadata string"))?;
    *offset += length;
    Ok(slice)
}

/// `.fwl2`: `[u32 declared length][u32 world version][name][seed][u64 uid]…[player list]`.
///
/// Only the *number* of players is returned: the entries themselves are Steam ids and character
/// names, which this scanner must never report.
pub fn parse_fwl2_bytes(bytes: &[u8]) -> Result<WorldMeta, ScanError> {
    if bytes.len() < 8 {
        return Err(error("fwl2 is too short"));
    }
    let declared = meta_u32(bytes, 0)? as usize;
    if declared != bytes.len() - 4 {
        return Err(error(format!(
            "fwl2 declares {declared} bytes but carries {}",
            bytes.len() - 4
        )));
    }
    let version = meta_u32(bytes, 4)?;
    let mut offset = 8;
    let name = read_pstring(bytes, &mut offset)?;
    let seed = read_pstring(bytes, &mut offset)?;
    Ok(WorldMeta {
        version: Some(version),
        name: Some(name),
        seed: Some(seed),
        player_count: detect_player_count(bytes, offset),
        global_keys: Vec::new(),
    })
}

/// The player list is four length-prefixed strings per player. Rather than trusting a fixed offset
/// (which our own reverse engineering could have got wrong), find the offset where that shape
/// consumes the rest of the file exactly; if no offset fits, report no count instead of a guess.
fn detect_player_count(bytes: &[u8], from: usize) -> Option<usize> {
    for start in from..bytes.len().min(from + 64) {
        let mut offset = start;
        let mut count = 0usize;
        loop {
            if offset == bytes.len() {
                return Some(count);
            }
            let mut first_field = 0usize;
            let mut ok = true;
            for slot in 0..4 {
                match read_pstring(bytes, &mut offset) {
                    Ok(value) if slot == 0 => first_field = value.len(),
                    Ok(_) => {}
                    Err(_) => {
                        ok = false;
                        break;
                    }
                }
            }
            if !ok || first_field == 0 {
                break;
            }
            count += 1;
        }
    }
    None
}

/// `.db2`: `[u32 version][u64 uid][u32 payload length][gzip payload][trailer]`.
///
/// Only the progression flags are returned; the payload also holds unrelated world state.
pub fn parse_db2_bytes(bytes: &[u8]) -> Result<WorldMeta, ScanError> {
    if bytes.len() < 20 {
        return Err(error("db2 is too short"));
    }
    let version = meta_u32(bytes, 0)?;
    let payload_len = meta_u32(bytes, 12)? as usize;
    let payload = bytes
        .get(16..16 + payload_len)
        .ok_or_else(|| error(format!("db2 payload of {payload_len} bytes does not fit")))?;
    let mut plain = Vec::new();
    GzDecoder::new(payload)
        .take(MAX_DB2_PLAIN_BYTES as u64 + 1)
        .read_to_end(&mut plain)
        .map_err(|cause| error(format!("db2 gzip: {cause}")))?;
    if plain.len() > MAX_DB2_PLAIN_BYTES {
        return Err(error("db2 payload exceeds the decompressed limit"));
    }
    Ok(WorldMeta {
        version: Some(version),
        global_keys: global_keys(&plain),
        ..WorldMeta::default()
    })
}

/// Walk the payload's length-prefixed strings, resynchronising one byte at a time so binary sections
/// do not stop the walk, and keep only progression flags.
fn global_keys(plain: &[u8]) -> Vec<GlobalKey> {
    let mut keys: Vec<GlobalKey> = Vec::new();
    let mut offset = 0;
    while offset < plain.len() && keys.len() < MAX_GLOBAL_KEYS {
        let before = offset;
        match read_ascii_pstring(plain, &mut offset) {
            Ok(text) => {
                if let Some(key) = global_key(&text) {
                    if !keys.iter().any(|existing| existing.key == key.key) {
                        keys.push(key);
                    }
                }
            }
            Err(_) => offset = before + 1,
        }
    }
    keys
}

fn global_key(text: &str) -> Option<GlobalKey> {
    let (key, value) = match text.split_once(' ') {
        Some((key, value)) => (key, Some(value.parse::<i64>().ok()?)),
        None => (text, None),
    };
    if key.is_empty() || key.len() > 48 {
        return None;
    }
    if !key
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return None;
    }
    if !GLOBAL_KEY_PREFIXES
        .iter()
        .any(|prefix| key.starts_with(prefix))
    {
        return None;
    }
    Some(GlobalKey {
        key: key.to_string(),
        value,
    })
}

/// Metadata parse failures are recorded, never fatal: the audit of the world itself still completes.
fn merge_world_meta(archive: &mut ArchiveScan, result: Result<WorldMeta, ScanError>) {
    match result {
        Ok(meta) => {
            if meta.version.is_some() {
                archive.world.version = meta.version;
            }
            if meta.name.is_some() {
                archive.world.name = meta.name;
            }
            if meta.seed.is_some() {
                archive.world.seed = meta.seed;
            }
            if meta.player_count.is_some() {
                archive.world.player_count = meta.player_count;
            }
            archive.world.global_keys.extend(meta.global_keys);
        }
        Err(cause) => archive.world_metadata_error = Some(cause.0),
    }
}

pub fn scan_tar_bytes(
    bytes: &[u8],
    archive_name: &str,
    prefabs: &PrefabNames,
) -> Result<ArchiveScan, ScanError> {
    let archive_name = archive_name.to_string();
    let snapshot = snapshot_id(&archive_name);
    let mut tar = TarStream::new(bytes);
    let mut archive = ArchiveScan {
        archive: archive_name.clone(),
        snapshot: snapshot.clone(),
        format: "metadata_only_incomplete".to_string(),
        ..ArchiveScan::default()
    };
    let mut metadata = Vec::new();
    tar.for_each_entry(|header, body| {
        let path_bytes = header.path();
        let internal_path =
            std::str::from_utf8(path_bytes).map_err(|_| error("invalid tar path"))?;
        validate_browser_tar_path(internal_path)?;
        if path_bytes.ends_with(b"dathost_settings_backup.json") {
            return Ok(());
        }
        if internal_path.ends_with(".fch") {
            archive.player_profiles_present = true;
            return Ok(());
        }
        if internal_path.ends_with(".chunks") {
            let (version, total, entries) = parse_chunks_metadata(body)?;
            archive.metadata_total = Some(total);
            archive.metadata_entries = entries.len();
            metadata = entries;
            if version != 41 {
                return Err(error(format!(
                    "unexpected chunks metadata version {version}"
                )));
            }
        } else if internal_path.ends_with(".chunk") {
            let output = parse_chunk_reader(
                body,
                header.size,
                &archive.archive,
                &archive.snapshot,
                internal_path,
                prefabs,
                &metadata,
                &mut ScrubSink::default(),
            )?;
            archive.format = "chunked_v41".to_string();
            merge_parse(&mut archive, output);
        } else if internal_path.ends_with("Dedicated.db") {
            let output = parse_legacy_reader(
                body,
                &archive.archive,
                &archive.snapshot,
                internal_path,
                prefabs,
            )?;
            archive.format = "legacy_v37".to_string();
            merge_parse(&mut archive, output);
        } else if internal_path.ends_with(".fwl2") {
            let parsed =
                read_bounded(body, MAX_WORLD_META_BYTES).and_then(|bytes| parse_fwl2_bytes(&bytes));
            merge_world_meta(&mut archive, parsed);
        } else if internal_path.ends_with(".db2") {
            let parsed =
                read_bounded(body, MAX_DB2_ENTRY_BYTES).and_then(|bytes| parse_db2_bytes(&bytes));
            merge_world_meta(&mut archive, parsed);
        }
        Ok(())
    })?;
    tar.reject_nonzero_trailing()?;
    if archive.format == "chunked_v41" {
        fill_chunk_revisions(&mut archive, &metadata);
        if archive.metadata_total != Some(archive.zdo_count as i32) {
            return Err(error(format!(
                "{} chunk total mismatch: metadata {:?}, parsed {}",
                archive.archive, archive.metadata_total, archive.zdo_count
            )));
        }
    } else if archive.format == "metadata_only_incomplete" {
        return Err(error("tar contains no recognized parseable world payload"));
    }
    Ok(archive)
}

const MAX_CHUNK_FILE_BYTES: u64 = 128 * 1024 * 1024;

/// One file the scrub writes out, with a path relative to `worlds_local/` (i.e. what the owner
/// would drop into their host's save directory).
#[derive(Debug, Clone)]
pub struct ScrubbedFile {
    pub path: String,
    pub bytes: Vec<u8>,
    /// Where this file's contents start inside the decompressed input tar.
    pub tar_offset: u64,
}

/// Result of `scrub_world_tar_bytes`: the world's files with cheat flags cleared, the exact list
/// of patches applied, and the ZDO count (equal to the input's, since Mode A deletes nothing).
#[derive(Debug, Default)]
pub struct ScrubOutcome {
    pub files: Vec<ScrubbedFile>,
    pub patches: Vec<ScrubPatch>,
    pub zdo_count: u64,
}

/// Path within the tar, relative to `worlds_local/` (the world save directory itself), or `None`
/// if the entry lives outside it (server config, admin lists, character saves, dathost settings,
/// ...) - those are not part of "the world" and the scrub never touches or copies them.
fn world_relative_path(internal_path: &str) -> Option<String> {
    let normalized = internal_path.replace('\\', "/");
    let parts: Vec<&str> = normalized.split('/').collect();
    let start = parts.iter().position(|part| *part == "worlds_local")?;
    Some(parts[start..].join("/"))
}

/// The world files inside a backup tar, each with its offset in the tar. Only v41 chunked worlds
/// are accepted; any file under `worlds_local/` whose layout is not known fails the read closed.
struct WorldTar {
    metadata: Vec<ChunkMeta>,
    metadata_total: i32,
    /// Sorted by path.
    chunks: Vec<ScrubbedFile>,
    /// Everything else under `worlds_local/`, the `.chunks` index included.
    passthrough: Vec<ScrubbedFile>,
}

fn read_world_tar(bytes: &[u8]) -> Result<WorldTar, ScanError> {
    let mut tar = TarStream::new(bytes);
    let mut metadata: Vec<ChunkMeta> = Vec::new();
    let mut metadata_total: Option<i32> = None;
    let mut chunks: Vec<ScrubbedFile> = Vec::new();
    let mut passthrough: Vec<ScrubbedFile> = Vec::new();
    tar.for_each_entry(|header, body| {
        let path_bytes = header.path();
        let internal_path =
            std::str::from_utf8(path_bytes).map_err(|_| error("invalid tar path"))?;
        validate_browser_tar_path(internal_path)?;
        let Some(world_path) = world_relative_path(internal_path) else {
            // Outside worlds_local/: not part of the world save, never copied or touched.
            return Ok(());
        };
        let tar_offset = (bytes.len() - body.reader.len()) as u64;
        let limit = if internal_path.ends_with(".chunks")
            || internal_path.ends_with(".fwl2")
            || internal_path.ends_with(".ok")
        {
            MAX_WORLD_META_BYTES
        } else if internal_path.ends_with(".chunk") {
            MAX_CHUNK_FILE_BYTES
        } else if internal_path.ends_with(".db2") {
            MAX_DB2_ENTRY_BYTES
        } else {
            // A legacy .fwl/.db world, or anything else under worlds_local/ this scrubber does not
            // know the layout of: fail closed rather than silently dropping or mis-copying it.
            return Err(error(format!(
                "cannot account for world file, refusing to scrub: {internal_path}"
            )));
        };
        let file = ScrubbedFile {
            path: world_path,
            bytes: read_bounded(body, limit)?,
            tar_offset,
        };
        if internal_path.ends_with(".chunks") {
            let (version, total, entries) = parse_chunks_metadata(&mut &file.bytes[..])?;
            if version != 41 {
                return Err(error(format!(
                    "unexpected chunks metadata version {version}"
                )));
            }
            metadata_total = Some(total);
            metadata = entries;
        }
        if internal_path.ends_with(".chunk") {
            chunks.push(file);
        } else {
            passthrough.push(file);
        }
        Ok(())
    })?;
    tar.reject_nonzero_trailing()?;
    let metadata_total = metadata_total
        .ok_or_else(|| error("world archive has no chunk metadata (.chunks) file"))?;
    if chunks.is_empty() {
        return Err(error("world archive has no chunk files"));
    }
    chunks.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(WorldTar {
        metadata,
        metadata_total,
        chunks,
        passthrough,
    })
}

/// Clears cheat flags in place, on a copy: reads a decompressed world tar, finds every
/// `cheated`/`cheatedQueued[+slot]` ZDO int and every item's cheated-flag byte the parser can
/// account for, and returns the world's files with exactly those bytes patched (same length, same
/// file set) plus the list of patches. Only v41 chunked worlds are supported; a legacy v37 world,
/// an unknown/future chunk version, or any file under `worlds_local/` the parser cannot fully
/// account for fails the whole scrub closed rather than writing a partially-cleaned world.
///
/// Every patch is re-verified against the actual bytes before being trusted (see
/// `push_item_cheated_bit_patch` and `apply_patch`), and every scrubbed chunk is re-parsed after
/// patching to confirm zero cheat evidence remains and the ZDO count, position grid and prefab
/// histogram are unchanged (see `verify_scrub`) - if any of that fails, this returns an error and
/// produces no output at all.
pub fn scrub_world_tar_bytes(
    bytes: &[u8],
    prefabs: &PrefabNames,
) -> Result<ScrubOutcome, ScanError> {
    let WorldTar {
        metadata,
        metadata_total,
        chunks: chunk_files,
        passthrough: passthrough_files,
    } = read_world_tar(bytes)?;

    let mut outcome = ScrubOutcome::default();
    for chunk in &chunk_files {
        let mut sink = ScrubSink::default();
        let mut reader = &chunk.bytes[..];
        let before = parse_chunk_reader(
            &mut reader,
            chunk.bytes.len() as u64,
            "",
            "",
            &chunk.path,
            prefabs,
            &metadata,
            &mut sink,
        )?;
        let mut patches = sink.patches;
        for patch in &mut patches {
            patch.file_path = chunk.path.clone();
        }

        let mut scrubbed = chunk.bytes.clone();
        for patch in &patches {
            apply_patch(&mut scrubbed, patch, &chunk.path)?;
        }
        verify_scrub(
            &chunk.path,
            &chunk.bytes,
            &scrubbed,
            &patches,
            &before,
            prefabs,
        )?;

        outcome.zdo_count += before.zdo_count;
        outcome.patches.extend(patches);
        outcome.files.push(ScrubbedFile {
            path: chunk.path.clone(),
            bytes: scrubbed,
            tar_offset: chunk.tar_offset,
        });
    }
    if metadata_total != outcome.zdo_count as i32 {
        return Err(error(format!(
            "chunk total mismatch: metadata {metadata_total}, parsed {}",
            outcome.zdo_count
        )));
    }

    outcome.files.extend(passthrough_files);
    outcome.files.sort_by(|a, b| a.path.cmp(&b.path));
    outcome
        .patches
        .sort_by(|a, b| (&a.file_path, a.offset).cmp(&(&b.file_path, b.offset)));
    Ok(outcome)
}

/// Mode A on a whole backup tar: returns the input with exactly the audited flag bytes cleared and
/// every other byte - tar headers, padding, end marker, and files outside the world such as admin
/// lists or host settings - identical, so the copy can go back wherever the original came from.
/// Each patch is applied at its file's offset inside the tar, the result must agree byte for byte
/// with `scrub_world_tar_bytes`'s own verified per-chunk output, and the patched tar is rescanned.
pub fn scrub_tar_in_place(
    bytes: &[u8],
    prefabs: &PrefabNames,
) -> Result<(Vec<u8>, ScrubOutcome), ScanError> {
    let outcome = scrub_world_tar_bytes(bytes, prefabs)?;
    if outcome
        .files
        .windows(2)
        .any(|pair| pair[0].path == pair[1].path)
    {
        return Err(error(
            "the tar holds two copies of one world file, refusing to scrub",
        ));
    }
    let mut patched = bytes.to_vec();
    let mut covered = std::collections::HashSet::new();
    for patch in &outcome.patches {
        let file = outcome
            .files
            .iter()
            .find(|file| file.path == patch.file_path)
            .ok_or_else(|| error(format!("{}: patched file is missing", patch.file_path)))?;
        let width = patch_width(patch.kind);
        if patch.offset + width > file.bytes.len() as u64 {
            return Err(error(format!(
                "{}: patch runs past the file",
                patch.file_path
            )));
        }
        let shifted = ScrubPatch {
            offset: file.tar_offset + patch.offset,
            ..patch.clone()
        };
        apply_patch(&mut patched, &shifted, &patch.file_path)?;
        covered.extend(shifted.offset..shifted.offset + width);
    }
    for file in &outcome.files {
        let start = file.tar_offset as usize;
        if patched.get(start..start + file.bytes.len()) != Some(&file.bytes[..]) {
            return Err(error(format!(
                "{}: patched tar disagrees with the scrubbed file",
                file.path
            )));
        }
    }
    for (index, (before, after)) in bytes.iter().zip(&patched).enumerate() {
        if before != after && !covered.contains(&(index as u64)) {
            return Err(error(format!(
                "tar byte {index} changed outside every audited patch"
            )));
        }
    }
    let rescanned = scan_tar_bytes(&patched, "scrubbed.tar", prefabs)?;
    if !rescanned.evidence.is_empty() || rescanned.zdo_count != outcome.zdo_count {
        return Err(error(format!(
            "rescan of the scrubbed tar found {} cheat evidence entries and {} ZDOs (expected 0 and {})",
            rescanned.evidence.len(),
            rescanned.zdo_count,
            outcome.zdo_count
        )));
    }
    Ok((patched, outcome))
}

/// Mode A on one standalone v41 `.chunk` file, verified the same way as a chunk inside a tar.
pub fn scrub_chunk_in_place(
    bytes: &[u8],
    name: &str,
    prefabs: &PrefabNames,
) -> Result<(Vec<u8>, ScrubOutcome), ScanError> {
    let mut sink = ScrubSink::default();
    let before = parse_chunk_reader(
        &mut &bytes[..],
        bytes.len() as u64,
        "",
        "",
        name,
        prefabs,
        &[],
        &mut sink,
    )?;
    let mut patches = sink.patches;
    for patch in &mut patches {
        patch.file_path = name.to_string();
    }
    let mut patched = bytes.to_vec();
    for patch in &patches {
        apply_patch(&mut patched, patch, name)?;
    }
    verify_scrub(name, bytes, &patched, &patches, &before, prefabs)?;
    let outcome = ScrubOutcome {
        files: vec![ScrubbedFile {
            path: name.to_string(),
            bytes: patched.clone(),
            tar_offset: 0,
        }],
        patches,
        zdo_count: before.zdo_count,
    };
    Ok((patched, outcome))
}

/// How a scrub treats flagged content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrubMode {
    /// Clear every flag in place and keep every object (same-length patches).
    Clean,
    /// Delete flagged items - items lying in the world, entries in containers, items on stands -
    /// and leave every other flagged object, flag and all.
    DeleteItems,
    /// Destroy every flagged object together with whatever it holds, and remove flagged items from
    /// containers and stands that are not flagged themselves. Nothing flagged survives.
    Destroy,
}

impl ScrubMode {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "clean" => Some(Self::Clean),
            "delete-items" => Some(Self::DeleteItems),
            "destroy" => Some(Self::Destroy),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::DeleteItems => "delete-items",
            Self::Destroy => "destroy",
        }
    }
}

/// One thing a scrub did, for the audit and the preview. `offset` is in the original file: the
/// patched value for `clear_flag`/`empty_stand`, the ZDO record start otherwise.
#[derive(Debug, Clone)]
pub struct ScrubAction {
    /// `clear_flag`, `delete_item`, `remove_from_container`, `empty_stand` or `destroy`.
    pub action: &'static str,
    pub file_path: String,
    pub offset: u64,
    pub key_name: String,
    pub owner_prefab_name: Option<String>,
    pub item_name: Option<String>,
    pub position: Position,
    /// Items a destroyed object held (container contents, a stand's item), gone with it.
    pub contents: usize,
}

/// Everything one scrub did, in the order found.
#[derive(Debug)]
pub struct ScrubReport {
    pub mode: ScrubMode,
    pub zdo_count_before: u64,
    pub zdo_count_after: u64,
    pub actions: Vec<ScrubAction>,
}

impl ScrubReport {
    fn from_patches(outcome: &ScrubOutcome) -> Self {
        Self {
            mode: ScrubMode::Clean,
            zdo_count_before: outcome.zdo_count,
            zdo_count_after: outcome.zdo_count,
            actions: outcome
                .patches
                .iter()
                .map(|patch| ScrubAction {
                    action: "clear_flag",
                    file_path: patch.file_path.clone(),
                    offset: patch.offset,
                    key_name: patch.key_name.clone(),
                    owner_prefab_name: patch.owner_prefab_name.clone(),
                    item_name: None,
                    position: patch.position,
                    contents: 0,
                })
                .collect(),
        }
    }
}

/// What a delete scrub does to one ZDO record.
enum ZdoPlan {
    Keep,
    Drop,
    /// Remove the listed items from the listed arrays (array index, item indices) and overwrite
    /// same-length values (absolute offset, new bytes).
    Edit {
        removals: Vec<(usize, Vec<usize>)>,
        overwrites: Vec<(u64, Vec<u8>)>,
    },
}

fn plan_zdo(
    zdo: &ZdoLayout,
    mode: ScrubMode,
    bytes: &[u8],
    path: &str,
    prefabs: &PrefabNames,
    actions: &mut Vec<ScrubAction>,
) -> Result<ZdoPlan, ScanError> {
    let prefab = |hash: i32| prefabs.name(hash).map(str::to_string);
    let action =
        |action: &'static str, key: &str, item: Option<String>, contents: usize| ScrubAction {
            action,
            file_path: path.to_string(),
            offset: zdo.start,
            key_name: key.to_string(),
            owner_prefab_name: prefab(zdo.owner_prefab_hash),
            item_name: item,
            position: zdo.position,
            contents,
        };
    // An `itemData` array without the `item` int an ItemStand keeps is an item lying in the world.
    let direct = zdo.arrays.iter().position(|array| array.key == ITEM_DATA);
    let is_stand = zdo.item_int.is_some();
    let direct_cheated = direct.is_some_and(|index| zdo.arrays[index].items[0].cheated);
    if mode == ScrubMode::Destroy && zdo.object_flags > 0 {
        let held = zdo
            .arrays
            .iter()
            .filter(|array| array.key != ITEM_DATA)
            .map(|array| array.items.len())
            .sum::<usize>()
            + usize::from(zdo.item_int.is_some_and(|(_, value)| value != 0));
        actions.push(action("destroy", "cheated", None, held));
        return Ok(ZdoPlan::Drop);
    }
    if direct.is_some() && !is_stand && (direct_cheated || zdo.object_flags > 0) {
        actions.push(action(
            "delete_item",
            "itemData",
            prefab(zdo.owner_prefab_hash),
            0,
        ));
        return Ok(ZdoPlan::Drop);
    }
    let mut removals = Vec::new();
    let mut overwrites = Vec::new();
    for (index, array) in zdo.arrays.iter().enumerate() {
        let cheated: Vec<usize> = (0..array.items.len())
            .filter(|&item| array.items[item].cheated)
            .collect();
        if cheated.is_empty() {
            continue;
        }
        if array.key == ITEMS {
            for &item in &cheated {
                let name = array.items[item].item_hash.and_then(prefab);
                actions.push(action("remove_from_container", "items", name, 0));
            }
            removals.push((index, cheated));
        } else if array.key == ITEM_DATA {
            // An ItemStand holding (or having held) a flagged item: empty it in place, as the
            // game's own ItemStand.DropItem does (`item` and `type` to 0), and clear the stale bit.
            let held = zdo
                .item_int
                .and_then(|(_, value)| (value != 0).then_some(value));
            for (offset, value) in [zdo.item_int, zdo.type_int].into_iter().flatten() {
                if value != 0 {
                    overwrites.push((offset, 0i32.to_le_bytes().to_vec()));
                }
            }
            let bit = array.items[0]
                .cheated_byte
                .ok_or_else(|| error(format!("{path}: stand item has no recorded flag byte")))?;
            overwrites.push((bit, vec![bytes[bit as usize] & 0xfe]));
            actions.push(action("empty_stand", "itemData", held.and_then(prefab), 0));
        } else {
            return Err(error(format!(
                "{path}: an armor-stand slot holds a flagged item, which this mode does not handle \
                 yet; use the clean mode for this save"
            )));
        }
    }
    Ok(if removals.is_empty() && overwrites.is_empty() {
        ZdoPlan::Keep
    } else {
        ZdoPlan::Edit {
            removals,
            overwrites,
        }
    })
}

/// The bytes of one edited ZDO record: overwrites applied, then flagged items cut out of their
/// arrays (last array first, so earlier offsets stay valid), with each array's item count and
/// length prefix rewritten.
fn edit_zdo_record(
    bytes: &[u8],
    zdo: &ZdoLayout,
    removals: &[(usize, Vec<usize>)],
    overwrites: &[(u64, Vec<u8>)],
    path: &str,
) -> Result<Vec<u8>, ScanError> {
    let base = zdo.start as usize;
    let mut record = bytes[base..zdo.end as usize].to_vec();
    for (offset, new) in overwrites {
        let at = *offset as usize - base;
        record[at..at + new.len()].copy_from_slice(new);
    }
    let mut ordered: Vec<&(usize, Vec<usize>)> = removals.iter().collect();
    ordered.sort_by_key(|(index, _)| std::cmp::Reverse(zdo.arrays[*index].len_offset));
    for (index, removed) in ordered {
        let array = &zdo.arrays[*index];
        let len_at = array.len_offset as usize - base;
        let data_at = len_at + 4;
        let data = &record[data_at..data_at + array.len as usize];
        let count = u16::from_le_bytes([data[4], data[5]]) as usize;
        let last_end = array.items.last().map_or(6, |item| item.end as usize);
        if count != array.items.len() || last_end != data.len() {
            return Err(error(format!(
                "{path}: container item list does not match its recorded layout"
            )));
        }
        let kept = array.items.len() - removed.len();
        let mut new_data = data[..4].to_vec();
        new_data.extend_from_slice(&(kept as u16).to_le_bytes());
        for (item_index, item) in array.items.iter().enumerate() {
            if !removed.contains(&item_index) {
                new_data.extend_from_slice(&data[item.start as usize..item.end as usize]);
            }
        }
        let mut replacement = (new_data.len() as i32).to_le_bytes().to_vec();
        replacement.extend(new_data);
        record.splice(len_at..data_at + array.len as usize, replacement);
    }
    Ok(record)
}

/// Bytes of a record outside its recorded arrays, with the given record-relative int positions
/// zeroed, so an edited record can be compared with its original independently of how it was
/// rebuilt. Ints precede byte arrays in a record, so their relative positions survive array edits.
fn record_skeleton(bytes: &[u8], zdo: &ZdoLayout, blank: &[u64]) -> Vec<u8> {
    let mut record = bytes[zdo.start as usize..zdo.end as usize].to_vec();
    for &at in blank {
        record[at as usize..at as usize + 4].fill(0);
    }
    let mut regions: Vec<(usize, usize)> = zdo
        .arrays
        .iter()
        .map(|array| {
            let start = (array.len_offset - zdo.start) as usize;
            (start, start + 4 + array.len as usize)
        })
        .collect();
    regions.sort();
    let mut out = Vec::new();
    let mut at = 0;
    for (start, end) in regions {
        out.extend_from_slice(&record[at..start]);
        at = end;
    }
    out.extend_from_slice(&record[at..]);
    out
}

/// Item byte slices of every recorded array of a record, per array.
fn record_items<'a>(bytes: &'a [u8], zdo: &ZdoLayout) -> Vec<Vec<&'a [u8]>> {
    zdo.arrays
        .iter()
        .map(|array| {
            let data = (array.len_offset + 4) as usize;
            array
                .items
                .iter()
                .map(|item| &bytes[data + item.start as usize..data + item.end as usize])
                .collect()
        })
        .collect()
}

struct RebuiltChunk {
    bytes: Vec<u8>,
    changed: bool,
    zdo_count_before: u64,
    dropped: u64,
    kept_object_flags: usize,
    actions: Vec<ScrubAction>,
}

/// Delete/destroy scrub of one chunk file, verified by parsing the result again: every kept record
/// must be byte-identical to its original, or - where edited - identical outside its arrays except
/// at the audited ints, with exactly the audited items missing from its arrays and nothing flagged
/// left in them; the ZDO count and prefab histogram must drop by exactly the removed records; and
/// the only evidence left must be what the mode deliberately keeps.
fn rebuild_chunk(
    bytes: &[u8],
    path: &str,
    prefabs: &PrefabNames,
    metadata: &[ChunkMeta],
    mode: ScrubMode,
) -> Result<RebuiltChunk, ScanError> {
    let parse = |data: &[u8]| -> Result<(ParseOutput, Vec<ZdoLayout>), ScanError> {
        let mut sink = ScrubSink::with_layouts();
        let output = parse_chunk_reader(
            &mut &data[..],
            data.len() as u64,
            "",
            "",
            path,
            prefabs,
            metadata,
            &mut sink,
        )?;
        Ok((output, sink.layouts.unwrap_or_default()))
    };
    let (before, layouts) = parse(bytes)?;
    let body_start = layouts.first().map_or(6, |zdo| zdo.start as usize);
    if body_start != 6 || layouts.last().map_or(6, |zdo| zdo.end as usize) != bytes.len() {
        return Err(error(format!(
            "{path}: chunk has bytes outside its ZDO records"
        )));
    }
    let mut actions = Vec::new();
    let plans = layouts
        .iter()
        .map(|zdo| plan_zdo(zdo, mode, bytes, path, prefabs, &mut actions))
        .collect::<Result<Vec<_>, _>>()?;
    if plans.iter().all(|plan| matches!(plan, ZdoPlan::Keep)) {
        return Ok(RebuiltChunk {
            bytes: Vec::new(),
            changed: false,
            zdo_count_before: before.zdo_count,
            dropped: 0,
            kept_object_flags: layouts.iter().map(|zdo| zdo.object_flags).sum(),
            actions,
        });
    }

    let mut out = bytes[..6].to_vec();
    let mut dropped = 0u64;
    let mut dropped_prefabs: HashMap<i32, u32> = HashMap::new();
    let mut kept_object_flags = 0usize;
    for (zdo, plan) in layouts.iter().zip(&plans) {
        match plan {
            ZdoPlan::Keep => out.extend_from_slice(&bytes[zdo.start as usize..zdo.end as usize]),
            ZdoPlan::Drop => {
                dropped += 1;
                *dropped_prefabs.entry(zdo.owner_prefab_hash).or_default() += 1;
                continue;
            }
            ZdoPlan::Edit {
                removals,
                overwrites,
            } => out.extend(edit_zdo_record(bytes, zdo, removals, overwrites, path)?),
        }
        kept_object_flags += zdo.object_flags;
    }
    let count_after = before.zdo_count - dropped;
    out[2..6].copy_from_slice(&(count_after as i32).to_le_bytes());

    // Verify against a fresh parse of the result.
    let (after, new_layouts) = parse(&out)?;
    let kept: Vec<(&ZdoLayout, &ZdoPlan)> = layouts
        .iter()
        .zip(&plans)
        .filter(|(_, plan)| !matches!(plan, ZdoPlan::Drop))
        .collect();
    if after.zdo_count != count_after || new_layouts.len() != kept.len() {
        return Err(error(format!(
            "{path}: rebuilt chunk has the wrong ZDO count"
        )));
    }
    for ((old, plan), new) in kept.iter().zip(&new_layouts) {
        let old_bytes = &bytes[old.start as usize..old.end as usize];
        let new_bytes = &out[new.start as usize..new.end as usize];
        let fail = || error(format!("{path}: a kept ZDO record changed unexpectedly"));
        match plan {
            ZdoPlan::Keep => {
                if old_bytes != new_bytes {
                    return Err(fail());
                }
            }
            ZdoPlan::Edit {
                removals,
                overwrites,
            } => {
                // The audited ints, as record-relative positions (the flag bit sits in an array).
                let blank: Vec<u64> = overwrites
                    .iter()
                    .filter(|(_, new)| new.len() == 4)
                    .map(|(at, _)| at - old.start)
                    .collect();
                if old.owner_prefab_hash != new.owner_prefab_hash
                    || old.arrays.len() != new.arrays.len()
                    || record_skeleton(bytes, old, &blank) != record_skeleton(&out, new, &blank)
                {
                    return Err(fail());
                }
                let old_items = record_items(bytes, old);
                let new_items = record_items(&out, new);
                for (index, (old_array, new_array)) in old_items.iter().zip(&new_items).enumerate()
                {
                    let layout = &old.arrays[index];
                    let expected: Vec<Vec<u8>> = if layout.key == ITEM_DATA {
                        // A stand's item data may differ only by the cleared flag bit.
                        let mut item = old_array[0].to_vec();
                        if let Some(bit) = layout.items[0].cheated_byte {
                            let at = (bit - layout.len_offset - 4 - layout.items[0].start) as usize;
                            item[at] &= 0xfe;
                        }
                        vec![item]
                    } else {
                        let removed = removals
                            .iter()
                            .find(|(array, _)| *array == index)
                            .map(|(_, items)| items.as_slice())
                            .unwrap_or(&[]);
                        old_array
                            .iter()
                            .enumerate()
                            .filter(|(item, _)| !removed.contains(item))
                            .map(|(_, bytes)| bytes.to_vec())
                            .collect()
                    };
                    let actual: Vec<Vec<u8>> = new_array.iter().map(|item| item.to_vec()).collect();
                    if expected != actual || new.arrays[index].items.iter().any(|item| item.cheated)
                    {
                        return Err(fail());
                    }
                }
            }
            ZdoPlan::Drop => unreachable!(),
        }
    }
    let mut expected_prefabs = before.prefabs.clone();
    for (hash, count) in &dropped_prefabs {
        if let Some(total) = expected_prefabs.get_mut(hash) {
            *total -= count;
            if *total == 0 {
                expected_prefabs.remove(hash);
            }
        }
    }
    if after.prefabs != expected_prefabs {
        return Err(error(format!(
            "{path}: prefab histogram changed beyond the removed ZDOs"
        )));
    }
    let object_evidence = after
        .evidence
        .iter()
        .filter(|hit| matches!(hit.kind.as_str(), "zdo_cheated" | "station_queued_cheated"))
        .count();
    let expected_object_evidence = match mode {
        ScrubMode::Destroy => 0,
        _ => kept_object_flags,
    };
    if object_evidence != after.evidence.len() || object_evidence != expected_object_evidence {
        return Err(error(format!(
            "{path}: {} cheat evidence entries remain after scrubbing, expected {expected_object_evidence}",
            after.evidence.len()
        )));
    }
    Ok(RebuiltChunk {
        bytes: out,
        changed: true,
        zdo_count_before: before.zdo_count,
        dropped,
        kept_object_flags,
        actions,
    })
}

/// Checks that every tar header is a plain file or a directory, whose sizes this scrub can
/// rewrite; extended (pax, GNU long-name) headers could carry a size of their own.
fn reject_extended_tar_headers(bytes: &[u8]) -> Result<(), ScanError> {
    let mut tar = TarStream::new(bytes);
    while let Some(header) = tar.next_header()? {
        if !matches!(header.kind, 0 | b'0' | b'5') {
            return Err(error(format!(
                "the backup uses a tar entry type ({}) whose size this scrub cannot rewrite; use \
                 the clean mode",
                header.kind as char
            )));
        }
        let padding = (512 - (header.size % 512)) % 512;
        skip_bytes(&mut tar.reader, header.size + padding)?;
    }
    Ok(())
}

/// Replaces the bodies of the given tar entries (by body offset), rewriting each header's size and
/// checksum and the padding; every other byte of the tar is copied as it was.
fn splice_tar(bytes: &[u8], mut replaced: Vec<(u64, &[u8])>) -> Result<Vec<u8>, ScanError> {
    replaced.sort_by_key(|(offset, _)| *offset);
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0usize;
    for (body_offset, body) in replaced {
        let body_offset = body_offset as usize;
        let header_offset = body_offset - 512;
        let mut header = [0u8; 512];
        header.copy_from_slice(&bytes[header_offset..body_offset]);
        let old_size = parse_tar_octal(&header[124..136])? as usize;
        out.extend_from_slice(&bytes[at..header_offset]);
        header[124..135].copy_from_slice(format!("{:011o}", body.len()).as_bytes());
        header[148..156].fill(b' ');
        let checksum: u64 = header.iter().map(|byte| *byte as u64).sum();
        header[148..154].copy_from_slice(format!("{checksum:06o}").as_bytes());
        header[154] = 0;
        out.extend_from_slice(&header);
        out.extend_from_slice(body);
        out.resize(out.len() + (512 - body.len() % 512) % 512, 0);
        at = body_offset + old_size + (512 - old_size % 512) % 512;
    }
    out.extend_from_slice(&bytes[at..]);
    Ok(out)
}

/// A scrub of a whole backup tar in any mode, returning the new tar. Clean is the same-length
/// in-place patch (`scrub_tar_in_place`); the delete modes rebuild each affected chunk
/// (`rebuild_chunk`), update the `.chunks` index counts in place, splice the resized chunks back
/// into the tar with fixed headers, and rescan the result.
pub fn scrub_tar(
    bytes: &[u8],
    prefabs: &PrefabNames,
    mode: ScrubMode,
) -> Result<(Vec<u8>, ScrubReport), ScanError> {
    if mode == ScrubMode::Clean {
        let (patched, outcome) = scrub_tar_in_place(bytes, prefabs)?;
        return Ok((patched, ScrubReport::from_patches(&outcome)));
    }
    reject_extended_tar_headers(bytes)?;
    let world = read_world_tar(bytes)?;
    let mut report = ScrubReport {
        mode,
        zdo_count_before: 0,
        zdo_count_after: 0,
        actions: Vec::new(),
    };
    let mut rebuilt: Vec<(&ScrubbedFile, RebuiltChunk)> = Vec::new();
    let mut kept_object_flags = 0usize;
    for chunk in &world.chunks {
        let result = rebuild_chunk(&chunk.bytes, &chunk.path, prefabs, &world.metadata, mode)?;
        report.zdo_count_before += result.zdo_count_before;
        report.zdo_count_after += result.zdo_count_before - result.dropped;
        kept_object_flags += result.kept_object_flags;
        if result.changed {
            rebuilt.push((chunk, result));
        }
    }
    if report.zdo_count_before != world.metadata_total as u64 {
        return Err(error(format!(
            "chunk total mismatch: metadata {}, parsed {}",
            world.metadata_total, report.zdo_count_before
        )));
    }

    // The `.chunks` index: [i16 version][i32 total][i32 entries][u16 chunk, u8 size, u32 version,
    // i32 ZDOs]*; counts are patched in place, so its length never changes.
    let index_file = world
        .passthrough
        .iter()
        .find(|file| file.path.ends_with(".chunks"))
        .ok_or_else(|| error("world archive has no chunk metadata (.chunks) file"))?;
    let mut index = index_file.bytes.clone();
    for (chunk, result) in &rebuilt {
        let name = chunk.path.rsplit('/').next().unwrap_or(&chunk.path);
        let (size, version, chunk_index) = parse_chunk_name(name)?;
        let entry = world
            .metadata
            .iter()
            .position(|meta| meta.chunk_index == chunk_index && meta.chunk_size == size)
            .ok_or_else(|| error(format!("{name}: not listed in the chunk index")))?;
        let meta = &world.metadata[entry];
        let count_at = 10 + entry * 11 + 7;
        let listed = i32::from_le_bytes(index[count_at..count_at + 4].try_into().unwrap());
        if meta.revision != version || listed as u64 != result.zdo_count_before {
            return Err(error(format!(
                "{name}: chunk index entry does not match the chunk"
            )));
        }
        let new_count = (result.zdo_count_before - result.dropped) as i32;
        index[count_at..count_at + 4].copy_from_slice(&new_count.to_le_bytes());
    }
    index[2..6].copy_from_slice(&(report.zdo_count_after as i32).to_le_bytes());

    let mut replaced: Vec<(u64, &[u8])> = rebuilt
        .iter()
        .map(|(chunk, result)| (chunk.tar_offset, result.bytes.as_slice()))
        .collect();
    replaced.push((index_file.tar_offset, index.as_slice()));
    let out = splice_tar(bytes, replaced)?;
    for (_, result) in rebuilt {
        report.actions.extend(result.actions);
    }

    // The result must read back as a world with exactly the counts the rebuild claims.
    let reread = read_world_tar(&out)?;
    let rescanned = scan_tar_bytes(&out, "scrubbed.tar", prefabs)?;
    let expected_evidence = if mode == ScrubMode::Destroy {
        0
    } else {
        kept_object_flags
    };
    let object_evidence = rescanned
        .evidence
        .iter()
        .filter(|hit| matches!(hit.kind.as_str(), "zdo_cheated" | "station_queued_cheated"))
        .count();
    if reread.metadata_total as u64 != report.zdo_count_after
        || rescanned.zdo_count != report.zdo_count_after
        || rescanned.evidence.len() != expected_evidence
        || object_evidence != expected_evidence
    {
        return Err(error(format!(
            "rescan of the scrubbed tar found {} ZDOs and {} evidence entries (expected {} and {})",
            rescanned.zdo_count,
            rescanned.evidence.len(),
            report.zdo_count_after,
            expected_evidence
        )));
    }
    Ok((out, report))
}

/// Adds a content checksum to one zstd frame made without one, as a host's `.tar.zst` backup has:
/// sets `Content_Checksum_Flag` (bit 2 of the frame header descriptor) and appends the low 32 bits
/// of XXH64(content, seed 0), exactly as RFC 8878 section 3.1.1 lays it out. `frame` must be a
/// single frame whose decompressed bytes are `content`; the caller verifies the round trip.
pub fn add_zstd_content_checksum(frame: &[u8], content: &[u8]) -> Result<Vec<u8>, ScanError> {
    const MAGIC: [u8; 4] = [0x28, 0xb5, 0x2f, 0xfd];
    const CHECKSUM_FLAG: u8 = 0x04;
    if frame.len() < 6 || frame[..4] != MAGIC {
        return Err(error("not a zstd frame"));
    }
    if frame[4] & CHECKSUM_FLAG != 0 {
        return Err(error("zstd frame already carries a content checksum"));
    }
    let mut out = frame.to_vec();
    out[4] |= CHECKSUM_FLAG;
    let checksum = twox_hash::XxHash64::oneshot(0, content) as u32;
    out.extend_from_slice(&checksum.to_le_bytes());
    Ok(out)
}

/// Overwrites one patch's bytes in `buffer`, after confirming the bytes actually there still match
/// `patch.old_value` exactly - a mismatch means the parse and the raw bytes disagree, which must
/// fail closed rather than silently patch the wrong location.
fn apply_patch(buffer: &mut [u8], patch: &ScrubPatch, path: &str) -> Result<(), ScanError> {
    let offset = patch.offset as usize;
    match patch.kind {
        ScrubPatchKind::ZdoIntFlag => {
            let slice: &mut [u8; 4] = buffer
                .get_mut(offset..offset + 4)
                .and_then(|slice| slice.try_into().ok())
                .ok_or_else(|| error(format!("{path}: patch offset {offset} out of range")))?;
            if i32::from_le_bytes(*slice) as i64 != patch.old_value {
                return Err(error(format!(
                    "{path}: patch at {offset} does not match the recorded old value"
                )));
            }
            *slice = (patch.new_value as i32).to_le_bytes();
        }
        ScrubPatchKind::ItemCheatedBit => {
            let byte = buffer
                .get_mut(offset)
                .ok_or_else(|| error(format!("{path}: patch offset {offset} out of range")))?;
            if *byte as i64 != patch.old_value {
                return Err(error(format!(
                    "{path}: patch at {offset} does not match the recorded old value"
                )));
            }
            *byte = patch.new_value as u8;
        }
    }
    Ok(())
}

/// Width in bytes of one patch's field, for checking that nothing outside the audited offsets
/// changed.
fn patch_width(kind: ScrubPatchKind) -> u64 {
    match kind {
        ScrubPatchKind::ZdoIntFlag => 4,
        ScrubPatchKind::ItemCheatedBit => 1,
    }
}

/// Confirms one scrubbed chunk is safe to hand back: every byte that differs from the original is
/// inside an audited patch's range (nothing unaccounted-for changed), and re-parsing the scrubbed
/// bytes finds zero cheat evidence while the ZDO count, position grid and prefab histogram - none
/// of which any patch touches - are byte-for-byte identical to the original parse.
fn verify_scrub(
    path: &str,
    original: &[u8],
    scrubbed: &[u8],
    patches: &[ScrubPatch],
    before: &ParseOutput,
    prefabs: &PrefabNames,
) -> Result<(), ScanError> {
    if original.len() != scrubbed.len() {
        return Err(error(format!("{path}: scrub changed the file length")));
    }
    let mut covered = std::collections::HashSet::new();
    for patch in patches {
        for delta in 0..patch_width(patch.kind) {
            covered.insert(patch.offset + delta);
        }
    }
    for (index, (before_byte, after_byte)) in original.iter().zip(scrubbed).enumerate() {
        if before_byte != after_byte && !covered.contains(&(index as u64)) {
            return Err(error(format!(
                "{path}: byte {index} changed outside every audited patch"
            )));
        }
    }
    let after = parse_chunk_bytes(scrubbed, path, prefabs)?;
    if !after.evidence.is_empty() {
        return Err(error(format!(
            "{path}: {} cheat evidence entries remain after scrubbing",
            after.evidence.len()
        )));
    }
    if after.zdo_count != before.zdo_count {
        return Err(error(format!("{path}: ZDO count changed while scrubbing")));
    }
    if after.grid != before.grid || after.prefabs != before.prefabs {
        return Err(error(format!(
            "{path}: position grid or prefab histogram changed while scrubbing"
        )));
    }
    Ok(())
}

/// Deterministic, privacy-safe (no absolute paths, no player ids) JSON audit of everything a scrub
/// did. Offsets are in the original file.
fn scrub_audit_json(report: &ScrubReport) -> String {
    let actions = report
        .actions
        .iter()
        .map(|action| {
            format!(
                "{{\"action\":{},\"file\":{},\"offset\":{},\"key\":{},\"prefab\":{},\"item\":{},\"position\":{},\"contents\":{}}}",
                json_string(action.action),
                json_string(&action.file_path),
                action.offset,
                json_string(&action.key_name),
                json_opt_string(action.owner_prefab_name.as_deref()),
                json_opt_string(action.item_name.as_deref()),
                json_position(action.position),
                action.contents,
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"mode\":{},\"zdo_count_before\":{},\"zdo_count_after\":{},\"action_count\":{},\"actions\":[{}]}}",
        json_string(report.mode.as_str()),
        report.zdo_count_before,
        report.zdo_count_after,
        report.actions.len(),
        actions
    )
}

/// Same audit as `scrub_audit_json`, as Markdown for a human to read.
fn scrub_audit_markdown(report: &ScrubReport) -> String {
    fn cell(value: &str) -> String {
        value.replace('|', r"\|").replace('\n', " ")
    }
    let mut md = format!(
        "# Scrub audit\n\nMode: {}\n\nZDOs: {} before, {} after\n\n",
        report.mode.as_str(),
        report.zdo_count_before,
        report.zdo_count_after
    );
    let mut summary: Vec<(&str, usize, usize)> = Vec::new();
    for action in &report.actions {
        match summary
            .iter_mut()
            .find(|(name, _, _)| *name == action.action)
        {
            Some(row) => {
                row.1 += 1;
                row.2 += action.contents;
            }
            None => summary.push((action.action, 1, action.contents)),
        }
    }
    for (name, count, contents) in &summary {
        md.push_str(&format!("- {name}: {count}"));
        if *contents > 0 {
            md.push_str(&format!(" (holding {contents} item(s), gone with them)"));
        }
        md.push('\n');
    }
    md.push_str("\n| action | file | offset | key | prefab | item | position | contents |\n");
    md.push_str("| --- | --- | --- | --- | --- | --- | --- | --- |\n");
    for action in &report.actions {
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | ({:.1}, {:.1}, {:.1}) | {} |\n",
            action.action,
            cell(&action.file_path),
            action.offset,
            cell(&action.key_name),
            action
                .owner_prefab_name
                .as_deref()
                .map(cell)
                .unwrap_or_else(|| "-".to_string()),
            action
                .item_name
                .as_deref()
                .map(cell)
                .unwrap_or_else(|| "-".to_string()),
            action.position.x,
            action.position.y,
            action.position.z,
            action.contents,
        ));
    }
    md
}

/// Writes a scrubbed world (and its audit report) to `output_dir`, refusing to touch the input
/// archive or an already-populated/overlapping output directory, and refuses to leave a partial
/// or unverified copy behind: on any failure after the first byte is written, everything written
/// so far is removed and the error is returned.
#[cfg(not(target_arch = "wasm32"))]
pub fn scrub_world_archive(
    archive_path: &Path,
    output_dir: &Path,
    zstd_program: &str,
    prefabs: &PrefabNames,
    mode: ScrubMode,
) -> Result<ScrubReport, ScanError> {
    let archive_dir = archive_path.parent().unwrap_or(Path::new("."));
    let archive_dir_abs = std::path::absolute(archive_dir).map_err(ScanError::from)?;
    let output_abs = std::path::absolute(output_dir).map_err(ScanError::from)?;
    if output_abs.starts_with(&archive_dir_abs) {
        return Err(error(
            "--scrub-out must not be the archive's directory or inside it",
        ));
    }
    if output_dir.is_file() {
        return Err(error(format!(
            "{} already exists as a file",
            output_dir.display()
        )));
    }
    if output_dir.is_dir() {
        let has_entries = std::fs::read_dir(output_dir)
            .map_err(|e| error(format!("cannot read {}: {e}", output_dir.display())))?
            .next()
            .is_some();
        if has_entries {
            return Err(error(format!(
                "{} already exists and is not empty",
                output_dir.display()
            )));
        }
    }

    let bytes = decompress_zstd_to_vec(archive_path, zstd_program)?;
    let (files, report) = if mode == ScrubMode::Clean {
        let outcome = scrub_world_tar_bytes(&bytes, prefabs)?;
        let report = ScrubReport::from_patches(&outcome);
        (outcome.files, report)
    } else {
        let (tar, report) = scrub_tar(&bytes, prefabs, mode)?;
        let world = read_world_tar(&tar)?;
        let mut files = world.chunks;
        files.extend(world.passthrough);
        (files, report)
    };

    if let Err(write_error) = write_scrub_output(output_dir, &files, &report) {
        let _ = std::fs::remove_dir_all(output_dir);
        return Err(write_error);
    }
    if let Err(verify_error) = verify_scrub_output_on_disk(output_dir, &files) {
        let _ = std::fs::remove_dir_all(output_dir);
        return Err(verify_error);
    }
    Ok(report)
}

#[cfg(not(target_arch = "wasm32"))]
fn write_scrub_output(
    output_dir: &Path,
    files: &[ScrubbedFile],
    report: &ScrubReport,
) -> Result<(), ScanError> {
    for file in files {
        let target = output_dir.join(&file.path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| error(format!("cannot create {}: {e}", parent.display())))?;
        }
        std::fs::write(&target, &file.bytes)
            .map_err(|e| error(format!("cannot write {}: {e}", target.display())))?;
    }
    std::fs::write(
        output_dir.join("SCRUB_AUDIT.md"),
        scrub_audit_markdown(report),
    )
    .map_err(|e| error(format!("cannot write scrub audit markdown: {e}")))?;
    std::fs::write(
        output_dir.join("scrub-audit.json"),
        scrub_audit_json(report),
    )
    .map_err(|e| error(format!("cannot write scrub audit json: {e}")))?;
    Ok(())
}

/// Reads every written file back and checks it against what was meant to be written: cheap
/// insurance against a filesystem-level surprise (partial write, wrong encoding, etc.) that the
/// in-memory verification in `scrub_world_tar_bytes` cannot see.
#[cfg(not(target_arch = "wasm32"))]
fn verify_scrub_output_on_disk(output_dir: &Path, files: &[ScrubbedFile]) -> Result<(), ScanError> {
    for file in files {
        let target = output_dir.join(&file.path);
        let on_disk = std::fs::read(&target)
            .map_err(|e| error(format!("cannot read back {}: {e}", target.display())))?;
        if on_disk != file.bytes {
            return Err(error(format!(
                "{} on disk does not match what was written",
                target.display()
            )));
        }
    }
    Ok(())
}

pub fn parse_chunk_bytes(
    bytes: &[u8],
    source_name: &str,
    prefabs: &PrefabNames,
) -> Result<ArchiveScan, ScanError> {
    let source_name = source_name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(source_name);
    let snapshot = snapshot_id(source_name);
    let output = parse_chunk_reader(
        &mut io::Cursor::new(bytes),
        bytes.len() as u64,
        source_name,
        &snapshot,
        source_name,
        prefabs,
        &[],
        &mut ScrubSink::default(),
    )?;
    let mut archive = ArchiveScan {
        archive: source_name.to_string(),
        snapshot,
        format: "chunked_v41".to_string(),
        ..ArchiveScan::default()
    };
    merge_parse(&mut archive, output);
    Ok(archive)
}

pub fn parse_legacy_bytes(
    bytes: &[u8],
    source_name: &str,
    prefabs: &PrefabNames,
) -> Result<ArchiveScan, ScanError> {
    let source_name = source_name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(source_name);
    let snapshot = snapshot_id(source_name);
    let output = parse_legacy_reader(
        &mut io::Cursor::new(bytes),
        source_name,
        &snapshot,
        source_name,
        prefabs,
    )?;
    let mut archive = ArchiveScan {
        archive: source_name.to_string(),
        snapshot,
        format: "legacy_v37".to_string(),
        ..ArchiveScan::default()
    };
    merge_parse(&mut archive, output);
    Ok(archive)
}

/// Decompresses a whole `.tar.zst` archive into memory. `scan_archive` above streams instead (a
/// scan never needs to hold the tar in memory at once), but the scrub CLI needs the full bytes: it
/// both writes them back out and re-parses the scrubbed copy for verification.
#[cfg(not(target_arch = "wasm32"))]
pub fn decompress_zstd_to_vec(path: &Path, zstd_program: &str) -> Result<Vec<u8>, ScanError> {
    let mut child = Command::new(zstd_program)
        .args(["-dc"])
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| error(format!("cannot start {zstd_program}: {e}")))?;
    let mut bytes = Vec::new();
    child
        .stdout
        .take()
        .ok_or_else(|| error("zstd has no stdout"))?
        .read_to_end(&mut bytes)
        .map_err(ScanError::from)?;
    let status = child.wait().map_err(ScanError::from)?;
    if !status.success() {
        return Err(error(format!(
            "{zstd_program} failed decompressing {}",
            path.display()
        )));
    }
    Ok(bytes)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn scan_archive(
    path: &Path,
    zstd_program: &str,
    prefabs: &PrefabNames,
) -> Result<ArchiveScan, ScanError> {
    let archive_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| error(format!("invalid archive path {}", path.display())))?
        .to_string();
    let snapshot = snapshot_id(&archive_name);
    let mut child = Command::new(zstd_program)
        .args(["-dc"])
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| error(format!("cannot start {zstd_program}: {e}")))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| error("zstd has no stdout"))?;
    let mut tar = TarStream::new(BufReader::with_capacity(1024 * 1024, stdout));
    let mut archive = ArchiveScan {
        archive: archive_name.clone(),
        snapshot: snapshot.clone(),
        format: "metadata_only_incomplete".to_string(),
        ..ArchiveScan::default()
    };
    let mut metadata = Vec::new();
    let scan_result = tar.for_each_entry(|header, body| {
        let path_bytes = header.path();
        if path_bytes.ends_with(b"dathost_settings_backup.json") {
            return Ok(());
        }
        let internal_path =
            std::str::from_utf8(path_bytes).map_err(|_| error("invalid tar path"))?;
        if internal_path.ends_with(".fch") {
            archive.player_profiles_present = true;
            return Ok(());
        }
        if internal_path.ends_with(".chunks") {
            let (version, total, entries) = parse_chunks_metadata(body)?;
            archive.metadata_total = Some(total);
            archive.metadata_entries = entries.len();
            metadata = entries;
            if version != 41 {
                return Err(error(format!(
                    "unexpected chunks metadata version {version}"
                )));
            }
        } else if internal_path.ends_with(".chunk") {
            let path_name = internal_path.rsplit('/').next().unwrap_or(internal_path);
            let output = parse_chunk_reader(
                body,
                header.size,
                &archive.archive,
                &archive.snapshot,
                internal_path,
                prefabs,
                &metadata,
                &mut ScrubSink::default(),
            )?;
            archive.format = "chunked_v41".to_string();
            merge_parse(&mut archive, output);
            let _ = path_name;
        } else if internal_path.ends_with("Dedicated.db") {
            let output = parse_legacy_reader(
                body,
                &archive.archive,
                &archive.snapshot,
                internal_path,
                prefabs,
            )?;
            archive.format = "legacy_v37".to_string();
            merge_parse(&mut archive, output);
        } else if internal_path.ends_with(".fwl2") {
            let parsed =
                read_bounded(body, MAX_WORLD_META_BYTES).and_then(|bytes| parse_fwl2_bytes(&bytes));
            merge_world_meta(&mut archive, parsed);
        } else if internal_path.ends_with(".db2") {
            let parsed =
                read_bounded(body, MAX_DB2_ENTRY_BYTES).and_then(|bytes| parse_db2_bytes(&bytes));
            merge_world_meta(&mut archive, parsed);
        }
        Ok(())
    });
    if let Err(scan_error) = scan_result {
        let _ = child.kill();
        let _ = io::copy(&mut tar.reader, &mut io::sink());
        drop(tar);
        let _ = child.wait();
        return Err(scan_error);
    }
    let status = child.wait().map_err(ScanError::from)?;
    if !status.success() {
        return Err(error(format!("zstd failed for {}", archive.archive)));
    }
    if archive.format == "chunked_v41" {
        fill_chunk_revisions(&mut archive, &metadata);
        if archive.metadata_total != Some(archive.zdo_count as i32) {
            return Err(error(format!(
                "{} chunk total mismatch: metadata {:?}, parsed {}",
                archive.archive, archive.metadata_total, archive.zdo_count
            )));
        }
    }
    Ok(archive)
}

fn fill_chunk_revisions(archive: &mut ArchiveScan, metadata: &[ChunkMeta]) {
    for evidence in &mut archive.evidence {
        let Some(filename) = evidence.chunk_filename.as_deref() else {
            continue;
        };
        let Ok((size, _version, index)) = parse_chunk_name(filename) else {
            continue;
        };
        evidence.chunk_revision = metadata
            .iter()
            .find(|meta| meta.chunk_index == index && meta.chunk_size == size)
            .map(|meta| meta.revision);
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn scan_archives(
    archive_dir: &Path,
    zstd_program: &str,
    prefab_path: &Path,
    biome_path: Option<&Path>,
) -> Result<Vec<ArchiveScan>, ScanError> {
    let prefabs = PrefabNames::load(prefab_path)?;
    // Biomes are optional: without the table the map simply has no biome layer.
    let prefabs = match biome_path {
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .map_err(|e| error(format!("cannot read {}: {e}", path.display())))?;
            prefabs.with_biome_text(&text)
        }
        None => prefabs,
    };
    let mut paths: Vec<PathBuf> = std::fs::read_dir(archive_dir)
        .map_err(|e| error(format!("cannot read {}: {e}", archive_dir.display())))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("zst"))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".tar.zst"))
        })
        .collect();
    paths.sort_by_key(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .map(snapshot_id)
            .unwrap_or_default()
    });
    if paths.is_empty() {
        return Err(error(format!(
            "no .tar.zst archives in {}",
            archive_dir.display()
        )));
    }
    paths
        .iter()
        .map(|path| scan_archive(path, zstd_program, &prefabs))
        .collect()
}

pub fn snapshot_id(archive_name: &str) -> String {
    archive_name
        .strip_suffix(".tar.zst")
        .unwrap_or(archive_name)
        .split('_')
        .next()
        .unwrap_or(archive_name)
        .to_string()
}

pub fn totals(archives: &[ArchiveScan]) -> (u64, u64, u64, u64, u64, u64, u64) {
    archives
        .iter()
        .fold((0, 0, 0, 0, 0, 0, 0), |mut totals, archive| {
            totals.0 += archive.zdo_count;
            totals.1 += archive.item_count;
            totals.2 += archive.direct_item_count;
            totals.3 += archive.container_item_count;
            totals.4 += archive.indexed_item_count;
            totals.5 += archive.zdo_cheated_count;
            totals.6 += archive.station_queued_cheated_count;
            totals
        })
}

fn json_escape(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\u{00}'..='\u{1f}' => output.push_str(&format!("\\u{:04x}", character as u32)),
            _ => output.push(character),
        }
    }
    output.push('"');
    output
}

fn json_string(value: &str) -> String {
    json_escape(value)
}

fn json_opt_string(value: Option<&str>) -> String {
    value.map(json_string).unwrap_or_else(|| "null".to_string())
}

fn json_opt_i32(value: Option<i32>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string())
}

fn json_opt_u16(value: Option<u16>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string())
}

fn json_opt_u8(value: Option<u8>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string())
}

fn json_opt_u32(value: Option<u32>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string())
}

fn json_position(position: Position) -> String {
    format!(
        "{{\"x\":{:.6},\"y\":{:.6},\"z\":{:.6}}}",
        position.x, position.y, position.z
    )
}

fn json_evidence(evidence: &Evidence, delta_status: &str) -> String {
    let sector = evidence
        .legacy_sector
        .map(|(x, y)| format!("{{\"x\":{x},\"y\":{y}}}"))
        .unwrap_or_else(|| "null".to_string());
    let grid = evidence
        .grid
        .map(|(x, y)| format!("{{\"x\":{x},\"y\":{y}}}"))
        .unwrap_or_else(|| "null".to_string());
    let custom_keys = evidence
        .custom_keys
        .iter()
        .map(|key| json_string(key))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"archive\":{},\"snapshot\":{},\"internal_path\":{},\"format\":{},\"chunk_filename\":{},\"chunk_version\":{},\"chunk_size\":{},\"chunk_revision\":{},\"zdo_ordinal\":{},\"owner_prefab_hash\":{},\"owner_prefab_name\":{},\"position\":{},\"legacy_sector\":{},\"key_hash\":{},\"key_name\":{},\"kind\":{},\"delta_status\":{},\"item_hash\":{},\"item_name\":{},\"grid\":{},\"quality\":{},\"stack\":{},\"variant\":{},\"crafter_id\":{},\"crafter_name\":{},\"world_level\":{},\"custom_data_keys\":[{}]}}",
        json_string(&evidence.archive),
        json_string(&evidence.snapshot),
        json_string(&evidence.internal_path),
        json_string(&evidence.format),
        json_opt_string(evidence.chunk_filename.as_deref()),
        evidence.chunk_version.map(|value| value.to_string()).unwrap_or_else(|| "null".to_string()),
        evidence.chunk_size.map(|value| value.to_string()).unwrap_or_else(|| "null".to_string()),
        json_opt_u32(evidence.chunk_revision),
        evidence.zdo_ordinal,
        evidence.owner_prefab_hash,
        json_opt_string(evidence.owner_prefab_name.as_deref()),
        json_position(evidence.position),
        sector,
        evidence.key_hash,
        json_string(&evidence.key_name),
        json_string(&evidence.kind),
        json_string(delta_status),
        json_opt_i32(evidence.item_hash),
        json_opt_string(evidence.item_name.as_deref()),
        grid,
        json_opt_u16(evidence.quality),
        json_opt_u16(evidence.stack),
        json_opt_i32(evidence.variant),
        "null",
        json_opt_string(evidence.crafter_name.as_deref()),
        json_opt_u8(evidence.world_level),
        custom_keys,
    )
}

fn evidence_key(evidence: &Evidence) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        evidence.kind,
        evidence.owner_prefab_hash,
        evidence.position.x.to_bits(),
        evidence.position.y.to_bits(),
        evidence.position.z.to_bits(),
        evidence.key_hash,
        evidence
            .item_hash
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        evidence
            .grid
            .map(|grid| grid.0.to_string())
            .unwrap_or_else(|| "none".to_string()),
        evidence
            .grid
            .map(|grid| grid.1.to_string())
            .unwrap_or_else(|| "none".to_string()),
        evidence
            .quality
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        evidence
            .variant
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        evidence
            .crafter_id
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        evidence
            .world_level
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
    )
}

/// Classify each evidence record against the previous archive: the last two scanned archives are
/// compared, whatever they happen to be named.
fn delta_statuses(archives: &[ArchiveScan]) -> Vec<Vec<String>> {
    let mut statuses = archives
        .iter()
        .map(|a| vec!["historical".to_string(); a.evidence.len()])
        .collect::<Vec<_>>();
    if archives.len() < 2 {
        return statuses;
    }
    let old_index = archives.len() - 2;
    let new_index = archives.len() - 1;
    let old = &archives[old_index].evidence;
    let new = &archives[new_index].evidence;
    let mut old_counts = HashMap::<String, usize>::new();
    let mut new_counts = HashMap::<String, usize>::new();
    for evidence in old {
        *old_counts.entry(evidence_key(evidence)).or_default() += 1;
    }
    for evidence in new {
        *new_counts.entry(evidence_key(evidence)).or_default() += 1;
    }
    let mut persisted = HashMap::<String, usize>::new();
    for (key, count) in old_counts {
        persisted.insert(
            key.clone(),
            count.min(new_counts.get(&key).copied().unwrap_or(0)),
        );
    }
    let mut new_persisted = persisted.clone();
    for (index, evidence) in old.iter().enumerate() {
        let remaining = persisted.entry(evidence_key(evidence)).or_default();
        if *remaining > 0 {
            statuses[old_index][index] = "persisted".to_string();
            *remaining -= 1;
        } else {
            statuses[old_index][index] = "removed_or_cleared".to_string();
        }
    }
    for (index, evidence) in new.iter().enumerate() {
        let remaining = new_persisted.entry(evidence_key(evidence)).or_default();
        if *remaining > 0 {
            statuses[new_index][index] = "persisted".to_string();
            *remaining -= 1;
        } else {
            statuses[new_index][index] = "new".to_string();
        }
    }
    statuses
}

pub fn report_json(archives: &[ArchiveScan]) -> String {
    report_json_with_characters(archives, &[])
}

fn report_json_with_characters(archives: &[ArchiveScan], characters: &[CharacterScan]) -> String {
    let totals = totals(archives);
    let statuses = delta_statuses(archives);
    let evidence = archives
        .iter()
        .enumerate()
        .flat_map(|(archive_index, archive)| {
            archive
                .evidence
                .iter()
                .enumerate()
                .map(move |(evidence_index, evidence)| (archive_index, evidence_index, evidence))
        })
        .map(|(archive_index, evidence_index, evidence)| {
            json_evidence(evidence, &statuses[archive_index][evidence_index])
        })
        .collect::<Vec<_>>()
        .join(",\n    ");
    let archive_json = archives
        .iter()
        .map(|archive| {
            format!(
                "{{\"snapshot\":{},\"archive\":{},\"format\":{},\"zdo_count\":{},\"decoded_item_count\":{},\"item_count\":{},\"direct_item_count\":{},\"container_item_count\":{},\"indexed_item_count\":{},\"zdo_cheated_count\":{},\"station_queued_cheated_count\":{},\"metadata_total\":{},\"metadata_entries\":{},\"player_profiles_present\":{},{}}}",
                json_string(&archive.snapshot),
                json_string(&archive.archive),
                json_string(&archive.format),
                archive.zdo_count,
                archive.decoded_item_count,
                archive.item_count,
                archive.direct_item_count,
                archive.container_item_count,
                archive.indexed_item_count,
                archive.zdo_cheated_count,
                archive.station_queued_cheated_count,
                archive.metadata_total.map(|value| value.to_string()).unwrap_or_else(|| "null".to_string()),
                archive.metadata_entries,
                archive.player_profiles_present,
                json_world(archive),
            )
        })
        .collect::<Vec<_>>()
        .join(",\n    ");
    let decoded_total: u64 = archives
        .iter()
        .map(|archive| archive.decoded_item_count)
        .sum();
    let canonical_id = characters
        .iter()
        .find(|character| character.canonical)
        .and_then(|character| character.player_id);
    let character_json = characters
        .iter()
        .map(|character| json_character(character, canonical_id))
        .collect::<Vec<_>>()
        .join(",\n    ");
    let player_profiles_present = !characters.is_empty();
    let stable_hashes = format!(
        "{{\"cheated\":{},\"cheatedQueued\":{},\"itemData\":{},\"items\":{}}}",
        CHEATED, CHEATED_QUEUED, ITEM_DATA, ITEMS
    );
    format!(
        "{{\n  \"tool\":\"valheim-backup-cheat-scanner-rust\",\n  \"summary\":{{\"archive_count\":{},\"zdo_count\":{},\"decoded_item_count\":{},\"item_count\":{},\"direct_item_count\":{},\"container_item_count\":{},\"indexed_item_count\":{},\"zdo_cheated_count\":{},\"station_queued_cheated_count\":{},\"evidence_records\":{},\"player_profiles_present\":{},\"visibility\":{}}},\n  \"observations\":{{\"identity_note\":\"Exact object identity across saves is approximate because chunked records omit persistent ZDOID; matching intentionally excludes stack and uses kind, owner hash, exact float-bit position, key, item, grid, quality, variant, crafter, and worldLevel.\",\"excluded_path\":\"dathost_settings_backup.json\",\"stable_hashes\":{}}},\n  \"archives\":[{}],\n  \"evidence\":[\n    {}\n  ],\n  \"character_timeline\":[\n    {}\n  ]\n}}\n",
        archives.len(),
        totals.0,
        decoded_total,
        totals.1,
        totals.2,
        totals.3,
        totals.4,
        totals.5,
        totals.6,
        totals.1 + totals.5 + totals.6,
        player_profiles_present,
        json_string(if player_profiles_present { "world-and-character" } else { "world-only" }),
        stable_hashes,
        archive_json,
        evidence,
        character_json,
    )
}

fn json_world(archive: &ArchiveScan) -> String {
    let keys = archive
        .world
        .global_keys
        .iter()
        .map(|key| {
            format!(
                "{{\"key\":{},\"value\":{}}}",
                json_string(&key.key),
                key.value
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "null".to_string())
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "\"world_version\":{},\"world_name\":{},\"world_seed\":{},\"world_player_count\":{},\"global_keys\":[{}],\"world_metadata_error\":{}",
        archive
            .world
            .version
            .map(|value| value.to_string())
            .unwrap_or_else(|| "null".to_string()),
        json_opt_string(archive.world.name.as_deref()),
        json_opt_string(archive.world.seed.as_deref()),
        archive
            .world
            .player_count
            .map(|value| value.to_string())
            .unwrap_or_else(|| "null".to_string()),
        keys,
        json_opt_string(archive.world_metadata_error.as_deref()),
    )
}

fn json_character(character: &CharacterScan, canonical_id: Option<i64>) -> String {
    let lineage = character_lineage(character, canonical_id);
    format!(
        "{{\"source\":{},\"canonical\":{},\"modified_unix\":{},\"trusted\":{},\"supported\":{},\"parse_error\":{},\"file_bytes\":{},\"payload_bytes\":{},\"hash_bytes\":{},\"hash_valid\":{},\"profile_name\":{},\"profile_version\":{},\"lineage\":{},\"m_usedCheats\":{},\"Cheats_stats_nonzero\":{},\"known_cheat_command_hits\":{},\"playerData_version\":{},\"inventory_version\":{},\"inventory_item_count\":{},\"cheated_inventory_count\":{},\"bypasscheatchecks_1\":{},\"player_data_complete\":{}}}",
        json_string(&character.source),
        character.canonical,
        character.modified_unix.map(|value| value.to_string()).unwrap_or_else(|| "null".to_string()),
        character.trusted,
        character.supported,
        json_opt_string(character.parse_error.as_deref()),
        character.file_bytes,
        character.payload_bytes,
        character.hash_bytes,
        character.hash_valid,
        json_string(&character.player_name),
        character.profile_version,
        json_string(lineage),
        character.used_cheats,
        character.cheat_stat_nonzero_count,
        character.known_command_hits,
        character.player_data_version.map(|value| value.to_string()).unwrap_or_else(|| "null".to_string()),
        character.inventory_version.map(|value| value.to_string()).unwrap_or_else(|| "null".to_string()),
        character.inventory_item_count,
        character.cheated_inventory_count,
        character.bypass_cheat_checks,
        character.player_data_complete,
    )
}

pub fn report_markdown(archives: &[ArchiveScan]) -> String {
    report_markdown_with_characters(archives, &[])
}

fn csv_escape(value: &str) -> String {
    if value
        .bytes()
        .any(|byte| matches!(byte, b',' | b'"' | b'\r' | b'\n'))
    {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn csv_row(fields: impl IntoIterator<Item = String>) -> String {
    fields
        .into_iter()
        .map(|field| csv_escape(&field))
        .collect::<Vec<_>>()
        .join(",")
        + "\n"
}

fn opt_display<T: ToString>(value: Option<T>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

#[derive(Debug, Clone)]
struct ConsolidatedEvidence {
    key: String,
    occurrence: usize,
    snapshots: [Option<Evidence>; 3],
}

/// Snapshot labels for the consolidated presence table: the last up-to-three archives, oldest
/// first, left-padded with empty labels when fewer archives were scanned.
fn consolidated_labels(archives: &[ArchiveScan]) -> [String; 3] {
    let window = &archives[archives.len().saturating_sub(3)..];
    let offset = 3 - window.len();
    let mut labels = [String::new(), String::new(), String::new()];
    for (slot, archive) in window.iter().enumerate() {
        labels[offset + slot] = archive.snapshot.clone();
    }
    labels
}

fn consolidated_evidence(archives: &[ArchiveScan]) -> Vec<ConsolidatedEvidence> {
    let window = &archives[archives.len().saturating_sub(3)..];
    let offset = 3 - window.len();
    let mut grouped = HashMap::<String, Vec<[Option<Evidence>; 3]>>::new();
    for (index, archive) in window.iter().enumerate() {
        let snapshot_index = offset + index;
        for evidence in &archive.evidence {
            let rows = grouped.entry(evidence_key(evidence)).or_default();
            let row_index = rows.iter().position(|row| row[snapshot_index].is_none());
            let row = if let Some(row_index) = row_index {
                &mut rows[row_index]
            } else {
                rows.push([None, None, None]);
                rows.last_mut().unwrap()
            };
            row[snapshot_index] = Some(evidence.clone());
        }
    }
    let mut output = grouped
        .into_iter()
        .flat_map(|(key, rows)| {
            rows.into_iter()
                .enumerate()
                .map(move |(index, snapshots)| ConsolidatedEvidence {
                    key: key.clone(),
                    occurrence: index + 1,
                    snapshots,
                })
        })
        .collect::<Vec<_>>();
    output.sort_by(|left, right| {
        left.key
            .cmp(&right.key)
            .then(left.occurrence.cmp(&right.occurrence))
    });
    output
}

fn consolidated_status(row: &ConsolidatedEvidence) -> &'static str {
    match (&row.snapshots[1], &row.snapshots[2]) {
        (Some(_), Some(_)) => "persisted",
        (Some(_), None) => "removed_or_cleared",
        (None, Some(_)) => "new",
        (None, None) => "historical",
    }
}

fn consolidated_value(row: &ConsolidatedEvidence) -> &Evidence {
    row.snapshots
        .iter()
        .find_map(Option::as_ref)
        .expect("consolidated evidence row has a snapshot")
}

fn consolidated_snapshot_fields(snapshot: &Option<Evidence>) -> [String; 5] {
    match snapshot {
        Some(evidence) => [
            "true".to_string(),
            opt_display(evidence.stack),
            evidence.archive.clone(),
            evidence.internal_path.clone(),
            evidence.chunk_filename.clone().unwrap_or_default(),
        ],
        None => [
            "false".to_string(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
        ],
    }
}

fn consolidated_fields(row: &ConsolidatedEvidence, archives: &[ArchiveScan]) -> Vec<String> {
    let evidence = consolidated_value(row);
    let mut fields = vec![
        row.occurrence.to_string(),
        evidence.kind.clone(),
        evidence.owner_prefab_hash.to_string(),
        evidence.owner_prefab_name.clone().unwrap_or_default(),
        opt_display(evidence.item_hash),
        evidence.item_name.clone().unwrap_or_default(),
        format!("{:.6}", evidence.position.x),
        format!("{:.6}", evidence.position.y),
        format!("{:.6}", evidence.position.z),
        evidence
            .grid
            .map(|grid| grid.0.to_string())
            .unwrap_or_default(),
        evidence
            .grid
            .map(|grid| grid.1.to_string())
            .unwrap_or_default(),
        evidence.key_hash.to_string(),
        evidence.key_name.clone(),
        opt_display(evidence.quality),
        opt_display(evidence.variant),
        evidence.crafter_name.clone().unwrap_or_default(),
        opt_display(evidence.world_level),
    ];
    let labels = consolidated_labels(archives);
    for (slot, snapshot) in row.snapshots.iter().enumerate() {
        // Padding slots have no header, so they must not emit fields.
        if !labels[slot].is_empty() {
            fields.extend(consolidated_snapshot_fields(snapshot));
        }
    }
    fields.push(consolidated_status(row).to_string());
    fields
}

fn consolidated_headers(archives: &[ArchiveScan]) -> Vec<String> {
    let mut headers = vec![
        "occurrence_index",
        "kind",
        "owner_prefab_hash",
        "owner_prefab_name",
        "item_hash",
        "item_name",
        "x",
        "y",
        "z",
        "grid_x",
        "grid_y",
        "key_hash",
        "key_name",
        "quality",
        "variant",
        "crafter_name",
        "world_level",
    ]
    .into_iter()
    .map(String::from)
    .collect::<Vec<_>>();
    for label in consolidated_labels(archives) {
        if label.is_empty() {
            continue;
        }
        let suffix = label
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() {
                    character
                } else {
                    '_'
                }
            })
            .collect::<String>();
        headers.extend([
            format!("present_{suffix}"),
            format!("stack_{suffix}"),
            format!("source_{suffix}"),
            format!("path_{suffix}"),
            format!("chunk_{suffix}"),
        ]);
    }
    headers.push("delta_status".to_string());
    headers
}

pub fn world_evidence_csv(archives: &[ArchiveScan]) -> String {
    let mut output = csv_row(consolidated_headers(archives));
    for row in consolidated_evidence(archives) {
        output.push_str(&csv_row(consolidated_fields(&row, archives)));
    }
    output
}

fn character_lineage(character: &CharacterScan, canonical_id: Option<i64>) -> &'static str {
    match (character.player_id, canonical_id) {
        (Some(player_id), Some(canonical)) if player_id == canonical => "same_lineage",
        (Some(_), Some(_)) => "different_lineage",
        _ => "unavailable",
    }
}

pub fn character_evidence_csv(characters: &[CharacterScan]) -> String {
    let canonical_id = characters
        .iter()
        .find(|character| character.canonical)
        .and_then(|character| character.player_id);
    let mut output = csv_row(
        [
            "timestamp_unix",
            "source",
            "canonical",
            "lineage",
            "profile_name",
            "profile_version",
            "trusted",
            "m_usedCheats",
            "Cheats_stats_nonzero",
            "known_cheat_command_hits",
            "playerData_version",
            "inventory_version",
            "inventory_item_count",
            "cheated_inventory_count",
            "bypasscheatchecks_1",
            "hash_valid",
            "file_bytes",
            "payload_bytes",
            "hash_bytes",
            "supported",
            "parse_error",
            "player_data_complete",
        ]
        .into_iter()
        .map(String::from),
    );
    for character in characters {
        output.push_str(&csv_row([
            opt_display(character.modified_unix),
            character.source.clone(),
            character.canonical.to_string(),
            character_lineage(character, canonical_id).to_string(),
            character.player_name.clone(),
            character.profile_version.to_string(),
            character.trusted.to_string(),
            character.used_cheats.to_string(),
            character.cheat_stat_nonzero_count.to_string(),
            character.known_command_hits.to_string(),
            opt_display(character.player_data_version),
            opt_display(character.inventory_version),
            character.inventory_item_count.to_string(),
            character.cheated_inventory_count.to_string(),
            character.bypass_cheat_checks.to_string(),
            character.hash_valid.to_string(),
            character.file_bytes.to_string(),
            character.payload_bytes.to_string(),
            character.hash_bytes.to_string(),
            character.supported.to_string(),
            character.parse_error.clone().unwrap_or_default(),
            character.player_data_complete.to_string(),
        ]));
    }
    output
}

fn report_markdown_with_characters(
    archives: &[ArchiveScan],
    characters: &[CharacterScan],
) -> String {
    let totals = totals(archives);
    let latest = archives.last();
    let previous = archives.iter().rev().nth(1);
    let mut output = String::new();
    let decoded_total: u64 = archives
        .iter()
        .map(|archive| archive.decoded_item_count)
        .sum();
    output.push_str("# Valheim cheat audit (Rust scanner)\n\n");
    output.push_str(if characters.is_empty() { "World-only scan: no matching player `.fch` profiles were found. `dathost_settings_backup.json` was not read.\n\n" } else { "World and character scan: player profiles are read-only inputs; the canonical file was already copied by the parent. `dathost_settings_backup.json` was not read.\n\n" });
    output.push_str("## Totals\n\n");
    output.push_str(&format!("- Archives: {}\n- ZDO records: {}\n- Decoded individual item records: {}\n- Cheated item evidence: {} (direct {}, container {}, indexed {})\n- Cheated ZDO flags: {}\n- Station queued-cheat hits: {}\n- Evidence records: {}\n", archives.len(), totals.0, decoded_total, totals.1, totals.2, totals.3, totals.4, totals.5, totals.6, totals.1 + totals.5 + totals.6));
    output.push_str("\n## Snapshot counts\n\n| Snapshot | Format | ZDOs | Decoded items | Evidence items | Direct | Container | ZDO flags | Queued flags |\n|---|---:|---:|---:|---:|---:|---:|---:|---:|\n");
    for archive in archives {
        output.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            archive.snapshot,
            archive.format,
            archive.zdo_count,
            archive.decoded_item_count,
            archive.item_count,
            archive.direct_item_count,
            archive.container_item_count,
            archive.zdo_cheated_count,
            archive.station_queued_cheated_count
        ));
    }
    let worlds = archives
        .iter()
        .filter(|archive| archive.world.version.is_some() || archive.world.name.is_some())
        .collect::<Vec<_>>();
    if !worlds.is_empty() {
        output.push_str("\n## World metadata\n\n| Snapshot | World | Version | Seed | Players | Progression flags |\n|---|---|---:|---|---:|---:|\n");
        for archive in &worlds {
            output.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} |\n",
                archive.snapshot,
                archive
                    .world
                    .name
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
                opt_display(archive.world.version),
                archive
                    .world
                    .seed
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
                opt_display(archive.world.player_count),
                archive.world.global_keys.len()
            ));
        }
        if let Some(latest) = worlds.last() {
            if !latest.world.global_keys.is_empty() {
                output.push_str(&format!(
                    "\nProgression flags in the latest snapshot (`{}`):\n\n| Flag | Value |\n|---|---:|\n",
                    latest.snapshot
                ));
                for key in &latest.world.global_keys {
                    output.push_str(&format!("| {} | {} |\n", key.key, opt_display(key.value)));
                }
            }
            output.push_str("\nPlayer names and ids from the world file are never reported.\n");
        }
        for archive in archives
            .iter()
            .filter(|archive| archive.world_metadata_error.is_some())
        {
            output.push_str(&format!(
                "\n- world metadata error in {}: {}\n",
                archive.snapshot,
                archive.world_metadata_error.clone().unwrap_or_default()
            ));
        }
    }
    if let Some(latest) = latest {
        if !latest.biomes.is_empty() {
            let mut histogram = [0usize; BIOME_COUNT];
            let mut coloured = 0usize;
            let empty_real = BiomeTally::default();
            for (cell, tally) in &latest.biomes {
                let real = latest.biomes_real.get(cell).unwrap_or(&empty_real);
                if let Some((index, _)) = biome_verdict_preferring_real(real, tally) {
                    histogram[index] += 1;
                    coloured += 1;
                }
            }
            let populated = latest.grid.len().max(1);
            let breakdown = BIOMES
                .iter()
                .enumerate()
                .filter(|(index, _)| histogram[*index] > 0)
                .map(|(index, (_, name))| format!("{name} {}", histogram[index]))
                .collect::<Vec<_>>()
                .join(", ");
            let detail = if breakdown.is_empty() {
                String::new()
            } else {
                format!(" Verdicts: {breakdown}.")
            };
            output.push_str(&format!(
                "\n## Map biome layer\n\n{coloured} of {populated} populated 64 m cells carry a biome verdict ({}% of the newest snapshot). Cells with too little evidence stay blank, so undeveloped, ocean and unexplored ground is never guessed.{detail}\n\n",
                coloured * 100 / populated,
            ));
        }
    }
    if let (Some(old), Some(new)) = (previous, latest) {
        output.push_str(&format!("\n## Latest snapshot change\n\nCompared with **{}**, latest **{}** has ZDO count {} -> {}, item hits {} -> {}, ZDO flags {} -> {}, and queued flags {} -> {}.\n\nMatching is approximate because chunked records omit persistent ZDOID.\n", old.snapshot, new.snapshot, old.zdo_count, new.zdo_count, old.item_count, new.item_count, old.zdo_cheated_count, new.zdo_cheated_count, old.station_queued_cheated_count, new.station_queued_cheated_count));
    }
    let consolidated = consolidated_evidence(archives);
    let persisted = consolidated
        .iter()
        .filter(|row| consolidated_status(row) == "persisted")
        .count();
    let removed = consolidated
        .iter()
        .filter(|row| consolidated_status(row) == "removed_or_cleared")
        .count();
    let new_rows = consolidated
        .iter()
        .filter(|row| consolidated_status(row) == "new")
        .count();
    let covered = archives.len().min(3);
    output.push_str(&format!("\n## Consolidated evidence summary\n\nThe consolidated table covers the last {} of {} scanned snapshots and has **{} logical rows**: **{} persisted**, **{} removed/cleared**, and **{} new**. Identity matching intentionally excludes stack and uses kind, owner hash, exact float-bit position, key, item, grid, quality, variant, crafter, and worldLevel. Stack is observation-only.\n\n", covered, archives.len(), consolidated.len(), persisted, removed, new_rows));
    output.push_str("## Consolidated evidence appendix\n\n");
    output.push_str(&format!(
        "| {} |\n|{}|\n",
        consolidated_headers(archives)
            .iter()
            .map(|header| header.as_str())
            .collect::<Vec<_>>()
            .join(" | "),
        consolidated_headers(archives)
            .iter()
            .map(|_| "---")
            .collect::<Vec<_>>()
            .join("|")
    ));
    for row in &consolidated {
        output.push_str(&format!(
            "| {} |\n",
            consolidated_fields(row, archives)
                .iter()
                .map(|field| field.replace('|', "\\|").replace('\n', " "))
                .collect::<Vec<_>>()
                .join(" | ")
        ));
    }
    output.push_str("\nThe complete machine-readable world table is `world-evidence.csv`; it retains source, path, chunk, ordinal, coordinates, and per-snapshot stack values.\n\n");
    output.push_str("## Cleanup guidance\n\n");
    output.push_str("Normal pickup, move, and drop operations preserve or spread the item flag. Crafting or building with any cheated matching resource contaminates the crafted output or placed piece. Hammer removal of cheated pieces creates cheated refunds, and normal container destruction drops flagged contents. Direct authoritative `ZNetScene.Destroy` deletion avoids explicit refund/drop callbacks; vanilla `forcedelete` can invoke `Destructible` callbacks and is not universally safe. `removedrops` also affects loaded clean drops and misses unloaded or inventory evidence.\n\nThe safest cleanup is a runtime server tool that deletes flagged inventory entries and directly deletes target ZDO/ItemDrop objects, followed by a clean rebuild/save/rescan. Do not use bypass settings as cleanup. Delete a flagged creature object before its death to avoid cheated drops. To keep the objects and only clear their flags, run `--scrub-world` on a copy of the archive (see RUST_README.md).\n\n");
    output.push_str("## Character timeline\n\n");
    if characters.is_empty() {
        output.push_str("No matching character files were found.\n");
    } else {
        let canonical_id = characters
            .iter()
            .find(|character| character.canonical)
            .and_then(|character| character.player_id);
        output.push_str("| Timestamp (Unix) | Profile | Version | Trusted | Lineage | m_usedCheats | Cheats stats | Cheated inventory | Bypass key | Hash valid | Status | Source |\n|---:|---|---:|---:|---|---:|---:|---:|---:|---:|---|---|\n");
        for character in characters {
            output.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | `{}` |\n",
                opt_display(character.modified_unix),
                character.player_name,
                character.profile_version,
                character.trusted,
                character_lineage(character, canonical_id),
                character.used_cheats,
                character.cheat_stat_nonzero_count,
                character.cheated_inventory_count,
                character.bypass_cheat_checks,
                character.hash_valid,
                if !character.trusted {
                    "untrusted"
                } else if character.supported {
                    "supported"
                } else {
                    "unsupported"
                },
                character.source
            ));
        }
        output.push_str("\nThe canonical file was already copied by the parent; this scanner reads profiles read-only. Matching Steam Cloud files were read directly; embedded playerID equality, not profile naming, determines the same-lineage label. Raw player IDs are intentionally omitted from reports. Unsupported old payloads and untrusted hashes are reported as untrusted/unsupported rather than guessed. The complete table is `character-evidence.csv`.\n");
    }
    output
}

#[cfg(not(target_arch = "wasm32"))]
pub fn write_reports(output_dir: &Path, archives: &[ArchiveScan]) -> Result<(), ScanError> {
    write_reports_with_characters(output_dir, archives, &[])
}

#[cfg(not(target_arch = "wasm32"))]
pub fn write_reports_with_characters(
    output_dir: &Path,
    archives: &[ArchiveScan],
    characters: &[CharacterScan],
) -> Result<(), ScanError> {
    std::fs::create_dir_all(output_dir)
        .map_err(|e| error(format!("cannot create {}: {e}", output_dir.display())))?;
    std::fs::write(
        output_dir.join("cheat-audit.json"),
        report_json_with_characters(archives, characters),
    )
    .map_err(|e| error(format!("cannot write JSON report: {e}")))?;
    std::fs::write(
        output_dir.join("CHEAT_AUDIT.md"),
        report_markdown_with_characters(archives, characters),
    )
    .map_err(|e| error(format!("cannot write Markdown report: {e}")))?;
    std::fs::write(
        output_dir.join("world-evidence.csv"),
        world_evidence_csv(archives),
    )
    .map_err(|e| error(format!("cannot write world CSV report: {e}")))?;
    std::fs::write(
        output_dir.join("character-evidence.csv"),
        character_evidence_csv(characters),
    )
    .map_err(|e| error(format!("cannot write character CSV report: {e}")))?;
    Ok(())
}

fn validate_browser_tar_path(value: &str) -> Result<(), ScanError> {
    let normalized = value.replace('\\', "/");
    if normalized.starts_with('/')
        || normalized
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == ".." || part.contains(':'))
    {
        return Err(error("invalid tar path"));
    }
    Ok(())
}

fn browser_filename(value: &str) -> String {
    value
        .rsplit(['/', '\\'])
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or("unnamed")
        .to_string()
}

fn browser_internal_path(value: &str) -> String {
    let mut parts = Vec::new();
    let normalized = value.replace('\\', "/");
    for part in normalized.split('/') {
        if part.is_empty() || part == "." || part.ends_with(':') {
            continue;
        }
        if part == ".." {
            parts.pop();
        } else {
            parts.push(part);
        }
    }
    if parts.is_empty() {
        "(root)".to_string()
    } else {
        parts.join("/")
    }
}

fn browser_float(value: f32) -> String {
    if value.is_finite() {
        format!("{value:.6}")
    } else {
        "null".to_string()
    }
}

fn browser_position(position: Position) -> String {
    format!(
        "{{\"x\":{},\"y\":{},\"z\":{}}}",
        browser_float(position.x),
        browser_float(position.y),
        browser_float(position.z)
    )
}

#[derive(Debug, Clone)]
struct BrowserLineage {
    status: &'static str,
    first_seen_snapshot: String,
    last_seen_snapshot: String,
    present_in_latest: bool,
    occurrence_index: usize,
}

#[derive(Debug)]
struct BrowserOccurrence {
    first_archive_index: usize,
    last_archive_index: usize,
    first_seen_snapshot: String,
    last_seen_snapshot: String,
    present_in_latest: bool,
}

fn browser_lineages(archives: &[ArchiveScan]) -> Vec<Vec<BrowserLineage>> {
    let latest_index = archives.len().checked_sub(1);
    let mut occurrences = HashMap::<String, Vec<BrowserOccurrence>>::new();
    let mut rows = archives
        .iter()
        .map(|archive| Vec::<(String, usize)>::with_capacity(archive.evidence.len()))
        .collect::<Vec<_>>();

    for (archive_index, archive) in archives.iter().enumerate() {
        let mut per_archive_counts = HashMap::<String, usize>::new();
        for evidence in &archive.evidence {
            let key = evidence_key(evidence);
            let occurrence_index = per_archive_counts.entry(key.clone()).or_default();
            *occurrence_index += 1;
            let occurrence_index = *occurrence_index;
            let history = occurrences.entry(key.clone()).or_default();
            if history.len() < occurrence_index {
                history.push(BrowserOccurrence {
                    first_archive_index: archive_index,
                    last_archive_index: archive_index,
                    first_seen_snapshot: archive.snapshot.clone(),
                    last_seen_snapshot: archive.snapshot.clone(),
                    present_in_latest: Some(archive_index) == latest_index,
                });
            } else {
                let occurrence = &mut history[occurrence_index - 1];
                occurrence.last_archive_index = archive_index;
                occurrence.last_seen_snapshot = archive.snapshot.clone();
                occurrence.present_in_latest = Some(archive_index) == latest_index;
            }
            rows[archive_index].push((key, occurrence_index));
        }
    }

    rows.into_iter()
        .map(|archive_rows| {
            archive_rows
                .into_iter()
                .map(|(key, occurrence_index)| {
                    let occurrence = &occurrences[&key][occurrence_index - 1];
                    let status = if archives.len() < 2 {
                        "observed"
                    } else if Some(occurrence.last_archive_index) == latest_index {
                        if occurrence.first_archive_index == occurrence.last_archive_index {
                            "new"
                        } else {
                            "persisted"
                        }
                    } else {
                        "removed_or_cleared"
                    };
                    BrowserLineage {
                        status,
                        first_seen_snapshot: occurrence.first_seen_snapshot.clone(),
                        last_seen_snapshot: occurrence.last_seen_snapshot.clone(),
                        present_in_latest: occurrence.present_in_latest,
                        occurrence_index,
                    }
                })
                .collect()
        })
        .collect()
}

fn browser_json_evidence(evidence: &Evidence, lineage: &BrowserLineage) -> String {
    let sector = evidence
        .legacy_sector
        .map(|(x, y)| format!("{{\"x\":{x},\"y\":{y}}}"))
        .unwrap_or_else(|| "null".to_string());
    let grid = evidence
        .grid
        .map(|(x, y)| format!("{{\"x\":{x},\"y\":{y}}}"))
        .unwrap_or_else(|| "null".to_string());
    let custom_keys = evidence
        .custom_keys
        .iter()
        .map(|key| json_string(key))
        .collect::<Vec<_>>()
        .join(",");
    let chunk = evidence.chunk_filename.as_deref().map(browser_filename);
    format!(
        "{{\"status\":{},\"first_seen_snapshot\":{},\"last_seen_snapshot\":{},\"present_in_latest\":{},\"occurrence_index\":{},\"archive\":{},\"snapshot\":{},\"source\":{},\"internal_path\":{},\"format\":{},\"chunk\":{},\"chunk_version\":{},\"chunk_size\":{},\"chunk_revision\":{},\"zdo_ordinal\":{},\"owner_prefab_hash\":{},\"owner_prefab_name\":{},\"position\":{},\"legacy_sector\":{},\"key_hash\":{},\"key_name\":{},\"kind\":{},\"item_hash\":{},\"item_name\":{},\"grid\":{},\"quality\":{},\"stack\":{},\"variant\":{},\"crafter_name\":{},\"world_level\":{},\"custom_data_keys\":[{}]}}",
        json_string(lineage.status),
        json_string(&lineage.first_seen_snapshot),
        json_string(&lineage.last_seen_snapshot),
        lineage.present_in_latest,
        lineage.occurrence_index,
        json_string(&browser_filename(&evidence.archive)),
        json_string(&browser_filename(&evidence.snapshot)),
        json_string(&browser_filename(&evidence.archive)),
        json_string(&browser_internal_path(&evidence.internal_path)),
        json_string(&evidence.format),
        chunk.as_deref().map(json_string).unwrap_or_else(|| "null".to_string()),
        evidence.chunk_version.map(|value| value.to_string()).unwrap_or_else(|| "null".to_string()),
        evidence.chunk_size.map(|value| value.to_string()).unwrap_or_else(|| "null".to_string()),
        json_opt_u32(evidence.chunk_revision),
        evidence.zdo_ordinal,
        evidence.owner_prefab_hash,
        json_opt_string(evidence.owner_prefab_name.as_deref()),
        browser_position(evidence.position),
        sector,
        evidence.key_hash,
        json_string(&evidence.key_name),
        json_string(&evidence.kind),
        json_opt_i32(evidence.item_hash),
        json_opt_string(evidence.item_name.as_deref()),
        grid,
        json_opt_u16(evidence.quality),
        json_opt_u16(evidence.stack),
        json_opt_i32(evidence.variant),
        json_opt_string(evidence.crafter_name.as_deref()),
        json_opt_u8(evidence.world_level),
        custom_keys,
    )
}

fn browser_character_status(character: &CharacterScan) -> &'static str {
    if !character.hash_valid {
        "untrusted"
    } else if !character.supported {
        "unsupported"
    } else if character.trusted {
        "trusted"
    } else {
        "untrusted"
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn browser_modified_unix(modified_unix_millis: f64) -> Option<i64> {
    if modified_unix_millis.is_finite() && modified_unix_millis >= 0.0 {
        Some((modified_unix_millis / 1000.0).floor() as i64)
    } else {
        None
    }
}

fn browser_character_available(character: &CharacterScan) -> bool {
    character.trusted && character.supported && character.parse_error.is_none()
}

#[cfg(any(target_arch = "wasm32", test))]
fn browser_canonical_index(characters: &[CharacterScan]) -> Option<usize> {
    let valid = characters
        .iter()
        .enumerate()
        .filter(|(_, character)| {
            character.canonical_eligible && browser_character_available(character)
        })
        .collect::<Vec<_>>();
    match valid.as_slice() {
        [] => None,
        [(_, _)] => Some(valid[0].0),
        _ => {
            let newest = valid
                .iter()
                .max_by_key(|(_, character)| character.modified_unix.unwrap_or_default())
                .map(|(index, character)| (*index, character.modified_unix.unwrap_or_default()))?;
            (valid
                .iter()
                .filter(|(_, character)| character.modified_unix.unwrap_or_default() == newest.1)
                .count()
                == 1)
                .then_some(newest.0)
        }
    }
}

fn browser_opt_bool(value: bool, available: bool) -> String {
    if available {
        value.to_string()
    } else {
        "null".to_string()
    }
}

fn browser_opt_u32(value: u32, available: bool) -> String {
    if available {
        value.to_string()
    } else {
        "null".to_string()
    }
}

fn browser_json_character(character: &CharacterScan, canonical_id: Option<i64>) -> String {
    let available = browser_character_available(character);
    format!(
        "{{\"status\":{},\"source\":{},\"canonical\":{},\"modified_unix\":{},\"trusted\":{},\"supported\":{},\"hash_valid\":{},\"parse_error\":{},\"profile_name\":{},\"profile_version\":{},\"lineage\":{},\"used_cheats\":{},\"cheat_stat_nonzero_count\":{},\"known_command_hits\":{},\"player_data_version\":{},\"inventory_version\":{},\"inventory_item_count\":{},\"cheated_inventory_count\":{},\"bypass_cheat_checks\":{},\"player_data_complete\":{},\"file_bytes\":{},\"payload_bytes\":{},\"hash_bytes\":{}}}",
        json_string(browser_character_status(character)),
        json_string(&browser_filename(&character.source)),
        character.canonical,
        character.modified_unix.map(|value| value.to_string()).unwrap_or_else(|| "null".to_string()),
        character.trusted,
        character.supported,
        character.hash_valid,
        json_opt_string(character.parse_error.as_deref()),
        if available { json_string(&character.player_name) } else { "null".to_string() },
        if available { character.profile_version.to_string() } else { "null".to_string() },
        json_string(character_lineage(character, canonical_id)),
        browser_opt_bool(character.used_cheats, available),
        browser_opt_u32(character.cheat_stat_nonzero_count, available),
        browser_opt_u32(character.known_command_hits, available),
        if available { character.player_data_version.map(|value| value.to_string()).unwrap_or_else(|| "null".to_string()) } else { "null".to_string() },
        if available { character.inventory_version.map(|value| value.to_string()).unwrap_or_else(|| "null".to_string()) } else { "null".to_string() },
        browser_opt_u32(character.inventory_item_count, available),
        browser_opt_u32(character.cheated_inventory_count, available),
        browser_opt_bool(character.bypass_cheat_checks, available),
        browser_opt_bool(character.player_data_complete, available),
        character.file_bytes,
        character.payload_bytes,
        character.hash_bytes,
    )
}

/// Top biome-vote weights for a cell, richest first, ties broken by biome index for a
/// deterministic order. Zero-weight biomes are dropped rather than padded in.
fn ranked_biome_votes(tally: &BiomeTally) -> Vec<(usize, u32)> {
    let mut ranked = tally[..BIOME_COUNT]
        .iter()
        .enumerate()
        .filter(|(_, weight)| **weight > 0)
        .map(|(index, weight)| (index, *weight))
        .collect::<Vec<_>>();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    ranked.truncate(3);
    ranked
}

/// Per-cell biome evidence behind the map popup: the combined tally's total weight, the decided
/// verdict (if any) and which tally decided it, and the top vote weights — enough for a popup to
/// show either the verdict or, when there is none, why ("too little evidence: 24 of 36" / "no clear
/// winner: top share 48%"). Only cells that carry any biome-tagged evidence appear; a populated cell
/// with none is left out, and the browser treats that the same as an all-zero entry.
fn browser_biome_detail_json(latest: &ArchiveScan) -> String {
    let empty = BiomeTally::default();
    let cells = latest
        .biomes
        .keys()
        .chain(latest.biomes_real.keys())
        .collect::<std::collections::BTreeSet<_>>();
    let body = cells
        .iter()
        .map(|cell| {
            let real = latest.biomes_real.get(*cell).unwrap_or(&empty);
            let combined = latest.biomes.get(*cell).unwrap_or(&empty);
            let (decided, source) = match biome_verdict(real) {
                Some(verdict) => (Some(verdict), "real"),
                None => match biome_verdict(combined) {
                    Some(verdict) => (Some(verdict), "hint"),
                    None => (None, "none"),
                },
            };
            let (decided_index, decided_share) = decided.unwrap_or((0, 0));
            let top = ranked_biome_votes(combined)
                .iter()
                .map(|(index, weight)| format!("[{index},{weight}]"))
                .collect::<Vec<_>>()
                .join(",");
            let (x, z) = **cell;
            format!(
                "[{x},{z},{},{},{},{},[{top}]]",
                combined[BIOME_TOTAL],
                if decided.is_some() {
                    decided_index as i64
                } else {
                    -1
                },
                decided_share,
                json_string(source),
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("[{body}]")
}

/// Spatial density for the map view: ZDO counts per world cell, taken from the
/// newest archive (the current world state). Derived from the save alone, so the
/// map needs no client-side cache.
fn browser_map_json(archives: &[ArchiveScan]) -> String {
    let Some(latest) = archives.last() else {
        return "null".to_string();
    };
    let mut cells = latest.grid.iter().collect::<Vec<_>>();
    // Deterministic order keeps generated reports diffable.
    cells.sort_by_key(|((x, z), _)| (*x, *z));
    let body = cells
        .iter()
        .map(|((x, z), count)| format!("[{x},{z},{count}]"))
        .collect::<Vec<_>>()
        .join(",");
    // Biome layer: only cells whose evidence clears the threshold, so undeveloped, ocean and
    // unexplored ground stays blank instead of being guessed.
    let empty_real = BiomeTally::default();
    let mut biome_cells = latest
        .biomes
        .iter()
        .filter_map(|(cell, tally)| {
            let real = latest.biomes_real.get(cell).unwrap_or(&empty_real);
            biome_verdict_preferring_real(real, tally).map(|(index, _)| (*cell, index))
        })
        .collect::<Vec<_>>();
    biome_cells.sort_by_key(|((x, z), _)| (*x, *z));
    let biome_body = biome_cells
        .iter()
        .map(|((x, z), index)| format!("[{x},{z},{index}]"))
        .collect::<Vec<_>>()
        .join(",");
    let biome_names = BIOMES
        .iter()
        .map(|(_, name)| json_string(name))
        .collect::<Vec<_>>()
        .join(",");
    let biome_detail = browser_biome_detail_json(latest);
    format!(
        "{{\"cell_meters\":{},\"snapshot\":{},\"cells\":[{}],\"biomes\":[{}],\"biome_names\":[{}],\"biome_detail\":{}}}",
        MAP_CELL_METERS,
        json_string(&browser_filename(&latest.snapshot)),
        body,
        biome_body,
        biome_names,
        biome_detail
    )
}

pub fn browser_report_json(archives: &[ArchiveScan], characters: &[CharacterScan]) -> String {
    let lineages = browser_lineages(archives);
    let totals = totals(archives);
    let decoded_total: u64 = archives
        .iter()
        .map(|archive| archive.decoded_item_count)
        .sum();
    let archive_json = archives
        .iter()
        .map(|archive| {
            format!(
                "{{\"archive\":{},\"snapshot\":{},\"format\":{},\"zdo_count\":{},\"decoded_item_count\":{},\"item_count\":{},\"direct_item_count\":{},\"container_item_count\":{},\"indexed_item_count\":{},\"zdo_cheated_count\":{},\"station_queued_cheated_count\":{},\"metadata_total\":{},\"metadata_entries\":{},\"player_profiles_present\":{},{}}}",
                json_string(&browser_filename(&archive.archive)),
                json_string(&browser_filename(&archive.snapshot)),
                json_string(&archive.format),
                archive.zdo_count,
                archive.decoded_item_count,
                archive.item_count,
                archive.direct_item_count,
                archive.container_item_count,
                archive.indexed_item_count,
                archive.zdo_cheated_count,
                archive.station_queued_cheated_count,
                archive.metadata_total.map(|value| value.to_string()).unwrap_or_else(|| "null".to_string()),
                archive.metadata_entries,
                archive.player_profiles_present,
                json_world(archive),
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let evidence_json = archives
        .iter()
        .zip(lineages.iter())
        .flat_map(|(archive, archive_lineages)| {
            archive
                .evidence
                .iter()
                .zip(archive_lineages.iter())
                .map(|(evidence, lineage)| browser_json_evidence(evidence, lineage))
        })
        .collect::<Vec<_>>()
        .join(",");
    let canonical_id = characters
        .iter()
        .find(|character| character.canonical)
        .and_then(|character| character.player_id);
    let character_json = characters
        .iter()
        .map(|character| browser_json_character(character, canonical_id))
        .collect::<Vec<_>>()
        .join(",");
    let map_json = browser_map_json(archives);
    format!(
        "{{\"tool\":\"Valheim Cheat Radar\",\"schema_version\":1,\"read_only\":true,\"summary\":{{\"archive_count\":{},\"zdo_count\":{},\"decoded_item_count\":{},\"item_count\":{},\"direct_item_count\":{},\"container_item_count\":{},\"indexed_item_count\":{},\"zdo_cheated_count\":{},\"station_queued_cheated_count\":{},\"evidence_records\":{},\"character_count\":{}}},\"archives\":[{}],\"evidence\":[{}],\"characters\":[{}],\"map\":{},\"timeline_note\":\"Ordering follows deterministic file names/snapshot IDs. Evidence identity excludes stack and uses parsed prefab, position, key, item, slot, quality, variant, crafter, and world-level fields; comparisons are approximate because saves do not provide a universal ZDO creation timestamp.\"}}",
        archives.len(), totals.0, decoded_total, totals.1, totals.2, totals.3, totals.4, totals.5,
        totals.6, totals.1 + totals.5 + totals.6, characters.len(), archive_json, evidence_json,
        character_json, map_json
    )
}

fn browser_csv_headers() -> Vec<String> {
    [
        "record_type",
        "status",
        "archive",
        "snapshot",
        "format",
        "zdo_count",
        "decoded_item_count",
        "item_count",
        "direct_item_count",
        "container_item_count",
        "indexed_item_count",
        "zdo_cheated_count",
        "station_queued_cheated_count",
        "source",
        "internal_path",
        "chunk",
        "chunk_version",
        "chunk_size",
        "chunk_revision",
        "zdo_ordinal",
        "owner_prefab_hash",
        "owner_prefab_name",
        "x",
        "y",
        "z",
        "legacy_sector_x",
        "legacy_sector_y",
        "key_hash",
        "key_name",
        "kind",
        "item_hash",
        "item_name",
        "slot_x",
        "slot_y",
        "quality",
        "stack",
        "variant",
        "crafter_name",
        "world_level",
        "custom_data_keys",
        "profile_name",
        "profile_version",
        "canonical",
        "trusted",
        "supported",
        "hash_valid",
        "used_cheats",
        "cheat_stat_nonzero_count",
        "known_command_hits",
        "inventory_version",
        "inventory_item_count",
        "cheated_inventory_count",
        "bypass_cheat_checks",
        "player_data_complete",
        "parse_error",
        "lineage",
        "first_seen_snapshot",
        "last_seen_snapshot",
        "present_in_latest",
        "occurrence_index",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

fn browser_blank_fields() -> Vec<String> {
    vec![String::new(); browser_csv_headers().len()]
}

pub fn browser_report_csv(archives: &[ArchiveScan], characters: &[CharacterScan]) -> String {
    let lineages = browser_lineages(archives);
    let mut output = csv_row(browser_csv_headers());
    for (archive_index, archive) in archives.iter().enumerate() {
        let mut archive_row = browser_blank_fields();
        archive_row[0] = "archive".to_string();
        archive_row[1] = "observed".to_string();
        archive_row[2] = browser_filename(&archive.archive);
        archive_row[3] = browser_filename(&archive.snapshot);
        archive_row[4] = archive.format.clone();
        archive_row[5] = archive.zdo_count.to_string();
        archive_row[6] = archive.decoded_item_count.to_string();
        archive_row[7] = archive.item_count.to_string();
        archive_row[8] = archive.direct_item_count.to_string();
        archive_row[9] = archive.container_item_count.to_string();
        archive_row[10] = archive.indexed_item_count.to_string();
        archive_row[11] = archive.zdo_cheated_count.to_string();
        archive_row[12] = archive.station_queued_cheated_count.to_string();
        archive_row[13] = browser_filename(&archive.archive);
        output.push_str(&csv_row(archive_row));
        for (evidence_index, evidence) in archive.evidence.iter().enumerate() {
            let mut row = browser_blank_fields();
            let lineage = &lineages[archive_index][evidence_index];
            row[0] = "evidence".to_string();
            row[1] = lineage.status.to_string();
            row[2] = browser_filename(&archive.archive);
            row[3] = browser_filename(&archive.snapshot);
            row[4] = archive.format.clone();
            row[13] = browser_filename(&archive.archive);
            row[14] = browser_internal_path(&evidence.internal_path);
            row[15] = evidence
                .chunk_filename
                .as_deref()
                .map(browser_filename)
                .unwrap_or_default();
            row[16] = opt_display(evidence.chunk_version);
            row[17] = opt_display(evidence.chunk_size);
            row[18] = opt_display(evidence.chunk_revision);
            row[19] = evidence.zdo_ordinal.to_string();
            row[20] = evidence.owner_prefab_hash.to_string();
            row[21] = evidence.owner_prefab_name.clone().unwrap_or_default();
            row[22] = browser_float(evidence.position.x);
            row[23] = browser_float(evidence.position.y);
            row[24] = browser_float(evidence.position.z);
            if let Some((x, y)) = evidence.legacy_sector {
                row[25] = x.to_string();
                row[26] = y.to_string();
            }
            row[27] = evidence.key_hash.to_string();
            row[28] = evidence.key_name.clone();
            row[29] = evidence.kind.clone();
            row[30] = opt_display(evidence.item_hash);
            row[31] = evidence.item_name.clone().unwrap_or_default();
            if let Some((x, y)) = evidence.grid {
                row[32] = x.to_string();
                row[33] = y.to_string();
            }
            row[34] = opt_display(evidence.quality);
            row[35] = opt_display(evidence.stack);
            row[36] = opt_display(evidence.variant);
            row[37] = evidence.crafter_name.clone().unwrap_or_default();
            row[38] = opt_display(evidence.world_level);
            row[39] = evidence.custom_keys.join("|");
            row[56] = lineage.first_seen_snapshot.clone();
            row[57] = lineage.last_seen_snapshot.clone();
            row[58] = lineage.present_in_latest.to_string();
            row[59] = lineage.occurrence_index.to_string();
            output.push_str(&csv_row(row));
        }
    }
    let canonical_id = characters
        .iter()
        .find(|character| character.canonical)
        .and_then(|character| character.player_id);
    for character in characters {
        let available = browser_character_available(character);
        let unavailable = || "unavailable".to_string();
        let mut row = browser_blank_fields();
        row[0] = "character".to_string();
        row[1] = browser_character_status(character).to_string();
        row[13] = browser_filename(&character.source);
        row[40] = if available {
            character.player_name.clone()
        } else {
            unavailable()
        };
        row[41] = if available {
            character.profile_version.to_string()
        } else {
            unavailable()
        };
        row[42] = character.canonical.to_string();
        row[43] = character.trusted.to_string();
        row[44] = character.supported.to_string();
        row[45] = character.hash_valid.to_string();
        row[46] = if available {
            character.used_cheats.to_string()
        } else {
            unavailable()
        };
        row[47] = if available {
            character.cheat_stat_nonzero_count.to_string()
        } else {
            unavailable()
        };
        row[48] = if available {
            character.known_command_hits.to_string()
        } else {
            unavailable()
        };
        row[49] = if available {
            opt_display(character.inventory_version)
        } else {
            unavailable()
        };
        row[50] = if available {
            character.inventory_item_count.to_string()
        } else {
            unavailable()
        };
        row[51] = if available {
            character.cheated_inventory_count.to_string()
        } else {
            unavailable()
        };
        row[52] = if available {
            character.bypass_cheat_checks.to_string()
        } else {
            unavailable()
        };
        row[53] = if available {
            character.player_data_complete.to_string()
        } else {
            unavailable()
        };
        row[54] = character.parse_error.clone().unwrap_or_default();
        row[55] = character_lineage(character, canonical_id).to_string();
        output.push_str(&csv_row(row));
    }
    output
}

fn browser_md_cell(value: &str) -> String {
    value
        .replace(['\r', '\n'], " ")
        .replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace('`', "\\`")
}

fn browser_md_code(value: &str) -> String {
    let value = browser_md_cell(value);
    let mut max_run = 0;
    let mut run = 0;
    for character in value.chars() {
        if character == '`' {
            run += 1;
            max_run = max_run.max(run);
        } else {
            run = 0;
        }
    }
    let delimiter = "`".repeat(max_run + 1);
    format!("{delimiter}{value}{delimiter}")
}

pub fn browser_report_markdown(archives: &[ArchiveScan], characters: &[CharacterScan]) -> String {
    let totals = totals(archives);
    let lineages = browser_lineages(archives);
    let mut output = String::from("# Valheim Cheat Radar report\n\nRead-only browser report. Input bytes remain local to the browser; no save editing is implemented.\n\n");
    output.push_str(&format!(
        "## Summary\n\n- Archives: {}\n- ZDO records: {}\n- Evidence records: {}\n- Character profiles: {}\n\n",
        archives.len(), totals.0, totals.1 + totals.5 + totals.6, characters.len()
    ));
    output.push_str("## Archives\n\n| Archive | Snapshot | Format | ZDOs | Evidence |\n|---|---|---|---:|---:|\n");
    for archive in archives {
        output.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            browser_md_cell(&browser_filename(&archive.archive)),
            browser_md_cell(&browser_filename(&archive.snapshot)),
            browser_md_cell(&archive.format),
            archive.zdo_count,
            archive.item_count + archive.zdo_cheated_count + archive.station_queued_cheated_count
        ));
    }
    output.push_str("\n## World evidence\n\n| Status | First seen | Last seen | Present in latest | Occurrence | Snapshot | Kind | Owner | Item | Crafter | Stack | Path | X | Y | Z |\n|---|---|---|---:|---:|---|---|---|---|---|---:|---|---:|---:|---:|\n");
    for (archive_index, archive) in archives.iter().enumerate() {
        for (evidence_index, evidence) in archive.evidence.iter().enumerate() {
            let lineage = &lineages[archive_index][evidence_index];
            output.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
                browser_md_cell(lineage.status),
                browser_md_cell(&lineage.first_seen_snapshot),
                browser_md_cell(&lineage.last_seen_snapshot),
                lineage.present_in_latest,
                lineage.occurrence_index,
                browser_md_cell(&browser_filename(&evidence.snapshot)),
                browser_md_cell(&evidence.kind),
                browser_md_cell(evidence.owner_prefab_name.as_deref().unwrap_or("unknown")),
                browser_md_cell(evidence.item_name.as_deref().unwrap_or("unknown")),
                browser_md_cell(evidence.crafter_name.as_deref().unwrap_or("unknown")),
                opt_display(evidence.stack),
                browser_md_code(&browser_internal_path(&evidence.internal_path)),
                browser_float(evidence.position.x),
                browser_float(evidence.position.y),
                browser_float(evidence.position.z)
            ));
        }
    }
    output.push_str("\n## Character profiles\n\n| Status | Source | Profile | Canonical | Trusted | Cheat indicators |\n|---|---|---|---:|---:|---:|\n");
    for character in characters {
        let available = browser_character_available(character);
        output.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            browser_md_cell(browser_character_status(character)),
            browser_md_code(&browser_filename(&character.source)),
            if available {
                browser_md_cell(&character.player_name)
            } else {
                "unavailable".to_string()
            },
            character.canonical,
            character.trusted,
            if available {
                (u32::from(character.used_cheats)
                    + character.cheat_stat_nonzero_count
                    + character.cheated_inventory_count)
                    .to_string()
            } else {
                "unavailable".to_string()
            }
        ));
    }
    output.push_str("\nTimeline comparisons use deterministic sorted archive order and stack-excluded identity. A single save reports observed evidence; later saves classify each duplicate occurrence as new, persisted, or removed_or_cleared relative to the latest save.\n");
    output
}

/// A scrubbed copy plus its audit, handed to the browser worker.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub struct BrowserScrub {
    bytes: Vec<u8>,
    audit_json: String,
    audit_markdown: String,
    zdo_count_before: u64,
    zdo_count_after: u64,
}

#[cfg(target_arch = "wasm32")]
impl BrowserScrub {
    fn new((bytes, report): (Vec<u8>, ScrubReport)) -> Self {
        Self {
            bytes,
            audit_json: scrub_audit_json(&report),
            audit_markdown: scrub_audit_markdown(&report),
            zdo_count_before: report.zdo_count_before,
            zdo_count_after: report.zdo_count_after,
        }
    }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
impl BrowserScrub {
    /// The scrubbed bytes; moved out, so a second call returns an empty array.
    pub fn take_bytes(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.bytes)
    }

    /// Every action taken; the page builds its preview from this.
    pub fn audit_json(&self) -> String {
        self.audit_json.clone()
    }

    pub fn audit_markdown(&self) -> String {
        self.audit_markdown.clone()
    }

    pub fn zdo_count_before(&self) -> f64 {
        self.zdo_count_before as f64
    }

    pub fn zdo_count_after(&self) -> f64 {
        self.zdo_count_after as f64
    }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn zstd_add_checksum(frame: &[u8], content: &[u8]) -> Result<Vec<u8>, JsValue> {
    add_zstd_content_checksum(frame, content).map_err(|error| JsValue::from_str(&error.to_string()))
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub struct BrowserScanner {
    prefabs: PrefabNames,
    archives: Vec<ArchiveScan>,
    characters: Vec<CharacterScan>,
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
impl BrowserScanner {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            prefabs: PrefabNames::from_text(include_str!("../prefab_names.txt"))
                .with_biome_text(include_str!("../prefab_biomes.txt")),
            archives: Vec::new(),
            characters: Vec::new(),
        }
    }

    pub fn add_tar(&mut self, name: &str, bytes: &[u8]) -> Result<(), JsValue> {
        scan_tar_bytes(bytes, &browser_filename(name), &self.prefabs)
            .map(|archive| self.archives.push(archive))
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        self.sort_inputs();
        Ok(())
    }

    pub fn add_file(&mut self, name: &str, bytes: &[u8]) -> Result<(), JsValue> {
        self.add_file_with_modified_unix(name, bytes, f64::NAN)
    }

    /// Experimental scrub of a decompressed backup tar in the given mode (`clean`,
    /// `delete-items` or `destroy`); never touches the scan results.
    pub fn scrub_tar(&self, bytes: &[u8], mode: &str) -> Result<BrowserScrub, JsValue> {
        let mode = ScrubMode::parse(mode)
            .ok_or_else(|| JsValue::from_str(&format!("unknown scrub mode {mode}")))?;
        scrub_tar(bytes, &self.prefabs, mode)
            .map(BrowserScrub::new)
            .map_err(|error| JsValue::from_str(&error.to_string()))
    }

    /// Experimental clean-mode scrub of one standalone v41 chunk file. The delete modes need the
    /// chunk index beside it, so they only run on whole backups.
    pub fn scrub_chunk(&self, name: &str, bytes: &[u8]) -> Result<BrowserScrub, JsValue> {
        scrub_chunk_in_place(bytes, name, &self.prefabs)
            .map(|(bytes, outcome)| BrowserScrub::new((bytes, ScrubReport::from_patches(&outcome))))
            .map_err(|error| JsValue::from_str(&error.to_string()))
    }

    pub fn add_file_with_modified_unix(
        &mut self,
        name: &str,
        bytes: &[u8],
        modified_unix_millis: f64,
    ) -> Result<(), JsValue> {
        self.add_file_with_metadata(name, bytes, modified_unix_millis, 0)
    }

    pub fn add_file_with_metadata(
        &mut self,
        name: &str,
        bytes: &[u8],
        modified_unix_millis: f64,
        input_id: u32,
    ) -> Result<(), JsValue> {
        let lower = name.to_ascii_lowercase();
        let is_backup = lower.ends_with(".fch.old") || lower.ends_with(".fch.bak");
        let result = if lower.ends_with(".fch") || is_backup {
            scan_character_bytes(bytes, &browser_filename(name)).map(|mut scan| {
                scan.modified_unix = browser_modified_unix(modified_unix_millis);
                scan.input_id = input_id;
                scan.canonical_eligible = !is_backup;
                self.characters.push(scan);
            })
        } else if lower.ends_with(".tar") {
            scan_tar_bytes(bytes, &browser_filename(name), &self.prefabs).map(|archive| {
                self.archives.push(archive);
            })
        } else if lower.ends_with(".db") {
            parse_legacy_bytes(bytes, &browser_filename(name), &self.prefabs).map(|archive| {
                self.archives.push(archive);
            })
        } else if lower.ends_with(".chunk") {
            parse_chunk_bytes(bytes, &browser_filename(name), &self.prefabs).map(|archive| {
                self.archives.push(archive);
            })
        } else if lower.ends_with(".tar.zst") {
            Err(error(
                ".tar.zst must be decompressed to .tar in the worker before scanning",
            ))
        } else {
            Err(error("unsupported file; use .tar, .db, .chunk, or .fch"))
        };
        result.map_err(|error| JsValue::from_str(&error.to_string()))?;
        self.sort_inputs();
        Ok(())
    }

    fn sort_inputs(&mut self) {
        self.archives.sort_by(|left, right| {
            left.snapshot
                .cmp(&right.snapshot)
                .then(left.archive.cmp(&right.archive))
        });
        self.characters.sort_by(|left, right| {
            left.source
                .cmp(&right.source)
                .then(left.input_id.cmp(&right.input_id))
        });
        for character in &mut self.characters {
            character.canonical = false;
        }
        if let Some(index) = browser_canonical_index(&self.characters) {
            self.characters[index].canonical = true;
        }
    }

    pub fn set_canonical(&mut self, index: usize) -> Result<(), JsValue> {
        let character = self
            .characters
            .get(index)
            .ok_or_else(|| JsValue::from_str("canonical character index is out of range"))?;
        if !character.canonical_eligible || !browser_character_available(character) {
            return Err(JsValue::from_str(
                "canonical character must be trusted, supported, and complete",
            ));
        }
        for character in &mut self.characters {
            character.canonical = false;
        }
        self.characters[index].canonical = true;
        Ok(())
    }

    pub fn reset(&mut self) {
        self.archives.clear();
        self.characters.clear();
    }

    pub fn report_json(&self) -> String {
        browser_report_json(&self.archives, &self.characters)
    }

    pub fn report_csv(&self) -> String {
        browser_report_csv(&self.archives, &self.characters)
    }

    pub fn report_markdown(&self) -> String {
        browser_report_markdown(&self.archives, &self.characters)
    }

    pub fn archive_count(&self) -> usize {
        self.archives.len()
    }

    pub fn character_count(&self) -> usize {
        self.characters.len()
    }
}

#[cfg(target_arch = "wasm32")]
impl Default for BrowserScanner {
    fn default() -> Self {
        Self::new()
    }
}

/// Generic, world-independent invariants: snapshot labels are unique and each archive's item counts
/// are internally consistent. These hold for any save set, so they belong in the source.
///
/// Expectations for one particular private save set do not; see [`validate_oracle_fixture`].
pub fn validate_oracle(archives: &[ArchiveScan]) -> Result<(), ScanError> {
    for (index, archive) in archives.iter().enumerate() {
        if archives[..index]
            .iter()
            .any(|other| other.snapshot == archive.snapshot)
        {
            return Err(error(format!(
                "duplicate snapshot label: {}",
                archive.snapshot
            )));
        }
        let parts =
            archive.direct_item_count + archive.container_item_count + archive.indexed_item_count;
        if archive.item_count != parts {
            return Err(error(format!(
                "{}: item count {} is not direct {} + container {} + indexed {}",
                archive.snapshot,
                archive.item_count,
                archive.direct_item_count,
                archive.container_item_count,
                archive.indexed_item_count
            )));
        }
    }
    Ok(())
}

/// Optional expectations for one private save set, kept out of the repository.
///
/// One directive per line, `#` starts a comment:
///
/// ```text
/// <snapshot> <zdo_count> [item_count] [zdo_cheated_count] [station_queued_cheated_count]
/// delta <old_snapshot> <new_snapshot> <old_persisted> <old_removed> <new_persisted> <new_new>
/// character clean
/// ```
///
/// A snapshot line with only a ZDO count asserts structure and completeness; additional counts
/// assert exact totals, so a leading-run snapshot can pin down that it was clean.
pub fn validate_oracle_fixture(
    archives: &[ArchiveScan],
    characters: &[CharacterScan],
    text: &str,
) -> Result<(), ScanError> {
    let mut delta: Option<(String, String, [usize; 4])> = None;
    let mut expect_clean_character = false;
    for (index, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }
        let line_number = index + 1;
        let fields = line.split_whitespace().collect::<Vec<_>>();
        let parse = |value: &str| {
            value.parse::<u64>().map_err(|_| {
                error(format!(
                    "fixture line {line_number}: `{value}` is not a count"
                ))
            })
        };
        match fields[0] {
            "delta" => {
                if fields.len() != 7 {
                    return Err(error(format!(
                        "fixture line {line_number}: delta needs 6 arguments"
                    )));
                }
                let mut counts = [0usize; 4];
                for (slot, value) in counts.iter_mut().zip(&fields[3..]) {
                    *slot = value.parse::<usize>().map_err(|_| {
                        error(format!(
                            "fixture line {line_number}: `{value}` is not a count"
                        ))
                    })?;
                }
                delta = Some((fields[1].to_string(), fields[2].to_string(), counts));
            }
            "character" => {
                if fields.get(1) != Some(&"clean") {
                    return Err(error(format!(
                        "fixture line {line_number}: expected `character clean`"
                    )));
                }
                expect_clean_character = true;
            }
            snapshot => {
                if fields.len() > 5 {
                    return Err(error(format!(
                        "fixture line {line_number}: too many counts"
                    )));
                }
                let archive = archives
                    .iter()
                    .find(|archive| archive.snapshot == snapshot)
                    .ok_or_else(|| {
                        error(format!("fixture line {line_number}: missing `{snapshot}`"))
                    })?;
                let actual = [
                    archive.zdo_count,
                    archive.item_count,
                    archive.zdo_cheated_count,
                    archive.station_queued_cheated_count,
                ];
                let names = ["zdo", "item", "zdo_cheated", "queued"];
                for (slot, (value, found)) in fields[1..].iter().zip(actual).enumerate() {
                    let want = parse(value)?;
                    if want != found {
                        return Err(error(format!(
                            "{snapshot}: expected {want} {} records, found {found}",
                            names[slot]
                        )));
                    }
                }
            }
        }
    }
    if expect_clean_character {
        let canonical = characters
            .iter()
            .find(|character| character.canonical)
            .ok_or_else(|| error("missing canonical character"))?;
        if canonical.used_cheats
            || canonical.cheat_stat_nonzero_count != 0
            || canonical.cheated_inventory_count != 0
            || canonical.bypass_cheat_checks
        {
            return Err(error("canonical character has cheat indicators"));
        }
    }
    if let Some((old_snapshot, new_snapshot, counts)) = delta {
        let statuses = delta_statuses(archives);
        let count = |snapshot: &str, status: &str| -> Result<usize, ScanError> {
            let index = archives
                .iter()
                .position(|archive| archive.snapshot == snapshot)
                .ok_or_else(|| error(format!("missing snapshot `{snapshot}`")))?;
            Ok(statuses[index]
                .iter()
                .filter(|value| value.as_str() == status)
                .count())
        };
        let found = [
            count(&old_snapshot, "persisted")?,
            count(&old_snapshot, "removed_or_cleared")?,
            count(&new_snapshot, "persisted")?,
            count(&new_snapshot, "new")?,
        ];
        if found != counts {
            return Err(error(format!(
                "delta {} -> {}: expected {} persisted / {} removed / {} persisted / {} new, found {} / {} / {} / {}",
                old_snapshot, new_snapshot, counts[0], counts[1], counts[2], counts[3], found[0], found[1], found[2], found[3]
            )));
        }
    }
    Ok(())
}

/// Read and check an oracle fixture from disk.
#[cfg(not(target_arch = "wasm32"))]
pub fn validate_oracle_fixture_file(
    archives: &[ArchiveScan],
    characters: &[CharacterScan],
    path: &Path,
) -> Result<(), ScanError> {
    let text = std::fs::read_to_string(path)
        .map_err(|cause| error(format!("oracle fixture {}: {cause}", path.display())))?;
    validate_oracle_fixture(archives, characters, &text)
}

/// Generic character structure: a canonical profile must be trusted, hash-valid, supported, and
/// complete. Whether it happens to be *clean* describes one save set rather than the format, so
/// that expectation belongs in the optional local oracle fixture instead.
pub fn validate_character_oracle(characters: &[CharacterScan]) -> Result<(), ScanError> {
    let canonical = characters
        .iter()
        .find(|character| character.canonical)
        .ok_or_else(|| error("missing canonical character"))?;
    if !canonical.trusted
        || !canonical.hash_valid
        || !canonical.supported
        || !canonical.player_data_complete
    {
        return Err(error(
            "canonical character is not trusted, supported, and complete",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_i32(bytes: &mut Vec<u8>, value: i32) {
        bytes.extend(value.to_le_bytes());
    }

    fn push_i16(bytes: &mut Vec<u8>, value: i16) {
        bytes.extend(value.to_le_bytes());
    }

    fn push_u16(bytes: &mut Vec<u8>, value: u16) {
        bytes.extend(value.to_le_bytes());
    }

    fn item_bytes(cheated: bool) -> Vec<u8> {
        let mut bytes = Vec::new();
        push_i32(&mut bytes, 10000);
        bytes.extend([0, 0, 0, 0x40]);
        push_i32(&mut bytes, stable_hash("Silver"));
        bytes.push(u8::from(cheated));
        bytes
    }

    fn inventory_bytes(cheated: bool) -> Vec<u8> {
        let mut bytes = Vec::new();
        push_i32(&mut bytes, 109);
        push_u16(&mut bytes, 1);
        bytes.extend(item_bytes(cheated));
        bytes
    }

    #[test]
    fn stable_hash_vectors_match_decompiled_code() {
        assert_eq!(stable_hash("darkwood_gate"), 1_276_642_083);
        assert_eq!(stable_hash("piece_chest_warderobe"), 2_009_412_434);
        assert_eq!(stable_hash("blastfurnace"), 1_048_742_812);
        assert_eq!(stable_hash("cheated"), CHEATED);
        assert_eq!(stable_hash("cheatedQueued"), CHEATED_QUEUED);
    }

    #[test]
    fn sha512_matches_known_vector() {
        let expected = "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f";
        let actual = sha512(b"abc")
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(actual, expected);
    }

    #[test]
    fn item_109_cheat_bit_is_decoded() {
        assert!(parse_direct_item(
            &[109]
                .into_iter()
                .chain(item_bytes(true))
                .collect::<Vec<_>>()
        )
        .unwrap()
        .is_some());
        assert!(parse_direct_item(
            &[109]
                .into_iter()
                .chain(item_bytes(false))
                .collect::<Vec<_>>()
        )
        .unwrap()
        .is_none());
    }

    #[test]
    fn inventory_109_decodes_individual_items() {
        let hits = parse_inventory(&inventory_bytes(true)).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].item_hash, Some(stable_hash("Silver")));
        assert_eq!(hits[0].quality, 1);
        assert_eq!(hits[0].world_level, 0);
    }

    /// Every ZDO is binned by position for the map view, and positions that are
    /// not places are excluded so one outlier cannot stretch the whole map.
    #[test]
    fn spatial_aggregation_bins_by_cell_and_skips_unplottable_positions() {
        let prefabs = PrefabNames {
            names: vec![(stable_hash("stone_wall_2x1"), "stone_wall_2x1".to_string())],
            ..PrefabNames::default()
        };
        let hash = stable_hash("stone_wall_2x1");
        let mut chunk = Vec::new();
        push_i16(&mut chunk, 41);
        push_i32(&mut chunk, 3);
        // 100/64 -> 1, -200/64 -> -4; 120/64 -> 1, -220/64 -> -4: same cell.
        for (x, z) in [(100f32, -200f32), (120f32, -220f32), (1_000_000f32, 0f32)] {
            push_u16(&mut chunk, 0);
            chunk.extend(x.to_le_bytes());
            chunk.extend(0f32.to_le_bytes());
            chunk.extend(z.to_le_bytes());
            push_i32(&mut chunk, hash);
        }
        let mut reader = &chunk[..];
        let output = parse_chunk_reader(
            &mut reader,
            chunk.len() as u64,
            "a",
            "a",
            "00_00__1_1.chunk",
            &prefabs,
            &[],
            &mut ScrubSink::default(),
        )
        .unwrap();

        assert_eq!(output.zdo_count, 3);
        assert_eq!(
            output.grid.get(&(1, -4)),
            Some(&2),
            "two ZDOs share one cell"
        );
        assert_eq!(
            output.grid.len(),
            1,
            "the million-metre outlier is not plotted"
        );
        assert_eq!(
            output.prefabs.get(&hash),
            Some(&3),
            "the prefab histogram still counts every ZDO"
        );
    }

    #[test]
    fn current_chunk_and_legacy_zdo_are_parseable() {
        let prefabs = PrefabNames {
            names: vec![(stable_hash("Silver"), "Silver".to_string())],
            ..PrefabNames::default()
        };
        let mut chunk = Vec::new();
        push_i16(&mut chunk, 41);
        push_i32(&mut chunk, 1);
        push_u16(&mut chunk, FLAG_INTS | FLAG_BYTE_ARRAYS);
        chunk.extend([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        push_i32(&mut chunk, stable_hash("piece_chest"));
        chunk.push(1);
        push_i32(&mut chunk, CHEATED);
        push_i32(&mut chunk, 1);
        chunk.push(1);
        push_i32(&mut chunk, ITEM_DATA);
        let direct: Vec<u8> = [109].into_iter().chain(item_bytes(true)).collect();
        push_i32(&mut chunk, direct.len() as i32);
        chunk.extend(direct);
        let mut reader = &chunk[..];
        let output = parse_chunk_reader(
            &mut reader,
            chunk.len() as u64,
            "a",
            "a",
            "00_00__1_1.chunk",
            &prefabs,
            &[],
            &mut ScrubSink::default(),
        )
        .unwrap();
        assert_eq!(output.zdo_count, 1);
        assert_eq!(output.evidence.len(), 2);
        assert!(output.evidence.iter().any(|hit| hit.kind == "zdo_cheated"));
        assert!(output
            .evidence
            .iter()
            .any(|hit| hit.kind == "direct_item_data"));

        let mut legacy = Vec::new();
        push_i32(&mut legacy, 37);
        legacy.extend(0f64.to_le_bytes());
        legacy.extend(0i64.to_le_bytes());
        legacy.extend(1u32.to_le_bytes());
        push_i32(&mut legacy, 1);
        push_u16(&mut legacy, FLAG_STRINGS);
        push_i16(&mut legacy, 2);
        push_i16(&mut legacy, -3);
        legacy.extend([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        push_i32(&mut legacy, stable_hash("piece_chest"));
        push_u8_vec(&mut legacy, 1);
        push_i32(&mut legacy, ITEMS);
        let encoded = base64_for_test(&inventory_bytes(true));
        push_dotnet_string_for_test(&mut legacy, &encoded);
        let mut legacy_reader = &legacy[..];
        let output =
            parse_legacy_reader(&mut legacy_reader, "a", "a", "Dedicated.db", &prefabs).unwrap();
        assert_eq!(output.zdo_count, 1);
        assert_eq!(output.counts.item_count, 1);
        assert_eq!(output.evidence[0].legacy_sector, Some((2, -3)));
    }

    /// Builds the same v41 chunk as `current_chunk_and_legacy_zdo_are_parseable` (one ZDO with a
    /// set `cheated` int flag and a cheated direct `itemData`), used by the scrub tests below.
    fn scrub_test_chunk() -> Vec<u8> {
        let mut chunk = Vec::new();
        push_i16(&mut chunk, 41);
        push_i32(&mut chunk, 1);
        push_u16(&mut chunk, FLAG_INTS | FLAG_BYTE_ARRAYS);
        chunk.extend([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        push_i32(&mut chunk, stable_hash("piece_chest"));
        chunk.push(1);
        push_i32(&mut chunk, CHEATED);
        push_i32(&mut chunk, 1);
        chunk.push(1);
        push_i32(&mut chunk, ITEM_DATA);
        let direct: Vec<u8> = [109].into_iter().chain(item_bytes(true)).collect();
        push_i32(&mut chunk, direct.len() as i32);
        chunk.extend(direct);
        chunk
    }

    #[test]
    fn zdo_int_and_item_cheat_patches_record_exact_offsets() {
        let prefabs = PrefabNames {
            names: vec![(stable_hash("piece_chest"), "piece_chest".to_string())],
            ..PrefabNames::default()
        };
        let chunk = scrub_test_chunk();
        let mut sink = ScrubSink::default();
        let mut reader = &chunk[..];
        parse_chunk_reader(
            &mut reader,
            chunk.len() as u64,
            "a",
            "a",
            "00_00__1_1.chunk",
            &prefabs,
            &[],
            &mut sink,
        )
        .unwrap();
        let patches = sink.patches;
        assert_eq!(patches.len(), 2);

        let int_patch = patches
            .iter()
            .find(|patch| patch.kind == ScrubPatchKind::ZdoIntFlag)
            .unwrap();
        assert_eq!(int_patch.offset, 29);
        assert_eq!(int_patch.key_name, "cheated");
        assert_eq!(int_patch.old_value, 1);
        assert_eq!(int_patch.new_value, 0);
        assert_eq!(
            i32::from_le_bytes(chunk[29..33].try_into().unwrap()),
            1,
            "the recorded offset must point at the actual int value"
        );

        let item_patch = patches
            .iter()
            .find(|patch| patch.kind == ScrubPatchKind::ItemCheatedBit)
            .unwrap();
        assert_eq!(item_patch.offset, 55);
        assert_eq!(item_patch.key_name, "itemData");
        assert_eq!(item_patch.old_value, 1);
        assert_eq!(item_patch.new_value, 0);
        assert_eq!(
            chunk[55] & 1,
            1,
            "the recorded offset must point at the actual cheated bit"
        );
    }

    fn append_tar_entry(tar: &mut Vec<u8>, name: &str, body: &[u8]) {
        tar.extend(tar_header(name, body.len() as u64));
        tar.extend(body);
        let padding = (512 - (body.len() % 512)) % 512;
        tar.extend(vec![0u8; padding]);
    }

    fn finish_tar(tar: &mut Vec<u8>) {
        tar.extend([0u8; 1024]);
    }

    fn chunks_meta_bytes(total: i32) -> Vec<u8> {
        let mut bytes = Vec::new();
        push_i16(&mut bytes, 41);
        push_i32(&mut bytes, total);
        push_i32(&mut bytes, 0);
        bytes
    }

    #[test]
    fn scrub_world_tar_clears_flags_and_touches_only_audited_bytes() {
        let prefabs = PrefabNames {
            names: vec![(stable_hash("piece_chest"), "piece_chest".to_string())],
            ..PrefabNames::default()
        };
        let chunk = scrub_test_chunk();
        let mut tar = Vec::new();
        append_tar_entry(
            &mut tar,
            "SaveDir/worlds_local/Dedicated/_main.1.chunks",
            &chunks_meta_bytes(1),
        );
        append_tar_entry(
            &mut tar,
            "SaveDir/worlds_local/Dedicated/00_00__1_1.chunk",
            &chunk,
        );
        append_tar_entry(
            &mut tar,
            "SaveDir/worlds_local/Dedicated/_main.1.ok",
            b"ok!!",
        );
        append_tar_entry(&mut tar, "SaveDir/adminlist.txt", b"not part of the world");
        finish_tar(&mut tar);

        let outcome = scrub_world_tar_bytes(&tar, &prefabs).unwrap();
        assert_eq!(outcome.zdo_count, 1);
        assert_eq!(outcome.patches.len(), 2);

        // Only worlds_local/ files are carried into the output; adminlist.txt is dropped.
        let paths: Vec<&str> = outcome
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect();
        assert_eq!(
            paths,
            [
                "worlds_local/Dedicated/00_00__1_1.chunk",
                "worlds_local/Dedicated/_main.1.chunks",
                "worlds_local/Dedicated/_main.1.ok",
            ]
        );

        let scrubbed_chunk = &outcome
            .files
            .iter()
            .find(|file| file.path.ends_with(".chunk"))
            .unwrap()
            .bytes;
        assert_eq!(scrubbed_chunk.len(), chunk.len());
        let changed: Vec<usize> = chunk
            .iter()
            .zip(scrubbed_chunk)
            .enumerate()
            .filter(|(_, (before, after))| before != after)
            .map(|(index, _)| index)
            .collect();
        assert_eq!(changed, [29, 55], "only the two audited bytes changed");
        assert_eq!(
            i32::from_le_bytes(scrubbed_chunk[29..33].try_into().unwrap()),
            0
        );
        assert_eq!(scrubbed_chunk[55] & 1, 0);

        // The passthrough .ok file is copied byte-identical.
        let ok_file = &outcome
            .files
            .iter()
            .find(|file| file.path.ends_with(".ok"))
            .unwrap()
            .bytes;
        assert_eq!(ok_file, b"ok!!");

        let rescanned = parse_chunk_bytes(scrubbed_chunk, "00_00__1_1.chunk", &prefabs).unwrap();
        assert!(rescanned.evidence.is_empty());
        assert_eq!(rescanned.zdo_count, 1);
    }

    #[test]
    fn scrub_tar_in_place_changes_only_flag_bytes_and_keeps_non_world_files() {
        let prefabs = PrefabNames {
            names: vec![(stable_hash("piece_chest"), "piece_chest".to_string())],
            ..PrefabNames::default()
        };
        let mut tar = Vec::new();
        append_tar_entry(
            &mut tar,
            "SaveDir/worlds_local/Dedicated/_main.1.chunks",
            &chunks_meta_bytes(1),
        );
        append_tar_entry(
            &mut tar,
            "SaveDir/worlds_local/Dedicated/00_00__1_1.chunk",
            &scrub_test_chunk(),
        );
        append_tar_entry(&mut tar, "SaveDir/adminlist.txt", b"not part of the world");
        finish_tar(&mut tar);

        let (patched, outcome) = scrub_tar_in_place(&tar, &prefabs).unwrap();
        assert_eq!(patched.len(), tar.len());
        let chunk_start = outcome
            .files
            .iter()
            .find(|file| file.path.ends_with(".chunk"))
            .unwrap()
            .tar_offset as usize;
        let changed: Vec<usize> = tar
            .iter()
            .zip(&patched)
            .enumerate()
            .filter(|(_, (before, after))| before != after)
            .map(|(index, _)| index - chunk_start)
            .collect();
        assert_eq!(
            changed,
            [29, 55],
            "only the two flag bytes, inside the chunk"
        );
        let admin = b"not part of the world";
        assert!(patched.windows(admin.len()).any(|window| window == admin));
    }

    fn named_item(name: &str, cheated: bool) -> Vec<u8> {
        let mut bytes = Vec::new();
        push_i32(&mut bytes, 10000);
        bytes.extend([0, 0, 0, 0x40]);
        push_i32(&mut bytes, stable_hash(name));
        bytes.push(u8::from(cheated));
        bytes
    }

    fn inventory_of(items: &[(&str, bool)]) -> Vec<u8> {
        let mut bytes = Vec::new();
        push_i32(&mut bytes, 109);
        push_u16(&mut bytes, items.len() as u16);
        for (name, cheated) in items {
            bytes.extend(named_item(name, *cheated));
        }
        bytes
    }

    fn zdo_record(prefab: &str, ints: &[(i32, i32)], arrays: &[(i32, Vec<u8>)]) -> Vec<u8> {
        let mut flags = 0;
        if !ints.is_empty() {
            flags |= FLAG_INTS;
        }
        if !arrays.is_empty() {
            flags |= FLAG_BYTE_ARRAYS;
        }
        let mut record = Vec::new();
        push_u16(&mut record, flags);
        record.extend([0u8; 12]);
        push_i32(&mut record, stable_hash(prefab));
        if !ints.is_empty() {
            record.push(ints.len() as u8);
            for (key, value) in ints {
                push_i32(&mut record, *key);
                push_i32(&mut record, *value);
            }
        }
        if !arrays.is_empty() {
            record.push(arrays.len() as u8);
            for (key, data) in arrays {
                push_i32(&mut record, *key);
                push_i32(&mut record, data.len() as i32);
                record.extend(data);
            }
        }
        record
    }

    /// One chunk holding every case the delete modes distinguish, in order: A a flagged wall,
    /// B a flagged dropped item, C a clean chest with one flagged item between two clean ones, D a
    /// flagged chest with clean contents, E a stand holding a flagged item, F a clean floor.
    fn mode_test_chunk() -> (Vec<u8>, PrefabNames) {
        let direct = |name: &str| [109].into_iter().chain(named_item(name, true)).collect();
        let records = [
            zdo_record("stone_wall_4x2", &[(CHEATED, 1)], &[]),
            zdo_record("Wood", &[], &[(ITEM_DATA, direct("Wood"))]),
            zdo_record(
                "piece_chest",
                &[],
                &[(
                    ITEMS,
                    inventory_of(&[("Coal", false), ("Silver", true), ("Stone", false)]),
                )],
            ),
            zdo_record(
                "piece_chest",
                &[(CHEATED, 1)],
                &[(ITEMS, inventory_of(&[("Resin", false), ("Flint", false)]))],
            ),
            zdo_record(
                "itemstand",
                &[(ITEM_INT, stable_hash("Silver")), (TYPE_INT, 1)],
                &[(ITEM_DATA, direct("Silver"))],
            ),
            zdo_record("wood_floor", &[], &[]),
        ];
        let mut chunk = Vec::new();
        push_i16(&mut chunk, 41);
        push_i32(&mut chunk, records.len() as i32);
        for record in records {
            chunk.extend(record);
        }
        let names = [
            "stone_wall_4x2",
            "Wood",
            "piece_chest",
            "itemstand",
            "wood_floor",
            "Silver",
        ];
        let prefabs = PrefabNames {
            names: names
                .map(|name| (stable_hash(name), name.to_string()))
                .to_vec(),
            ..PrefabNames::default()
        };
        (chunk, prefabs)
    }

    fn surviving_prefabs(chunk: &[u8], prefabs: &PrefabNames) -> Vec<String> {
        let mut sink = ScrubSink::with_layouts();
        parse_chunk_reader(
            &mut &chunk[..],
            0,
            "",
            "",
            "00_00__1_1.chunk",
            prefabs,
            &[],
            &mut sink,
        )
        .unwrap();
        sink.layouts
            .unwrap()
            .iter()
            .map(|zdo| prefabs.name(zdo.owner_prefab_hash).unwrap().to_string())
            .collect()
    }

    #[test]
    fn delete_items_mode_removes_only_flagged_items() {
        let (chunk, prefabs) = mode_test_chunk();
        let result = rebuild_chunk(
            &chunk,
            "00_00__1_1.chunk",
            &prefabs,
            &[],
            ScrubMode::DeleteItems,
        )
        .unwrap();
        assert_eq!(result.dropped, 1, "only the dropped item goes");
        assert_eq!(
            surviving_prefabs(&result.bytes, &prefabs),
            [
                "stone_wall_4x2",
                "piece_chest",
                "piece_chest",
                "itemstand",
                "wood_floor"
            ]
        );
        let actions: Vec<&str> = result.actions.iter().map(|action| action.action).collect();
        assert_eq!(
            actions,
            ["delete_item", "remove_from_container", "empty_stand"]
        );
        // Flagged objects keep their flags: the wall and the flagged chest.
        let rescan = parse_chunk_bytes(&result.bytes, "00_00__1_1.chunk", &prefabs).unwrap();
        assert_eq!(rescan.evidence.len(), 2);
        assert!(rescan.evidence.iter().all(|hit| hit.kind == "zdo_cheated"));
        // The clean chest keeps Coal and Stone, in order; the flagged Silver is gone.
        let clean_chest = inventory_of(&[("Coal", false), ("Stone", false)]);
        assert!(result
            .bytes
            .windows(clean_chest.len())
            .any(|window| window == clean_chest));
    }

    #[test]
    fn destroy_mode_destroys_flagged_objects_with_contents_and_nothing_else() {
        let (chunk, prefabs) = mode_test_chunk();
        let result = rebuild_chunk(
            &chunk,
            "00_00__1_1.chunk",
            &prefabs,
            &[],
            ScrubMode::Destroy,
        )
        .unwrap();
        assert_eq!(result.dropped, 3, "wall, dropped item and flagged chest");
        assert_eq!(
            surviving_prefabs(&result.bytes, &prefabs),
            ["piece_chest", "itemstand", "wood_floor"]
        );
        let destroyed_chest = result
            .actions
            .iter()
            .find(|action| action.action == "destroy" && action.contents > 0)
            .unwrap();
        assert_eq!(
            destroyed_chest.contents, 2,
            "its two clean items go with it"
        );
        let rescan = parse_chunk_bytes(&result.bytes, "00_00__1_1.chunk", &prefabs).unwrap();
        assert!(rescan.evidence.is_empty(), "nothing flagged survives");
    }

    #[test]
    fn delete_modes_rewrite_the_tar_index_and_headers_and_keep_other_files() {
        let (chunk, prefabs) = mode_test_chunk();
        let mut index = Vec::new();
        push_i16(&mut index, 41);
        push_i32(&mut index, 6);
        push_i32(&mut index, 1);
        push_u16(&mut index, 0);
        index.push(1);
        index.extend(1u32.to_le_bytes());
        push_i32(&mut index, 6);
        let mut tar = Vec::new();
        append_tar_entry(
            &mut tar,
            "SaveDir/worlds_local/Dedicated/_main.1.chunks",
            &index,
        );
        append_tar_entry(
            &mut tar,
            "SaveDir/worlds_local/Dedicated/00_00__1_1.chunk",
            &chunk,
        );
        append_tar_entry(&mut tar, "SaveDir/adminlist.txt", b"not part of the world");
        finish_tar(&mut tar);

        let (out, report) = scrub_tar(&tar, &prefabs, ScrubMode::Destroy).unwrap();
        assert_eq!((report.zdo_count_before, report.zdo_count_after), (6, 3));
        let world = read_world_tar(&out).unwrap();
        assert_eq!(world.metadata_total, 3);
        let listed = i32::from_le_bytes(world.passthrough[0].bytes[17..21].try_into().unwrap());
        assert_eq!(
            listed, 3,
            "the chunk's own index entry follows its new count"
        );
        let admin = b"not part of the world";
        assert!(out.windows(admin.len()).any(|window| window == admin));
        assert!(scan_tar_bytes(&out, "out.tar", &prefabs)
            .unwrap()
            .evidence
            .is_empty());
    }

    #[test]
    fn item_and_type_key_hashes_match_the_game() {
        assert_eq!(stable_hash("item"), ITEM_INT);
        assert_eq!(stable_hash("type"), TYPE_INT);
    }

    #[test]
    fn scrub_chunk_in_place_clears_a_standalone_chunk() {
        let chunk = scrub_test_chunk();
        let (patched, outcome) =
            scrub_chunk_in_place(&chunk, "00_00__1_1.chunk", &PrefabNames::default()).unwrap();
        assert_eq!(outcome.patches.len(), 2);
        let changed: Vec<usize> = chunk
            .iter()
            .zip(&patched)
            .enumerate()
            .filter(|(_, (before, after))| before != after)
            .map(|(index, _)| index)
            .collect();
        assert_eq!(changed, [29, 55]);
    }

    /// A minimal single-segment zstd frame holding `content` in one raw block, without a checksum.
    fn raw_zstd_frame(content: &[u8]) -> Vec<u8> {
        assert!(content.len() < 256);
        let mut frame = vec![0x28, 0xb5, 0x2f, 0xfd, 0x20, content.len() as u8];
        let block_header = ((content.len() as u32) << 3) | 1;
        frame.extend_from_slice(&block_header.to_le_bytes()[..3]);
        frame.extend_from_slice(content);
        frame
    }

    #[test]
    fn zstd_checksum_sets_the_flag_and_appends_xxh64_low_bits() {
        let content = b"valheim cheat radar";
        let frame = raw_zstd_frame(content);
        let checked = add_zstd_content_checksum(&frame, content).unwrap();
        assert_eq!(checked[4], frame[4] | 0x04);
        assert_eq!(checked[5..frame.len()], frame[5..]);
        let expected = twox_hash::XxHash64::oneshot(0, content) as u32;
        assert_eq!(checked[frame.len()..], expected.to_le_bytes());
        assert!(
            add_zstd_content_checksum(&checked, content).is_err(),
            "never twice"
        );
        assert!(add_zstd_content_checksum(b"not zstd", content).is_err());
    }

    #[test]
    fn scrub_fails_closed_on_a_world_file_it_cannot_account_for() {
        let prefabs = PrefabNames::default();
        let mut tar = Vec::new();
        append_tar_entry(
            &mut tar,
            "SaveDir/worlds_local/Dedicated/_main.1.chunks",
            &chunks_meta_bytes(1),
        );
        append_tar_entry(
            &mut tar,
            "SaveDir/worlds_local/Dedicated/00_00__1_1.chunk",
            &scrub_test_chunk(),
        );
        // A legacy-layout file alongside the v41 chunks: the scrubber does not know this shape and
        // must refuse the whole world rather than write a partially-accounted-for copy.
        append_tar_entry(
            &mut tar,
            "SaveDir/worlds_local/Dedicated.db",
            b"legacy world data",
        );
        finish_tar(&mut tar);

        assert!(scrub_world_tar_bytes(&tar, &prefabs).is_err());
    }

    #[test]
    fn truncation_is_an_error() {
        let mut reader = &[41u8, 0, 1, 0][..];
        assert!(parse_chunk_reader(
            &mut reader,
            4,
            "a",
            "a",
            "00_00__1_1.chunk",
            &PrefabNames::default(),
            &[],
            &mut ScrubSink::default()
        )
        .is_err());
    }

    #[test]
    fn unsupported_versions_are_rejected() {
        assert!(parse_direct_item(&[110]).is_err());
        let mut inventory = Vec::new();
        push_i32(&mut inventory, 110);
        push_u16(&mut inventory, 0);
        assert!(parse_inventory(&inventory).is_err());

        let mut chunk = Vec::new();
        push_i16(&mut chunk, 40);
        assert!(parse_chunk_reader(
            &mut &chunk[..],
            chunk.len() as u64,
            "a",
            "a",
            "00_00__1_1.chunk",
            &PrefabNames::default(),
            &[],
            &mut ScrubSink::default()
        )
        .is_err());
        let mut legacy = Vec::new();
        push_i32(&mut legacy, 38);
        assert!(parse_legacy_reader(
            &mut &legacy[..],
            "a",
            "a",
            "Dedicated.db",
            &PrefabNames::default()
        )
        .is_err());
        let mut profile = Vec::new();
        push_i32(&mut profile, 43);
        push_i32(&mut profile, 205);
        push_i32(&mut profile, 10);
        assert!(parse_character_profile(&profile).is_err());
    }

    #[test]
    fn invalid_fch_hash_is_untrusted_and_suppresses_values() {
        let path =
            std::env::temp_dir().join(format!("valheim-invalid-hash-{}.fch", std::process::id()));
        let payload = 46i32.to_le_bytes();
        let mut wrapper = Vec::new();
        push_i32(&mut wrapper, payload.len() as i32);
        wrapper.extend(payload);
        push_i32(&mut wrapper, 64);
        wrapper.extend([0u8; 64]);
        std::fs::write(&path, wrapper).unwrap();
        let scan = scan_character(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        assert!(!scan.hash_valid);
        assert!(!scan.trusted);
        assert!(!scan.supported);
        assert!(scan.player_name.is_empty());
        assert!(!scan.used_cheats);
        assert_eq!(scan.cheat_stat_nonzero_count, 0);
        assert_eq!(scan.cheated_inventory_count, 0);
    }

    fn tar_header(name: &str, size: u64) -> [u8; 512] {
        let mut block = [0u8; 512];
        block[..name.len()].copy_from_slice(name.as_bytes());
        let size_text = format!("{size:011o}");
        block[124..135].copy_from_slice(size_text.as_bytes());
        block[135] = 0;
        block[148..156].fill(b' ');
        let checksum = block.iter().map(|byte| *byte as u64).sum::<u64>();
        let checksum_text = format!("{checksum:06o}");
        block[148..154].copy_from_slice(checksum_text.as_bytes());
        block[154] = 0;
        block[155] = b' ';
        block
    }

    #[test]
    fn tar_requires_checksum_and_two_end_blocks() {
        let mut valid = tar_header("file", 0).to_vec();
        valid.extend([0u8; 1024]);
        let mut tar = TarStream::new(&valid[..]);
        let mut entries = 0;
        tar.for_each_entry(|_, body| {
            entries += 1;
            body.drain()
        })
        .unwrap();
        tar.reject_nonzero_trailing().unwrap();
        assert_eq!(entries, 1);

        let mut zero_padding = valid.clone();
        zero_padding.extend([0u8; 17]);
        let mut tar = TarStream::new(&zero_padding[..]);
        tar.for_each_entry(|_, body| body.drain()).unwrap();
        tar.reject_nonzero_trailing().unwrap();

        let mut trailing = valid.clone();
        trailing.extend([0u8, 1]);
        let mut tar = TarStream::new(&trailing[..]);
        tar.for_each_entry(|_, body| body.drain()).unwrap();
        assert!(tar.reject_nonzero_trailing().is_err());

        let mut bad_checksum = valid.clone();
        bad_checksum[0] ^= 1;
        assert!(TarStream::new(&bad_checksum[..])
            .for_each_entry(|_, body| body.drain())
            .is_err());

        let missing_end = tar_header("file", 0)
            .into_iter()
            .chain([0u8; 512])
            .collect::<Vec<_>>();
        assert!(TarStream::new(&missing_end[..])
            .for_each_entry(|_, body| body.drain())
            .is_err());
        assert!(TarStream::new(&[1u8][..])
            .for_each_entry(|_, body| body.drain())
            .is_err());
    }

    fn test_evidence(snapshot: &str, stack: u16) -> Evidence {
        Evidence {
            archive: format!("{snapshot}.tar.zst"),
            snapshot: snapshot.to_string(),
            internal_path: "world/00_00__0_1.chunk".to_string(),
            format: "chunked_v41".to_string(),
            chunk_filename: Some("00_00__0_1.chunk".to_string()),
            chunk_version: Some(41),
            chunk_size: Some(0),
            chunk_revision: Some(1),
            zdo_ordinal: 1,
            owner_prefab_hash: stable_hash("piece_chest"),
            owner_prefab_name: Some("piece_chest".to_string()),
            position: Position {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            },
            legacy_sector: None,
            key_hash: ITEM_DATA,
            key_name: "itemData".to_string(),
            kind: "direct_item_data".to_string(),
            item_hash: Some(stable_hash("Silver")),
            item_name: Some("Silver".to_string()),
            grid: Some((0, 0)),
            quality: Some(1),
            stack: Some(stack),
            variant: Some(0),
            crafter_id: None,
            crafter_name: None,
            world_level: Some(0),
            custom_keys: Vec::new(),
        }
    }

    #[test]
    fn consolidated_table_excludes_stack_from_identity() {
        let mut old = ArchiveScan {
            snapshot: "older".to_string(),
            ..ArchiveScan::default()
        };
        old.evidence.push(test_evidence("older", 9));
        let mut latest = ArchiveScan {
            snapshot: "latest".to_string(),
            ..ArchiveScan::default()
        };
        latest.evidence.push(test_evidence("latest", 4));
        let archives = vec![
            ArchiveScan {
                snapshot: "earliest".to_string(),
                ..ArchiveScan::default()
            },
            old,
            latest,
        ];
        let rows = consolidated_evidence(&archives);
        assert_eq!(rows.len(), 1);
        assert_eq!(consolidated_status(&rows[0]), "persisted");
        let csv = world_evidence_csv(&archives);
        assert_eq!(csv.lines().count(), 2);
        assert!(csv.lines().next().unwrap().contains("present_earliest"));
        assert!(csv.contains("false,,"));
        assert!(csv.contains("true,9,"));
        assert!(csv.contains("true,4,"));
    }

    fn browser_archive(snapshot: &str, stacks: &[u16]) -> ArchiveScan {
        let mut archive = ArchiveScan {
            snapshot: snapshot.to_string(),
            archive: format!("{snapshot}.tar.zst"),
            ..ArchiveScan::default()
        };
        archive.evidence = stacks
            .iter()
            .map(|stack| test_evidence(snapshot, *stack))
            .collect();
        archive
    }

    #[test]
    fn browser_lineage_handles_one_two_and_three_saves_with_duplicates() {
        let one = browser_lineages(&[browser_archive("one", &[1, 2])]);
        assert_eq!(
            one[0].iter().map(|row| row.status).collect::<Vec<_>>(),
            ["observed", "observed"]
        );
        assert_eq!(one[0][1].occurrence_index, 2);

        let two = browser_lineages(&[
            browser_archive("one", &[1]),
            browser_archive("two", &[1, 2]),
        ]);
        assert_eq!(two[0][0].status, "persisted");
        assert_eq!(two[1][0].status, "persisted");
        assert_eq!(two[1][1].status, "new");
        assert_eq!(two[1][1].first_seen_snapshot, "two");

        let three = browser_lineages(&[
            browser_archive("one", &[1]),
            browser_archive("two", &[1, 2, 3]),
            browser_archive("three", &[1, 2]),
        ]);
        assert_eq!(three[1][1].status, "persisted");
        assert_eq!(three[2][1].status, "persisted");
        assert_eq!(three[1][2].status, "removed_or_cleared");
        assert!(!three[1][2].present_in_latest);
    }

    #[test]
    fn browser_lineage_uses_archive_indexes_when_snapshots_repeat() {
        let rows =
            browser_lineages(&[browser_archive("same", &[1]), browser_archive("same", &[1])]);
        assert_eq!(rows[1][0].status, "persisted");
        assert_eq!(rows[1][0].first_seen_snapshot, "same");
        assert_eq!(rows[1][0].last_seen_snapshot, "same");
    }

    #[test]
    fn browser_map_plots_the_newest_archive_and_orders_cells_deterministically() {
        let mut older = browser_archive("older", &[]);
        older.grid.insert((1, 2), 5);
        let mut latest = browser_archive("latest", &[]);
        latest.grid.insert((3, 4), 7);
        latest.grid.insert((-1, 0), 2);
        let json = browser_report_json(&[older, latest], &[]);
        assert!(json.contains("\"cell_meters\":64"));
        assert!(json.contains("[-1,0,2],[3,4,7]"), "cells sort by (x, z)");
        assert!(
            !json.contains("[1,2,5]"),
            "only the newest archive's world is plotted"
        );
    }

    #[test]
    fn browser_summary_keeps_zdos_and_decoded_items_in_order() {
        let mut archive = browser_archive("one", &[]);
        archive.zdo_count = 7;
        archive.decoded_item_count = 3;
        let report = browser_report_json(&[archive], &[]);
        assert!(report
            .contains("\"summary\":{\"archive_count\":1,\"zdo_count\":7,\"decoded_item_count\":3"));
    }

    #[test]
    fn browser_tar_requires_a_parseable_world_payload() {
        let mut tar = tar_header("metadata.txt", 0).to_vec();
        tar.extend([0u8; 1024]);
        let error = scan_tar_bytes(&tar, "metadata.tar", &PrefabNames::default()).unwrap_err();
        assert!(error
            .to_string()
            .contains("no recognized parseable world payload"));

        let mut invalid_path = tar_header("../metadata.txt", 0).to_vec();
        invalid_path.extend([0u8; 1024]);
        let error =
            scan_tar_bytes(&invalid_path, "metadata.tar", &PrefabNames::default()).unwrap_err();
        assert!(error.to_string().contains("invalid tar path"));
    }

    #[test]
    fn browser_markdown_escapes_untrusted_text_and_uses_newlines() {
        let mut archive = browser_archive("bad|archive\nname", &[1]);
        archive.evidence[0].internal_path = "world/bad|path`\r\n.chunk".to_string();
        archive.evidence[0].owner_prefab_name = Some("owner|`name".to_string());
        archive.evidence[0].item_name = Some("item\nname".to_string());
        archive.evidence[0].crafter_name = Some("crafter|name".to_string());
        let markdown = browser_report_markdown(&[archive], &[]);
        assert!(markdown.contains("# Valheim Cheat Radar report\n\n"));
        assert!(!markdown.contains("\\n"));
        assert!(markdown.contains("bad\\|archive name"));
        assert!(markdown.contains("owner\\|\\`name"));
        assert!(markdown.contains("bad\\|path"));
    }

    #[test]
    fn browser_canonical_requires_an_unambiguous_trusted_supported_profile() {
        assert_eq!(
            browser_modified_unix(1_700_000_000_999.0),
            Some(1_700_000_000)
        );
        assert_eq!(browser_modified_unix(f64::NAN), None);
        let mut invalid = CharacterScan {
            source: "z-invalid.fch".to_string(),
            canonical_eligible: true,
            modified_unix: Some(9_999),
            trusted: false,
            supported: false,
            ..CharacterScan::default()
        };
        let mut older = invalid.clone();
        older.source = "z-supported.fch".to_string();
        older.modified_unix = Some(100);
        older.trusted = true;
        older.supported = true;
        let mut newer = older.clone();
        newer.source = "a-supported.fch".to_string();
        newer.modified_unix = Some(200);
        assert_eq!(
            browser_canonical_index(&[invalid.clone(), older.clone(), newer.clone()]),
            Some(2)
        );
        let mut tie = newer;
        tie.input_id = 99;
        tie.modified_unix = Some(100);
        assert_eq!(browser_canonical_index(&[older, tie]), None);
        invalid.trusted = true;
        invalid.supported = true;
        invalid.modified_unix = Some(300);
        assert_eq!(
            browser_canonical_index(&[
                invalid,
                CharacterScan {
                    source: "a.fch".to_string(),
                    ..CharacterScan::default()
                }
            ]),
            Some(0)
        );
    }

    #[test]
    fn unicode_json_is_preserved() {
        let character = CharacterScan {
            source: "character-saves/active.fch".to_string(),
            canonical: true,
            canonical_eligible: true,
            input_id: 0,
            modified_unix: None,
            file_bytes: 1,
            payload_bytes: 1,
            hash_bytes: 64,
            hash_valid: true,
            trusted: true,
            supported: true,
            parse_error: None,
            profile_version: 46,
            stat_slots: 205,
            stat_profiles: 10,
            player_name: "Test ÆgirÞór".to_string(),
            player_id: Some(1),
            used_cheats: false,
            cheat_stat_nonzero_count: 0,
            known_command_hits: 0,
            player_data_version: Some(33),
            inventory_version: Some(109),
            inventory_item_count: 21,
            cheated_inventory_count: 0,
            bypass_cheat_checks: false,
            player_data_complete: true,
        };
        assert!(validate_character_oracle(std::slice::from_ref(&character)).is_ok());
        let json = report_json_with_characters(&[], &[character]);
        assert!(json.contains("\"profile_name\":\"Test ÆgirÞór\""));
        assert!(!json.contains('Ã'));
    }

    #[test]
    fn byte_entry_points_and_browser_reports_are_generic_and_private() {
        let prefabs = PrefabNames::from_text("piece_chest\nSilver\n");
        let mut chunk = Vec::new();
        push_i16(&mut chunk, 41);
        push_i32(&mut chunk, 0);
        let parsed_chunk =
            parse_chunk_bytes(&chunk, "C:\\private\\00_00__1_1.chunk", &prefabs).unwrap();
        assert_eq!(parsed_chunk.zdo_count, 0);

        let mut legacy = Vec::new();
        push_i32(&mut legacy, 37);
        legacy.extend(0f64.to_le_bytes());
        legacy.extend(0i64.to_le_bytes());
        legacy.extend(0u32.to_le_bytes());
        push_i32(&mut legacy, 0);
        let parsed_legacy = parse_legacy_bytes(&legacy, "Dedicated.db", &prefabs).unwrap();
        assert_eq!(parsed_legacy.zdo_count, 0);

        let mut character = CharacterScan {
            source: "C:\\private\\active.fch".to_string(),
            canonical: true,
            canonical_eligible: true,
            input_id: 0,
            modified_unix: None,
            file_bytes: 1,
            payload_bytes: 1,
            hash_bytes: 64,
            hash_valid: true,
            trusted: true,
            supported: true,
            parse_error: None,
            profile_version: 46,
            stat_slots: 205,
            stat_profiles: 10,
            player_name: "Browser Tester".to_string(),
            player_id: Some(987654321),
            used_cheats: false,
            cheat_stat_nonzero_count: 0,
            known_command_hits: 0,
            player_data_version: Some(33),
            inventory_version: Some(109),
            inventory_item_count: 2,
            cheated_inventory_count: 0,
            bypass_cheat_checks: false,
            player_data_complete: true,
        };
        let mut archive = ArchiveScan {
            archive: "C:\\private\\2024-01-02.tar.zst".to_string(),
            snapshot: "2024-01-02".to_string(),
            ..ArchiveScan::default()
        };
        archive.evidence.push(test_evidence("2024-01-02", 4));
        let archives = [archive];
        let characters = [character.clone()];
        let mut earlier = ArchiveScan {
            snapshot: "2024-01-01".to_string(),
            ..ArchiveScan::default()
        };
        earlier.evidence.push(test_evidence("2024-01-01", 9));
        let lineages = browser_lineages(&[earlier, {
            let mut later = ArchiveScan {
                snapshot: "2024-01-02".to_string(),
                ..ArchiveScan::default()
            };
            later.evidence.push(test_evidence("2024-01-02", 4));
            later
        }]);
        assert_eq!(lineages[0][0].status, "persisted");
        assert_eq!(lineages[1][0].status, "persisted");
        assert_eq!(lineages[0][0].first_seen_snapshot, "2024-01-01");
        assert_eq!(lineages[0][0].last_seen_snapshot, "2024-01-02");
        assert!(lineages[1][0].present_in_latest);
        let json = browser_report_json(&archives, &characters);
        assert!(json.contains("owner_prefab_hash"));
        assert!(json.contains("stack"));
        assert!(json.contains("position"));
        assert!(!json.contains("987654321"));
        assert!(!json.contains("C:\\\\private"));
        assert!(!json.contains("new_world_lineage"));
        assert!(!json.contains("first_evidence_window"));
        assert!(!json.contains("present_2024"));
        assert!(!json.contains("delta_status"));
        let csv = browser_report_csv(&archives, &characters);
        assert!(!csv.contains("present_2024"));
        assert!(!csv.contains("delta_status"));
        assert!(browser_report_markdown(&archives, &characters).contains("A single save"));
        character.canonical = false;
        assert_eq!(browser_character_status(&character), "trusted");
        assert!(browser_report_csv(&archives, &characters).contains("lineage"));
    }

    #[test]
    fn validate_oracle_rejects_duplicate_labels_and_inconsistent_item_counts() {
        let first = ArchiveScan {
            snapshot: "same".to_string(),
            ..ArchiveScan::default()
        };
        let second = ArchiveScan {
            snapshot: "same".to_string(),
            ..ArchiveScan::default()
        };
        assert!(validate_oracle(&[first, second]).is_err());
        let inconsistent = ArchiveScan {
            snapshot: "one".to_string(),
            item_count: 2,
            direct_item_count: 1,
            ..ArchiveScan::default()
        };
        assert!(validate_oracle(std::slice::from_ref(&inconsistent)).is_err());
        let consistent = ArchiveScan {
            item_count: 2,
            direct_item_count: 2,
            ..inconsistent
        };
        assert!(validate_oracle(std::slice::from_ref(&consistent)).is_ok());
    }

    #[test]
    fn consolidated_csv_keeps_headers_aligned_with_rows_for_any_archive_count() {
        for archives in [
            vec![browser_archive("only", &[1])],
            vec![
                browser_archive("one", &[1]),
                browser_archive("two", &[1, 2]),
            ],
            vec![
                browser_archive("one", &[1]),
                browser_archive("two", &[1, 2]),
                browser_archive("three", &[1, 2]),
            ],
        ] {
            let csv = world_evidence_csv(&archives);
            let mut lines = csv.lines();
            let headers = lines.next().unwrap().split(',').count();
            let mut rows = 0;
            for row in lines {
                rows += 1;
                assert_eq!(row.split(',').count(), headers, "{csv}");
            }
            assert!(rows > 0);
        }
    }

    #[test]
    fn oracle_fixture_checks_snapshots_deltas_and_character_cleanliness() {
        let mut archives = vec![
            browser_archive("old", &[1]),
            browser_archive("latest", &[1, 2]),
        ];
        archives[0].zdo_count = 10;
        archives[1].zdo_count = 20;
        let character = CharacterScan {
            canonical: true,
            trusted: true,
            hash_valid: true,
            supported: true,
            player_data_complete: true,
            ..CharacterScan::default()
        };
        let passing = "old 10 0\nlatest 20\n# comment\ndelta old latest 1 0 1 1\ncharacter clean";
        assert!(
            validate_oracle_fixture(&archives, std::slice::from_ref(&character), passing).is_ok()
        );
        assert!(validate_oracle_fixture(&archives, &[], "old 11").is_err());
        let dirty = CharacterScan {
            used_cheats: true,
            ..character
        };
        assert!(validate_oracle_fixture(
            &archives,
            std::slice::from_ref(&dirty),
            "character clean"
        )
        .is_err());
    }

    fn pstring(value: &str) -> Vec<u8> {
        let mut bytes = vec![value.len() as u8];
        bytes.extend(value.as_bytes());
        bytes
    }

    /// Mirrors the real `.fwl2` layout: declared length, world version, name, seed, uid, two small
    /// header words, a flag byte, the player count, then four strings per player.
    fn fwl2_bytes(name: &str, seed: &str, players: &[(&str, &str, &str, &str)]) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend(41u32.to_le_bytes());
        body.extend(pstring(name));
        body.extend(pstring(seed));
        body.extend(1_234_567_890_123u64.to_le_bytes());
        body.extend(0u32.to_le_bytes());
        body.extend(2u32.to_le_bytes());
        body.extend(1u32.to_le_bytes());
        body.push(0);
        body.extend((players.len() as u32).to_le_bytes());
        for (player_id, character, player, uid) in players {
            for value in [player_id, character, player, uid] {
                body.extend(pstring(value));
            }
        }
        let mut bytes = (body.len() as u32).to_le_bytes().to_vec();
        bytes.extend(body);
        bytes
    }

    /// Mirrors the real `.db2` layout: version, uid, payload length, gzip member, 40-byte trailer.
    fn db2_bytes(keys: &[&str]) -> Vec<u8> {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;
        let mut plain = Vec::new();
        plain.extend(0x2e70u32.to_le_bytes());
        plain.extend(0u32.to_le_bytes());
        plain.extend((keys.len() as u32).to_le_bytes());
        for key in keys {
            plain.extend(pstring(key));
        }
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&plain).unwrap();
        let gz = encoder.finish().unwrap();
        let mut bytes = 41u32.to_le_bytes().to_vec();
        bytes.extend(0u64.to_le_bytes());
        bytes.extend((gz.len() as u32).to_le_bytes());
        bytes.extend(&gz);
        bytes.extend([0u8; 40]);
        bytes
    }

    #[test]
    fn world_metadata_reads_name_seed_and_player_count_but_drops_identities() {
        let bytes = fwl2_bytes(
            "TestWorld",
            "AbCdEf1234",
            &[
                (
                    "Steam_76561190000000001",
                    "Owchar",
                    "Owchar",
                    "AAAABBBBCCCCDDDD",
                ),
                (
                    "Steam_76561190000000002",
                    "second",
                    "second",
                    "EEEEFFFFGGGGHHHH",
                ),
            ],
        );
        let meta = parse_fwl2_bytes(&bytes).unwrap();
        assert_eq!(meta.version, Some(41));
        assert_eq!(meta.name.as_deref(), Some("TestWorld"));
        assert_eq!(meta.seed.as_deref(), Some("AbCdEf1234"));
        assert_eq!(meta.player_count, Some(2));
        let rendered = format!("{meta:?}");
        assert!(!rendered.contains("Steam_"), "ids must not survive parsing");
        assert!(
            !rendered.contains("Owchar"),
            "names must not survive parsing"
        );
    }

    #[test]
    fn db2_global_keys_keep_only_progression_namespaces() {
        let bytes = db2_bytes(&[
            "defeated_eikthyr",
            "killedtroll",
            "activebosses 3",
            "seph",
            "Steam_76561190000000001",
            "worldmodifier 1",
        ]);
        let meta = parse_db2_bytes(&bytes).unwrap();
        assert_eq!(meta.version, Some(41));
        let keys = meta
            .global_keys
            .iter()
            .map(|key| (key.key.as_str(), key.value))
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            vec![
                ("defeated_eikthyr", None),
                ("killedtroll", None),
                ("activebosses", Some(3)),
            ]
        );
    }

    #[test]
    fn prefab_biome_table_parses_names_and_ignores_unknown_biomes() {
        let prefabs = PrefabNames::from_text("Wolf\nSilverOre\n").with_biome_text(
            "# comment\nWolf\tmountain\nSilverOre\tmountain,icecave\nmissing-tab\n\n",
        );
        assert_eq!(prefabs.biome_count(), 2);
        assert_eq!(prefabs.biome_mask(stable_hash("Wolf")), 0x04);
        // An unknown biome name is dropped, leaving the known one.
        assert_eq!(prefabs.biome_mask(stable_hash("SilverOre")), 0x04);
        assert_eq!(prefabs.biome_mask(stable_hash("Absent")), 0);
    }

    #[test]
    fn biome_verdict_needs_evidence_and_a_clear_winner() {
        let tally = |votes: &[(usize, u32)]| {
            let mut tally = [0u32; BIOME_COUNT + 1];
            for (index, weight) in votes {
                tally[*index] = *weight;
                tally[BIOME_TOTAL] += *weight;
            }
            tally
        };
        // Two objects is below the three-object bar.
        assert_eq!(biome_verdict(&tally(&[(0, 24)])), None);
        // Three single-biome objects agree: mountain (index 2).
        assert_eq!(biome_verdict(&tally(&[(2, 36)])), Some((2, 100)));
        // A 50/50 split is not a verdict.
        assert_eq!(biome_verdict(&tally(&[(0, 24), (2, 24)])), None);
        // A clear majority is.
        assert_eq!(biome_verdict(&tally(&[(0, 48), (2, 24)])), Some((0, 66)));
        // Nothing at all stays blank.
        assert_eq!(biome_verdict(&tally(&[])), None);
    }

    #[test]
    fn hint_column_marks_a_prefab_as_heuristic_not_real() {
        let prefabs = PrefabNames::from_text("Wolf\nFirTree\n")
            .with_biome_text("Wolf\tmountain\nFirTree\tblackforest,deepnorth\thint\n");
        assert_eq!(prefabs.biome_mask(stable_hash("Wolf")), 0x04);
        assert_eq!(prefabs.real_biome_mask(stable_hash("Wolf")), 0x04);
        // The hint column keeps the combined mask (still used for weighting and colouring)...
        assert_eq!(prefabs.biome_mask(stable_hash("FirTree")), 0x08 | 0x40);
        // ...but drops out of the real-only mask, so it can never win a cell on its own.
        assert_eq!(prefabs.real_biome_mask(stable_hash("FirTree")), 0);
    }

    #[test]
    fn real_evidence_wins_over_denser_hint_votes_in_a_mixed_cell() {
        let mut real = [0u32; BIOME_COUNT + 1];
        let mut combined = [0u32; BIOME_COUNT + 1];
        // Three real single-biome mountain objects (weight 12 each): a sparse but decisive signal.
        real[2] = 36;
        real[BIOME_TOTAL] = 36;
        combined[2] = 36;
        // A flood of hint-derived two-biome tree votes (weight 6 each) for blackforest, denser than
        // the mountain evidence and enough to swamp it in the combined tally alone.
        combined[3] = 120;
        combined[BIOME_TOTAL] = 36 + 120;
        // The combined tally alone would call this blackforest (120 of 156 = 76%).
        assert_eq!(biome_verdict(&combined), Some((3, 76)));
        // But real evidence alone already clears the bar for mountain, so it wins outright.
        assert_eq!(
            biome_verdict_preferring_real(&real, &combined),
            Some((2, 100))
        );
        // A cell with no real evidence at all still falls back to the combined (hint) tally.
        let no_real = [0u32; BIOME_COUNT + 1];
        assert_eq!(
            biome_verdict_preferring_real(&no_real, &combined),
            Some((3, 76))
        );
    }

    #[test]
    fn map_json_reports_coloured_cells_only() {
        let mut archive = browser_archive("latest", &[]);
        let mut strong = [0u32; BIOME_COUNT + 1];
        strong[1] = 36;
        strong[BIOME_TOTAL] = 36;
        archive.biomes.insert((0, 0), strong);
        let mut weak = [0u32; BIOME_COUNT + 1];
        weak[1] = 12;
        weak[BIOME_TOTAL] = 12;
        archive.biomes.insert((5, 5), weak);
        let json = browser_report_json(std::slice::from_ref(&archive), &[]);
        assert!(
            json.contains("\"biomes\":[[0,0,1]]"),
            "only the strong cell is coloured: {json}"
        );
        assert!(
            !json.contains("\"biomes\":[[0,0,1],[5,5"),
            "the thin cell stays blank in the fill layer"
        );
        assert!(json.contains("\"biome_names\":[\"meadows\",\"swamp\""));
        // The thin cell still gets a `biome_detail` entry, so a popup on it can explain why it has
        // no fill: too little evidence (12 of the 36-weight bar).
        assert!(
            json.contains(r#"[5,5,12,-1,0,"none",[[1,12]]]"#),
            "thin cell explains itself in biome_detail: {json}"
        );
    }

    #[test]
    fn biome_detail_json_reports_verdict_and_source_or_the_reason_for_none() {
        let mut archive = browser_archive("latest", &[]);
        // (0,0): real evidence alone is decisive — swamp (index 1), 100% share.
        let mut decisive_real = [0u32; BIOME_COUNT + 1];
        decisive_real[1] = 36;
        decisive_real[BIOME_TOTAL] = 36;
        archive.biomes_real.insert((0, 0), decisive_real);
        archive.biomes.insert((0, 0), decisive_real);
        // (1, 1): real evidence is too thin, but hint-derived votes in the combined tally decide it —
        // mountain (index 2) at 100%.
        let mut thin_real = [0u32; BIOME_COUNT + 1];
        thin_real[2] = 12;
        thin_real[BIOME_TOTAL] = 12;
        archive.biomes_real.insert((1, 1), thin_real);
        let mut hint_combined = [0u32; BIOME_COUNT + 1];
        hint_combined[2] = 48;
        hint_combined[BIOME_TOTAL] = 48;
        archive.biomes.insert((1, 1), hint_combined);
        // (2, 2): total weight never clears the 36-weight bar.
        let mut too_thin = [0u32; BIOME_COUNT + 1];
        too_thin[0] = 24;
        too_thin[BIOME_TOTAL] = 24;
        archive.biomes.insert((2, 2), too_thin);
        archive.biomes_real.insert((2, 2), too_thin);
        // (3, 3): plenty of weight but no biome clears the 60% share bar.
        let mut no_winner = [0u32; BIOME_COUNT + 1];
        no_winner[0] = 24;
        no_winner[2] = 24;
        no_winner[BIOME_TOTAL] = 48;
        archive.biomes.insert((3, 3), no_winner);
        archive.biomes_real.insert((3, 3), no_winner);

        let detail = browser_biome_detail_json(&archive);
        assert!(
            detail.contains(r#"[0,0,36,1,100,"real",[[1,36]]]"#),
            "decisive real cell: {detail}"
        );
        assert!(
            detail.contains(r#"[1,1,48,2,100,"hint",[[2,48]]]"#),
            "hint-decided cell: {detail}"
        );
        assert!(
            detail.contains(r#"[2,2,24,-1,0,"none",[[0,24]]]"#),
            "too-little-evidence cell: {detail}"
        );
        assert!(
            detail.contains(r#"[3,3,48,-1,0,"none",[[0,24],[2,24]]]"#),
            "no-clear-winner cell: {detail}"
        );
    }

    #[test]
    fn world_metadata_failures_are_recorded_rather_than_fatal() {
        let mut archive = ArchiveScan::default();
        merge_world_meta(&mut archive, parse_fwl2_bytes(&[1, 2, 3]));
        assert!(archive.world_metadata_error.is_some());
        assert!(archive.world.name.is_none());
        let rendered = format!(
            "{}:{}",
            json_world(&archive),
            browser_report_json(std::slice::from_ref(&archive), &[])
        );
        assert!(rendered.contains("world_metadata_error"));
    }

    #[test]
    fn scan_character_bytes_rejects_trailing_data() {
        let mut wrapper = Vec::new();
        push_i32(&mut wrapper, 4);
        wrapper.extend(46i32.to_le_bytes());
        push_i32(&mut wrapper, 0);
        wrapper.push(1);
        assert!(scan_character_bytes(&wrapper, "active.fch").is_err());
    }

    fn push_u8_vec(bytes: &mut Vec<u8>, value: u8) {
        bytes.push(value);
    }

    fn push_dotnet_string_for_test(bytes: &mut Vec<u8>, value: &str) {
        bytes.push(value.len() as u8);
        bytes.extend(value.as_bytes());
    }

    fn base64_for_test(bytes: &[u8]) -> String {
        const TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut output = String::new();
        for chunk in bytes.chunks(3) {
            let a = chunk[0];
            let b = *chunk.get(1).unwrap_or(&0);
            let c = *chunk.get(2).unwrap_or(&0);
            output.push(TABLE[(a >> 2) as usize] as char);
            output.push(TABLE[((a << 4 | b >> 4) & 63) as usize] as char);
            output.push(if chunk.len() > 1 {
                TABLE[((b << 2 | c >> 6) & 63) as usize] as char
            } else {
                '='
            });
            output.push(if chunk.len() > 2 {
                TABLE[(c & 63) as usize] as char
            } else {
                '='
            });
        }
        output
    }
}
