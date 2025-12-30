use std::io::Write;
use crate::video::data::Error::{ExtHeaderLength, ExtHeaderValue};
use bitfld::layout;
use snafu::{OptionExt, ResultExt, Snafu, Whatever, ensure};
use zerocopy_derive::FromBytes;
use zerocopy_derive::IntoBytes;

layout!({
    #[derive(IntoBytes, FromBytes)]
    pub struct BadVstrmHeader(u64);
    {
        let magic: Bits<63, 60>;
        let packet_type: Bits<59, 58>;
        let seq_id: Bits<57, 48>;
        let init: Bit<47>;
        let frame_begin: Bit<46>;
        let chunk_end: Bit<45>;
        let frame_end: Bit<44>;
        let has_timestamp: Bit<43>;
        let payload_size: Bits<42, 32>;
    }
});

#[derive(Debug, Clone)]
pub struct VstrmHeader {
    pub magic: u8,
    pub packet_type: u8,
    pub seq_id: u16,
    pub init: bool,
    pub frame_begin: bool,
    pub chunk_end: bool,
    pub frame_end: bool,
    pub has_timestamp: bool,
    pub payload_size: u16,
    pub timestamp: u32,
    pub ext_headers: Vec<ExtOption>,
}

impl VstrmHeader {
    pub const SIZE: usize = 16;

    pub fn into_bytes(self) -> Result<[u8; Self::SIZE], Error> {
        let mut buffer = [0u8; 16];
        buffer[0] = self.magic << 4;
        buffer[0] |= (self.packet_type << 2) & 0b1100;
        buffer[0] |= (self.seq_id >> 8) as u8 & 0b11;  // 4 bit magic (F), 2 bit packet type (0), 2 bit seqid (0)
        buffer[1] = self.seq_id as u8;
        buffer[2] = (self.init as u8) << 7;
        buffer[2] |= (self.frame_begin as u8) << 6;
        buffer[2] |= (self.chunk_end as u8) << 5;
        buffer[2] |= (self.frame_end as u8) << 4;
        buffer[2] |= (self.has_timestamp as u8) << 3;
        let payload_size = self.payload_size.to_be_bytes();
        buffer[2] |= (payload_size[0]) & 0b111;
        buffer[3] = payload_size[1];
        buffer[4..8].copy_from_slice(&self.timestamp.to_be_bytes());
        buffer[8..].copy_from_slice(&ExtOption::encode(&self.ext_headers)?);
        Ok(buffer)
    }
}

impl Default for VstrmHeader {
    fn default() -> Self {
        Self {
            magic: 0xF,
            packet_type: 0,
            seq_id: 0,
            init: false,
            frame_begin: false,
            chunk_end: false,
            frame_end: false,
            has_timestamp: true,
            payload_size: 0,
            timestamp: 0,
            ext_headers: vec![ExtOption::ForceDecoding, ExtOption::NumMbRowsInChunk(6)],
        }
    }
}

#[repr(u8)]
#[derive(Debug, Copy, Clone)]
pub enum ExtOption {
    Idr,
    Unimplemented(u8),
    FrameRate(FrameRate),
    ForceDecoding,
    UnsetForceFlag,
    NumMbRowsInChunk(u8),
}

impl ExtOption {
    pub fn encode(options: &[ExtOption]) -> Result<[u8; 8], Error> {
        let mut result = vec![];
        for opt in options {
            match opt {
                ExtOption::Idr => result.push(0x80),
                ExtOption::Unimplemented(v) => result.extend([0x81, *v]),
                ExtOption::FrameRate(f) => result.extend([0x82, *f as u8]),
                ExtOption::ForceDecoding => result.push(0x83),
                ExtOption::UnsetForceFlag => result.push(0x84),
                ExtOption::NumMbRowsInChunk(v) => result.extend([0x85, *v]),
            }
        }
        ensure!(
            result.len() <= 8,
            ExtHeaderTooLongSnafu {
                length: result.len()
            }
        );
        result.resize(8, 0);
        Ok(result.try_into().unwrap())
    }

    pub fn decode(value: &[u8]) -> Result<Vec<Self>, Error> {
        ensure!(
            value.len() == 8,
            ExtHeaderLengthSnafu {
                length: value.len()
            }
        );
        let mut bytes = value.iter();
        let mut options = vec![];
        while let Some(byte) = bytes.next().copied() {
            match byte {
                0 => {
                    break;
                }
                0x80 => {
                    options.push(ExtOption::Idr);
                }
                0x81 => {
                    let param = bytes.next().copied().context(ExtHeaderParamSnafu {
                        instr: "Unimplemented",
                    })?;
                    options.push(ExtOption::Unimplemented(param));
                }
                0x82 => {
                    let param = bytes
                        .next()
                        .copied()
                        .context(ExtHeaderParamSnafu { instr: "FrameRate" })?;
                    options.push(ExtOption::FrameRate(FrameRate::try_from(param)?));
                }
                0x83 => {
                    options.push(ExtOption::ForceDecoding);
                }
                0x84 => {
                    options.push(ExtOption::UnsetForceFlag);
                }
                0x85 => {
                    let param = bytes.next().copied().context(ExtHeaderParamSnafu {
                        instr: "NumMbRowsInChunk",
                    })?;
                    options.push(ExtOption::NumMbRowsInChunk(param));
                }
                value => {
                    ensure!(false, ExtHeaderValueSnafu { value });
                }
            }
        }
        Ok(options)
    }
}

#[repr(u8)]
#[derive(Debug, Copy, Clone)]
pub enum FrameRate {
    Sixty = 0,
    Fifty = 1,
    Thirty = 2,
    TwentyFive = 3,
}

impl FrameRate {
    pub const fn freq(self) -> f32 {
        match self {
            FrameRate::Sixty => 59.94,
            FrameRate::Fifty => 50.0,
            FrameRate::Thirty => 29.97,
            FrameRate::TwentyFive => 25.0,
        }
    }
}

impl TryFrom<u8> for FrameRate {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self, Error> {
        match value {
            0 => Ok(Self::Sixty),
            1 => Ok(Self::Fifty),
            2 => Ok(Self::Thirty),
            3 => Ok(Self::TwentyFive),
            value => Err(Error::InvalidFramerate { value }),
        }
    }
}

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("extended header is too long ({length} > 8)"))]
    ExtHeaderTooLong { length: usize },
    #[snafu(display("invalid value {value} for extended header option"))]
    ExtHeaderValue { value: u8 },
    #[snafu(display("invalid extended header length {length}"))]
    ExtHeaderLength { length: usize },
    #[snafu(display("expected parameter to extended header option {instr}"))]
    ExtHeaderParam { instr: &'static str },
    #[snafu(display("Invalid framerate value {value}"))]
    InvalidFramerate { value: u8 },
}
