//! Core annotation logic for JXL bitstreams.

use jxl_bitstream::annotate::{
    Annotation, AnnotationManifest, Checkpoint, FrameInfo, ImageInfo, Segment, SegmentKind,
};
use jxl_frame::data::TocGroupKind;
use jxl_oxide::JxlImage;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

/// Options for annotation.
#[derive(Debug, Clone)]
pub struct AnnotateOptions {
    pub include_ans: bool,
    pub include_checkpoints: bool,
    pub max_depth: usize,
    pub frame_filter: Option<Vec<u32>>,
}

impl Default for AnnotateOptions {
    fn default() -> Self {
        Self {
            include_ans: false,
            include_checkpoints: false,
            max_depth: 10,
            frame_filter: None,
        }
    }
}

/// Result of annotating a JXL file.
#[derive(Debug)]
pub struct AnnotationResult {
    pub manifest: AnnotationManifest,
    pub segments: Vec<Segment>,
    pub checkpoints: Vec<Checkpoint>,
}

/// Annotate a JXL file.
pub fn annotate_file(
    input_path: &Path,
    options: &AnnotateOptions,
) -> Result<AnnotationResult, Box<dyn std::error::Error + Send + Sync + 'static>> {
    let data = fs::read(input_path)?;
    let file_size = data.len() as u64;

    // Compute SHA256
    let mut hasher = Sha256::new();
    hasher.update(&data);
    let hash = hex::encode(hasher.finalize());

    // Parse the JXL file
    let image = JxlImage::builder().read(&*data)?;
    let header = image.image_header();

    // Create basic image info
    let image_info = ImageInfo {
        width: header.size.width,
        height: header.size.height,
        bit_depth: header.metadata.bit_depth.bits_per_sample(),
        num_channels: if header.metadata.grayscale() { 1 } else { 3 }
            + header.metadata.ec_info.len() as u32,
        has_alpha: header.metadata.ec_info.iter().any(|ec| {
            matches!(ec.ty, jxl_image::ExtraChannelType::Alpha { .. })
        }),
        color_space: None, // TODO: extract from color encoding
    };

    // Collect frame info
    let mut frames = Vec::new();

    // Get frame info from the loaded frame
    if let Some(frame) = image.frame_by_keyframe(0) {
        let frame_header = frame.header();
        frames.push(FrameInfo {
            index: 0,
            encoding: if frame_header.encoding == jxl_frame::header::Encoding::VarDct {
                "VarDct".to_string()
            } else {
                "Modular".to_string()
            },
            width: frame_header.width,
            height: frame_header.height,
            num_lf_groups: frame_header.num_lf_groups(),
            num_passes: frame_header.passes.num_passes,
        });
    }

    // Create segments with TOC data
    let segments = create_segments_with_toc(&image, options)?;

    // Create checkpoints if requested
    let checkpoints = if options.include_checkpoints {
        collect_checkpoints(&image, options)?
    } else {
        Vec::new()
    };

    let manifest = AnnotationManifest {
        version: 1,
        tool: format!("jxl-annotate {}", env!("CARGO_PKG_VERSION")),
        source_file: input_path.to_path_buf(),
        source_size: file_size,
        source_sha256: Some(hash),
        image: image_info,
        frames,
        segment_files: Vec::new(), // Filled in by output module
    };

    Ok(AnnotationResult {
        manifest,
        segments,
        checkpoints,
    })
}

