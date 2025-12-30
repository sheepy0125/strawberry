mod data;
mod encoder;
mod tsf;

use crate::video::data::{ExtOption, FrameRate, VstrmHeader};
pub use data::Error as DataError;
pub use encoder::{Encoder, Error as EncoderError};
use snafu::{ResultExt, Snafu};
use std::fs::File;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::UdpSocket;
use tokio::time::Instant;
use x264::Image;

const MAX_PAYLOAD_SIZE: usize = 1400;

pub struct Streamer {
    initial: bool,
    a_seq_id: u16,
    v_seq_id: u16,
    encoder: Encoder,
    v_connection: UdpSocket,
    a_connection: UdpSocket,
    next_send: Instant,
    next_timestamp: u64,
    resync: Arc<AtomicBool>,
    debug_file: Option<File>,
}

fn dump_headers(mut file: impl Write) {
    let nal_start_code = [0x00, 0x00, 0x00, 0x01];
    let gamepad_sps = [
        0x67, 0x64, 0x00, 0x20, 0xac, 0x2b, 0x40, 0x6c, 0x1e, 0xf3, 0x68,
    ];
    let gamepad_pps = [0x68, 0xee, 0x06, 0x0c, 0xe8];

    file.write_all(&nal_start_code).unwrap();
    file.write_all(&gamepad_sps).unwrap();
    file.write_all(&nal_start_code).unwrap();
    file.write_all(&gamepad_pps).unwrap();
}

fn nal_escape(src: &[&[u8]]) -> Vec<u8> {
    let mut output = Vec::with_capacity(src.len() * 2);
    for byte in src.iter().copied().flatten().copied() {
        if byte <= 0x03
            && output.len() > 2
            && output[output.len() - 2] == 0
            && output[output.len() - 1] == 0
        {
            output.push(0x03);
        }
        output.push(byte);
    }
    output
}

fn dump_frame(mut file: impl Write, chunks: &[&[u8]], is_idr: bool) {
    static FRAME_NUMBER: AtomicU8 = AtomicU8::new(0);

    let nal_start_code = [0x00, 0x00, 0x00, 0x01];
    let nal_idr_frame = [0x25, 0xb8, 0x04, 0xff];
    let mut nal_p_frame = [0x21, 0xe0, 0x03, 0xff];

    file.write_all(&nal_start_code).unwrap();

    if is_idr {
        FRAME_NUMBER.store(0, Ordering::Relaxed);
        file.write_all(&nal_idr_frame).unwrap();
    } else {
        let frame_number = FRAME_NUMBER.fetch_add(1, Ordering::Relaxed) as u8;
        nal_p_frame[1] |= frame_number >> 3;
        nal_p_frame[2] |= frame_number << 5;
        file.write_all(&nal_p_frame).unwrap();
    }

    file.write_all(&nal_escape(chunks)).unwrap();
}

async fn msg_handler(resync: Arc<AtomicBool>) {
    let socket = UdpSocket::bind("192.168.1.10:50010").await.unwrap();
    loop {
        let mut buf = [0u8; 4];
        socket.recv(&mut buf).await.unwrap();
        if buf == [1, 0, 0, 0] {
            // eprintln!("{:?} resync request", Instant::now());
            resync.store(true, Ordering::Relaxed);
        } else {
            eprintln!("unexpected {buf:?}");
        }
    }
}

impl Streamer {
    pub async fn new() -> Result<Self, Error> {
        let v_connection =
            UdpSocket::bind("192.168.1.10:50020")
                .await
                .context(ConnectingSnafu {
                    ty: ConnectionType::Video,
                })?;
        v_connection
            .connect("192.168.1.11:50120")
            .await
            .context(ConnectingSnafu {
                ty: ConnectionType::Video,
            })?;
        eprintln!("opened video port");
        let a_connection =
            UdpSocket::bind("192.168.1.10:50021")
                .await
                .context(ConnectingSnafu {
                    ty: ConnectionType::Audio,
                })?;
        a_connection
            .connect("192.168.1.11:50121")
            .await
            .context(ConnectingSnafu {
                ty: ConnectionType::Audio,
            })?;
        eprintln!("opened audio port");
        let encoder = Encoder::new().context(EncoderCreateSnafu)?;
        eprintln!("started encoder");
        // let mut debug_file = File::create("./debug.data").unwrap();
        // dump_headers(&mut debug_file);
        let resync = Arc::new(AtomicBool::new(true));
        tokio::spawn(msg_handler(resync.clone()));
        Ok(Self {
            initial: true,
            v_seq_id: 0,
            a_seq_id: 0,
            encoder,
            v_connection,
            a_connection,
            next_send: Instant::now(),
            next_timestamp: tsf::timestamp(),
            resync,
            debug_file: None,
        })
    }

