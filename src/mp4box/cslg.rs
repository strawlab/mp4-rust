use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use serde::Serialize;
use std::convert::TryFrom;
use std::io::{Read, Seek, Write};

use crate::mp4box::*;

/// CompositionToDecodeBox ('cslg'), ISO/IEC 14496-12 §8.6.1.4.
///
/// Relates a track's composition and decode timelines; present (per
/// ISO/IEC 14496-12:2012/Amd.3:2015) when the track's CompositionOffsetBox
/// ('ctts') uses negative sample_offset values. All five fields are
/// signed int(32) in version 0 and signed int(64) in version 1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CslgBox {
    pub version: u8,
    pub flags: u32,
    pub composition_to_dts_shift: i64,
    pub least_decode_to_display_delta: i64,
    pub greatest_decode_to_display_delta: i64,
    pub composition_start_time: i64,
    pub composition_end_time: i64,
}

impl CslgBox {
    pub fn get_type(&self) -> BoxType {
        BoxType::CslgBox
    }

    pub fn get_size(&self) -> u64 {
        let field_size = if self.version == 1 { 8 } else { 4 };
        HEADER_SIZE + HEADER_EXT_SIZE + 5 * field_size
    }
}

impl Default for CslgBox {
    fn default() -> Self {
        CslgBox {
            version: 0,
            flags: 0,
            composition_to_dts_shift: 0,
            least_decode_to_display_delta: 0,
            greatest_decode_to_display_delta: 0,
            composition_start_time: 0,
            composition_end_time: 0,
        }
    }
}

impl Mp4Box for CslgBox {
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
        let s = format!(
            "composition_to_dts_shift={} least_decode_to_display_delta={} greatest_decode_to_display_delta={} composition_start_time={} composition_end_time={}",
            self.composition_to_dts_shift,
            self.least_decode_to_display_delta,
            self.greatest_decode_to_display_delta,
            self.composition_start_time,
            self.composition_end_time,
        );
        Ok(s)
    }
}

impl<R: Read + Seek> ReadBox<&mut R> for CslgBox {
    fn read_box(reader: &mut R, size: u64) -> Result<Self> {
        let start = box_start(reader)?;

        let (version, flags) = read_box_header_ext(reader)?;

        let (
            composition_to_dts_shift,
            least_decode_to_display_delta,
            greatest_decode_to_display_delta,
            composition_start_time,
            composition_end_time,
        ) = if version == 1 {
            (
                reader.read_i64::<BigEndian>()?,
                reader.read_i64::<BigEndian>()?,
                reader.read_i64::<BigEndian>()?,
                reader.read_i64::<BigEndian>()?,
                reader.read_i64::<BigEndian>()?,
            )
        } else if version == 0 {
            (
                reader.read_i32::<BigEndian>()? as i64,
                reader.read_i32::<BigEndian>()? as i64,
                reader.read_i32::<BigEndian>()? as i64,
                reader.read_i32::<BigEndian>()? as i64,
                reader.read_i32::<BigEndian>()? as i64,
            )
        } else {
            return Err(Error::InvalidData("cslg version must be 0 or 1"));
        };

        skip_bytes_to(reader, start + size)?;

        Ok(CslgBox {
            version,
            flags,
            composition_to_dts_shift,
            least_decode_to_display_delta,
            greatest_decode_to_display_delta,
            composition_start_time,
            composition_end_time,
        })
    }
}

impl<W: Write> WriteBox<&mut W> for CslgBox {
    fn write_box(&self, writer: &mut W) -> Result<u64> {
        let size = self.box_size();
        BoxHeader::new(self.box_type(), size).write(writer)?;

        write_box_header_ext(writer, self.version, self.flags)?;

        if self.version == 1 {
            writer.write_i64::<BigEndian>(self.composition_to_dts_shift)?;
            writer.write_i64::<BigEndian>(self.least_decode_to_display_delta)?;
            writer.write_i64::<BigEndian>(self.greatest_decode_to_display_delta)?;
            writer.write_i64::<BigEndian>(self.composition_start_time)?;
            writer.write_i64::<BigEndian>(self.composition_end_time)?;
        } else if self.version == 0 {
            for value in [
                self.composition_to_dts_shift,
                self.least_decode_to_display_delta,
                self.greatest_decode_to_display_delta,
                self.composition_start_time,
                self.composition_end_time,
            ] {
                let value = i32::try_from(value).map_err(|_| {
                    Error::InvalidData("cslg version 0 field does not fit in i32; use version 1")
                })?;
                writer.write_i32::<BigEndian>(value)?;
            }
        } else {
            return Err(Error::InvalidData("cslg version must be 0 or 1"));
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
    fn test_cslg32() {
        let src_box = CslgBox {
            version: 0,
            flags: 0,
            composition_to_dts_shift: 512,
            least_decode_to_display_delta: -1024,
            greatest_decode_to_display_delta: 3072,
            composition_start_time: 0,
            composition_end_time: 90000,
        };
        let mut buf = Vec::new();
        src_box.write_box(&mut buf).unwrap();
        assert_eq!(buf.len(), src_box.box_size() as usize);

        let mut reader = Cursor::new(&buf);
        let header = BoxHeader::read(&mut reader).unwrap();
        assert_eq!(header.name, BoxType::CslgBox);
        assert_eq!(src_box.box_size(), header.size);

        let dst_box = CslgBox::read_box(&mut reader, header.size).unwrap();
        assert_eq!(src_box, dst_box);
    }

    #[test]
    fn test_cslg64() {
        let src_box = CslgBox {
            version: 1,
            flags: 0,
            composition_to_dts_shift: i64::from(i32::MAX) + 1000,
            least_decode_to_display_delta: -(i64::from(i32::MAX) + 1000),
            greatest_decode_to_display_delta: 3072,
            composition_start_time: 0,
            composition_end_time: 90000,
        };
        let mut buf = Vec::new();
        src_box.write_box(&mut buf).unwrap();
        assert_eq!(buf.len(), src_box.box_size() as usize);

        let mut reader = Cursor::new(&buf);
        let header = BoxHeader::read(&mut reader).unwrap();
        assert_eq!(header.name, BoxType::CslgBox);
        assert_eq!(src_box.box_size(), header.size);

        let dst_box = CslgBox::read_box(&mut reader, header.size).unwrap();
        assert_eq!(src_box, dst_box);
    }

    #[test]
    fn test_cslg_version0_overflow_errors() {
        let src_box = CslgBox {
            version: 0,
            flags: 0,
            composition_to_dts_shift: i64::from(i32::MAX) + 1,
            least_decode_to_display_delta: 0,
            greatest_decode_to_display_delta: 0,
            composition_start_time: 0,
            composition_end_time: 0,
        };
        let mut buf = Vec::new();
        assert!(src_box.write_box(&mut buf).is_err());
    }
}
