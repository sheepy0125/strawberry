use zerocopy::{big_endian, FromBytes, Immutable, IntoBytes, KnownLayout, Unaligned};
use zerocopy_derive::{FromBytes, Immutable, IntoBytes, KnownLayout, Unaligned};
use crate::cmd::data::Payload;

#[repr(C)]
#[derive(Debug, FromBytes, IntoBytes, KnownLayout, Immutable, Unaligned)]
pub struct GenericCommandHeader {
    magic: u8,
    version: u8,
    id: [u8; 3],
    flags: u8,
    service_id: u8,
    method_id: u8,
    error_code: big_endian::U16,
    payload_size: big_endian::U16,
}

#[repr(C)]
#[derive(Debug, FromBytes, IntoBytes, KnownLayout, Immutable)]
pub struct GenericCommandPacket<T: ?Sized> {
    header: GenericCommandHeader,
    payload: T
}

pub trait GenericPayload {
    type Response: FromBytes + KnownLayout + Immutable + Unaligned;
    const SERVICE_ID: u8;
    const METHOD_ID: u8;

    fn generic_payload_size(&self) -> usize;
    fn write_generic_payload(&self, buffer: &mut [u8]);
}

impl<T: GenericPayload> Payload for T {
    type Response = GenericCommandPacket<<Self as GenericPayload>::Response>;
    const QUERY_TYPE: u16 = 0;

    fn payload_size(&self) -> usize {
        size_of::<GenericCommandHeader>() + self.generic_payload_size()
    }

    fn write_payload(&self, buffer: &mut [u8]) {
        let packet = GenericCommandPacket::<[u8]>::mut_from_bytes(buffer).expect("buffer size");
        packet.header = GenericCommandHeader {
            magic: 0x7E,
            version: 0x01,
            id: [0, 8, 0],  // TODO: is this right?
            flags: 0x40,
            service_id: T::SERVICE_ID,
            method_id: T::METHOD_ID,
            error_code: 0.into(),
            payload_size: (self.generic_payload_size() as u16).into()
        };
        self.write_generic_payload(&mut packet.payload);
    }
}

pub struct GetUicFirmware;

impl GenericPayload for GetUicFirmware {
    type Response = [u8; 772];
    const SERVICE_ID: u8 = 0x05;
    const METHOD_ID: u8 = 0x06;

    fn generic_payload_size(&self) -> usize {
        0
    }

    fn write_generic_payload(&self, buffer: &mut [u8]) {
        assert_eq!(buffer.len(), 0, "buffer size");
    }
}