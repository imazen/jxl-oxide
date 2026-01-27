//! Inspect command implementation.

use crate::annotator::{annotate_file, get_vardct_annotations, AnnotateOptions};
use crate::output;
use jxl_oxide::JxlImage;
use std::collections::HashMap;
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
pub fn run_info(input: &Path, json_output: bool, per_frame: bool) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    let data = std::fs::read(input)?;
    let image = JxlImage::builder().read(&*data)?;
    let header = image.image_header();

    if json_output {
        // Collect frame info with optional per-frame VarDCT stats
        let mut frames_json = Vec::new();
        let mut all_vardct_anns = Vec::new();

        for frame_idx in 0..image.num_loaded_frames() {
            if let Some(frame) = image.frame(frame_idx) {
                let fh = frame.header();
                let frame_anns = get_vardct_annotations(&image, frame_idx).ok();

                let frame_vardct_stats = if per_frame {
                    frame_anns.as_ref().map(|anns| compute_vardct_stats_json(anns))
                } else {
                    None
                };

                if let Some(anns) = frame_anns {
                    all_vardct_anns.extend(anns);
                }

                let mut frame_json = serde_json::json!({
                    "index": frame_idx,
                    "encoding": if fh.encoding == jxl_frame::header::Encoding::VarDct { "VarDCT" } else { "Modular" },
                    "width": fh.width,
                    "height": fh.height,
                    "passes": fh.passes.num_passes,
                    "lf_groups": fh.num_lf_groups(),
                    "groups": fh.num_groups(),
                });

                if let Some(stats) = frame_vardct_stats {
                    frame_json["vardct_stats"] = stats;
                }

                frames_json.push(frame_json);
            }
        }

        // Aggregate VarDCT stats
        let vardct_stats = if !all_vardct_anns.is_empty() {
            Some(compute_vardct_stats_json(&all_vardct_anns))
        } else {
            None
        };

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
            "frames": frames_json,
            "vardct_stats": vardct_stats,
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

        // Check for additional frames
        let total_frames = image.num_loaded_frames();
        if total_frames > 1 {
            println!();
            println!("Total frames: {}", total_frames);
        }

        // Get VarDCT stats from all frames
        let mut all_vardct_anns = Vec::new();
        for frame_idx in 0..total_frames {
            if let Ok(anns) = get_vardct_annotations(&image, frame_idx) {
                if per_frame && !anns.is_empty() {
                    println!();
                    println!("Frame {} VarDCT Statistics:", frame_idx);
                    print_vardct_stats_compact(&anns);
                }
                all_vardct_anns.extend(anns);
            }
        }

        // Print overall stats
        if !all_vardct_anns.is_empty() {
            if per_frame && total_frames > 1 {
                println!();
                println!("Overall VarDCT Statistics:");
            }
            print_vardct_stats(&all_vardct_anns);
        }
    }

    Ok(())
}

/// Print VarDCT statistics from annotations.
fn print_vardct_stats(annotations: &[jxl_bitstream::annotate::HfMetadataAnnotation]) {
    if annotations.is_empty() {
        return;
    }

    let mut total_blocks = 0u32;
    let mut dct_type_counts: HashMap<String, u32> = HashMap::new();
    let mut hf_mul_sum = 0i64;
    let mut hf_mul_min = i32::MAX;
    let mut hf_mul_max = i32::MIN;

    for ann in annotations {
        total_blocks += ann.num_varblocks;
        for block in &ann.varblocks {
            *dct_type_counts.entry(block.dct_select.clone()).or_default() += 1;
            hf_mul_sum += block.hf_mul as i64;
            hf_mul_min = hf_mul_min.min(block.hf_mul);
            hf_mul_max = hf_mul_max.max(block.hf_mul);
        }
    }

    println!();
    println!("VarDCT Statistics:");
    println!("  Total varblocks: {}", total_blocks);

    // Sort DCT types by count
    let mut dct_types: Vec<_> = dct_type_counts.into_iter().collect();
    dct_types.sort_by(|a, b| b.1.cmp(&a.1));

    println!("  DCT transform types:");
    for (dct_type, count) in dct_types.iter().take(8) {
        let pct = (*count as f64 / total_blocks as f64) * 100.0;
        println!("    {:15} {:6} ({:5.1}%)", dct_type, count, pct);
    }
    if dct_types.len() > 8 {
        println!("    ... and {} more types", dct_types.len() - 8);
    }

    if total_blocks > 0 {
        let hf_mul_avg = hf_mul_sum as f64 / total_blocks as f64;
        println!("  HF multiplier: min={}, max={}, avg={:.1}", hf_mul_min, hf_mul_max, hf_mul_avg);
    }
}

