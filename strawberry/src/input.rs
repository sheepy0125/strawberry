use crate::data::InputData;
use snafu::{ResultExt, Snafu};
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::watch;
use tokio::sync::watch::error::{RecvError, SendError};
use tokio::sync::watch::Ref;
use zerocopy::transmute;
use crate::input::InputError::{RecvSocketClosed, SendSocketClosed};

pub mod data;

pub struct InputReader {
    recv: watch::Receiver<Result<InputData, Arc<InputError>>>,
}

impl InputReader {
    pub async fn new() -> Result<Self, InputError> {
        let sock: UdpSocket = UdpSocket::bind(("192.168.1.10", 50022)).await.context(UdpSetupSnafu)?;
        let (send, recv) = watch::channel(Ok(zerocopy::FromZeros::new_zeroed()));
        tokio::task::spawn(async move {
            if let Err(e) = (|| async {
                loop {
                    let mut buff = [0u8; 128];
                    let read_count = sock.recv(buff.as_mut_slice()).await.context(InputReadSnafu)?;
                    snafu::ensure!(read_count == 128, IncompleteInputSnafu { bytes: read_count });
                    if let Err(e) = send.send(Ok(transmute!(buff))) {
                        return SendSocketClosedSnafu.fail::<()>();
                    }
                }
            })().await {
                let _ = send.send(Err(Arc::new(e)));
            }
        });
        Ok(Self {
            recv
        })
    }

    pub async fn read(&mut self) -> Result<InputData, Arc<InputError>> {
        self.recv.changed().await.context(RecvSocketClosedSnafu).unwrap();
        match &*self.recv.borrow_and_update() {
            Ok(i) => Ok(*i),
            Err(e) => Err(e.clone()),
        }
    }
}

#[derive(Debug, Snafu)]
pub enum InputError {
    /// Opening UDP Socket
    UdpSetup { source: std::io::Error },
    /// Reading input data
    InputRead { source: std::io::Error },
    #[snafu(display("Read incomplete input packet ({bytes} != 128)"))]
    IncompleteInput { bytes: usize },
    /// socket closed
    SendSocketClosed,
    /// socket closed
    RecvSocketClosed { source: RecvError }
}
