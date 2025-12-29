mod data;
mod encoder;
mod tsf;

use crate::video::data::{ExtOption, FrameRate, VstrmHeader};
pub use data::Error as DataError;
pub use encoder::{Encoder, Error as EncoderError};
use snafu::{ResultExt, Snafu};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpStream, UdpSocket};
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
    resync: Arc<AtomicBool>,
}

async fn msg_handler(resync: Arc<AtomicBool>) {
    let socket = UdpSocket::bind("192.168.1.10:50010").await.unwrap();
    loop {
        let mut buf = [0u8; 4];
        let bytes = socket.recv(&mut buf).await.unwrap();
        if buf == [1, 0, 0, 0] {
            eprintln!("resync request");
            resync.store(true, Ordering::Relaxed);
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
        eprintln!("connected to video port");
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
        eprintln!("connected to audio port");
        let encoder = Encoder::new().context(EncoderCreateSnafu)?;
        eprintln!("connected to gamepad");
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
            resync,
        })
    }

    async fn send_video_format(&mut self, ts: u32) -> Result<(), Error> {
        let mut packet = [0u8; 32];
        let seq_id = self.a_seq_id;
        self.a_seq_id = (seq_id + 1) % 1024;
        packet[0] = 0x04; // video fmt
        packet[0] |= (seq_id >> 8) as u8 & 0b11;
        packet[1] = seq_id as u8;
        packet[2..4].copy_from_slice(&24u16.to_be_bytes());
        packet[4..8].copy_from_slice(&0x00100000u32.to_le_bytes()); // TODO: why LE?
        packet[8..].copy_from_slice(&[
            0x00, 0x00, 0x00, 0x00, 0x80, 0x3e, 0x00, 0x00, 0x80, 0x3e, 0x00, 0x00, 0x80, 0x3e,
            0x00, 0x00, 0x80, 0x3e, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ]);
        packet[8..12].copy_from_slice(&ts.to_le_bytes()); // TODO: why LE?
        let ret = self.a_connection.send(&packet).await.context(SendSnafu {
            ty: ConnectionType::Audio,
        })?;
        assert_eq!(ret, packet.len());
        Ok(())
    }

    pub async fn push_frame(&mut self, image: Image<'_>) -> Result<(), Error> {
        tokio::time::sleep_until(self.next_send).await;

        let timestamp = tsf::timestamp();
        self.send_video_format(timestamp).await?;
        let resync = self
            .resync
            .compare_exchange(true, false, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok();
        let (chunks, idr) = self.encoder.encode(image, resync).context(EncodingSnafu)?;

        if idr {
            eprintln!("resyncing");
        }
        let init_flag = self.initial;
        self.initial = false;
        for (i, mut chunk) in chunks.into_iter().enumerate() {
            assert!(chunk.len() > 0, "empty chunks are possible?");
            let mut first_packet = true;
            let first_chunk = i == 0;
            let last_chunk = i == chunks.len() - 1;

            while chunk.len() > 0 {
                let packet;
                if let Some((before, after)) = chunk.split_at_checked(1400) {
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
                    timestamp,
                    init: init_flag,
                    frame_begin: first_packet && first_chunk,
                    chunk_end: last_packet,
                    frame_end: last_packet && last_chunk,
                    ..VstrmHeader::default()
                };
                if idr {
                    header.ext_headers.push(ExtOption::Idr);
                }
                header
                    .ext_headers
                    .push(ExtOption::FrameRate(FrameRate::TwentyFive));

                first_packet = false;
                let mut buffer = Vec::with_capacity(packet.len() + VstrmHeader::SIZE);
                buffer.extend(header.into_bytes().context(DataSnafu)?);
                buffer.extend(packet);
                let ret = self.v_connection.send(&buffer).await.context(SendSnafu {
                    ty: ConnectionType::Video,
                })?;
                assert_eq!(ret, buffer.len());
            }
        }

        self.next_send += Duration::from_micros(1000000 / 25);

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
enum ConnectionType {
    Video,
    Audio,
}
