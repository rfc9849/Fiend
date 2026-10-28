use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use flate2::read::ZlibDecoder;
use thiserror::Error;

/// Largest compressed rule set that the parser accepts.
pub const MAX_SRS_BYTES: usize = 32 << 20;
/// Largest decompressed stream that the parser accepts.
pub const MAX_EXPANDED_BYTES: usize = 64 << 20;
/// Guard against a malformed trie that never ends.
const MAX_TRIE_STEPS: usize = 64 << 20;
const MAX_VERSION: u8 = 5;
const MAGIC: [u8; 3] = *b"SRS";
const PREFIX_LABEL: char = '\r';
const ROOT_LABEL: char = '\n';

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SrsError {
    #[error("rule set is larger than {MAX_SRS_BYTES} bytes")]
    TooLarge,
    #[error("rule set is not a sing-box binary rule set")]
    Magic,
    #[error("rule set version {0} is not supported")]
    Version(u8),
    #[error("rule set is malformed")]
    Malformed,
    #[error("rule set expands to more than {MAX_EXPANDED_BYTES} bytes")]
    Expanded,
}

/// Rule items that this parser cannot apply. A rule with one of these items is
/// dropped, so the remaining entries never become broader than the source.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SrsSkipped {
    pub rules: usize,
    pub regex: usize,
    pub adguard: usize,
    pub logical: usize,
}

#[derive(Debug, Default)]
pub struct SrsList {
    pub suffixes: Vec<String>,
    pub exact: Vec<String>,
    pub keywords: Vec<String>,
    pub ranges: Vec<(IpAddr, IpAddr)>,
    pub skipped: SrsSkipped,
}

/// Parse a sing-box binary rule set. The format is the magic `SRS`, one
/// version byte, a zlib stream, an unsigned varint rule count, then tagged
/// rule items. Domain items hold a succinct trie of reversed domain names.
pub fn parse(bytes: &[u8]) -> Result<SrsList, SrsError> {
    if bytes.len() > MAX_SRS_BYTES {
        return Err(SrsError::TooLarge);
    }
    if bytes.len() < 4 || bytes[..3] != MAGIC {
        return Err(SrsError::Magic);
    }
    let version = bytes[3];
    if version == 0 || version > MAX_VERSION {
        return Err(SrsError::Version(version));
    }
    let mut expanded = Vec::new();
    ZlibDecoder::new(&bytes[4..])
        .take((MAX_EXPANDED_BYTES + 1) as u64)
        .read_to_end(&mut expanded)
        .map_err(|_| SrsError::Malformed)?;
    if expanded.len() > MAX_EXPANDED_BYTES {
        return Err(SrsError::Expanded);
    }
    let mut reader = Reader::new(&expanded);
    let count = reader.uvarint()?;
    let mut list = SrsList::default();
    for _ in 0..count {
        read_rule(&mut reader, &mut list, true, 0)?;
    }
    Ok(list)
}

