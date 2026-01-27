//! Inspect command implementation.

use crate::annotator::{annotate_file, AnnotateOptions};
use crate::output;
use jxl_oxide::JxlImage;
use std::path::Path;

/// Run the inspect command.
pub fn run_inspect(
    input: &Path,
    output_dir: &Path,
    include_ans: bool,
    include_checkpoints: bool,
    max_depth: usize,
    frame_filter: Option<&[u32]>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    tracing::info!("Inspecting {}", input.display());

    let options = AnnotateOptions {
        include_ans,
        include_checkpoints,
        max_depth,
        frame_filter: frame_filter.map(|f| f.to_vec()),
    };

    let result = annotate_file(input, &options)?;

    // Write output
    output::write_annotations(output_dir, &result)?;

    tracing::info!(
        "Wrote {} segments to {}",
        result.segments.len(),
        output_dir.display()
    );

    Ok(())
}

/// Run the info command.
pub fn run_info(input: &Path, json_output: bool) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    let data = std::fs::read(input)?;
    let image = JxlImage::builder().read(&*data)?;
    let header = image.image_header();

    if json_output {
        let info = serde_json::json!({
            "file": input.to_string_lossy(),
            "size_bytes": data.len(),
            "image": {
                "width": header.size.width,
                "height": header.size.height,
                "bit_depth": header.metadata.bit_depth.bits_per_sample(),
                "grayscale": header.metadata.grayscale(),
                "xyb_encoded": header.metadata.xyb_encoded,
                "orientation": header.metadata.orientation,
                "extra_channels": header.metadata.ec_info.len(),
                "animation": header.metadata.animation.as_ref().map(|a| {
                    serde_json::json!({
                        "tps_numerator": a.tps_numerator,
                        "tps_denominator": a.tps_denominator,
                        "num_loops": a.num_loops,
                    })
                }),
            },
        });
        println!("{}", serde_json::to_string_pretty(&info)?);
    } else {
        println!("File: {}", input.display());
        println!("Size: {} bytes", data.len());
        println!();
        println!("Image:");
        println!("  Dimensions: {}x{}", header.size.width, header.size.height);
        println!(
            "  Bit depth: {} bits",
            header.metadata.bit_depth.bits_per_sample()
        );
        println!(
            "  Color: {}",
            if header.metadata.grayscale() {
                "Grayscale"
            } else {
                "Color"
            }
        );
        println!(
            "  XYB encoded: {}",
            if header.metadata.xyb_encoded {
                "Yes"
            } else {
                "No"
            }
        );
        println!("  Orientation: {}", header.metadata.orientation);
        println!(
            "  Extra channels: {}",
            header.metadata.ec_info.len()
        );

        if let Some(anim) = &header.metadata.animation {
            println!();
            println!("Animation:");
            println!(
                "  TPS: {}/{}",
                anim.tps_numerator, anim.tps_denominator
            );
            println!("  Loops: {}", anim.num_loops);
        }

        // Get first frame info
        if let Some(frame) = image.frame_by_keyframe(0) {
            let frame_header = frame.header();

            println!();
            println!("Frame 0:");
            println!(
                "  Encoding: {}",
                if frame_header.encoding == jxl_frame::header::Encoding::VarDct {
                    "VarDCT"
                } else {
                    "Modular"
                }
            );
            println!("  Size: {}x{}", frame_header.width, frame_header.height);
            println!("  Passes: {}", frame_header.passes.num_passes);
            println!("  LF groups: {}", frame_header.num_lf_groups());
            println!("  Groups: {}", frame_header.num_groups());

            if frame_header.encoding == jxl_frame::header::Encoding::VarDct {
                println!("  X QM scale: {}", frame_header.x_qm_scale);
                println!("  B QM scale: {}", frame_header.b_qm_scale);
            }
        }
    }

    Ok(())
}