/// Create segments using TOC data for accurate byte offsets.
fn create_segments_with_toc(
    image: &JxlImage,
    _options: &AnnotateOptions,
) -> Result<Vec<Segment>, Box<dyn std::error::Error + Send + Sync + 'static>> {
    let mut segments = Vec::new();

    // Signature segment (always at bytes 0-2 for bare codestream, different for container)
    // Check if this is a container by looking at the first bytes
    let signature_end = 2u64;
    segments.push(Segment {
        kind: SegmentKind::Signature,
        byte_range: (0, signature_end),
        bit_range: (0, signature_end * 8),
        annotations: vec![Annotation {
            bit_start: 0,
            bit_length: 16,
            path: "".to_string(),
            field_name: "signature".to_string(),
            value: jxl_bitstream::annotate::AnnotatedValue::U32(0x0AFF),
            encoding: jxl_bitstream::annotate::EncodingType::Bits { n: 16 },
            spec_ref: Some("ISO 18181-1:2022 A.4.1".to_string()),
            decoder_location: None,
        }],
        children: Vec::new(),
    });

    // ImageHeader segment - starts after signature
    let header = image.image_header();
    segments.push(Segment {
        kind: SegmentKind::ImageHeader,
        byte_range: (signature_end, 0), // End will be determined by frame offset
        bit_range: (signature_end * 8, 0),
        annotations: create_image_header_annotations(header),
        children: Vec::new(),
    });

    // Process frames
    for frame_idx in 0..image.num_loaded_frames() {
        let Some(frame) = image.frame(frame_idx) else {
            continue;
        };

        // Get frame offset in the codestream
        let frame_offset = image.frame_offset(frame_idx).unwrap_or(0) as u64;

        let frame_header = frame.header();
        let encoding_str = if frame_header.encoding == jxl_frame::header::Encoding::VarDct {
            "VarDct"
        } else {
            "Modular"
        };

        // Frame header segment
        let toc = frame.toc();
        let toc_bookmark = toc.bookmark() as u64;

        segments.push(Segment {
            kind: SegmentKind::FrameHeader {
                frame_idx: frame_idx as u32,
                encoding: encoding_str.to_string(),
            },
            byte_range: (frame_offset, frame_offset + toc_bookmark),
            bit_range: (frame_offset * 8, (frame_offset + toc_bookmark) * 8),
            annotations: create_frame_header_annotations(frame_header, frame_idx as u32),
            children: Vec::new(),
        });

        // TOC segment
        segments.push(Segment {
            kind: SegmentKind::Toc {
                frame_idx: frame_idx as u32,
            },
            byte_range: (frame_offset, frame_offset + toc_bookmark),
            bit_range: (frame_offset * 8, (frame_offset + toc_bookmark) * 8),
            annotations: Vec::new(),
            children: Vec::new(),
        });

        // Create segments from TOC groups
        for toc_group in toc.iter_bitstream_order() {
            let group_start = frame_offset + toc_group.offset as u64;
            let group_end = group_start + toc_group.size as u64;

            let segment_kind = match toc_group.kind {
                TocGroupKind::All => {
                    // Single-group frame - represents all data
                    SegmentKind::LfGlobal {
                        frame_idx: frame_idx as u32,
                    }
                }
                TocGroupKind::LfGlobal => SegmentKind::LfGlobal {
                    frame_idx: frame_idx as u32,
                },
                TocGroupKind::LfGroup(lf_group_idx) => {
                    if frame_header.encoding == jxl_frame::header::Encoding::VarDct {
                        // VarDCT: this contains HfMetadata + LfCoeff
                        SegmentKind::HfMetadata {
                            frame_idx: frame_idx as u32,
                            lf_group_idx,
                        }
                    } else {
                        // Modular: LF coefficients
                        SegmentKind::LfCoeff {
                            frame_idx: frame_idx as u32,
                            lf_group_idx,
                        }
                    }
                }
                TocGroupKind::HfGlobal => SegmentKind::HfGlobal {
                    frame_idx: frame_idx as u32,
                },
                TocGroupKind::GroupPass {
                    pass_idx,
                    group_idx,
                } => {
                    if frame_header.encoding == jxl_frame::header::Encoding::VarDct {
                        SegmentKind::HfCoeff {
                            frame_idx: frame_idx as u32,
                            pass_idx,
                            group_idx,
                            ans_symbols_file: None,
                        }
                    } else {
                        SegmentKind::ModularGroup {
                            frame_idx: frame_idx as u32,
                            group_idx,
                        }
                    }
                }
            };

            segments.push(Segment {
                kind: segment_kind,
                byte_range: (group_start, group_end),
                bit_range: (group_start * 8, group_end * 8),
                annotations: Vec::new(),
                children: Vec::new(),
            });
        }
    }

    Ok(segments)
}

