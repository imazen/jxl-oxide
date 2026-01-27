//! Inspect command implementation.

use crate::annotator::{annotate_file, get_quantization_params, get_vardct_annotations, AnnotateOptions};
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
pub fn run_info(input: &Path, json_output: bool, per_frame: bool, summary: bool) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    let data = std::fs::read(input)?;
    let image = JxlImage::builder().read(&*data)?;
    let header = image.image_header();

    // Summary mode: one-line output
    if summary {
        let num_frames = image.num_loaded_frames();
        let encoding = if let Some(frame) = image.frame_by_keyframe(0) {
            if frame.header().encoding == jxl_frame::header::Encoding::VarDct {
                "VarDCT"
            } else {
                "Modular"
            }
        } else {
            "?"
        };

        // Get VarDCT block count if applicable
        let mut total_blocks = 0u32;
        let mut top_dct = String::new();
        for frame_idx in 0..num_frames {
            if let Ok(anns) = get_vardct_annotations(&image, frame_idx) {
                for ann in &anns {
                    total_blocks += ann.num_varblocks;
                }
            }
        }
        if total_blocks > 0 {
            // Get top DCT type
            let mut dct_counts: HashMap<String, u32> = HashMap::new();
            for frame_idx in 0..num_frames {
                if let Ok(anns) = get_vardct_annotations(&image, frame_idx) {
                    for ann in &anns {
                        for block in &ann.varblocks {
                            *dct_counts.entry(block.dct_select.clone()).or_default() += 1;
                        }
                    }
                }
            }
            if let Some((top, count)) = dct_counts.iter().max_by_key(|(_, c)| *c) {
                let pct = (*count as f64 / total_blocks as f64) * 100.0;
                top_dct = format!(" {}:{:.0}%", top, pct);
            }
        }

        let anim_str = if header.metadata.animation.is_some() {
            format!(" {}f", num_frames)
        } else {
            String::new()
        };

        let block_str = if total_blocks > 0 {
            format!(" {}blk{}", total_blocks, top_dct)
        } else {
            String::new()
        };

        println!(
            "{}: {}x{} {}bit {} {}{}{} {}B",
            input.file_name().unwrap_or_default().to_string_lossy(),
            header.size.width,
            header.size.height,
            header.metadata.bit_depth.bits_per_sample(),
            if header.metadata.grayscale() { "gray" } else { "color" },
            encoding,
            anim_str,
            block_str,
            data.len()
        );
        return Ok(());
    }

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

                // Add VarDCT-specific fields
                if fh.encoding == jxl_frame::header::Encoding::VarDct {
                    frame_json["x_qm_scale"] = serde_json::json!(fh.x_qm_scale);
                    frame_json["b_qm_scale"] = serde_json::json!(fh.b_qm_scale);
                    let epf_iters = match &fh.restoration_filter.epf {
                        jxl_frame::filter::EdgePreservingFilter::Disabled => 0,
                        jxl_frame::filter::EdgePreservingFilter::Enabled(params) => params.iters,
                    };
                    frame_json["epf_iters"] = serde_json::json!(epf_iters);

                    // Add quantization parameters
                    if let Some(qparams) = get_quantization_params(&image, frame_idx) {
                        frame_json["quantization"] = serde_json::json!({
                            "global_scale": qparams.global_scale,
                            "quant_lf": qparams.quant_lf,
                            "dc_quant_step": qparams.dc_quant_step,
                        });
                    }
                }

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

        // Get first VarDCT frame info (or first frame if none)
        let mut first_vardct_idx: Option<usize> = None;
        for frame_idx in 0..image.num_loaded_frames() {
            if let Some(frame) = image.frame(frame_idx)
                && frame.header().encoding == jxl_frame::header::Encoding::VarDct
            {
                first_vardct_idx = Some(frame_idx);
                break;
            }
        }

        // Show first frame (preferring VarDCT if present)
        let display_frame_idx = first_vardct_idx.unwrap_or(0);
        if let Some(frame) = image.frame(display_frame_idx) {
            let frame_header = frame.header();

            println!();
            println!("Frame {}:", display_frame_idx);
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
                let epf_iters = match &frame_header.restoration_filter.epf {
                    jxl_frame::filter::EdgePreservingFilter::Disabled => 0,
                    jxl_frame::filter::EdgePreservingFilter::Enabled(params) => params.iters,
                };
                println!("  EPF iterations: {}", epf_iters);

                // Show quantization parameters
                if let Some(qparams) = get_quantization_params(&image, display_frame_idx) {
                    println!("  Global scale: {}", qparams.global_scale);
                    println!("  Quant LF: {}", qparams.quant_lf);
                    println!("  DC quant step: {:.6}", qparams.dc_quant_step);
                }
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

/// Run the hexdump command.
pub fn run_hexdump(
    input: &Path,
    bytes_limit: Option<usize>,
    offset: usize,
) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    let data = std::fs::read(input)?;

    // Calculate the range to display
    let start = offset.min(data.len());
    let end = match bytes_limit {
        Some(limit) => (start + limit).min(data.len()),
        None => data.len(),
    };
    let slice = &data[start..end];

    // Get basic annotations for the file
    let options = AnnotateOptions::default();
    let annotations = match annotate_file(input, &options) {
        Ok(result) => {
            // Flatten all annotations from all segments
            result
                .segments
                .iter()
                .flat_map(|s| s.annotations.iter().cloned())
                .collect::<Vec<_>>()
        }
        Err(_) => Vec::new(),
    };

    // Filter annotations to the visible range
    let visible_annotations: Vec<_> = annotations
        .iter()
        .filter(|ann| {
            let ann_start = (ann.bit_start / 8) as usize;
            let ann_end = ((ann.bit_start + ann.bit_length as u64).div_ceil(8)) as usize;
            ann_start < end && ann_end > start
        })
        .cloned()
        .collect();

    // Print header
    println!("File: {}", input.display());
    println!("Size: {} bytes", data.len());
    if offset > 0 || bytes_limit.is_some() {
        println!("Showing bytes {}-{}", start, end);
    }
    println!();

    // Print hex dump with annotations
    let hex_output = output::format_hex_annotated(slice, &visible_annotations);
    print!("{}", hex_output);

    Ok(())
}

