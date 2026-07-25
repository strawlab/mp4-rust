use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use serde::Serialize;
use std::io::{Read, Seek, Write};
use std::mem::size_of;

use crate::mp4box::*;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct ElstBox {
    /// Version as parsed from the box header. Only meaningful for boxes
    /// obtained via `read_box`; ignored when writing, since the version
    /// actually written is derived from `entries` (see `computed_version`).
    pub version: u8,
    pub flags: u32,

    #[serde(skip_serializing)]
    pub entries: Vec<ElstEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct ElstEntry {
    /// Unsigned in both versions: `int(32)` at version 0, `int(64)` at
    /// version 1.
    pub segment_duration: u64,
    /// Signed in both versions: `int(32)` at version 0, `int(64)` at version
    /// 1. A value of -1 denotes an empty edit.
    pub media_time: i64,
    pub media_rate: u16,
    pub media_rate_fraction: u16,
}

impl ElstBox {
    pub fn get_type(&self) -> BoxType {
        BoxType::ElstBox
    }

    pub fn get_size(&self) -> u64 {
        let mut size = HEADER_SIZE + HEADER_EXT_SIZE + 4;
        if self.computed_version() == 1 {
            size += self.entries.len() as u64 * 20;
        } else {
            size += self.entries.len() as u64 * 12;
        }
        size
    }

    /// ISO/IEC 14496-12 §8.6.6.2: `segment_duration` and `media_time` are
    /// `int(32)`/`int(64)` at version 0/1 respectively. A value that doesn't
    /// fit in 32 bits can only be represented in version 1, so derive the
    /// version to write from the entries rather than trusting a stored value
    /// that may predate such an entry being added.
    fn computed_version(&self) -> u8 {
        let needs_v1 = self.entries.iter().any(|entry| {
            entry.segment_duration > u32::MAX as u64
                || entry.media_time > i32::MAX as i64
                || entry.media_time < i32::MIN as i64
        });
        if needs_v1 {
            1
        } else {
            0
        }
    }
}

impl Mp4Box for ElstBox {
    fn box_type(&self) -> BoxType {
        self.get_type()
    }

    fn box_size(&self) -> u64 {
        self.get_size()
    }

    fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string(&self).unwrap())
    }

    fn summary(&self) -> Result<String> {
        let s = format!("elst_entries={}", self.entries.len());
        Ok(s)
    }
}

impl<R: Read + Seek> ReadBox<&mut R> for ElstBox {
    fn read_box(reader: &mut R, size: u64) -> Result<Self> {
        let start = box_start(reader)?;

        let (version, flags) = read_box_header_ext(reader)?;

        let header_size = HEADER_SIZE + HEADER_EXT_SIZE;
        let entry_count = reader.read_u32::<BigEndian>()?;
        let other_size = size_of::<i32>(); // entry_count
        let entry_size = {
            let mut entry_size = 0;
            entry_size += if version == 1 {
                size_of::<u64>() + size_of::<i64>() // segment_duration + media_time
            } else {
                size_of::<u32>() + size_of::<i32>() // segment_duration + media_time
            };
            entry_size += size_of::<i16>() + size_of::<i16>(); // media_rate_integer + media_rate_fraction
            entry_size
        };
        if u64::from(entry_count)
            > size
                .saturating_sub(header_size)
                .saturating_sub(other_size as u64)
                / entry_size as u64
        {
            return Err(Error::InvalidData(
                "elst entry_count indicates more entries than could fit in the box",
            ));
        }
        let mut entries = Vec::with_capacity(entry_count as usize);
        for _ in 0..entry_count {
            let (segment_duration, media_time) = if version == 1 {
                (
                    reader.read_u64::<BigEndian>()?,
                    reader.read_i64::<BigEndian>()?,
                )
            } else {
                (
                    reader.read_u32::<BigEndian>()? as u64,
                    reader.read_i32::<BigEndian>()? as i64,
                )
            };

            let entry = ElstEntry {
                segment_duration,
                media_time,
                media_rate: reader.read_u16::<BigEndian>()?,
                media_rate_fraction: reader.read_u16::<BigEndian>()?,
            };
            entries.push(entry);
        }

        skip_bytes_to(reader, start + size)?;

        Ok(ElstBox {
            version,
            flags,
            entries,
        })
    }
}

