//! Output formatting and file writing for annotations.

use crate::annotator::AnnotationResult;
use jxl_bitstream::annotate::{SegmentFileRef, SegmentKind};
use std::fs;
use std::path::Path;

/// Write annotations to output directory.
pub fn write_annotations(
    output_dir: &Path,
    result: &AnnotationResult,
) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    // Create output directory structure
    fs::create_dir_all(output_dir)?;
    fs::create_dir_all(output_dir.join("segments"))?;

    let mut segment_refs = Vec::new();

    // Write each segment to its own file
    for segment in &result.segments {
        let (path, kind_str) = segment_path(&segment.kind);
        let full_path = output_dir.join("segments").join(&path);

        // Create parent directories if needed
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let json = serde_json::to_string_pretty(segment)?;
        fs::write(&full_path, json)?;

        segment_refs.push(SegmentFileRef {
            kind: kind_str,
            path: Path::new("segments").join(&path),
            bit_range: segment.bit_range,
        });
    }

    // Write manifest
    let mut manifest = result.manifest.clone();
    manifest.segment_files = segment_refs;

    let manifest_json = serde_json::to_string_pretty(&manifest)?;
    fs::write(output_dir.join("manifest.json"), manifest_json)?;

    // Write checkpoints if present
    if !result.checkpoints.is_empty() {
        let checkpoints_json = serde_json::to_string_pretty(&result.checkpoints)?;
        fs::write(output_dir.join("checkpoints.json"), checkpoints_json)?;
    }

    // Write VarDCT annotations if present
    for ann in &result.vardct_annotations {
        let path = output_dir.join(format!(
            "segments/frame_{}/lf_group_{}/vardct_blocks.json",
            ann.frame_idx, ann.lf_group_idx
        ));
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(ann)?;
        fs::write(&path, json)?;
    }

    Ok(())
}

/// Get the file path for a segment kind.
fn segment_path(kind: &SegmentKind) -> (String, String) {
    match kind {
        SegmentKind::Container { box_type } => {
            (format!("container_{}.json", box_type), "Container".to_string())
        }
        SegmentKind::Signature => ("signature.json".to_string(), "Signature".to_string()),
        SegmentKind::ImageHeader => ("image_header.json".to_string(), "ImageHeader".to_string()),
        SegmentKind::FrameHeader { frame_idx, encoding } => (
            format!("frame_{}/header.json", frame_idx),
            format!("FrameHeader({})", encoding),
        ),
        SegmentKind::Toc { frame_idx } => (
            format!("frame_{}/toc.json", frame_idx),
            "Toc".to_string(),
        ),
        SegmentKind::LfGlobal { frame_idx } => (
            format!("frame_{}/lf_global.json", frame_idx),
            "LfGlobal".to_string(),
        ),
        SegmentKind::HfGlobal { frame_idx } => (
            format!("frame_{}/hf_global.json", frame_idx),
            "HfGlobal".to_string(),
        ),
        SegmentKind::HfMetadata {
            frame_idx,
            lf_group_idx,
        } => (
            format!("frame_{}/lf_group_{}/hf_metadata.json", frame_idx, lf_group_idx),
            format!("HfMetadata(lf_group={})", lf_group_idx),
        ),
        SegmentKind::LfCoeff {
            frame_idx,
            lf_group_idx,
        } => (
            format!("frame_{}/lf_group_{}/lf_coeff.json", frame_idx, lf_group_idx),
            format!("LfCoeff(lf_group={})", lf_group_idx),
        ),
        SegmentKind::HfCoeff {
            frame_idx,
            pass_idx,
            group_idx,
            ..
        } => (
            format!(
                "frame_{}/pass_{}/group_{}/hf_coeff.json",
                frame_idx, pass_idx, group_idx
            ),
            format!("HfCoeff(pass={}, group={})", pass_idx, group_idx),
        ),
        SegmentKind::ModularGlobal { frame_idx } => (
            format!("frame_{}/modular_global.json", frame_idx),
            "ModularGlobal".to_string(),
        ),
        SegmentKind::ModularGroup {
            frame_idx,
            group_idx,
        } => (
            format!("frame_{}/modular_group_{}.json", frame_idx, group_idx),
            format!("ModularGroup({})", group_idx),
        ),
    }
}