/// Export VarDCT stats to CSV.
pub fn run_export_csv(
    input: &Path,
    output_path: &Path,
    per_block: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    let data = std::fs::read(input)?;
    let image = JxlImage::builder().read(&*data)?;

    let mut output = String::new();

    if per_block {
        // Export per-block data
        output.push_str("frame_idx,lf_group_idx,block_x,block_y,dct_type,size_w,size_h,hf_mul\n");

        for frame_idx in 0..image.num_loaded_frames() {
            if let Ok(anns) = get_vardct_annotations(&image, frame_idx) {
                for ann in &anns {
                    for block in &ann.varblocks {
                        output.push_str(&format!(
                            "{},{},{},{},{},{},{},{}\n",
                            frame_idx,
                            ann.lf_group_idx,
                            block.block_x,
                            block.block_y,
                            block.dct_select,
                            block.size_blocks.0,
                            block.size_blocks.1,
                            block.hf_mul
                        ));
                    }
                }
            }
        }
    } else {
        // Export summary stats per frame
        output.push_str("frame_idx,encoding,total_blocks,");
        output.push_str("dct8_pct,dct16_pct,dct32_pct,dct8x16_pct,dct16x8_pct,");
        output.push_str("hf_mul_min,hf_mul_max,hf_mul_avg\n");

        for frame_idx in 0..image.num_loaded_frames() {
            let Some(frame) = image.frame(frame_idx) else { continue };
            let fh = frame.header();
            let encoding = if fh.encoding == jxl_frame::header::Encoding::VarDct {
                "VarDCT"
            } else {
                "Modular"
            };

            if let Ok(anns) = get_vardct_annotations(&image, frame_idx) {
                if anns.is_empty() {
                    output.push_str(&format!("{},{},0,0,0,0,0,0,0,0,0\n", frame_idx, encoding));
                    continue;
                }

                let mut total = 0u32;
                let mut dct_counts: HashMap<String, u32> = HashMap::new();
                let mut hf_sum = 0i64;
                let mut hf_min = i32::MAX;
                let mut hf_max = i32::MIN;

                for ann in &anns {
                    total += ann.num_varblocks;
                    for block in &ann.varblocks {
                        *dct_counts.entry(block.dct_select.clone()).or_default() += 1;
                        hf_sum += block.hf_mul as i64;
                        hf_min = hf_min.min(block.hf_mul);
                        hf_max = hf_max.max(block.hf_mul);
                    }
                }

                let get_pct = |name: &str| -> f64 {
                    if total == 0 { return 0.0; }
                    (*dct_counts.get(name).unwrap_or(&0) as f64 / total as f64) * 100.0
                };

                let hf_avg = if total > 0 { hf_sum as f64 / total as f64 } else { 0.0 };

                output.push_str(&format!(
                    "{},{},{},{:.1},{:.1},{:.1},{:.1},{:.1},{},{},{:.2}\n",
                    frame_idx, encoding, total,
                    get_pct("Dct8"), get_pct("Dct16"), get_pct("Dct32"),
                    get_pct("Dct8x16"), get_pct("Dct16x8"),
                    hf_min, hf_max, hf_avg
                ));
            } else {
                output.push_str(&format!("{},{},0,0,0,0,0,0,0,0,0\n", frame_idx, encoding));
            }
        }
    }

    std::fs::write(output_path, output)?;
    println!("Exported to {}", output_path.display());

    Ok(())
}