pub fn is_srs(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes[..3] == MAGIC
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn byte(&mut self) -> Result<u8, SrsError> {
        let byte = *self.data.get(self.pos).ok_or(SrsError::Malformed)?;
        self.pos += 1;
        Ok(byte)
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], SrsError> {
        let end = self.pos.checked_add(count).ok_or(SrsError::Malformed)?;
        let slice = self.data.get(self.pos..end).ok_or(SrsError::Malformed)?;
        self.pos = end;
        Ok(slice)
    }

    fn uvarint(&mut self) -> Result<u64, SrsError> {
        let mut value = 0u64;
        for shift in (0..70).step_by(7) {
            let byte = self.byte()?;
            if shift == 63 && byte > 1 {
                return Err(SrsError::Malformed);
            }
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(SrsError::Malformed)
    }

    fn count(&mut self, item: usize) -> Result<usize, SrsError> {
        let count = self.uvarint()?;
        let count = usize::try_from(count).map_err(|_| SrsError::Malformed)?;
        let remaining = self.data.len().saturating_sub(self.pos);
        if count > remaining.min(MAX_EXPANDED_BYTES) + 1 {
            return Err(SrsError::Malformed);
        }
        let _ = item;
        Ok(count)
    }

    fn u64_slice(&mut self) -> Result<Vec<u64>, SrsError> {
        let count = self.count(8)?;
        let bytes = self.take(count.checked_mul(8).ok_or(SrsError::Malformed)?)?;
        Ok(bytes
            .as_chunks::<8>()
            .0
            .iter()
            .map(|chunk| u64::from_be_bytes(*chunk))
            .collect())
    }

    fn bytes_slice(&mut self) -> Result<&'a [u8], SrsError> {
        let count = self.count(1)?;
        self.take(count)
    }

    fn strings(&mut self) -> Result<Vec<String>, SrsError> {
        let count = self.count(1)?;
        let mut values = Vec::new();
        for _ in 0..count {
            let bytes = self.bytes_slice()?;
            let text = std::str::from_utf8(bytes).map_err(|_| SrsError::Malformed)?;
            values.push(text.to_owned());
        }
        Ok(values)
    }

    fn u16_slice(&mut self) -> Result<(), SrsError> {
        let count = self.count(2)?;
        self.take(count.checked_mul(2).ok_or(SrsError::Malformed)?)?;
        Ok(())
    }

    fn u8_slice(&mut self) -> Result<(), SrsError> {
        let count = self.count(1)?;
        self.take(count)?;
        Ok(())
    }
}

struct SuccinctSet {
    leaves: Vec<u64>,
    bitmap: Vec<u64>,
    labels: Vec<u8>,
}

fn read_succinct(reader: &mut Reader) -> Result<SuccinctSet, SrsError> {
    let _version = reader.byte()?;
    let mut leaves = reader.u64_slice()?;
    let bitmap = reader.u64_slice()?;
    let labels = reader.bytes_slice()?.to_vec();
    if bitmap.is_empty() {
        return Err(SrsError::Malformed);
    }
    let mut ones = 0usize;
    let mut last_one: Option<usize> = None;
    for (index, word) in bitmap.iter().enumerate() {
        ones += word.count_ones() as usize;
        if *word != 0 {
            last_one = Some(index * 64 + 63 - word.leading_zeros() as usize);
        }
    }
    let last = last_one.ok_or(SrsError::Malformed)?;
    let zeros = (last + 1).checked_sub(ones).ok_or(SrsError::Malformed)?;
    if ones != zeros + 1 || labels.len() != zeros {
        return Err(SrsError::Malformed);
    }
    let words = ones.div_ceil(64);
    if leaves.len() < words {
        leaves.resize(words, 0);
    }
    Ok(SuccinctSet {
        leaves,
        bitmap,
        labels,
    })
}

fn get_bit(words: &[u64], index: usize) -> bool {
    words
        .get(index >> 6)
        .is_some_and(|word| word & (1 << (index & 63)) != 0)
}

fn rank(words: &[u64], ranks: &[usize], index: usize) -> usize {
    let word_index = index >> 6;
    let bit = index & 63;
    let base = ranks.get(word_index).copied().unwrap_or(0);
    let mask = if bit == 0 { 0 } else { u64::MAX >> (64 - bit) };
    base + words
        .get(word_index)
        .map_or(0, |word| (word & mask).count_ones() as usize)
}

fn count_zeros(words: &[u64], ranks: &[usize], index: usize) -> usize {
    index.saturating_sub(rank(words, ranks, index))
}

fn index_rank(words: &[u64]) -> Vec<usize> {
    let mut ranks = Vec::with_capacity(words.len() + 1);
    let mut ones = 0usize;
    for word in words {
        ranks.push(ones);
        ones += word.count_ones() as usize;
    }
    ranks.push(ones);
    ranks
}

