mod input;
mod video;
pub mod cmd;

use std::net::Ipv4Addr;
pub use input::{data, InputReader};
pub use video::{Streamer, Error as StreamerError, frame};

pub struct Gamepad {
    addr: Ipv4Addr,
}
