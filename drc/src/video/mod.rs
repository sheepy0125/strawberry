mod data;
mod encoder;
pub mod frame;
mod tsf;

use crate::frame::Frame;
use crate::video::data::{ExtOption, FrameRate, VstrmHeader};
use crate::video::tsf::Tsf;
pub use data::Error as DataError;
pub use encoder::{Encoder, Error as EncoderError};
use snafu::{Report, ResultExt, Snafu};
use std::collections::VecDeque;
use std::io::Write;
use std::process::Termination;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::net::UdpSocket;
use tokio::runtime::Handle;
use tokio::sync::watch;

const MAX_PAYLOAD_SIZE: usize = 1400;

pub struct Streamer<T: Frame + Send + Sync> {
    send: watch::Sender<Option<T>>,
    audio_queue: Arc<Mutex<VecDeque<u8>>>,
}

impl<T: Frame + Send + Sync + 'static> Streamer<T> {
    pub async fn new() -> Result<Self, Error> {
        let (send, recv) = watch::channel(None);
        let audio_queue = Default::default();
        VideoRunner::spawn(recv, Arc::clone(&audio_queue));
        Ok(Self { send, audio_queue })
    }

    pub fn push_frame(&self, frame: T) -> Result<(), Error> {
        self.send.send(Some(frame)).map_err(|_| Error::Queue)?;
        Ok(())
    }

    pub fn push_audio(&self, data: impl IntoIterator<Item = u8>) {
        let mut guard = self.audio_queue.lock().unwrap();
        guard.extend(data);
    }
}

struct VideoRunner<T: Frame + Send + Sync> {
    recv: watch::Receiver<Option<T>>,
    initial: bool,
    v_seq_id: u16,
    encoder: Encoder,
    v_connection: UdpSocket,
    a_connection: Arc<UdpSocket>,
    tsf: Tsf,
    next_timestamp: u64,
    resync: Arc<AtomicBool>,
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
    let mut counter = 1;
    loop {
        let mut buf = [0u8; 4];
        socket.recv(&mut buf).await.unwrap();
        if buf == [1, 0, 0, 0] {
            eprintln!("resync {counter}");
            counter += 1;
            resync.store(true, Ordering::Relaxed);
        } else {
            eprintln!("unexpected {buf:?}");
        }
    }
}