fn index_select(words: &[u64]) -> Vec<usize> {
    let mut selects = Vec::new();
    let mut seen = 0usize;
    for index in 0..words.len() * 64 {
        if get_bit(words, index) {
            if seen.is_multiple_of(32) {
                selects.push(index);
            }
            seen += 1;
        }
    }
    selects
}

fn select_ith(words: &[u64], selects: &[usize], ranks: &[usize], index: usize) -> usize {
    let Some(&start) = selects.get(index >> 5) else {
        return words.len() * 64;
    };
    let mut word_index = start >> 6;
    while ranks.get(word_index + 1).is_some_and(|ones| *ones <= index) {
        word_index += 1;
    }
    let base = word_index << 6;
    let mut remaining = index.saturating_sub(ranks.get(word_index).copied().unwrap_or(0));
    let mut word = words.get(word_index).copied().unwrap_or(0);
    let mut position = 0usize;
    loop {
        if word & 1 != 0 {
            if remaining == 0 {
                return base + position;
            }
            remaining -= 1;
        }
        word >>= 1;
        position += 1;
        if word == 0 {
            return base + position;
        }
    }
}

impl SuccinctSet {
    /// Rebuild every key in the trie. Keys are reversed domain names with an
    /// optional label prefix.
    fn keys(&self) -> Result<Vec<String>, SrsError> {
        let ranks = index_rank(&self.bitmap);
        let selects = index_select(&self.bitmap);
        let mut keys = Vec::new();
        let mut current: Vec<u8> = Vec::new();
        if get_bit(&self.leaves, 0) {
            keys.push(String::new());
        }
        let mut stack: Vec<(usize, usize)> = vec![(0, 0)];
        let mut steps = 0usize;
        while let Some(&(node, index)) = stack.last() {
            steps += 1;
            if steps > MAX_TRIE_STEPS {
                return Err(SrsError::Malformed);
            }
            if get_bit(&self.bitmap, index) {
                stack.pop();
                if !stack.is_empty() {
                    current.pop();
                    if let Some(parent) = stack.last_mut() {
                        parent.1 += 1;
                    }
                }
                continue;
            }
            let label = *self
                .labels
                .get(index.checked_sub(node).ok_or(SrsError::Malformed)?)
                .ok_or(SrsError::Malformed)?;
            current.push(label);
            let next_node = count_zeros(&self.bitmap, &ranks, index + 1);
            let next_index =
                select_ith(&self.bitmap, &selects, &ranks, next_node.saturating_sub(1))
                    .checked_add(1)
                    .ok_or(SrsError::Malformed)?;
            if get_bit(&self.leaves, next_node) {
                keys.push(String::from_utf8_lossy(&current).into_owned());
            }
            stack.push((next_node, next_index));
        }
        Ok(keys)
    }
}

fn read_rule(
    reader: &mut Reader,
    list: &mut SrsList,
    collect: bool,
    depth: usize,
) -> Result<(), SrsError> {
    if depth > 32 {
        return Err(SrsError::Malformed);
    }
    match reader.byte()? {
        0 => read_default_rule(reader, list, collect),
        1 => {
            if collect {
                list.skipped.logical += 1;
            }
            read_logical_rule(reader, list, depth)
        }
        _ => Err(SrsError::Malformed),
    }
}

fn read_logical_rule(
    reader: &mut Reader,
    list: &mut SrsList,
    depth: usize,
) -> Result<(), SrsError> {
    let _mode = reader.byte()?;
    let count = reader.count(1)?;
    for _ in 0..count {
        read_rule(reader, list, false, depth + 1)?;
    }
    let _invert = reader.byte()?;
    Ok(())
}

#[derive(Default)]
struct Parts {
    suffixes: Vec<String>,
    exact: Vec<String>,
    keywords: Vec<String>,
    ranges: Vec<(IpAddr, IpAddr)>,
    usable: bool,
    regex: bool,
    adguard: bool,
}

