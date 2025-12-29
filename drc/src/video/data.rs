use crate::video::data::Error::{ExtHeaderLength, ExtHeaderValue};
use bitfld::layout;
use snafu::{OptionExt, ResultExt, Snafu, Whatever, ensure};
use zerocopy_derive::FromBytes;
use zerocopy_derive::IntoBytes;

layout!({
    #[derive(IntoBytes, FromBytes)]
    pub struct VstrmHeader(u64);
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
    pub fn encode(options: &[ExtOption]) -> Result<Vec<u8>, Error> {
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
        Ok(result)
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