/// Visualize block strategy map as ASCII art.
pub fn run_block_map(
    input: &Path,
    frame_idx_opt: Option<usize>,
    max_width: usize,
) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    let data = std::fs::read(input)?;
    let image = JxlImage::builder().read(&*data)?;

    // Find frame to display
    let frame_idx = frame_idx_opt.unwrap_or_else(|| {
        for idx in 0..image.num_loaded_frames() {
            if let Some(frame) = image.frame(idx)
                && frame.header().encoding == jxl_frame::header::Encoding::VarDct
            {
                return idx;
            }
        }
        0
    });

    let Some(frame) = image.frame(frame_idx) else {
        return Err(format!("Frame {} not found", frame_idx).into());
    };

    let fh = frame.header();
    if fh.encoding != jxl_frame::header::Encoding::VarDct {
        return Err(format!("Frame {} is Modular, not VarDCT", frame_idx).into());
    }

    // Get block annotations
    let anns = get_vardct_annotations(&image, frame_idx)?;
    if anns.is_empty() {
        return Err("No VarDCT block annotations found".into());
    }

    // Determine image size in 8x8 blocks
    let width_blocks = fh.width.div_ceil(8);
    let height_blocks = fh.height.div_ceil(8);

    // Build a map of block positions to DCT types
    let mut block_map: HashMap<(u32, u32), char> = HashMap::new();
    for ann in &anns {
        for block in &ann.varblocks {
            // Map DCT type to a character
            let ch = match block.dct_select.as_str() {
                "Dct8" => '8',
                "Dct16" => 'G', // 16 in hex
                "Dct32" => 'T', // 32 = Thirty-two
                "Dct64" => 'S', // 64 = Sixty-four
                "Dct8x16" | "Dct16x8" => 'R', // Rectangle
                "Dct8x32" | "Dct32x8" => 'r',
                "Dct16x32" | "Dct32x16" => 'W', // Wide
                "Dct32x64" | "Dct64x32" => 'w',
                "Dct4x8" | "Dct8x4" => 's', // small
                "Afv0" | "Afv1" | "Afv2" | "Afv3" => 'A', // AFV
                "Hornuss" => 'H',
                "Dct4" => '4',
                "Dct2x2" => '2',
                _ => '?',
            };

            // Fill the block area
            let (sw, sh) = block.size_blocks;
            for dy in 0..sh {
                for dx in 0..sw {
                    block_map.insert((block.block_x + dx, block.block_y + dy), ch);
                }
            }
        }
    }

    // Calculate scale factor to fit in max_width
    let scale = if width_blocks as usize > max_width {
        (width_blocks as usize).div_ceil(max_width)
    } else {
        1
    };

    let display_width = (width_blocks as usize).div_ceil(scale);
    let display_height = (height_blocks as usize).div_ceil(scale);

    println!("Block Strategy Map for frame {} ({}x{} blocks, scale 1:{})",
             frame_idx, width_blocks, height_blocks, scale);
    println!("Legend: 8=Dct8 G=Dct16 T=Dct32 S=Dct64 R=8x16/16x8 W=16x32/32x16 A=AFV s=small");
    println!();

    // Print the map
    for y in 0..display_height {
        let mut line = String::new();
        for x in 0..display_width {
            // Sample the block at the center of this display cell
            let bx = (x * scale + scale / 2) as u32;
            let by = (y * scale + scale / 2) as u32;
            let ch = block_map.get(&(bx, by)).copied().unwrap_or('.');
            line.push(ch);
        }
        println!("{}", line);
    }

    Ok(())
}
