// #[derive(Copy, Clone)]
// pub enum Command {
//     Generic,
//     UvcUac,
//     Time,
// }
//
// pub enum GenericCommand {
//
// }

use std::fmt::Debug;
use zerocopy::{big_endian, little_endian, FromBytes, Immutable, IntoBytes, KnownLayout};

#[repr(C)]
#[derive(Debug, FromBytes, IntoBytes, KnownLayout, Immutable)]
pub struct CommandHeader {
    pub packet_type: little_endian::U16,
    pub query_type: little_endian::U16,
    pub payload_size: little_endian::U16,
    pub seq_id: little_endian::U16,
}

#[repr(C)]
#[derive(Debug, FromBytes, IntoBytes, KnownLayout, Immutable)]
pub struct CommandPacket {
    pub header: CommandHeader,
    pub payload: [u8]
}

pub trait Payload {
    type Response: FromBytes + KnownLayout + Immutable;
    const QUERY_TYPE: u16;

    fn payload_size(&self) -> usize;
    fn write_payload(&self, buffer: &mut [u8]);

    fn packet_size(&self) -> usize {
        size_of::<CommandHeader>() + self.payload_size()
    }

    fn write_packet(&self, seq_id: u16, buffer: &mut [u8]) {
        let packet = CommandPacket::mut_from_bytes(buffer).expect("buffer size");
        packet.header = CommandHeader {
            packet_type: 0.into(),
            query_type: Self::QUERY_TYPE.into(),
            payload_size: (self.payload_size() as u16).into(),
            seq_id: seq_id.into(),
        };
        self.write_payload(&mut packet.payload);
    }
}

#[repr(C)]
#[derive(Debug, FromBytes, IntoBytes, KnownLayout, Immutable)]
pub struct UvcUacPayload { // TODO: verify endianness of camera stuff
    pub f1: u8,
    pub unknown0: u8,
    pub unknown1: u8,
    pub f3: u8,
    pub mic_enable: u8,
    pub mic_mute: u8,
    pub mic_volume: big_endian::I16,
    pub mic_jack_volume: big_endian::I16,
    pub unknown2: u8,
    pub unknown3: u8,
    pub mic_freq: big_endian::U16,
    pub cam_enable: u8,
    pub cam_power: u8,
    pub cam_power_freq: u8,
    pub cam_auto_expo: u8,
    pub cam_expo_absolute: big_endian::U32,
    pub cam_brightness: big_endian::U16,
    pub cam_contrast: big_endian::U16,
    pub cam_gain: big_endian::U16,
    pub cam_hue: big_endian::U16,
    pub cam_saturation: big_endian::U16,
    pub cam_sharpness: big_endian::U16,
    pub cam_gamma: big_endian::U16,
    pub cam_key_frame: u8,
    pub cam_white_balance_auto: u8,
    pub cam_white_balance: big_endian::U32,
    pub cam_multiplier: big_endian::U16,
    pub cam_multiplier_limit: big_endian::U16,
    pub unknown4: u8,
    pub unknown5: u8,
}

#[repr(C)]
#[derive(Debug, FromBytes, IntoBytes, KnownLayout, Immutable)]
pub struct UvcUacResponse {
    pub mic_volume: little_endian::I16,
    pub mic_jack_volume: little_endian::I16,
    pub unknown0: [u8; 8],
    pub mic_enabled: u8,
    pub cam_power_freq:u8,
    pub cam_auto_expo: u8,
    pub unknown1: u8,
}

impl Default for UvcUacPayload {
    fn default() -> Self {
        Self {
            f1: 0,
            unknown0: 0,
            unknown1: 0,
            f3: 0,
            mic_enable: 0,
            mic_mute: 0,
            mic_volume: Default::default(),
            mic_jack_volume: Default::default(),
            unknown2: 0,
            unknown3: 0,
            mic_freq: 16000.into(),
            cam_enable: 0,
            cam_power: 0,
            cam_power_freq: 0,
            cam_auto_expo: 0,
            cam_expo_absolute: Default::default(),
            cam_brightness: Default::default(),
            cam_contrast: Default::default(),
            cam_gain: Default::default(),
            cam_hue: Default::default(),
            cam_saturation: Default::default(),
            cam_sharpness: Default::default(),
            cam_gamma: Default::default(),
            cam_key_frame: 0,
            cam_white_balance_auto: 0,
            cam_white_balance: Default::default(),
            cam_multiplier: Default::default(),
            cam_multiplier_limit: Default::default(),
            unknown4: 0,
            unknown5: 0,
        }
    }
}

impl Payload for UvcUacPayload {
    type Response = UvcUacResponse;
    const QUERY_TYPE: u16 = 1;

    fn payload_size(&self) -> usize {
        size_of::<Self>()
    }

    fn write_payload(&self, buffer: &mut [u8]) {
        self.write_to(buffer).expect("buffer size");
    }
}