    async fn send_video_format(conn: &UdpSocket, ts: u32) -> Result<(), Error> {
        let mut packet = [0u8; 32];
        packet[0] = 0x04; // video fmt
        packet[2..4].copy_from_slice(&24u16.to_be_bytes());
        packet[4..8].copy_from_slice(&0x00100000u32.to_le_bytes()); // TODO: why LE?
        packet[8..].copy_from_slice(&[
            0x00, 0x00, 0x00, 0x00, //
            0x80, 0x3e, 0x00, 0x00, //
            0x80, 0x3e, 0x00, 0x00, //
            0x80, 0x3e, 0x00, 0x00, //
            0x80, 0x3e, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, //
        ]);
        packet[8..12].copy_from_slice(&ts.to_le_bytes()); // TODO: why LE?
        let ret = conn.send(&packet).await.context(SendSnafu {
            ty: ConnectionType::Audio,
        })?;
        assert_eq!(ret, packet.len());
        Ok(())
    }

    pub async fn push_frame(&mut self, image: Image<'_>) -> Result<(), Error> {
        const FRAMERATE: FrameRate = FrameRate::TwentyFive;

        let resync = self
            .resync
            .compare_exchange(true, false, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok();
        let (chunks, idr) = self.encoder.encode(image, resync).context(EncodingSnafu)?;
        if let Some(file) = &mut self.debug_file {
            dump_frame(file, &chunks, idr);
        }
        let timestamp = tsf::timestamp();

        let init_flag = self.initial;
        self.initial = false;
        let mut packets = Vec::new();
        for (i, mut chunk) in chunks.into_iter().enumerate() {
            assert!(chunk.len() > 0, "empty chunks are possible?");
            let mut first_packet = true;
            let first_chunk = i == 0;
            let last_chunk = i == chunks.len() - 1;

            while chunk.len() > 0 {
                let packet;
                if let Some((before, after)) = chunk.split_at_checked(MAX_PAYLOAD_SIZE) {
                    packet = before;
                    chunk = after;
                } else {
                    packet = chunk;
                    chunk = &[];
                }

                let last_packet = chunk.len() == 0;
                let seq_id = self.v_seq_id;
                self.v_seq_id = (seq_id + 1) % 1024;
                let mut header = VstrmHeader {
                    seq_id,
                    payload_size: packet.len() as u16,
                    timestamp: timestamp as u32,
                    init: init_flag,
                    frame_begin: first_packet && first_chunk,
                    chunk_end: last_packet,
                    frame_end: last_packet && last_chunk,
                    ..VstrmHeader::default()
                };
                header
                    .ext_headers
                    .push(ExtOption::FrameRate(FRAMERATE));
                if idr {
                    header.ext_headers.push(ExtOption::Idr);
                }

                first_packet = false;
                let mut buffer = Vec::with_capacity(packet.len() + VstrmHeader::SIZE);
                buffer.extend(header.into_bytes().context(DataSnafu)?);
                buffer.extend(packet);
                packets.push(buffer);
            }
        }

        tokio::time::sleep_until(self.next_send).await;
        Self::send_video_format(&self.a_connection, timestamp as u32).await?;
        // println!("{} packets", packets.len());
        for packet in packets {
            let ret = self.v_connection.send(&packet).await.context(SendSnafu {
                ty: ConnectionType::Video,
            })?;
            assert_eq!(ret, packet.len());
        }

        let now_timestamp = tsf::timestamp();
        let overflown;
        (self.next_timestamp, overflown) = self.next_timestamp.overflowing_add((1000000.0 / FRAMERATE.freq()) as u64);
        if !overflown && self.next_timestamp < now_timestamp {
            eprintln!("too slow");
            self.next_timestamp = now_timestamp + 1000;
        }
        self.next_send = Instant::now() + Duration::from_micros(self.next_timestamp - now_timestamp);
        Ok(())
    }
}

#[derive(Debug, Snafu)]
pub enum Error {
    /// constructing packet header
    Data { source: DataError },
    /// initializing encoder
    EncoderCreate { source: EncoderError },
    /// encoding frame
    Encoding { source: EncoderError },
    #[snafu(display("failed to connect to gamepad {ty:?}"))]
    Connecting {
        ty: ConnectionType,
        source: std::io::Error,
    },
    #[snafu(display("failed to send to gamepad {ty:?}"))]
    Send {
        ty: ConnectionType,
        source: std::io::Error,
    },
}

#[derive(Debug)]
pub enum ConnectionType {
    Video,
    Audio,
}
