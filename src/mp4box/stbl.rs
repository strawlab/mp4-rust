use serde::Serialize;
use std::io::{Read, Seek, Write};

use crate::mp4box::*;
use crate::mp4box::{
    co64::Co64Box, cslg::CslgBox, ctts::CttsBox, stco::StcoBox, stsc::StscBox, stsd::StsdBox,
    stss::StssBox, stsz::StszBox, stts::SttsBox,
};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct StblBox {
    pub stsd: StsdBox,
    pub stts: SttsBox,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub ctts: Option<CttsBox>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub cslg: Option<CslgBox>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub stss: Option<StssBox>,
    pub stsc: StscBox,
    pub stsz: StszBox,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub stco: Option<StcoBox>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub co64: Option<Co64Box>,
}

impl StblBox {
    pub fn get_type(&self) -> BoxType {
        BoxType::StblBox
    }

    pub fn get_size(&self) -> u64 {
        let mut size = HEADER_SIZE;
        size += self.stsd.box_size();
        size += self.stts.box_size();
        if let Some(ref ctts) = self.ctts {
            size += ctts.box_size();
        }
        if let Some(ref cslg) = self.cslg {
            size += cslg.box_size();
        }
        if let Some(ref stss) = self.stss {
            size += stss.box_size();
        }
        size += self.stsc.box_size();
        size += self.stsz.box_size();
        if let Some(ref stco) = self.stco {
            size += stco.box_size();
        }
        if let Some(ref co64) = self.co64 {
            size += co64.box_size();
        }
        size
    }
}

impl Mp4Box for StblBox {
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
        let s = String::new();
        Ok(s)
    }
}

impl<R: Read + Seek> ReadBox<&mut R> for StblBox {
    fn read_box(reader: &mut R, size: u64) -> Result<Self> {
        let start = box_start(reader)?;

        let mut stsd = None;
        let mut stts = None;
        let mut ctts = None;
        let mut cslg = None;
        let mut stss = None;
        let mut stsc = None;
        let mut stsz = None;
        let mut stco = None;
        let mut co64 = None;

        let mut current = reader.stream_position()?;
        let end = start + size;
        while current < end {
            // Get box header.
            let header = BoxHeader::read(reader)?;
            let BoxHeader { name, size: s } = header;
            if s > size {
                return Err(Error::InvalidData(
                    "stbl box contains a box with a larger size than it",
                ));
            }

            match name {
                BoxType::StsdBox => {
                    stsd = Some(StsdBox::read_box(reader, s)?);
                }
                BoxType::SttsBox => {
                    stts = Some(SttsBox::read_box(reader, s)?);
                }
                BoxType::CttsBox => {
                    ctts = Some(CttsBox::read_box(reader, s)?);
                }
                BoxType::CslgBox => {
                    cslg = Some(CslgBox::read_box(reader, s)?);
                }
                BoxType::StssBox => {
                    stss = Some(StssBox::read_box(reader, s)?);
                }
                BoxType::StscBox => {
                    stsc = Some(StscBox::read_box(reader, s)?);
                }
                BoxType::StszBox => {
                    stsz = Some(StszBox::read_box(reader, s)?);
                }
                BoxType::StcoBox => {
                    stco = Some(StcoBox::read_box(reader, s)?);
                }
                BoxType::Co64Box => {
                    co64 = Some(Co64Box::read_box(reader, s)?);
                }
                _ => {
                    // XXX warn!()
                    skip_box(reader, s)?;
                }
            }
            current = reader.stream_position()?;
        }

        if stsd.is_none() {
            return Err(Error::BoxNotFound(BoxType::StsdBox));
        }
        if stts.is_none() {
            return Err(Error::BoxNotFound(BoxType::SttsBox));
        }
        if stsc.is_none() {
            return Err(Error::BoxNotFound(BoxType::StscBox));
        }
        if stsz.is_none() {
            return Err(Error::BoxNotFound(BoxType::StszBox));
        }
        if stco.is_none() && co64.is_none() {
            return Err(Error::Box2NotFound(BoxType::StcoBox, BoxType::Co64Box));
        }

        skip_bytes_to(reader, start + size)?;

        Ok(StblBox {
            stsd: stsd.unwrap(),
            stts: stts.unwrap(),
            ctts,
            cslg,
            stss,
            stsc: stsc.unwrap(),
            stsz: stsz.unwrap(),
            stco,
            co64,
        })
    }
}

impl<W: Write> WriteBox<&mut W> for StblBox {
    fn write_box(&self, writer: &mut W) -> Result<u64> {
        let size = self.box_size();
        BoxHeader::new(self.box_type(), size).write(writer)?;

        self.stsd.write_box(writer)?;
        self.stts.write_box(writer)?;
        if let Some(ref ctts) = self.ctts {
            ctts.write_box(writer)?;
        }
        if let Some(ref cslg) = self.cslg {
            cslg.write_box(writer)?;
        }
        if let Some(ref stss) = self.stss {
            stss.write_box(writer)?;
        }
        self.stsc.write_box(writer)?;
        self.stsz.write_box(writer)?;
        if let Some(ref stco) = self.stco {
            stco.write_box(writer)?;
        }
        if let Some(ref co64) = self.co64 {
            co64.write_box(writer)?;
        }

        Ok(size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mp4box::ctts::CttsEntry;
    use std::io::Cursor;

    #[test]
    fn test_stbl_with_ctts_and_cslg() {
        // A B-frame track: negative ctts offsets alongside the cslg box
        // that the 2015 amendment recommends accompany them.
        let src_box = StblBox {
            stsd: StsdBox::default(),
            stts: SttsBox::default(),
            ctts: Some(CttsBox {
                version: 0,
                flags: 0,
                entries: vec![
                    CttsEntry {
                        sample_count: 1,
                        sample_offset: 0,
                    },
                    CttsEntry {
                        sample_count: 1,
                        sample_offset: -2000,
                    },
                    CttsEntry {
                        sample_count: 1,
                        sample_offset: 3000,
                    },
                ],
            }),
            cslg: Some(CslgBox {
                version: 0,
                flags: 0,
                composition_to_dts_shift: 2000,
                least_decode_to_display_delta: -2000,
                greatest_decode_to_display_delta: 3000,
                composition_start_time: 0,
                composition_end_time: 5000,
            }),
            stss: None,
            stsc: StscBox::default(),
            stsz: StszBox::default(),
            stco: Some(StcoBox::default()),
            co64: None,
        };

        let mut buf = Vec::new();
        src_box.write_box(&mut buf).unwrap();
        assert_eq!(buf.len(), src_box.box_size() as usize);

        let mut reader = Cursor::new(&buf);
        let header = BoxHeader::read(&mut reader).unwrap();
        assert_eq!(header.name, BoxType::StblBox);
        assert_eq!(src_box.box_size(), header.size);

        let dst_box = StblBox::read_box(&mut reader, header.size).unwrap();
        // ctts is written as version 1 because it has a negative entry
        // (see CttsBox::computed_version); read_box reflects that back.
        assert_eq!(dst_box.ctts.as_ref().unwrap().version, 1);
        assert_eq!(dst_box.ctts.unwrap().entries, src_box.ctts.unwrap().entries);
        assert_eq!(dst_box.cslg, src_box.cslg);
        assert_eq!(dst_box.cslg.unwrap().version, 0);
    }
}
