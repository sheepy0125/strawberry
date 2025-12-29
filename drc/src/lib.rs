mod input;
mod video;

use std::net::Ipv4Addr;

pub use input::{data, InputReader};
pub struct Gamepad {
    addr: Ipv4Addr,
}
