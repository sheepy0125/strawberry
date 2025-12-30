mod input;
mod video;
mod cmd;

use std::net::Ipv4Addr;

pub use input::{data, InputReader};
pub use video::{Streamer, Error as StreamerError};
pub use cmd::{CommandHandler, Error as CommandError};
pub use cmd::data::*;

pub struct Gamepad {
    addr: Ipv4Addr,
}
