use drc::{CommandHandler, Streamer, UvcUacPayload};
use ffmpeg_next::codec::Context;
use ffmpeg_next::ffi::EAGAIN;
use ffmpeg_next::format::input;
use ffmpeg_next::media::Type;
use ffmpeg_next::{format, frame};
use snafu::OptionExt;
use snafu::{Report, ResultExt};
use std::process::Termination;
use std::thread;
use std::time::Duration;
use tokio::task::LocalSet;
use drc::frame::Frame;
use x264::{Colorspace, Image, Plane};

struct MyFrame(frame::Video);

impl Frame for MyFrame {
    fn as_image(&self) -> Image<'_> {
        assert_eq!(self.0.format(), format::Pixel::YUV420P);
        debug_assert_eq!(self.0.planes(), 3);
        let planes = [0, 1, 2]
            .map(|i| Plane {
                stride: self.0.stride(i) as i32,
                data: self.0.data(i),
            });
        Image::new(Colorspace::I420, self.0.width() as i32, self.0.height() as i32, &planes)
    }
}

#[snafu::report]
async fn uvc_handler() -> Result<(), snafu::Whatever> {
    let cmd_handler = CommandHandler::new()
        .await
        .whatever_context("command handler")?;
    let mut state = UvcUacPayload::default();
    let result: Result<(), snafu::Whatever> = (async {
        loop {
            let resp = cmd_handler
                .command(UvcUacPayload::default())
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

#[snafu::report]
#[tokio::main]
async fn main() -> Result<(), snafu::Whatever> {
    LocalSet::new().run_until(async {
        tokio::spawn(async move { uvc_handler().await.report() });

        ffmpeg_next::init().whatever_context("init ffmpeg")?;
        let mut input = input("/home/ruben/rick.mkv").whatever_context("load video")?;
        let input_video = input
            .streams()
            .best(Type::Video)
            .whatever_context("No video stream")?;
        let video_idx = input_video.index();
        let decoder_ctx = Context::from_parameters(input_video.parameters())
            .whatever_context("making video decoder ctx")?;
        // let input_audio = input.streams().best(Type::Audio)
        //     .whatever_context("No audio stream")?;
        // let audio_idx = input_audio.index();
        let mut decoder = decoder_ctx
            .decoder()
            .video()
            .whatever_context("video decoder")?;
        let streamer = Streamer::new().await.whatever_context("gamepad streamer")?;

        for (stream, packet) in input.packets() {
            if stream.index() != video_idx {
                continue;
            }
            decoder.send_packet(&packet).whatever_context("decoding")?;
            loop {
                let mut frame = frame::Video::empty();
                match decoder.receive_frame(&mut frame) {
                    Ok(()) => {
                        streamer
                            .push_frame(Box::new(MyFrame(frame)))
                            .whatever_context("streaming")?;
                        tokio::time::sleep(Duration::from_millis(1000 / 25)).await;
                    }
                    Err(ffmpeg_next::Error::Other { errno }) if errno == EAGAIN => break,
                    Err(e) => {
                        return Err(e).whatever_context("uh oh");
                    }
                }
            }
        }

        Ok(())
    }).await
}