/// Print compact VarDCT statistics (for per-frame output).
fn print_vardct_stats_compact(annotations: &[jxl_bitstream::annotate::HfMetadataAnnotation]) {
    if annotations.is_empty() {
        return;
    }

    let mut total_blocks = 0u32;
    let mut dct_type_counts: HashMap<String, u32> = HashMap::new();
    let mut hf_mul_sum = 0i64;
    let mut hf_mul_min = i32::MAX;
    let mut hf_mul_max = i32::MIN;

    for ann in annotations {
        total_blocks += ann.num_varblocks;
        for block in &ann.varblocks {
            *dct_type_counts.entry(block.dct_select.clone()).or_default() += 1;
            hf_mul_sum += block.hf_mul as i64;
            hf_mul_min = hf_mul_min.min(block.hf_mul);
            hf_mul_max = hf_mul_max.max(block.hf_mul);
        }
    }

    // Sort DCT types by count
    let mut dct_types: Vec<_> = dct_type_counts.into_iter().collect();
    dct_types.sort_by(|a, b| b.1.cmp(&a.1));

    // Compact output: blocks count, top 3 DCT types, HF mul range
    let top_types: Vec<String> = dct_types
        .iter()
        .take(3)
        .map(|(t, c)| format!("{} ({:.0}%)", t, (*c as f64 / total_blocks as f64) * 100.0))
        .collect();

    if total_blocks > 0 {
        let hf_mul_avg = hf_mul_sum as f64 / total_blocks as f64;
        println!(
            "  Blocks: {}  Top types: {}  HF mul: {}-{} (avg {:.1})",
            total_blocks,
            top_types.join(", "),
            hf_mul_min,
            hf_mul_max,
            hf_mul_avg
        );
    }
}

/// Compute VarDCT statistics as JSON.
fn compute_vardct_stats_json(
    annotations: &[jxl_bitstream::annotate::HfMetadataAnnotation],
) -> serde_json::Value {
    let mut total_blocks = 0u32;
    let mut dct_type_counts: HashMap<String, u32> = HashMap::new();
    let mut hf_mul_sum = 0i64;
    let mut hf_mul_min = i32::MAX;
    let mut hf_mul_max = i32::MIN;

    for ann in annotations {
        total_blocks += ann.num_varblocks;
        for block in &ann.varblocks {
            *dct_type_counts.entry(block.dct_select.clone()).or_default() += 1;
            hf_mul_sum += block.hf_mul as i64;
            hf_mul_min = hf_mul_min.min(block.hf_mul);
            hf_mul_max = hf_mul_max.max(block.hf_mul);
        }
    }

    // Sort DCT types by count
    let mut dct_types: Vec<_> = dct_type_counts.into_iter().collect();
    dct_types.sort_by(|a, b| b.1.cmp(&a.1));

    let dct_distribution: Vec<_> = dct_types
        .iter()
        .map(|(dct_type, count)| {
            serde_json::json!({
                "type": dct_type,
                "count": count,
                "percentage": (*count as f64 / total_blocks as f64) * 100.0,
            })
        })
        .collect();

    let hf_mul_avg = if total_blocks > 0 {
        hf_mul_sum as f64 / total_blocks as f64
    } else {
        0.0
    };

    serde_json::json!({
        "total_varblocks": total_blocks,
        "dct_distribution": dct_distribution,
        "hf_multiplier": {
            "min": if total_blocks > 0 { hf_mul_min } else { 0 },
            "max": if total_blocks > 0 { hf_mul_max } else { 0 },
            "avg": hf_mul_avg,
        },
    })
}
