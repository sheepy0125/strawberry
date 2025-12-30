use crate::cmd::data::{Command, CommandHeader, CommandPacket};
use snafu::{Report, ResultExt, Snafu, ensure};
use std::process::Termination;
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::select;
use tokio::sync::broadcast;
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout, transmute_mut};

pub mod data;

pub struct CommandHandler {
    seq_id: AtomicU16,
    socket: Arc<UdpSocket>,
    broadcast: broadcast::Sender<Arc<[u8]>>,
}

impl CommandHandler {
    pub async fn new() -> Result<Self, Error> {
        let socket = UdpSocket::bind("192.168.1.10:50023")
            .await
            .context(ConnectingSnafu)?;
        socket
            .connect("192.168.1.11:50123")
            .await
            .context(ConnectingSnafu)?;
        let socket = Arc::new(socket);
        let (broadcast, _) = broadcast::channel(16);
        let sock = socket.clone();
        let bc = broadcast.clone().downgrade();

        tokio::spawn(async move {
            let result: Report<Error> = (async {
                loop {
                    let mut buff = vec![0; 1800]; // TODO: introduce MTU constant
                    let bytes = sock.recv(&mut buff).await.context(ReceiveSnafu)?;
                    buff.resize(bytes, 0);
                    let Some(broadcast) = bc.upgrade() else {
                        return Ok(());
                    };
                    let _ = broadcast.send(Arc::from(buff));
                }
            })
            .await
            .into();
            eprintln!("closed command handler");
            result.report();
        });

        Ok(CommandHandler {
            seq_id: AtomicU16::new(0),
            socket,
            broadcast,
        })
    }

    fn next_seq_id(&self) -> u16 {
        self.seq_id.fetch_add(1, Ordering::Relaxed)
    }

    async fn send_packet<T: IntoBytes + Immutable + KnownLayout>(
        &self,
        packet_type: u16,
        query_type: u16,
        seq_id: u16,
        payload: T,
    ) -> Result<(), std::io::Error> {
        let mut buffer = vec![0u8; size_of::<CommandHeader>() + size_of::<T>()];
        let command: &mut CommandPacket =
            CommandPacket::mut_from_bytes(&mut buffer).expect("no bytes");
        command.header = CommandHeader {
            packet_type: packet_type.into(),
            query_type: query_type.into(),
            payload_size: (size_of::<T>() as u16).into(),
            seq_id: seq_id.into(),
        };
        payload
            .write_to(&mut command.payload)
            .expect("payload size mismatch");

        // println!("sending {:#?}", command.header);

        self.socket.send(&buffer).await?;
        Ok(())
    }

    async fn recv_packet(
        &self,
        rcv: &mut broadcast::Receiver<Arc<[u8]>>,
        seq_id: u16,
    ) -> Result<Arc<[u8]>, Error> {
        loop {
            let mut retries = 0;
            let packet = loop {
                select! {
                    res = rcv.recv() => break res.unwrap(),
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {
                        retries += 1;
                        if retries == 10 {
                            return Err(Error::Timeout)
                        }
                    },
                }
            };
            let packet_data =
                CommandPacket::ref_from_bytes(&packet).map_err(|x| Error::Incomplete {
                    reason: x.to_string(),
                })?;
            if packet_data.header.seq_id != seq_id {
                continue;
            }

            // println!("recv {:#?}", packet_data.header);
            ensure!(
                packet_data.header.payload_size.get() as usize == packet_data.payload.len(),
                PayloadLengthSnafu
            );
            return Ok(packet);
        }
    }

    pub async fn command<T: Command>(&self, data: T) -> Result<T::RecvPayload, Error> {
        let seq_id = self.next_seq_id();
        let mut rcv = self.broadcast.subscribe();

        self.send_packet(0, T::QUERY_TYPE, seq_id, data.payload())
            .await
            .context(SendSnafu)?;

        let ack = self.recv_packet(&mut rcv, seq_id).await?;
        let ack = CommandPacket::ref_from_bytes(&ack).expect("already unpacked");

        ensure!(ack.header.packet_type == 1, AckExpectedSnafu);
        ensure!(ack.payload.len() == 0, PayloadLengthSnafu);

        let response = self.recv_packet(&mut rcv, seq_id).await?;
        let response = CommandPacket::ref_from_bytes(&response).expect("already unpacked");

        ensure!(response.header.packet_type == 2, ResponseExpectedSnafu);
        self.send_packet(3, T::QUERY_TYPE, seq_id, ())
            .await
            .context(SendSnafu)?;
        T::RecvPayload::read_from_bytes(&response.payload).map_err(|x| Error::Incomplete {
            reason: x.to_string(),
        })
    }
}

#[derive(Debug, Snafu)]
pub enum Error {
    /// failed to open UDP socket
    Connecting { source: std::io::Error },
    /// socket send error
    Send { source: std::io::Error },
    /// socket receive error
    Receive { source: std::io::Error },
    #[snafu(display("incomplete packet: {reason}"))]
    Incomplete { reason: String },
    /// expected an ACK packet
    AckExpected,
    /// expected a response packet
    ResponseExpected,
    /// invalid payload length
    PayloadLength,
    /// Timeout
    Timeout,
}
