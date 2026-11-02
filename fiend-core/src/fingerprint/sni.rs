use std::ops::Range;

const HEADER: usize = 5;
const CLIENT_HELLO: u8 = 0x01;

/// Finds the host name bytes of the server name extension in a ClientHello
/// record.
pub fn server_name(record: &[u8]) -> Option<Range<usize>> {
    let mut cursor = Cursor {
        data: record,
        at: HEADER,
    };
    if cursor.u8()? != CLIENT_HELLO {
        return None;
    }
    cursor.skip(3 + 2 + 32)?;
    let session = usize::from(cursor.u8()?);
    cursor.skip(session)?;
    let suites = cursor.u16()?;
    cursor.skip(suites)?;
    let compression = usize::from(cursor.u8()?);
    cursor.skip(compression)?;
    let extensions = cursor.u16()?;
    let last = cursor.at.checked_add(extensions)?.min(record.len());
    while cursor.at + 4 <= last {
        let kind = cursor.u16()?;
        let length = cursor.u16()?;
        let body = cursor.at;
        if body + length > last {
            return None;
        }
        if kind == 0 {
            let mut inner = Cursor {
                data: &record[..body + length],
                at: body,
            };
            inner.u16()?;
            if inner.u8()? != 0 {
                return None;
            }
            let size = inner.u16()?;
            let start = inner.at;
            inner.skip(size)?;
            return (size > 0).then_some(start..start + size);
        }
        cursor.skip(length)?;
    }
    None
}

/// Returns the host name of the server name extension in a ClientHello
/// record, in lower case.
pub fn server_name_str(record: &[u8]) -> Option<String> {
    let range = server_name(record)?;
    let name = std::str::from_utf8(&record[range]).ok()?;
    Some(name.to_ascii_lowercase())
}

struct Cursor<'a> {
    data: &'a [u8],
    at: usize,
}

impl Cursor<'_> {
    fn skip(&mut self, count: usize) -> Option<()> {
        let next = self.at.checked_add(count)?;
        (next <= self.data.len()).then(|| self.at = next)
    }

    fn u8(&mut self) -> Option<u8> {
        let value = *self.data.get(self.at)?;
        self.at += 1;
        Some(value)
    }

    fn u16(&mut self) -> Option<usize> {
        let bytes = self.data.get(self.at..self.at + 2)?;
        self.at += 2;
        Some(usize::from(u16::from_be_bytes([bytes[0], bytes[1]])))
    }
}
