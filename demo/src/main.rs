use std::thread;
use std::time::{Duration, Instant};
use snafu::ResultExt;
use drc::data::InputData;
use drc::InputReader;

#[snafu::report]
#[tokio::main]
async fn main() -> Result<(), snafu::Whatever>{
    let mut reader: InputReader = InputReader::new().await.whatever_context("new reader")?;
    let mut now = Instant::now();
    for i in (0..90).cycle() {
        let pack: InputData = reader.read().await.whatever_context("reading input")?;
        if i == 0 {
            println!("{:#?}", pack.left_stick_x);
            println!("{:?}", now.elapsed());
            now = Instant::now();
        }
    }
    Ok(())
}
