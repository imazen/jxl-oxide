// Regression tests for valid JPEG XL bitstreams that older versions of
// jxl-oxide rejected with `UnexpectedEof` errors. The fixtures here are
// produced by third-party encoders (specifically `jxl-encoder` from
// imazen/jxl-encoder) and accepted by both the libjxl reference decoder
// (`djxl`) and `jxl-rs`. They exercise corner cases in modular sub-bitstream
// parsing where every channel of a section is deferred to PassGroups, leaving
// the global modular section without per-pixel symbols (and, for some
// encoders, without a local MA tree).
//
// See `crates/jxl-modular/src/lib.rs` (`read_and_validate_local_modular_header`)
// and `crates/jxl-modular/src/image.rs` (`TransformedModularSubimage::decode_inner`)
// for the matching libjxl-parity early-outs that fix these cases.

use jxl_oxide::{AllocTracker, JxlImage};

fn decode_to_completion(data: &[u8]) {
    let image = JxlImage::builder()
        .alloc_tracker(AllocTracker::with_limit(128 * 1024 * 1024))
        .read(std::io::Cursor::new(data))
        .expect("Failed to parse JXL header");

    let header = image.image_header();
    let width = header.size.width;
    let height = header.size.height;
    assert!(
        width > 0 && height > 0,
        "image must have non-zero dimensions"
    );

    assert!(
        image.num_loaded_keyframes() > 0,
        "expected at least one keyframe"
    );

    for keyframe_idx in 0..image.num_loaded_keyframes() {
        let frame = image
            .render_frame(keyframe_idx)
            .expect("Failed to render frame; this likely re-introduces an empty-modular-section EOF regression");

        let planar = frame.image_planar();
        assert!(
            !planar.is_empty(),
            "rendered frame should have at least one channel"
        );
        for stream in planar.iter() {
            assert!(
                stream.width() > 0 && stream.height() > 0,
                "rendered channel must have non-zero dimensions"
            );
        }
    }
}

/// A 512x512 RGBA VarDCT image (`d=1.0`, `e=7`) produced by jxl-encoder. The
/// alpha extra channel is multi-group (>group_dim on at least one axis), so
/// jxl-encoder's modular global section carries only the GroupHeader (4 bits)
/// and defers every channel to PassGroups. Older jxl-oxide attempted to parse
/// a local MA tree from the empty section and failed with `UnexpectedEof`
/// inside `MaConfig::parse`. djxl and jxl-rs accept the same bitstream.
#[test]
fn multigroup_vardct_alpha_empty_global() {
    let data = include_bytes!("multigroup_vardct_alpha_empty_global.jxl");
    decode_to_completion(data);
}