fn read_default_rule(
    reader: &mut Reader,
    list: &mut SrsList,
    collect: bool,
) -> Result<(), SrsError> {
    let mut parts = Parts {
        usable: true,
        ..Parts::default()
    };
    loop {
        let item = reader.byte()?;
        if item == 0xff {
            if reader.byte()? != 0 {
                parts.usable = false;
            }
            break;
        }
        read_item(reader, item, &mut parts)?;
    }
    if !collect {
        return Ok(());
    }
    if parts.usable {
        list.suffixes.extend(parts.suffixes);
        list.exact.extend(parts.exact);
        list.keywords.extend(parts.keywords);
        list.ranges.extend(parts.ranges);
    } else {
        list.skipped.rules += 1;
        if parts.regex {
            list.skipped.regex += 1;
        }
        if parts.adguard {
            list.skipped.adguard += 1;
        }
    }
    Ok(())
}

fn read_item(reader: &mut Reader, item: u8, parts: &mut Parts) -> Result<(), SrsError> {
    match item {
        // query_type
        0 => {
            reader.u16_slice()?;
            parts.usable = false;
        }
        // network
        1 => {
            reader.strings()?;
            parts.usable = false;
        }
        // domain
        2 => {
            let set = read_succinct(reader)?;
            for key in set.keys()? {
                classify(&key, parts);
            }
        }
        // domain_keyword
        3 => parts.keywords.extend(reader.strings()?),
        // domain_regex
        4 => {
            reader.strings()?;
            parts.regex = true;
            parts.usable = false;
        }
        // source_ip_cidr
        5 => {
            read_ip_set(reader)?;
            parts.usable = false;
        }
        // ip_cidr
        6 => parts.ranges.extend(read_ip_set(reader)?),
        // source_port, port
        7 | 9 => {
            reader.u16_slice()?;
            parts.usable = false;
        }
        // source_port_range, port_range, process_name, process_path,
        // process_path_regex, package_name, wifissid, wifibssid,
        // package_name_regex
        8 | 10 | 11 | 12 | 13 | 14 | 15 | 23 => {
            reader.strings()?;
            parts.usable = false;
        }
        // adguard_domain
        16 => {
            let _set = read_succinct(reader)?;
            parts.adguard = true;
            parts.usable = false;
        }
        // network_type
        17 => {
            reader.u8_slice()?;
            parts.usable = false;
        }
        // network_is_expensive, network_is_constrained
        18 | 19 => parts.usable = false,
        // network_interface_address
        20 => {
            let maps = reader.count(1)?;
            for _ in 0..maps {
                reader.byte()?;
                let prefixes = reader.count(1)?;
                for _ in 0..prefixes {
                    read_prefix(reader)?;
                }
            }
            parts.usable = false;
        }
        // default_interface_address
        21 => {
            let prefixes = reader.count(1)?;
            for _ in 0..prefixes {
                read_prefix(reader)?;
            }
            parts.usable = false;
        }
        _ => return Err(SrsError::Malformed),
    }
    Ok(())
}

fn read_prefix(reader: &mut Reader) -> Result<(), SrsError> {
    let length = reader.count(1)?;
    if length != 4 && length != 16 {
        return Err(SrsError::Malformed);
    }
    reader.take(length)?;
    let bits = reader.byte()?;
    if bits > 128 {
        return Err(SrsError::Malformed);
    }
    Ok(())
}