impl<T: Frame + Send + Sync + 'static> VideoRunner<T> {
    fn spawn(recv: watch::Receiver<Option<T>>, audio_queue: Arc<Mutex<VecDeque<u8>>>) {
        tokio::task::spawn_blocking(move || {
            let result: Report<Error> = Report::capture(|| {
                let mut runner = Handle::current().block_on(Self::new(recv))?;
                // let mut last_loop = Instant::now();
                tokio::spawn(audio_loop(runner.a_connection.clone(), audio_queue));
                loop {
                    // println!("since last loop {:?}", last_loop.elapsed());
                    // last_loop = Instant::now();
                    runner.update_frame()?;
                }
            });
            Report::from(result).report();
        });
    }

    async fn new(recv: watch::Receiver<Option<T>>) -> Result<Self, Error> {
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
        // v_connection.set_tos(0x10).expect("set TOS"); // TODO: constant IPTOS_LOWDELAY
        eprintln!("opened video port");
        let a_connection = Arc::new(UdpSocket::bind("192.168.1.10:50021").await.context(
            ConnectingSnafu {
                ty: ConnectionType::Audio,
            },
        )?);
        a_connection
            .connect("192.168.1.11:50121")
            .await
            .context(ConnectingSnafu {
                ty: ConnectionType::Audio,
            })?;
        eprintln!("opened audio port");
        let encoder = Encoder::new().context(EncoderCreateSnafu)?;
        eprintln!("started encoder");

        let resync = Arc::new(AtomicBool::new(true));
        tokio::spawn(msg_handler(resync.clone()));
        let mut tsf = Tsf::new();
        let next_timestamp = tsf.timestamp();
        Ok(Self {
            recv,
            initial: true,
            v_seq_id: 0,
            encoder,
            v_connection,
            a_connection,
            tsf,
            next_timestamp,
            resync,
        })
    }

    fn make_video_format(ts: u64) -> [u8; 32] {
        let mut packet = [0u8; 32];
        packet[0] = 0x04; // video fmt
        packet[2..4].copy_from_slice(&24u16.to_be_bytes());
        packet[4..8].copy_from_slice(&0x00100000u32.to_le_bytes()); // TODO: why LE?
        // Payload (24 bytes)
        packet[8..12].copy_from_slice(&(ts as u32).to_le_bytes()); // TODO: why LE?
        packet[28..].copy_from_slice(&[
            0x01, // vid_format
            0x00, 0x00, 0x00, // padding
        ]);

        // TODO: Figure out what these values do, and why these give better results than the ones used in libdrc
        packet[12..16].copy_from_slice(&0u32.to_le_bytes()); // mc_video[0]
        packet[16..20].copy_from_slice(&0u32.to_le_bytes()); // mc_video[1]
        packet[20..24].copy_from_slice(&16000u32.to_le_bytes()); // mc_sync[0]
        packet[24..28].copy_from_slice(&16000u32.to_le_bytes()); // mc_sync[1]
        packet
    }

    async fn send_video_format(conn: &UdpSocket, packet: &[u8; 32]) -> Result<(), Error> {
        let ret = conn.send(packet).await.context(SendSnafu {
            ty: ConnectionType::Audio,
        })?;
        assert_eq!(ret, packet.len());
        Ok(())
    }

    const FRAMERATE: FrameRate = FrameRate::Fifty;

    fn prepare_packets(&mut self, timestamp: u64, resync: bool) -> Result<Vec<Vec<u8>>, Error> {
        let image = self.recv.borrow_and_update();
        let Some(im) = &*image else { return Ok(vec![]) };

        let init_flag = self.initial;
        self.initial = false;

        let (chunks, idr) = self
            .encoder
            .encode(im.as_image(), resync || init_flag)
            .context(EncodingSnafu)?;
        debug_assert!(if resync || init_flag { idr } else { true });
        if idr {
            println!("idr");
        }
        drop(image);

        let mut packets = Vec::new();
        for (i, mut chunk) in chunks.into_iter().enumerate() {
            debug_assert!(chunk.len() > 0, "empty chunks are possible?");
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
                    .push(ExtOption::FrameRate(Self::FRAMERATE));
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
        Ok(packets)
    }

    async fn send_packets(
        &mut self,
        format_packet: &[u8; 32],
        packets: &[Vec<u8>],
    ) -> Result<(), Error> {
        Self::send_video_format(&self.a_connection, &format_packet).await?;
        for packet in packets {
            let ret = self.v_connection.send(&packet).await.context(SendSnafu {
                ty: ConnectionType::Video,
            })?;
            assert_eq!(ret, packet.len());
        }
        Ok(())
    }

    fn update_frame(&mut self) -> Result<(), Error> {
        let resync = self.resync.compare_exchange(true, false, Ordering::Relaxed, Ordering::Relaxed).is_ok();

        let video = self.prepare_packets(self.next_timestamp, resync)?;
        if video.is_empty() {
            return Ok(());
        }
        let audio = Self::make_video_format(self.next_timestamp);

        let current_timestamp = self.tsf.timestamp();
        if self.next_timestamp > current_timestamp {
            std::thread::sleep(Duration::from_micros(
                self.next_timestamp - current_timestamp,
            ));
        } else if current_timestamp > self.next_timestamp + 50000 {
            eprintln!("Behind by more than 50ms, pausing 100ms");
            self.next_timestamp = current_timestamp + 100000;
        }
        self.next_timestamp += (1000000.0 / Self::FRAMERATE.freq()) as u64;
        Handle::current().block_on(self.send_packets(&audio, video.as_slice()))?;

        Ok(())
    }
}

const SAMPLES_PER_PACKET: usize = 384;
const BYTES_PER_PACKET: usize = SAMPLES_PER_PACKET * 2 * size_of::<i16>();
const PACKET_INTERVAL: Duration = Duration::from_millis(8);
async fn audio_loop(connection: Arc<UdpSocket>, audio_queue: Arc<Mutex<VecDeque<u8>>>) {
    let mut next_time = tokio::time::Instant::now();
    let mut tsf = Tsf::new();
    let mut packet = vec![0u8; 8 + BYTES_PER_PACKET];
    let mut seq_id = 0u16;
    packet[0] = 1 << 5;
    packet[2..4].copy_from_slice(&(BYTES_PER_PACKET as u16).to_be_bytes());
    loop {
        packet[0] = (packet[0] & 0b11111100) | ((seq_id >> 8) as u8 & 0b11);
        packet[1] = seq_id as u8;
        seq_id = (seq_id + 1) % 1024;
        {
            let mut audio = audio_queue.lock().unwrap();
            let range = ..usize::min(BYTES_PER_PACKET, audio.len());
            for (dst, src) in packet[8..]
                .iter_mut()
                .zip(audio.drain(range).chain(std::iter::repeat(0)))
            {
                *dst = src;
            }
        }
        let ts = tsf.timestamp();
        packet[4..8].copy_from_slice(&(ts as u32).to_le_bytes());
        connection.send(&packet).await.expect("uh oh");

        next_time += PACKET_INTERVAL;
        if !next_time.elapsed().is_zero() {
            eprintln!("audio behind deadline");
        }
        tokio::time::sleep_until(next_time).await;
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
    /// TODO
    Queue,
}

#[derive(Debug)]
pub enum ConnectionType {
    Video,
    Audio,
}