/// Create annotations for the image header.
fn create_image_header_annotations(header: &jxl_image::ImageHeader) -> Vec<Annotation> {
    let mut annotations = Vec::new();

    // Size info
    annotations.push(Annotation {
        bit_start: 16, // After signature
        bit_length: 0, // Variable
        path: "ImageHeader.size".to_string(),
        field_name: "width".to_string(),
        value: jxl_bitstream::annotate::AnnotatedValue::U32(header.size.width),
        encoding: jxl_bitstream::annotate::EncodingType::U32 { selector: 0, extra_bits: 0 },
        spec_ref: Some("ISO 18181-1:2022 A.4.2".to_string()),
        decoder_location: None,
    });

    annotations.push(Annotation {
        bit_start: 0,
        bit_length: 0,
        path: "ImageHeader.size".to_string(),
        field_name: "height".to_string(),
        value: jxl_bitstream::annotate::AnnotatedValue::U32(header.size.height),
        encoding: jxl_bitstream::annotate::EncodingType::U32 { selector: 0, extra_bits: 0 },
        spec_ref: Some("ISO 18181-1:2022 A.4.2".to_string()),
        decoder_location: None,
    });

    // Metadata
    annotations.push(Annotation {
        bit_start: 0,
        bit_length: 0,
        path: "ImageHeader.metadata".to_string(),
        field_name: "xyb_encoded".to_string(),
        value: jxl_bitstream::annotate::AnnotatedValue::Bool(header.metadata.xyb_encoded),
        encoding: jxl_bitstream::annotate::EncodingType::Bool,
        spec_ref: Some("ISO 18181-1:2022 A.4.3".to_string()),
        decoder_location: None,
    });

    annotations.push(Annotation {
        bit_start: 0,
        bit_length: 0,
        path: "ImageHeader.metadata".to_string(),
        field_name: "orientation".to_string(),
        value: jxl_bitstream::annotate::AnnotatedValue::U32(header.metadata.orientation),
        encoding: jxl_bitstream::annotate::EncodingType::U32 { selector: 0, extra_bits: 0 },
        spec_ref: Some("ISO 18181-1:2022 A.4.3".to_string()),
        decoder_location: None,
    });

    annotations.push(Annotation {
        bit_start: 0,
        bit_length: 0,
        path: "ImageHeader.metadata".to_string(),
        field_name: "num_extra_channels".to_string(),
        value: jxl_bitstream::annotate::AnnotatedValue::U32(header.metadata.ec_info.len() as u32),
        encoding: jxl_bitstream::annotate::EncodingType::U32 { selector: 0, extra_bits: 0 },
        spec_ref: Some("ISO 18181-1:2022 A.4.3".to_string()),
        decoder_location: None,
    });

    annotations
}

