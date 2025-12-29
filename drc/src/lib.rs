mod input;
mod video;

use std::net::Ipv4Addr;

pub use input::{data, InputReader};
pub use video::{Encoder, EncoderError};

pub struct Gamepad {
    addr: Ipv4Addr,
}