fn read_ip_set(reader: &mut Reader) -> Result<Vec<(IpAddr, IpAddr)>, SrsError> {
    if reader.byte()? != 1 {
        return Err(SrsError::Malformed);
    }
    // The range count is a fixed 64-bit big-endian value, not a varint.
    let count = u64::from_be_bytes(
        reader
            .take(8)?
            .try_into()
            .map_err(|_| SrsError::Malformed)?,
    );
    let count = usize::try_from(count).map_err(|_| SrsError::Malformed)?;
    if count > reader.data.len().saturating_sub(reader.pos) {
        return Err(SrsError::Malformed);
    }
    let mut ranges = Vec::with_capacity(count.min(4096));
    for _ in 0..count {
        let from = read_ip(reader)?;
        let to = read_ip(reader)?;
        match (from, to) {
            (IpAddr::V4(from), IpAddr::V4(to)) if from <= to => {
                ranges.push((IpAddr::V4(from), IpAddr::V4(to)));
            }
            (IpAddr::V6(from), IpAddr::V6(to)) if from <= to => {
                ranges.push((IpAddr::V6(from), IpAddr::V6(to)));
            }
            _ => return Err(SrsError::Malformed),
        }
    }
    Ok(ranges)
}

fn read_ip(reader: &mut Reader) -> Result<IpAddr, SrsError> {
    let length = reader.count(1)?;
    match length {
        4 => {
            let bytes: [u8; 4] = reader
                .take(4)?
                .try_into()
                .map_err(|_| SrsError::Malformed)?;
            Ok(IpAddr::V4(Ipv4Addr::from(bytes)))
        }
        16 => {
            let bytes: [u8; 16] = reader
                .take(16)?
                .try_into()
                .map_err(|_| SrsError::Malformed)?;
            Ok(IpAddr::V6(Ipv6Addr::from(bytes)))
        }
        _ => Err(SrsError::Malformed),
    }
}

fn classify(key: &str, parts: &mut Parts) {
    let reversed: String = key.chars().rev().collect();
    let mut chars = reversed.chars();
    match chars.next() {
        Some(PREFIX_LABEL) => {
            let rest: String = chars.collect();
            let host = rest.trim_start_matches('.');
            if let Some(host) = crate::rules::normalize(host) {
                parts.suffixes.push(host);
            }
        }
        Some(ROOT_LABEL) => {
            let rest: String = chars.collect();
            if let Some(host) = crate::rules::normalize(&rest) {
                parts.suffixes.push(host);
            }
        }
        Some(_) => {
            if let Some(host) = crate::rules::normalize(&reversed) {
                parts.exact.push(host);
            }
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data")
            .join(name);
        std::fs::read(&path).expect("fixture")
    }

    #[test]
    fn reads_a_small_rule_set() {
        let list = parse(&fixture("geosite-2gis.srs")).expect("parse");
        assert!(list.suffixes.contains(&"2gis.ru".to_owned()));
        assert!(list.exact.contains(&"2gis.ru".to_owned()));
        assert!(list.keywords.is_empty());
        assert!(list.ranges.is_empty());
        assert_eq!(list.skipped, SrsSkipped::default());
    }

    #[test]
    fn reads_an_ip_rule_set() {
        let list = parse(&fixture("geoip-ru.srs")).expect("parse");
        assert!(list.suffixes.is_empty());
        assert!(list.ranges.len() > 1000);
        assert!(
            list.ranges
                .iter()
                .any(|(from, to)| matches!((from, to), (IpAddr::V4(_), IpAddr::V4(_))))
        );
        assert!(
            list.ranges
                .iter()
                .all(|(from, to)| std::mem::discriminant(from) == std::mem::discriminant(to))
        );
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(parse(b"SRS").err(), Some(SrsError::Magic));
        assert_eq!(parse(b"XXXX\x01").err(), Some(SrsError::Magic));
        assert_eq!(parse(b"SRS\x00").err(), Some(SrsError::Version(0)));
        assert_eq!(parse(b"SRS\x09").err(), Some(SrsError::Version(9)));
        assert_eq!(parse(b"SRS\x01garbage").err(), Some(SrsError::Malformed));
        assert_eq!(
            parse(&vec![b'X'; MAX_SRS_BYTES + 1]).err(),
            Some(SrsError::TooLarge)
        );
    }

    #[test]
    fn detects_the_magic() {
        assert!(is_srs(b"SRS\x01"));
        assert!(!is_srs(b"SRS"));
        assert!(!is_srs(b"domain:example.com\n"));
    }
}
