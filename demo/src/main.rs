use drc::cmd::data::UvcUacPayload;
use drc::cmd::{generic, CommandHandler};
use drc::frame::Frame;
use drc::Streamer;
use ffmpeg_next::codec::Context;
use ffmpeg_next::ffi::EAGAIN;
use ffmpeg_next::format::input;
use ffmpeg_next::media::Type;
use ffmpeg_next::{format, frame};
use snafu::OptionExt;
use snafu::{Report, ResultExt};
use std::process::Termination;
use std::time::Duration;
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
        if cfg!(debug_assertions) {
            Image::new(Colorspace::I420, self.0.width() as i32, self.0.height() as i32, &planes)
        } else {
            unsafe {
                Image::new_unchecked(Colorspace::I420.into(), self.0.width() as i32, self.0.height() as i32, &planes)
            }
        }
    }
}

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
    let resp = cmd_handler.command(&generic::GetUicFirmware).await.whatever_context("get uic firmware")?;
    eprintln!("{resp:?}");
    tokio::spawn(async move { uvc_handler(cmd_handler).await.report() });
    Ok(())
}

#[snafu::report]
#[tokio::main]
async fn main() -> Result<(), snafu::Whatever> {
    launch_uvc().await?;

    ffmpeg_next::init().whatever_context("init ffmpeg")?;
    let mut input = input("/home/ruben/rick.mkv").whatever_context("load video")?;
    let input_video = input
        .streams()
        .best(Type::Video)
        .whatever_context("No video stream")?;
    let video_idx = input_video.index();
    let video_ctx = Context::from_parameters(input_video.parameters())
        .whatever_context("video ctx")?;
    let input_audio = input.streams().best(Type::Audio)
        .whatever_context("No audio stream")?;
    let audio_idx = input_audio.index();
    let mut video_decoder = video_ctx
        .decoder()
        .video()
        .whatever_context("video decoder")?;
    let audio_ctx = Context::from_parameters(input_audio.parameters()).whatever_context("audio ctx")?;
    let mut audio_decoder = audio_ctx.decoder().audio().whatever_context("audio decoder")?;
    let streamer = Streamer::new().await.whatever_context("gamepad streamer")?;

    for (stream, packet) in input.packets() {
        if stream.index() == video_idx {
            video_decoder.send_packet(&packet).whatever_context("decoding video")?;
            loop {
                let mut frame = frame::Video::empty();
                match video_decoder.receive_frame(&mut frame) {
                    Ok(()) => {
                        streamer
                            .push_frame(MyFrame(frame))
                            .whatever_context("streaming")?;
                        tokio::time::sleep(Duration::from_millis(1000 / 25)).await;
                    }
                    Err(ffmpeg_next::Error::Other { errno }) if errno == EAGAIN => break,
                    Err(e) => {
                        return Err(e).whatever_context("uh oh");
                    }
                }
            }
        } else if stream.index() == audio_idx {
            audio_decoder.send_packet(&packet).whatever_context("decoding audio")?;
            loop {
                let mut frame = frame::Audio::empty();
                match audio_decoder.receive_frame(&mut frame) {
                    Ok(()) => {
                        let data = frame.plane::<f32>(0).iter()
                            .zip(frame.plane::<f32>(1))
                            .flat_map(|(l, r)| [*l, *r])
                            .map(|s| (s * (i16::MAX as f32)) as i16)
                            .flat_map(|s| s.to_le_bytes());
                        streamer.push_audio(data);
                    }
                    Err(ffmpeg_next::Error::Other { errno }) if errno == EAGAIN => break,
                    Err(e) => {
                        return Err(e).whatever_context("uh oh");
                    }
                }
            }
        }
    }

    Ok(())
}
