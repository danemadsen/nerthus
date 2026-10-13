//! How often each node type is added and which nodes follow which ports
//! (`StatisticsData`), kept in `usage.stats`: MessagePack,
//! `[NodeUsage, ConnectionStats]` with each connection statistic
//! `[Node, Port, Target, Count]`.
//!
//! The counts are read into the node catalogue once, at start
//! (`NodeMetadata.Usage`); the toolbox and the node search rank by those.
//! What happens during a session counts from the next one on.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const USAGE_FILE: &str = "usage.stats";

/// `ConnectionStatistic`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionStatistic {
    pub node: String,
    pub port: String,
    pub target: String,
    pub count: i32,
}

/// `StatisticsData`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatisticsData {
    pub node_usage: BTreeMap<String, i32>,
    pub connection_stats: BTreeMap<String, ConnectionStatistic>,
}

/// The statistics as they were when Nerthus started and as they are now.
#[derive(Clone, Debug, Default)]
pub struct UsageStats {
    /// `NodeMetadata.Usage` for every type, as read at start.
    startup: BTreeMap<String, i32>,
    current: StatisticsData,
    path: Option<PathBuf>,
    changed: bool,
}

impl UsageStats {
    /// The saved statistics, or none.
    pub fn load(path: PathBuf) -> Self {
        let current = std::fs::read(&path).ok().and_then(|bytes| decode(&bytes)).unwrap_or_default();
        let startup = current.node_usage.clone();
        Self { startup, current, path: Some(path), changed: false }
    }

    /// `NodeMetadata.Usage`: how often the type was added, as of the start.
    pub fn usage(&self, type_name: &str) -> i32 {
        self.startup.get(type_name).copied().unwrap_or(0)
    }

    /// Every type's usage as of the start, by type.
    pub fn startup_usage(&self) -> &BTreeMap<String, i32> {
        &self.startup
    }

    /// How many types have been added so far (`NodeUsage.Count`).
    pub fn used_type_count(&self) -> usize {
        self.current.node_usage.len()
    }

    /// The `limit` types added most so far, most first (ties in name
    /// order).
    pub fn most_used(&self, limit: usize) -> Vec<String> {
        let mut counts: Vec<(&String, &i32)> = self.current.node_usage.iter().collect();
        counts.sort_by_key(|(_, count)| std::cmp::Reverse(**count));
        counts.into_iter().take(limit).map(|(name, _)| name.clone()).collect()
    }

    /// A node of the type was added.
    pub fn record_node(&mut self, type_name: &str) {
        *self.current.node_usage.entry(type_name.to_string()).or_insert(0) += 1;
        self.changed = true;
    }

    /// `target` was connected to `node`'s `port` (node
    /// names, as the statistics key them).
    pub fn record_connection(&mut self, node: &str, port: &str, target: &str) {
        let key = format!("{node}-{port}-{target}");
        let entry = self.current.connection_stats.entry(key).or_insert_with(|| ConnectionStatistic {
            node: node.to_string(),
            port: port.to_string(),
            target: target.to_string(),
            count: 0,
        });
        entry.count += 1;
        self.changed = true;
    }

    /// The targets most often connected to `node`'s
    /// `port`, those made more than ten times, most first.
    pub fn suggestions(&self, node: &str, port: &str, limit: usize) -> Vec<String> {
        let frequent = self.current.connection_stats.values().filter(|statistic| statistic.node == node && statistic.port == port && statistic.count > 10);
        let mut found: Vec<&ConnectionStatistic> = frequent.collect();
        found.sort_by_key(|statistic| std::cmp::Reverse(statistic.count));
        found.into_iter().take(limit).map(|statistic| statistic.target.clone()).collect()
    }

    /// Written when it changed.
    pub fn save_if_changed(&mut self) {
        if !self.changed {
            return;
        }
        let Some(path) = self.path.clone() else {
            self.changed = false;
            return;
        };
        let bytes = encode(&self.current);
        if write_file(&path, &bytes) {
            self.changed = false;
        }
    }
}

fn write_file(path: &Path, bytes: &[u8]) -> bool {
    if let Some(folder) = path.parent()
        && let Err(error) = std::fs::create_dir_all(folder)
    {
        log::warn!("cannot make {}: {error}", folder.display());
        return false;
    }
    let temporary = path.with_extension("stats.tmp");
    let written = std::fs::write(&temporary, bytes).and_then(|_| std::fs::rename(&temporary, path));
    if let Err(error) = written {
        log::warn!("cannot write {}: {error}", path.display());
        return false;
    }
    true
}