/// Create annotations for a frame header.
fn create_frame_header_annotations(
    header: &jxl_frame::FrameHeader,
    frame_idx: u32,
) -> Vec<Annotation> {
    let mut annotations = Vec::new();

    annotations.push(Annotation {
        bit_start: 0,
        bit_length: 0,
        path: format!("Frame[{}].header", frame_idx),
        field_name: "encoding".to_string(),
        value: jxl_bitstream::annotate::AnnotatedValue::Enum {
            name: "Encoding".to_string(),
            variant: if header.encoding == jxl_frame::header::Encoding::VarDct {
                "VarDct".to_string()
            } else {
                "Modular".to_string()
            },
            value: if header.encoding == jxl_frame::header::Encoding::VarDct { 1 } else { 0 },
        },
        encoding: jxl_bitstream::annotate::EncodingType::Enum { type_name: "Encoding".to_string() },
        spec_ref: Some("ISO 18181-2:2022 B.2".to_string()),
        decoder_location: None,
    });

    annotations.push(Annotation {
        bit_start: 0,
        bit_length: 0,
        path: format!("Frame[{}].header", frame_idx),
        field_name: "width".to_string(),
        value: jxl_bitstream::annotate::AnnotatedValue::U32(header.width),
        encoding: jxl_bitstream::annotate::EncodingType::U32 { selector: 0, extra_bits: 0 },
        spec_ref: Some("ISO 18181-2:2022 B.2".to_string()),
        decoder_location: None,
    });

    annotations.push(Annotation {
        bit_start: 0,
        bit_length: 0,
        path: format!("Frame[{}].header", frame_idx),
        field_name: "height".to_string(),
        value: jxl_bitstream::annotate::AnnotatedValue::U32(header.height),
        encoding: jxl_bitstream::annotate::EncodingType::U32 { selector: 0, extra_bits: 0 },
        spec_ref: Some("ISO 18181-2:2022 B.2".to_string()),
        decoder_location: None,
    });

    annotations.push(Annotation {
        bit_start: 0,
        bit_length: 0,
        path: format!("Frame[{}].header", frame_idx),
        field_name: "num_passes".to_string(),
        value: jxl_bitstream::annotate::AnnotatedValue::U32(header.passes.num_passes),
        encoding: jxl_bitstream::annotate::EncodingType::U32 { selector: 0, extra_bits: 0 },
        spec_ref: Some("ISO 18181-2:2022 B.2".to_string()),
        decoder_location: None,
    });

    annotations.push(Annotation {
        bit_start: 0,
        bit_length: 0,
        path: format!("Frame[{}].header", frame_idx),
        field_name: "num_lf_groups".to_string(),
        value: jxl_bitstream::annotate::AnnotatedValue::U32(header.num_lf_groups()),
        encoding: jxl_bitstream::annotate::EncodingType::U32 { selector: 0, extra_bits: 0 },
        spec_ref: Some("ISO 18181-2:2022 B.2".to_string()),
        decoder_location: None,
    });

    annotations.push(Annotation {
        bit_start: 0,
        bit_length: 0,
        path: format!("Frame[{}].header", frame_idx),
        field_name: "num_groups".to_string(),
        value: jxl_bitstream::annotate::AnnotatedValue::U32(header.num_groups()),
        encoding: jxl_bitstream::annotate::EncodingType::U32 { selector: 0, extra_bits: 0 },
        spec_ref: Some("ISO 18181-2:2022 B.2".to_string()),
        decoder_location: None,
    });

    // VarDCT-specific fields
    if header.encoding == jxl_frame::header::Encoding::VarDct {
        annotations.push(Annotation {
            bit_start: 0,
            bit_length: 0,
            path: format!("Frame[{}].header", frame_idx),
            field_name: "x_qm_scale".to_string(),
            value: jxl_bitstream::annotate::AnnotatedValue::I32(header.x_qm_scale as i32),
            encoding: jxl_bitstream::annotate::EncodingType::U32 { selector: 0, extra_bits: 0 },
            spec_ref: Some("ISO 18181-2:2022 B.2".to_string()),
            decoder_location: None,
        });

        annotations.push(Annotation {
            bit_start: 0,
            bit_length: 0,
            path: format!("Frame[{}].header", frame_idx),
            field_name: "b_qm_scale".to_string(),
            value: jxl_bitstream::annotate::AnnotatedValue::I32(header.b_qm_scale as i32),
            encoding: jxl_bitstream::annotate::EncodingType::U32 { selector: 0, extra_bits: 0 },
            spec_ref: Some("ISO 18181-2:2022 B.2".to_string()),
            decoder_location: None,
        });
    }

    annotations
}

/// Collect decoded value checkpoints from the rendering pipeline.
fn collect_checkpoints(
    _image: &JxlImage,
    _options: &AnnotateOptions,
) -> Result<Vec<Checkpoint>, Box<dyn std::error::Error + Send + Sync + 'static>> {
    // TODO: Hook into the render pipeline to capture intermediate values
    // For now, return empty
    let checkpoints = Vec::new();

    // Future implementation will:
    // 1. Decode with checkpoint capture enabled
    // 2. At each pipeline stage, snapshot the buffer
    // 3. Compute statistics and optionally save full data

    Ok(checkpoints)
}

/// Get VarDCT block info for a frame.
#[allow(dead_code)]
pub fn get_vardct_block_info(
    image: &JxlImage,
    frame_idx: usize,
) -> Result<
    Vec<jxl_bitstream::annotate::VarBlockAnnotation>,
    Box<dyn std::error::Error + Send + Sync + 'static>,
> {
    let blocks = Vec::new();

    let Some(frame) = image.frame_by_keyframe(frame_idx) else {
        return Ok(blocks);
    };
    let frame_header = frame.header();

    if frame_header.encoding != jxl_frame::header::Encoding::VarDct {
        return Ok(blocks);
    }

    // Access the HfMetadata for each LF group
    // This requires accessing internal frame data which may not be exposed
    // TODO: Add accessor methods to jxl-frame for annotation purposes

    Ok(blocks)
}