/// Extract a specific segment from an annotation directory.
pub fn extract_segment(
    input_dir: &Path,
    segment_path: &str,
    output: &Path,
) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    // Parse segment path like "frame0.lf_group0.hf_metadata"
    let parts: Vec<&str> = segment_path.split('.').collect();

    // Map to file path
    let file_path = match parts.as_slice() {
        ["signature"] => "segments/signature.json".to_string(),
        ["image_header"] => "segments/image_header.json".to_string(),
        [frame, "header"] if frame.starts_with("frame") => {
            let idx: u32 = frame.strip_prefix("frame").unwrap_or("0").parse()?;
            format!("segments/frame_{}/header.json", idx)
        }
        [frame, lf_group, "hf_metadata"]
            if frame.starts_with("frame") && lf_group.starts_with("lf_group") =>
        {
            let frame_idx: u32 = frame.strip_prefix("frame").unwrap_or("0").parse()?;
            let lf_idx: u32 = lf_group.strip_prefix("lf_group").unwrap_or("0").parse()?;
            format!(
                "segments/frame_{}/lf_group_{}/hf_metadata.json",
                frame_idx, lf_idx
            )
        }
        [frame, pass, group, "hf_coeff"]
            if frame.starts_with("frame")
                && pass.starts_with("pass")
                && group.starts_with("group") =>
        {
            let frame_idx: u32 = frame.strip_prefix("frame").unwrap_or("0").parse()?;
            let pass_idx: u32 = pass.strip_prefix("pass").unwrap_or("0").parse()?;
            let group_idx: u32 = group.strip_prefix("group").unwrap_or("0").parse()?;
            format!(
                "segments/frame_{}/pass_{}/group_{}/hf_coeff.json",
                frame_idx, pass_idx, group_idx
            )
        }
        _ => {
            return Err(format!("Unknown segment path: {}", segment_path).into());
        }
    };

    let full_path = input_dir.join(&file_path);
    if !full_path.exists() {
        return Err(format!("Segment file not found: {}", full_path.display()).into());
    }

    // Copy or link the file
    fs::copy(&full_path, output)?;

    tracing::info!("Extracted {} to {}", file_path, output.display());

    Ok(())
}

/// Format a hex dump with annotations.
pub fn format_hex_annotated(
    data: &[u8],
    annotations: &[jxl_bitstream::annotate::Annotation],
) -> String {
    let mut output = String::new();
    let mut byte_idx = 0;

    // Group annotations by byte
    let mut byte_annotations: std::collections::HashMap<usize, Vec<_>> =
        std::collections::HashMap::new();
    for ann in annotations {
        let start_byte = (ann.bit_start / 8) as usize;
        let end_byte = ((ann.bit_start + ann.bit_length as u64 + 7) / 8) as usize;
        for b in start_byte..end_byte {
            byte_annotations.entry(b).or_default().push(ann);
        }
    }

    while byte_idx < data.len() {
        // Offset
        output.push_str(&format!("{:08x}  ", byte_idx));

        // Hex bytes (16 per line)
        for i in 0..16 {
            if byte_idx + i < data.len() {
                output.push_str(&format!("{:02x} ", data[byte_idx + i]));
            } else {
                output.push_str("   ");
            }
            if i == 7 {
                output.push(' ');
            }
        }

        output.push_str(" |");

        // ASCII
        for i in 0..16 {
            if byte_idx + i < data.len() {
                let b = data[byte_idx + i];
                if b.is_ascii_graphic() || b == b' ' {
                    output.push(b as char);
                } else {
                    output.push('.');
                }
            }
        }

        output.push_str("|\n");

        // Annotations for this line
        for i in 0..16 {
            if let Some(anns) = byte_annotations.get(&(byte_idx + i)) {
                for ann in anns {
                    if (ann.bit_start / 8) as usize == byte_idx + i {
                        output.push_str(&format!(
                            "          ; {} = {:?} ({} bits)\n",
                            ann.field_name, ann.value, ann.bit_length
                        ));
                    }
                }
            }
        }

        byte_idx += 16;
    }

    output
}