impl<W: Write> WriteBox<&mut W> for ElstBox {
    fn write_box(&self, writer: &mut W) -> Result<u64> {
        let size = self.box_size();
        BoxHeader::new(self.box_type(), size).write(writer)?;

        let version = self.computed_version();
        write_box_header_ext(writer, version, self.flags)?;

        writer.write_u32::<BigEndian>(self.entries.len() as u32)?;
        for entry in self.entries.iter() {
            if version == 1 {
                writer.write_u64::<BigEndian>(entry.segment_duration)?;
                writer.write_i64::<BigEndian>(entry.media_time)?;
            } else {
                writer.write_u32::<BigEndian>(entry.segment_duration as u32)?;
                writer.write_i32::<BigEndian>(entry.media_time as i32)?;
            }
            writer.write_u16::<BigEndian>(entry.media_rate)?;
            writer.write_u16::<BigEndian>(entry.media_rate_fraction)?;
        }

        Ok(size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mp4box::BoxHeader;
    use std::io::Cursor;

    #[test]
    fn test_elst32() {
        let src_box = ElstBox {
            version: 0,
            flags: 0,
            entries: vec![ElstEntry {
                segment_duration: 634634,
                media_time: 0,
                media_rate: 1,
                media_rate_fraction: 0,
            }],
        };
        let mut buf = Vec::new();
        src_box.write_box(&mut buf).unwrap();
        assert_eq!(buf.len(), src_box.box_size() as usize);

        let mut reader = Cursor::new(&buf);
        let header = BoxHeader::read(&mut reader).unwrap();
        assert_eq!(header.name, BoxType::ElstBox);
        assert_eq!(src_box.box_size(), header.size);

        let dst_box = ElstBox::read_box(&mut reader, header.size).unwrap();
        assert_eq!(src_box, dst_box);
    }

    #[test]
    fn test_elst64() {
        // media_time beyond i32::MAX can only be represented in version 1;
        // computed_version() must pick it regardless of the stored version.
        let src_box = ElstBox {
            version: 1,
            flags: 0,
            entries: vec![ElstEntry {
                segment_duration: 634634,
                media_time: i32::MAX as i64 + 1,
                media_rate: 1,
                media_rate_fraction: 0,
            }],
        };
        let mut buf = Vec::new();
        src_box.write_box(&mut buf).unwrap();
        assert_eq!(buf.len(), src_box.box_size() as usize);
        assert_eq!(
            buf[8], 1,
            "version byte must be 1 when media_time overflows i32"
        );

        let mut reader = Cursor::new(&buf);
        let header = BoxHeader::read(&mut reader).unwrap();
        assert_eq!(header.name, BoxType::ElstBox);
        assert_eq!(src_box.box_size(), header.size);

        let dst_box = ElstBox::read_box(&mut reader, header.size).unwrap();
        assert_eq!(dst_box.version, 1);
        assert_eq!(dst_box.entries, src_box.entries);
    }

    #[test]
    fn test_elst_negative_media_time_empty_edit() {
        // -1 denotes an empty edit and must survive round-tripping even at
        // version 0, where media_time is signed int(32).
        let src_box = ElstBox {
            version: 0,
            flags: 0,
            entries: vec![ElstEntry {
                segment_duration: 1000,
                media_time: -1,
                media_rate: 1,
                media_rate_fraction: 0,
            }],
        };
        let mut buf = Vec::new();
        src_box.write_box(&mut buf).unwrap();
        assert_eq!(buf.len(), src_box.box_size() as usize);
        assert_eq!(buf[8], 0, "media_time=-1 fits in i32, so version stays 0");

        let mut reader = Cursor::new(&buf);
        let header = BoxHeader::read(&mut reader).unwrap();
        assert_eq!(src_box.box_size(), header.size);

        let dst_box = ElstBox::read_box(&mut reader, header.size).unwrap();
        assert_eq!(dst_box.entries[0].media_time, -1);
        assert_eq!(dst_box.entries, src_box.entries);
    }

    #[test]
    fn test_elst_version_forced_by_large_segment_duration() {
        // A segment_duration beyond u32::MAX can only be represented in
        // version 1, regardless of the version stored on the struct.
        let src_box = ElstBox {
            version: 0,
            flags: 0,
            entries: vec![ElstEntry {
                segment_duration: u32::MAX as u64 + 1,
                media_time: 1024,
                media_rate: 1,
                media_rate_fraction: 0,
            }],
        };
        let mut buf = Vec::new();
        src_box.write_box(&mut buf).unwrap();
        assert_eq!(buf.len(), src_box.box_size() as usize);
        assert_eq!(
            buf[8], 1,
            "version byte must be 1 when segment_duration overflows u32"
        );

        let mut reader = Cursor::new(&buf);
        let header = BoxHeader::read(&mut reader).unwrap();
        assert_eq!(src_box.box_size(), header.size);

        let dst_box = ElstBox::read_box(&mut reader, header.size).unwrap();
        assert_eq!(dst_box.version, 1);
        assert_eq!(dst_box.entries, src_box.entries);
    }
}
