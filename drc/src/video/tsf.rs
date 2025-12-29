use std::fs::File;
use std::io::Read;

pub fn timestamp() -> u32 {
    tsf() as u32
}

fn tsf() -> u64 {
    let mut buff = [0u8; 8];
    File::open("/sys/class/net/wlp3s0/tsf").expect("opening TSF").read_exact(&mut buff).expect("reading TSF");
    u64::from_ne_bytes(buff)
}

#[cfg(test)]
mod test {
    use crate::video::tsf::*;

    #[test]
    fn get_timestamp() {
        eprintln!("{}", timestamp());
    }
}