/// `MessagePackSerializer.Serialize(StatisticsData)`.
pub fn encode(data: &StatisticsData) -> Vec<u8> {
    let mut bytes = Vec::new();
    write_array_header(&mut bytes, 2);
    write_map_header(&mut bytes, data.node_usage.len());
    for (name, count) in &data.node_usage {
        write_string(&mut bytes, name);
        write_int(&mut bytes, *count);
    }
    write_map_header(&mut bytes, data.connection_stats.len());
    for (key, statistic) in &data.connection_stats {
        write_string(&mut bytes, key);
        write_array_header(&mut bytes, 4);
        write_string(&mut bytes, &statistic.node);
        write_string(&mut bytes, &statistic.port);
        write_string(&mut bytes, &statistic.target);
        write_int(&mut bytes, statistic.count);
    }
    bytes
}

/// `MessagePackSerializer.Deserialize<StatisticsData>`; none when the
/// bytes are not a statistics object.
pub fn decode(bytes: &[u8]) -> Option<StatisticsData> {
    let mut reader = Reader { bytes, at: 0 };
    let fields = reader.array_header()?;
    let mut data = StatisticsData::default();
    if fields >= 1 {
        let count = reader.map_header()?;
        for _ in 0..count {
            let name = reader.string()?;
            let usage = reader.int()?;
            data.node_usage.insert(name, usage);
        }
    }
    if fields >= 2 {
        let count = reader.map_header()?;
        for _ in 0..count {
            let key = reader.string()?;
            let members = reader.array_header()?;
            let node = if members > 0 { reader.string()? } else { String::new() };
            let port = if members > 1 { reader.string()? } else { String::new() };
            let target = if members > 2 { reader.string()? } else { String::new() };
            let count = if members > 3 { reader.int()? } else { 1 };
            for _ in 4..members {
                reader.skip()?;
            }
            data.connection_stats.insert(key, ConnectionStatistic { node, port, target, count });
        }
    }
    Some(data)
}

fn write_array_header(bytes: &mut Vec<u8>, length: usize) {
    if length < 16 {
        bytes.push(0x90 | length as u8);
    } else if length <= u16::MAX as usize {
        bytes.push(0xdc);
        bytes.extend_from_slice(&(length as u16).to_be_bytes());
    } else {
        bytes.push(0xdd);
        bytes.extend_from_slice(&(length as u32).to_be_bytes());
    }
}

fn write_map_header(bytes: &mut Vec<u8>, length: usize) {
    if length < 16 {
        bytes.push(0x80 | length as u8);
    } else if length <= u16::MAX as usize {
        bytes.push(0xde);
        bytes.extend_from_slice(&(length as u16).to_be_bytes());
    } else {
        bytes.push(0xdf);
        bytes.extend_from_slice(&(length as u32).to_be_bytes());
    }
}

fn write_string(bytes: &mut Vec<u8>, text: &str) {
    let utf8 = text.as_bytes();
    let length = utf8.len();
    if length < 32 {
        bytes.push(0xa0 | length as u8);
    } else if length <= u8::MAX as usize {
        bytes.push(0xd9);
        bytes.push(length as u8);
    } else if length <= u16::MAX as usize {
        bytes.push(0xda);
        bytes.extend_from_slice(&(length as u16).to_be_bytes());
    } else {
        bytes.push(0xdb);
        bytes.extend_from_slice(&(length as u32).to_be_bytes());
    }
    bytes.extend_from_slice(utf8);
}

