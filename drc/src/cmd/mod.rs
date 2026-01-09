use crate::cmd::data::{CommandHeader, CommandPacket, Payload};
use crate::cmd::generic::GenericPayload;
use snafu::{ensure, Report, ResultExt, Snafu};
use std::process::Termination;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::select;
use tokio::sync::broadcast;
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

pub mod data;
pub mod generic;

pub struct CommandHandler {
    seq_id: AtomicU16,
    socket: Arc<UdpSocket>,
    broadcast: broadcast::Sender<Arc<[u8]>>,
}

impl CommandHandler {
    const TIMEOUT: Duration = Duration::from_millis(1000);
    const RETRIES: usize = 10;

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

    async fn send_packet<T: Payload>(
        &self,
        seq_id: u16,
        payload: &T,
    ) -> Result<(), std::io::Error> {
        let mut buffer = vec![0u8; payload.packet_size()];
        payload.write_packet(seq_id, &mut buffer);

        self.socket.send(&buffer).await?;
        Ok(())
    }

    async fn send_ack<T: Payload>(&self, seq_id: u16, payload: &T) -> Result<(), std::io::Error> {
        let command = CommandHeader {
            packet_type: 3.into(),
            query_type: T::QUERY_TYPE.into(),
            payload_size: 0.into(),
            seq_id: seq_id.into()
        };
        self.socket.send(command.as_bytes()).await?;
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
                    _ = tokio::time::sleep(Self::TIMEOUT) => {
                        retries += 1;
                        if retries >= Self::RETRIES {
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
            ensure!(
                packet_data.header.payload_size.get() as usize == packet_data.payload.len(),
                PayloadLengthSnafu
            );
            return Ok(packet);
        }
    }

    pub async fn command<T: Payload>(&self, data: &T) -> Result<T::Response, Error> {
        let seq_id = self.next_seq_id();
        let mut rcv = self.broadcast.subscribe();

        self.send_packet(seq_id, data)
            .await
            .context(SendSnafu)?;

        let ack = self.recv_packet(&mut rcv, seq_id).await?;
        let ack = CommandPacket::ref_from_bytes(&ack).expect("already unpacked");

        ensure!(ack.header.packet_type == 1, AckExpectedSnafu);
        ensure!(ack.payload.len() == 0, PayloadLengthSnafu);

        let response = self.recv_packet(&mut rcv, seq_id).await?;
        let response = CommandPacket::ref_from_bytes(&response).expect("already unpacked");

        ensure!(response.header.packet_type == 2, ResponseExpectedSnafu);
        self.send_ack(seq_id, data)
            .await
            .context(SendSnafu)?;
        T::Response::read_from_bytes(&response.payload).map_err(|x| Error::Incomplete {
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
