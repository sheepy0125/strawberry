use snafu::{ResultExt, Snafu, ensure};
use std::ffi::c_void;
use strawberry_x264::{Colorspace, Encoding, Image, Preset, Tune};
use x264_sys::{X264_ANALYSE_PSUB16x16, X264_CSP_I420, X264_KEYINT_MAX_INFINITE, X264_LOG_INFO, X264_RC_CQP, x264_nal_t, x264_t, X264_ME_UMH, X264_B_ADAPT_TRELLIS, X264_DIRECT_PRED_AUTO, X264_ANALYSE_PSUB8x8, X264_ANALYSE_I8x8, X264_RC_CRF, X264_RC_ABR};

pub struct Encoder {
    encoder: strawberry_x264::Encoder,
}

const WIDTH: i32 = 864;
const HEIGHT: i32 = 480;
const CHUNKS_PER_FRAME: i32 = 5;

impl Encoder {
    pub fn new() -> Result<Self, Error> {
        let mut builder = strawberry_x264::Setup::preset(Preset::Medium, Tune::None, false, true);
        unsafe {
            const ENABLE_INTRA_REFRESH: bool = true;
            let raw = builder.raw();

            // Old slow preset
            // raw.analyse.i_me_method = X264_ME_UMH as i32;
            // raw.analyse.i_subpel_refine = 8;
            // raw.i_frame_reference = 5;
            // raw.i_bframe_adaptive = X264_B_ADAPT_TRELLIS as i32;
            // raw.analyse.i_direct_mv_pred = X264_DIRECT_PRED_AUTO as i32;
            // raw.rc.i_lookahead = 50;
            //
            // old zerolatency preset
            // raw.rc.i_lookahead = 0;
            // raw.i_sync_lookahead = 0;
            // raw.i_bframe = 0;
            // raw.b_sliced_threads = 1;
            // raw.b_vfr_input = 0;
            // raw.rc.b_mb_tree = 0;

            raw.analyse.inter &= !X264_ANALYSE_PSUB16x16;
            if ENABLE_INTRA_REFRESH {
                raw.i_keyint_min = 10;
                raw.i_keyint_max = 30;
            } else {
                raw.i_keyint_min = X264_KEYINT_MAX_INFINITE as i32;
                raw.i_keyint_max = X264_KEYINT_MAX_INFINITE as i32;
            }
            raw.i_scenecut_threshold = -1;
            raw.b_cabac = 1;
            raw.b_interlaced = 0;
            raw.i_bframe = 0;
            raw.i_bframe_pyramid = 0;
            raw.i_frame_reference = 1;
            raw.b_constrained_intra = 1;
            raw.b_intra_refresh = ENABLE_INTRA_REFRESH as i32;
            raw.analyse.i_weighted_pred = 0;
            raw.analyse.b_weighted_bipred = 0;
            raw.analyse.b_transform_8x8 = 0;
            raw.analyse.i_chroma_qp_offset = 0;

            // Set QP = 32 for all frames.
            raw.rc.i_rc_method = X264_RC_CQP as i32;
            raw.rc.i_qp_constant = 32;
            raw.rc.i_qp_min = 32;
            raw.rc.i_qp_max = 32;
            raw.rc.f_ip_factor = 1.0;

            // Do not output SPS/PPS/SEI/unit delimeters.
            raw.b_repeat_headers = 0;
            raw.b_aud = 0;

            // return macroblock rows intead of NAL units
            // XXX x264 must also be modified to not produce macroblocks utilizing
            // planar prediction. this isn't toggleable at runtime for now...
            raw.b_drh_mode = 1;

            // Yield one complete frame serially.
            raw.i_threads = 1;
            raw.b_sliced_threads = 0;
            raw.i_slice_count = 1;

            raw.i_level_idc = 10;

            unsafe extern "C" fn process_nal_unit_trampoline(
                _handle: *mut x264_t,
                nal: *mut x264_nal_t,
                opaque: *mut c_void,
            ) {
                let ctx: &mut Context = unsafe { &mut *opaque.cast() };
                let nal = unsafe { &*nal };
                process_nal_unit(nal, ctx);
            }

            raw.nalu_process = Some(process_nal_unit_trampoline);
        }
        let encoder = builder
            .main()
            .build(Colorspace::I420, WIDTH, HEIGHT)
            .map_err(|_| Error::EncoderBuild)?;
        Ok(Self { encoder })
    }

    pub fn encode(&mut self, image: Image, resync: bool) -> Result<([&[u8]; 5], bool), Error> {
        let mut context: Context = Context::default();
        unsafe {
            self.encoder
                .encode_drh(image, resync, (&raw mut context).cast())
                .map_err(|_| Error::Encoder)?;
        }
        let chunks: [(*const u8, usize); 5] = context.chunk_array.try_into().map_err(|v: ChunkArray| Error::ChunkCount {length: v.len()})?;
        let chunks = chunks.map(|(ptr, size)| {
            unsafe {
                std::slice::from_raw_parts(ptr, size)
            }
        });
        Ok((chunks, context.is_idr))
    }
}

type ChunkArray = Vec<(*const u8, usize)>;

struct Context {
    chunk_array: ChunkArray,
    is_idr: bool,
}

impl Default for Context {
    fn default() -> Self {
        Self {
            chunk_array: Vec::with_capacity(5),
            is_idr: false,
        }
    }
}

const NAL_SEI: i32 = 6;
const NAL_PRIORITY_DISPOSABLE: i32 = 0;
const NAL_SLICE_IDR: i32 = 5;

fn process_nal_unit(nal: &x264_nal_t, ctx: &mut Context) {
    if nal.i_type == NAL_SEI {
        return;
    }
    let mb_per_frame = ((WIDTH + 15) / 16) * ((HEIGHT + 15) / 16);
    let mb_per_chunk = mb_per_frame / CHUNKS_PER_FRAME;
    let chunk_idx = nal.i_first_mb / mb_per_chunk;

    assert_eq!(chunk_idx, ctx.chunk_array.len() as i32);

    ctx.chunk_array.push((nal.p_payload, nal.i_payload as usize));

    assert!(ctx.chunk_array.len() <= 5);

    if ctx.chunk_array.len() == 5 {
        ctx.is_idr = nal.i_ref_idc != NAL_PRIORITY_DISPOSABLE && nal.i_type == NAL_SLICE_IDR;
    }
}

#[derive(Debug, Snafu)]
pub enum Error {
    /// error building x264 encoder
    EncoderBuild,
    /// encoding error
    Encoder,
    #[snafu(display("Unexpected number of chunks {length} != 5"))]
    ChunkCount { length: usize },
}
