mod input;
mod video;

use std::net::Ipv4Addr;

pub use input::{data, InputReader};
pub use video::{Streamer, Error as StreamerError};

pub struct Gamepad {
    addr: Ipv4Addr,
}
