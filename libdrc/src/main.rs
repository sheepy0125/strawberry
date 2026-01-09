use std::ffi::{c_uint, c_ushort};
use std::time::Duration;
use ffmpeg_next::codec::Context;
use ffmpeg_next::ffi::EAGAIN;
use ffmpeg_next::format::{input, Pixel};
use ffmpeg_next::frame;
use ffmpeg_next::media::Type;
use ffmpeg_next::software::scaling;
use ffmpeg_next::software::scaling::Flags;
use snafu::{OptionExt, ResultExt};
use crate::libdrc::{drc_delete_streamer, drc_flipping_mode_DRC_NO_FLIP, drc_pixel_format_DRC_BGRA, drc_shutdown_pad};

mod libdrc;

fn main() -> Result<(), snafu::Whatever> {
    let streamer = unsafe {
        let streamer = libdrc::drc_new_streamer();
        let result = libdrc::drc_start_streamer(streamer);
        assert_eq!(result, 1);
        streamer
    };
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

    let mut scaler = scaling::context::Context::get(
        video_decoder.format(),
        video_decoder.width(),
        video_decoder.height(),
        Pixel::BGRA,
        video_decoder.width(),
        video_decoder.height(),
        Flags::BILINEAR
    ).whatever_context("making scaler")?;
    let mut frame = frame::Video::empty();
    for (stream, packet) in input.packets() {
        if stream.index() == video_idx {
            video_decoder.send_packet(&packet).whatever_context("decoding")?;
            loop {
                match video_decoder.receive_frame(&mut frame) {
                    Ok(()) => {
                        let mut rgb_frame = frame::Video::empty();
                        scaler.run(&frame, &mut rgb_frame).whatever_context("conversion")?;
                        let data = rgb_frame.data(0);
                        unsafe {
                            libdrc::drc_push_vid_frame(streamer, data.as_ptr(), data.len() as c_uint, video_decoder.width() as c_ushort, video_decoder.height() as c_ushort, drc_pixel_format_DRC_BGRA, drc_flipping_mode_DRC_NO_FLIP);
                        };
                        std::thread::sleep(Duration::from_micros((1000000.0 / 25.0) as u64));
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
                        let data: Vec<i16> = frame.plane::<f32>(0).iter()
                            .zip(frame.plane::<f32>(1))
                            .flat_map(|(l, r)| [*l, *r])
                            .map(|s| (s * (i16::MAX as f32)) as i16)
                            .collect();
                        unsafe {
                            libdrc::drc_push_aud_frame(streamer, data.as_ptr(), data.len() as u32);
                        }
                    }
                    Err(ffmpeg_next::Error::Other { errno }) if errno == EAGAIN => break,
                    Err(e) => {
                        return Err(e).whatever_context("uh oh");
                    }
                }
            }
        }
    }
    unsafe {
        drc_delete_streamer(streamer);
    }
    Ok(())
}
