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
    let decoder_ctx = Context::from_parameters(input_video.parameters())
        .whatever_context("making video decoder ctx")?;
    // let input_audio = input.streams().best(Type::Audio)
    //     .whatever_context("No audio stream")?;
    // let audio_idx = input_audio.index();
    let mut decoder = decoder_ctx
        .decoder()
        .video()
        .whatever_context("video decoder")?;

    let mut scaler = scaling::context::Context::get(
        decoder.format(),
        decoder.width(),
        decoder.height(),
        Pixel::BGRA,
        decoder.width(),
        decoder.height(),
        Flags::BILINEAR
    ).whatever_context("making scaler")?;
    let mut frame = frame::Video::empty();
    for (stream, packet) in input.packets() {
        if stream.index() != video_idx {
            continue;
        }
        decoder.send_packet(&packet).whatever_context("decoding")?;
        loop {
            match decoder.receive_frame(&mut frame) {
                Ok(()) => {
                    let mut rgb_frame = frame::Video::empty();
                    scaler.run(&frame, &mut rgb_frame).whatever_context("conversion")?;
                    let data = rgb_frame.data(0);
                    unsafe {
                        libdrc::drc_push_vid_frame(streamer, data.as_ptr(), data.len() as c_uint, decoder.width() as c_ushort, decoder.height() as c_ushort, drc_pixel_format_DRC_BGRA, drc_flipping_mode_DRC_NO_FLIP);
                    };
                    std::thread::sleep(Duration::from_micros((1000000.0 / 25.0) as u64));
                }
                Err(ffmpeg_next::Error::Other { errno }) if errno == EAGAIN => break,
                Err(e) => {
                    return Err(e).whatever_context("uh oh");
                }
            }
        }
    }
    unsafe {
        drc_delete_streamer(streamer);
    }
    Ok(())
}