/// `MessagePackWriter.Write(int)`: the shortest form.
fn write_int(bytes: &mut Vec<u8>, value: i32) {
    if (0..=0x7f).contains(&value) {
        bytes.push(value as u8);
    } else if (-32..0).contains(&value) {
        bytes.push(value as i8 as u8);
    } else if (0..=u8::MAX as i32).contains(&value) {
        bytes.push(0xcc);
        bytes.push(value as u8);
    } else if (0..=u16::MAX as i32).contains(&value) {
        bytes.push(0xcd);
        bytes.extend_from_slice(&(value as u16).to_be_bytes());
    } else if value >= 0 {
        bytes.push(0xce);
        bytes.extend_from_slice(&(value as u32).to_be_bytes());
    } else if value >= i8::MIN as i32 {
        bytes.push(0xd0);
        bytes.push(value as i8 as u8);
    } else if value >= i16::MIN as i32 {
        bytes.push(0xd1);
        bytes.extend_from_slice(&(value as i16).to_be_bytes());
    } else {
        bytes.push(0xd2);
        bytes.extend_from_slice(&value.to_be_bytes());
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn byte(&mut self) -> Option<u8> {
        let byte = *self.bytes.get(self.at)?;
        self.at += 1;
        Some(byte)
    }

    fn take(&mut self, count: usize) -> Option<&[u8]> {
        let end = self.at.checked_add(count)?;
        let slice = self.bytes.get(self.at..end)?;
        self.at = end;
        Some(slice)
    }

    fn big_endian(&mut self, count: usize) -> Option<u64> {
        let slice = self.take(count)?;
        Some(slice.iter().fold(0u64, |value, byte| (value << 8) | *byte as u64))
    }

    fn array_header(&mut self) -> Option<usize> {
        let marker = self.byte()?;
        match marker {
            0x90..=0x9f => Some((marker & 0x0f) as usize),
            0xdc => self.big_endian(2).map(|length| length as usize),
            0xdd => self.big_endian(4).map(|length| length as usize),
            _ => None,
        }
    }

    fn map_header(&mut self) -> Option<usize> {
        let marker = self.byte()?;
        match marker {
            0x80..=0x8f => Some((marker & 0x0f) as usize),
            0xde => self.big_endian(2).map(|length| length as usize),
            0xdf => self.big_endian(4).map(|length| length as usize),
            // A null dictionary reads as an empty one.
            0xc0 => Some(0),
            _ => None,
        }
    }

    fn string(&mut self) -> Option<String> {
        let marker = self.byte()?;
        let length = match marker {
            0xa0..=0xbf => (marker & 0x1f) as usize,
            0xd9 => self.big_endian(1)? as usize,
            0xda => self.big_endian(2)? as usize,
            0xdb => self.big_endian(4)? as usize,
            0xc0 => return Some(String::new()),
            _ => return None,
        };
        let utf8 = self.take(length)?;
        Some(String::from_utf8_lossy(utf8).to_string())
    }

    fn int(&mut self) -> Option<i32> {
        let marker = self.byte()?;
        let value = match marker {
            0x00..=0x7f => marker as i64,
            0xe0..=0xff => marker as i8 as i64,
            0xcc => self.big_endian(1)? as i64,
            0xcd => self.big_endian(2)? as i64,
            0xce => self.big_endian(4)? as i64,
            0xcf => self.big_endian(8)? as i64,
            0xd0 => self.big_endian(1)? as u8 as i8 as i64,
            0xd1 => self.big_endian(2)? as u16 as i16 as i64,
            0xd2 => self.big_endian(4)? as u32 as i32 as i64,
            0xd3 => self.big_endian(8)? as i64,
            _ => return None,
        };
        i32::try_from(value).ok()
    }

    /// Steps over one value of any kind.
    fn skip(&mut self) -> Option<()> {
        let marker = *self.bytes.get(self.at)?;
        match marker {
            0x80..=0x8f | 0xde | 0xdf => {
                let count = self.map_header()?;
                for _ in 0..count * 2 {
                    self.skip()?;
                }
            }
            0x90..=0x9f | 0xdc | 0xdd => {
                let count = self.array_header()?;
                for _ in 0..count {
                    self.skip()?;
                }
            }
            0xa0..=0xbf | 0xd9 | 0xda | 0xdb => {
                self.string()?;
            }
            0xc0 | 0xc2 | 0xc3 => {
                self.byte()?;
            }
            0xca => {
                self.take(5)?;
            }
            0xcb => {
                self.take(9)?;
            }
            _ => {
                self.int()?;
            }
        }
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statistics_round_trip_as_messagepack() {
        let mut data = StatisticsData::default();
        data.node_usage.insert("Mountain".to_string(), 300);
        data.node_usage.insert("ErosionPro".to_string(), 5);
        let statistic = ConnectionStatistic { node: "Mountain".to_string(), port: "Out".to_string(), target: "ErosionPro".to_string(), count: 12 };
        data.connection_stats.insert("Mountain-Out-ErosionPro".to_string(), statistic);
        let bytes = encode(&data);
        // `[ {..}, {..} ]` as MessagePack-CSharp writes it.
        assert_eq!(bytes[0], 0x92);
        assert_eq!(decode(&bytes), Some(data));
    }

    #[test]
    fn suggestions_need_more_than_ten_connections() {
        let mut stats = UsageStats::default();
        for _ in 0..11 {
            stats.record_connection("Mountain", "Out", "ErosionPro");
        }
        for _ in 0..3 {
            stats.record_connection("Mountain", "Out", "Blur");
        }
        assert_eq!(stats.suggestions("Mountain", "Out", 5), vec!["ErosionPro".to_string()]);
    }
}
