use bitflags::bitflags;
use bitfld::layout;
use zerocopy::{big_endian, little_endian};
use zerocopy::FromBytes;

#[repr(C)]
#[derive(Debug, Copy, Clone, FromBytes)]
pub struct InputData {
    pub seq_id: big_endian::U16,
    pub buttons: Buttons,
    pub power_status: PowerStatus,
    pub battery_charge: u8,
    pub left_stick_x: little_endian::U16,
    pub left_stick_y: u16,
    pub right_stick_x: u16,
    pub right_stick_y: u16,
    pub audio_volume: u8,
    pub accelerometer: Accelerometer,
    pub gyro: Gyroscope,
    pub magnet: Magnet,
    pub touchscreen: Touchscreen,
    unk0: [u8; 4],
    pub extra_buttons: ExtraButtons,
    unk1: [u8; 46],
    pub fw_version_neg: u8,
}

#[repr(transparent)]
#[derive(Debug, Copy, Clone, FromBytes)]
pub struct Buttons(u16);

bitflags! {
    impl Buttons: u16 {
        const SYNC = 0x0001;
        const HOME = 0x0002;
        const MINUS = 0x0004;
        const PLUS = 0x0008;
        const R = 0x0010;
        const L = 0x0020;
        const ZR = 0x0040;
        const ZL = 0x0080;
        const DOWN = 0x0100;
        const UP = 0x0200;
        const RIGHT = 0x0400;
        const LEFT = 0x0800;
        const Y = 0x1000;
        const X = 0x2000;
        const B = 0x4000;
        const A = 0x8000;
    }
}

#[repr(transparent)]
#[derive(Debug, Copy, Clone, FromBytes)]
pub struct PowerStatus(u8);

bitflags! {
    impl PowerStatus: u8 {
        const AC_PLUGGED_IN = 0x01;
        const POWER_BUTTON_PRESSED = 0x02;
        const CHARGING = 0x40;
        const POWER_USB = 0x80;
        const _ = !0;
    }
}

#[repr(transparent)]
#[derive(Debug, Copy, Clone, FromBytes)]
pub struct ExtraButtons(u8);

bitflags! {
    impl ExtraButtons: u8 {
        const TV_MENU_OPEN = 0x10;
        const TV = 0x20;
        const R3 = 0x40;
        const L3 = 0x80;
        const _ = !0;
    }
}

#[repr(C)]
#[derive(Debug, Copy, Clone, FromBytes)]
pub struct Accelerometer {
    pub z_accel: little_endian::I16,
    pub x_accel: little_endian::I16,
    pub y_accel: little_endian::I16,
}

#[repr(C)]
#[derive(Debug, Copy, Clone, FromBytes)]
pub struct Gyroscope {
    pub roll: i16,
    pub yaw: i16,
    pub pitch: i16,
    pub pad: i16,
}

#[repr(C)]
#[derive(Debug, Copy, Clone, FromBytes)]
pub struct Magnet {
    unknown: [u8; 6],
}

#[repr(C)]
#[derive(Debug, Copy, Clone, FromBytes)]
pub struct Touchscreen {
    pub points: [[Coord; 2]; 10],
}

layout!({
    #[derive(FromBytes)]
    pub struct Coord(u16);
    {
        let extra: Bits<14, 12>;
        let value: Bits<11, 0>;
    }
});