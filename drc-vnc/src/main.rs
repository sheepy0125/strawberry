use std::io::BufReader;
use drc::frame::Frame;
use drc::{Streamer, StreamerError};
use image::{GenericImage, GenericImageView, ImageBuffer, ImageError, RgbImage, Rgba, RgbaImage};
use snafu::{OptionExt, ResultExt, Snafu, ensure};
use std::time::Instant;
use image::codecs::jpeg::JpegDecoder;
use tokio::net::{TcpStream, ToSocketAddrs};
use vnc::{PixelFormat, Rect, VncClient, VncConnector, VncError, VncEvent, X11Event};
use x264::{Colorspace, Plane};
use yuv::{
    YuvChromaSubsampling, YuvConversionMode, YuvError, YuvPlanarImage, YuvPlanarImageMut, YuvRange,
    YuvStandardMatrix, rgb_to_yuv420, rgba_to_yuv420,
};

#[snafu::report]
#[tokio::main]
async fn main() -> Result<(), Error> {
    let password = std::env::args().nth(1).expect("password expected");
    let mut client = VncDrc::new("127.0.0.1:5901", password).await?;

    loop {
        client.handle_event().await?;
    }
}

pub struct VncDrc {
    vnc: VncClient,
    drc: Streamer<VncFrame>,
    canvas: RgbaImage,
    last_refresh: Instant,
}

impl VncDrc {
    pub async fn new(address: impl ToSocketAddrs, password: String) -> Result<Self, Error> {
        let tcp = TcpStream::connect(address).await.context(TcpConnectSnafu)?;
        let vnc = VncConnector::new(tcp)
            .set_auth_method(async move { Ok(password) })
            .add_encoding(vnc::VncEncoding::Raw)
            .add_encoding(vnc::VncEncoding::CopyRect)
            // .add_encoding(vnc::VncEncoding::Tight)
            .add_encoding(vnc::VncEncoding::CursorPseudo)
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

        Ok(Self {
            vnc,
            drc,
            canvas,
            last_refresh: Instant::now(),
        })
    }

    async fn refresh_canvas(&mut self) -> Result<(), Error> {
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
        Ok(())
    }

    pub async fn handle_event(&mut self) -> Result<(), Error> {
        if self.last_refresh.elapsed().as_millis() > 16 {
            self.last_refresh = Instant::now();
            self.vnc
                .input(X11Event::Refresh)
                .await
                .context(VncSendSnafu)?;
            self.refresh_canvas().await?;
        }
        let Some(ev) = self.vnc.poll_event().await.context(VncEventSnafu)? else {
            return Ok(());
        };
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
            }
            VncEvent::JpegImage(rect, data) => {
                eprintln!("got jpeg data {}x{}", rect.width, rect.height);
                // let buf = JpegDecoder::new(BufReader::new(data.as_slice()));
            }
            ev => eprintln!("unhandled ev {ev:?}"),
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
}
