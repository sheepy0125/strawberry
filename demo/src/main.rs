use drc::Streamer;
use ffmpeg_next::codec::Context;
use ffmpeg_next::ffi::EAGAIN;
use ffmpeg_next::format::input;
use ffmpeg_next::media::Type;
use ffmpeg_next::{format, frame};
use snafu::ResultExt;
use snafu::OptionExt;
use x264::{Colorspace, Plane};

struct YuvFrame<'a> {
    width: i32,
    height: i32,
    planes: Vec<Plane<'a>>
}

impl YuvFrame<'_> {
    fn new(frame: &frame::Video) -> YuvFrame<'_> {
        assert_eq!(frame.format(), format::Pixel::YUV420P);
        let planes = (0..frame.planes()).map(|i| {
            Plane {
                stride: frame.stride(i) as i32,
                data: frame.data(i),
            }
        }).collect();
        YuvFrame {
            width: frame.width() as i32,
            height: frame.height() as i32,
            planes
        }
    }

    fn image(&self) -> x264::Image<'_> {
        x264::Image::new(Colorspace::I420, self.width, self.height, self.planes.as_slice())
    }
}

#[snafu::report]
#[tokio::main]
async fn main() -> Result<(), snafu::Whatever>{
    ffmpeg_next::init().whatever_context("init ffmpeg")?;
    let mut input = input("/home/ruben/rick.mkv").whatever_context("load video")?;
    let input_video = input.streams()
        .best(Type::Video)
        .whatever_context("No video stream")?;
    let video_idx = input_video.index();
    let decoder_ctx = Context::from_parameters(input_video.parameters()).whatever_context("making decoder ctx")?;
    let mut decoder = decoder_ctx.decoder().video().whatever_context("video decoder")?;
    let mut streamer = Streamer::new().await.whatever_context("gamepad streamer")?;

    let mut frame = frame::Video::empty();
    for (stream, packet) in input.packets() {
        if stream.index() != video_idx {
            continue;
        }
        decoder.send_packet(&packet).whatever_context("decoding")?;
        loop {
            match decoder.receive_frame(&mut frame) {
                Ok(()) => {
                    let frame = YuvFrame::new(&frame);
                    let image = frame.image();
                    streamer.push_frame(image).await.whatever_context("streaming")?;
                }
                Err(ffmpeg_next::Error::Other { errno }) if errno == EAGAIN => { break },
                Err(e) => {
                    return Err(e).whatever_context("uh oh");
                }
            }
        }
    }

    Ok(())
}