use pnet::{datalink::interfaces, ipnetwork::IpNetwork};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::net::Ipv4Addr;
use std::str::FromStr;

fn get_interface_of_ipv4(addr: Ipv4Addr) -> Option<String> {
    let ifa = pnet::datalink::interfaces().into_iter().find(|ifa| {
        for ip in &ifa.ips {
            if let IpNetwork::V4(ip) = ip
                && ip.is_supernet_of(addr.into())
            {
                return true;
            }
        }
        false
    });
    ifa.map(|ifa| ifa.name)
}

pub struct Tsf {
    file: File,
}

impl Tsf {
    pub fn new() -> Self {
        let iface = get_interface_of_ipv4(Ipv4Addr::from_str("192.168.1.10").unwrap())
            .expect("no interface for 192.168.1.10");
        Self {
            file: File::open(format!("/sys/class/net/{iface}/tsf")).expect("opening TSF"),
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
    use crate::video::tsf::*;
    use std::thread;
    use std::time::{Duration, Instant};

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
