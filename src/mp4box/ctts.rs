use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use serde::Serialize;
use std::convert::TryFrom;
use std::io::{Read, Seek, Write};
use std::mem::size_of;

use crate::mp4box::*;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct CttsBox {
    /// Version as parsed from the box header. Only meaningful for boxes
    /// obtained via `read_box`; ignored when writing, since the version
    /// actually written is derived from `entries` (see `computed_version`).
    pub version: u8,
    pub flags: u32,

    #[serde(skip_serializing)]
    pub entries: Vec<CttsEntry>,
}

impl CttsBox {
    pub fn get_type(&self) -> BoxType {
        BoxType::CttsBox
    }

    pub fn get_size(&self) -> u64 {
        HEADER_SIZE + HEADER_EXT_SIZE + 4 + (8 * self.entries.len() as u64)
    }

    /// ISO/IEC 14496-12 §8.6.1.3: `sample_offset` is unsigned in version 0
    /// and signed in version 1. A negative entry can only be represented in
    /// version 1, so derive the version to write from the entries rather
    /// than trusting a stored value that may predate a negative entry being
    /// added.
    fn computed_version(&self) -> u8 {
        if self.entries.iter().any(|entry| entry.sample_offset < 0) {
            1
        } else {
            0
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct CttsEntry {
    pub sample_count: u32,
    pub sample_offset: i32,
}

impl Mp4Box for CttsBox {
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
        let s = format!("entries_count={}", self.entries.len());
        Ok(s)
    }
}

impl<R: Read + Seek> ReadBox<&mut R> for CttsBox {
    fn read_box(reader: &mut R, size: u64) -> Result<Self> {
        let start = box_start(reader)?;

        let (version, flags) = read_box_header_ext(reader)?;

        let header_size = HEADER_SIZE + HEADER_EXT_SIZE;
        let entry_count = reader.read_u32::<BigEndian>()?;
        let entry_size = size_of::<u32>() + size_of::<i32>(); // sample_count + sample_offset
                                                              // (sample_offset might be a u32, but the size is the same.)
        let other_size = size_of::<i32>(); // entry_count
        if u64::from(entry_count)
            > size
                .saturating_sub(header_size)
                .saturating_sub(other_size as u64)
                / entry_size as u64
        {
            return Err(Error::InvalidData(
                "ctts entry_count indicates more entries than could fit in the box",
            ));
        }
        let mut entries = Vec::with_capacity(entry_count as usize);
        for _ in 0..entry_count {
            let sample_count = reader.read_u32::<BigEndian>()?;
            // Version 0: sample_offset is unsigned int(32). Version 1:
            // sample_offset is signed int(32). Reading a version 0 value
            // with read_i32 would silently turn offsets >= 2^31 negative.
            let sample_offset = if version == 0 {
                let raw = reader.read_u32::<BigEndian>()?;
                i32::try_from(raw).map_err(|_| {
                    Error::InvalidData(
                        "ctts version 0 sample_offset exceeds i32::MAX and cannot be represented",
                    )
                })?
            } else {
                reader.read_i32::<BigEndian>()?
            };
            entries.push(CttsEntry {
                sample_count,
                sample_offset,
            });
        }

        skip_bytes_to(reader, start + size)?;

        Ok(CttsBox {
            version,
            flags,
            entries,
        })
    }
}

impl<W: Write> WriteBox<&mut W> for CttsBox {
    fn write_box(&self, writer: &mut W) -> Result<u64> {
        let size = self.box_size();
        BoxHeader::new(self.box_type(), size).write(writer)?;

        write_box_header_ext(writer, self.computed_version(), self.flags)?;

        writer.write_u32::<BigEndian>(self.entries.len() as u32)?;
        for entry in self.entries.iter() {
            writer.write_u32::<BigEndian>(entry.sample_count)?;
            writer.write_i32::<BigEndian>(entry.sample_offset)?;
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
    fn test_ctts_version0_non_negative() {
        // All offsets non-negative: version should be written as 0, and the
        // on-disk bytes must be identical to what the library has always
        // produced (a plain u32 entry_count followed by (u32, i32) entries
        // with version/flags header word 0x0000_0000).
        let src_box = CttsBox {
            version: 0,
            flags: 0,
            entries: vec![
                CttsEntry {
                    sample_count: 1,
                    sample_offset: 200,
                },
                CttsEntry {
                    sample_count: 2,
                    sample_offset: 0,
                },
            ],
        };
        let mut buf = Vec::new();
        src_box.write_box(&mut buf).unwrap();
        assert_eq!(buf.len(), src_box.box_size() as usize);

        let expected: &[u8] = &[
            0x00, 0x00, 0x00, 0x00, // version(0) + flags(0)
            0x00, 0x00, 0x00, 0x02, // entry_count
            0x00, 0x00, 0x00, 0x01, // sample_count
            0x00, 0x00, 0x00, 0xC8, // sample_offset = 200
            0x00, 0x00, 0x00, 0x02, // sample_count
            0x00, 0x00, 0x00, 0x00, // sample_offset = 0
        ];
        // Body starts right after the 8-byte box header (size + type).
        assert_eq!(&buf[8..], expected);

        let mut reader = Cursor::new(&buf);
        let header = BoxHeader::read(&mut reader).unwrap();
        assert_eq!(header.name, BoxType::CttsBox);
        assert_eq!(src_box.box_size(), header.size);

        let dst_box = CttsBox::read_box(&mut reader, header.size).unwrap();
        assert_eq!(dst_box.version, 0);
        assert_eq!(src_box, dst_box);
    }

    #[test]
    fn test_ctts_version1_negative_offset() {
        // A negative offset forces version 1 on write, regardless of the
        // version stored on the struct.
        let src_box = CttsBox {
            version: 0,
            flags: 0,
            entries: vec![
                CttsEntry {
                    sample_count: 1,
                    sample_offset: 200,
                },
                CttsEntry {
                    sample_count: 2,
                    sample_offset: -100,
                },
            ],
        };
        let mut buf = Vec::new();
        src_box.write_box(&mut buf).unwrap();
        assert_eq!(buf.len(), src_box.box_size() as usize);
        assert_eq!(buf[8], 1, "version byte must be 1 when any offset is negative");

        let mut reader = Cursor::new(&buf);
        let header = BoxHeader::read(&mut reader).unwrap();
        assert_eq!(header.name, BoxType::CttsBox);
        assert_eq!(src_box.box_size(), header.size);

        let dst_box = CttsBox::read_box(&mut reader, header.size).unwrap();
        assert_eq!(dst_box.version, 1);
        assert_eq!(dst_box.entries, src_box.entries);
    }

    #[test]
    fn test_ctts_version0_offset_beyond_i32_max_errors() {
        // Version 0 offsets are unsigned; a value above i32::MAX but within
        // u32 range is legal and must not be misread as negative.
        let raw_offset: u32 = i32::MAX as u32 + 1000;
        let mut buf = Vec::new();
        buf.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // version(0) + flags
        buf.extend_from_slice(&1u32.to_be_bytes()); // entry_count
        buf.extend_from_slice(&1u32.to_be_bytes()); // sample_count
        buf.extend_from_slice(&raw_offset.to_be_bytes()); // sample_offset (unsigned)

        let mut full = Vec::new();
        let size = HEADER_SIZE + buf.len() as u64;
        BoxHeader::new(BoxType::CttsBox, size).write(&mut full).unwrap();
        full.extend_from_slice(&buf);

        let mut reader = Cursor::new(&full);
        let header = BoxHeader::read(&mut reader).unwrap();
        let result = CttsBox::read_box(&mut reader, header.size);
        assert!(
            result.is_err(),
            "a version-0 offset exceeding i32::MAX cannot be represented by this library's i32 field and must error, not silently wrap negative"
        );
    }
}
