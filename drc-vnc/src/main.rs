use drc::cmd::data::UvcUacPayload;
use drc::cmd::{CommandHandler, generic};
use drc::frame::Frame;
use drc::{Streamer, StreamerError};
use image::{GenericImage, GenericImageView, ImageError, RgbaImage};
use snafu::{OptionExt, Report, ResultExt, Snafu, Whatever, ensure};
use std::process::Termination;
use tokio::net::{TcpStream, ToSocketAddrs};
use tokio::time::{Duration, Instant, Interval, MissedTickBehavior, interval, interval_at};
use vnc::{PixelFormat, Rect, VncClient, VncConnector, VncError, VncEvent, X11Event};
use x264::{Colorspace, Plane};
use yuv::{
    YuvChromaSubsampling, YuvConversionMode, YuvError, YuvPlanarImageMut, YuvRange,
    YuvStandardMatrix, rgba_to_yuv420,
};

// TODO: move to drc crate
#[snafu::report]
async fn uvc_handler(cmd_handler: CommandHandler) -> Result<(), snafu::Whatever> {
    let mut state = UvcUacPayload::default();
    let result: Result<(), snafu::Whatever> = (async {
        loop {
            let resp = cmd_handler
                .command(&state)
                .await
                .whatever_context("send uvc uac")?;

            state.mic_volume = resp.mic_volume.get().into();
            state.mic_jack_volume = resp.mic_jack_volume.get().into();
            state.mic_enable = resp.mic_enabled;
            state.cam_power_freq = resp.cam_power_freq;
            state.cam_auto_expo = resp.cam_auto_expo;
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    })
    .await;
    Report::from(result).report();
    Ok(())
}

async fn launch_uvc() -> Result<(), snafu::Whatever> {
    let cmd_handler = CommandHandler::new()
        .await
        .whatever_context("command handler")?;
    let resp = cmd_handler
        .command(&generic::GetUicFirmware)
        .await
        .whatever_context("get uic firmware")?;
    eprintln!("{resp:?}");
    tokio::spawn(async move { uvc_handler(cmd_handler).await.report() });
    Ok(())
}

#[snafu::report]
#[tokio::main]
async fn main() -> Result<(), Error> {
    let password = std::env::args().nth(1).expect("password expected");
    let mut client = VncDrc::new("127.0.0.1:5901", password).await?;

    loop {
        client.handle_events().await?;
    }
}

pub struct VncDrc {
    vnc: VncClient,
    drc: Streamer<VncFrame>,
    canvas: RgbaImage,
    dirty: bool,
    last_tick: Instant,
    interval: Interval,
}

impl VncDrc {
    pub async fn new(address: impl ToSocketAddrs, password: String) -> Result<Self, Error> {
        launch_uvc().await.context(OtherSnafu)?;

        let tcp = TcpStream::connect(address).await.context(TcpConnectSnafu)?;
        let vnc = VncConnector::new(tcp)
            .set_auth_method(async move { Ok(password) })
            .add_encoding(vnc::VncEncoding::Raw)
            .add_encoding(vnc::VncEncoding::CopyRect)
            .add_encoding(vnc::VncEncoding::LastRectPseudo)
            .allow_shared(true)
            .set_pixel_format(PixelFormat::rgba())
            .build()
            .context(VncSetupSnafu)?
            .try_start()
            .await
            .context(VncConnectSnafu)?
            .finish()
            .context(VncConnectSnafu)?;
        let drc = Streamer::new().await.context(DrcConnectSnafu)?;
        let canvas = RgbaImage::new(864, 480);
        let last_tick = Instant::now();
        let mut interval = interval_at(last_tick, Duration::from_millis(16));
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

        let this = Self {
            vnc,
            drc,
            canvas,
            dirty: true,
            last_tick,
            interval,
        };

        Ok(this)
    }

    async fn refresh_canvas(&mut self) -> Result<(), Error> {
        if !self.dirty {
            return Ok(());
        }
        let mut image = YuvPlanarImageMut::alloc(
            self.canvas.width(),
            self.canvas.height(),
            YuvChromaSubsampling::Yuv420,
        );
        rgba_to_yuv420(
            &mut image,
            self.canvas.as_raw(),
            4 * self.canvas.width(),
            YuvRange::Full,
            YuvStandardMatrix::Bt709,
            YuvConversionMode::Balanced,
        )
        .context(YuvSnafu)?;

        self.drc
            .push_frame(VncFrame(image))
            .context(DrcFrameSnafu)?;
        self.dirty = false;
        Ok(())
    }

    pub async fn handle_events(&mut self) -> Result<(), Error> {
        while let Some(ev) = self.vnc.poll_event().await.context(VncEventSnafu)? {
            match ev {
                VncEvent::SetResolution(res) => {
                    // eprintln!("resolution {res:?}");
                    assert_eq!(self.canvas.width(), res.width as u32);
                    assert_eq!(self.canvas.height(), res.height as u32);
                }
                VncEvent::RawImage(rect, data) => {
                    // eprintln!("got img data {}x{}", rect.width, rect.height);
                    let length = data.len();
                    let buf = RgbaImage::from_raw(rect.width as u32, rect.height as u32, data)
                        .context(ImageDataSnafu {
                            width: rect.width,
                            height: rect.height,
                            length,
                        })?;
                    self.canvas
                        .copy_from(&buf, rect.x as u32, rect.y as u32)
                        .context(ImageSnafu)?;
                    self.dirty = true;
                }
                VncEvent::Copy(dst, src) => {
                    assert_eq!(src.width, dst.width, "width mismatch in copy");
                    assert_eq!(src.height, dst.height, "height mismatch in copy");
                    ensure!(
                        self.canvas.copy_within(
                            image::math::Rect {
                                x: src.x as u32,
                                y: src.y as u32,
                                width: src.width as u32,
                                height: src.height as u32
                            },
                            dst.x as u32,
                            dst.y as u32
                        ),
                        CopySnafu { src, dst }
                    );
                    self.dirty = true;
                }
                VncEvent::JpegImage(rect, data) => {
                    eprintln!("got jpeg data {}x{}", rect.width, rect.height);
                    // let buf = JpegDecoder::new(BufReader::new(data.as_slice()));
                }
                ev => eprintln!("unhandled ev {ev:?}"),
            }
        }
        self.vnc
            .input(X11Event::Refresh)
            .await
            .context(VncSendSnafu)?;
        self.refresh_canvas().await?;
        let this_tick = self.interval.tick().await;
        let duration = this_tick - self.last_tick;
        self.last_tick = this_tick;
        if duration.as_millis() > 20 {
            // eprintln!(
            //     "Behind schedule by {:?}",
            //     duration - Duration::from_millis(16)
            // );
        }
        Ok(())
    }
}

pub struct VncFrame(YuvPlanarImageMut<'static, u8>);

impl Frame for VncFrame {
    fn as_image(&self) -> x264::Image<'_> {
        let VncFrame(frame) = self;
        x264::Image::new(
            Colorspace::I420,
            frame.width as i32,
            frame.height as i32,
            &[
                Plane {
                    data: frame.y_plane.borrow(),
                    stride: frame.y_stride as i32,
                },
                Plane {
                    data: frame.u_plane.borrow(),
                    stride: frame.u_stride as i32,
                },
                Plane {
                    data: frame.v_plane.borrow(),
                    stride: frame.v_stride as i32,
                },
            ],
        )
    }
}

#[derive(Debug, Snafu)]
pub enum Error {
    /// connecting to tcp port
    TcpConnect { source: tokio::io::Error },
    /// setting up VNC handshake
    VncSetup { source: VncError },
    /// connecting to VNC
    VncConnect { source: VncError },
    /// sending VNC event
    VncSend { source: VncError },
    /// handling VNC event
    VncEvent { source: VncError },
    /// connecting to gamepad
    DrcConnect { source: StreamerError },
    #[snafu(display("invalid image data, w: {width}, h: {height}, len: {length}"))]
    ImageData {
        width: u16,
        height: u16,
        length: usize,
    },
    #[snafu(display("invalid copyrect encoding {src:?} {dst:?}"))]
    Copy { src: Rect, dst: Rect },
    /// processing image
    Image { source: ImageError },
    /// YUV conversion
    Yuv { source: YuvError },
    /// Pushing frame
    DrcFrame { source: StreamerError },
    /// Other
    Other { source: Whatever },
}
