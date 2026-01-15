use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

pub fn timestamp() -> u64 {
    tsf()
}

fn tsf() -> u64 {
    let mut buff = [0u8; 8];
    File::open("/sys/class/net/wlp3s0/tsf").expect("opening TSF").read_exact(&mut buff).expect("reading TSF");
    u64::from_ne_bytes(buff)
}

pub struct Tsf {
    file: File
}

impl Tsf {
    pub fn new() -> Self {
        Self {
            file: File::open("/sys/class/net/wlp3s0/tsf").expect("opening TSF"),
        }
    }

    pub fn timestamp(&mut self) -> u64 {
        self.file.seek(SeekFrom::Start(0)).expect("rewind time");
        let mut buff = [0u8; 8];
        self.file.read_exact(&mut buff).expect("read TSF");
        u64::from_ne_bytes(buff)
    }
}

#[cfg(test)]
mod test {
    use std::thread;
    use std::time::{Duration, Instant};
    use crate::video::tsf::*;

    #[test]
    fn get_timestamp() {
        eprintln!("{}", timestamp());
    }
    #[test]
    fn get_timestamps() {
        let mut last_timestamp = 0;
        for i in 0..50 {
            let before = Instant::now();
            let timestamp = timestamp();
            assert_ne!(timestamp, last_timestamp);
            eprintln!("n {} ({:?})", timestamp, before.elapsed());
            last_timestamp = timestamp;
            thread::sleep(Duration::from_millis(100));
        }
    }
    #[test]
    fn get_timestamps_fd() {
        let mut tsf = Tsf::new();
        let mut last_timestamp = 0;
        for i in 0..50 {
            let before = Instant::now();
            let timestamp = tsf.timestamp();
            assert_ne!(timestamp, last_timestamp);
            eprintln!("tsf {} ({:?})", timestamp, before.elapsed());
            last_timestamp = timestamp;
            thread::sleep(Duration::from_millis(100));
        }
    }
}