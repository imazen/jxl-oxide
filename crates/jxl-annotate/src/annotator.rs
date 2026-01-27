//! Core annotation logic for JXL bitstreams.

use jxl_bitstream::annotate::{
    Annotation, AnnotationManifest, Checkpoint, FrameInfo, ImageInfo, Segment, SegmentKind,
};
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

    // Create segments (basic structure for now)
    let segments = create_basic_segments(&image, options)?;

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

/// Create basic segment structure from parsed image.
fn create_basic_segments(
    image: &JxlImage,
    _options: &AnnotateOptions,
) -> Result<Vec<Segment>, Box<dyn std::error::Error + Send + Sync + 'static>> {
    let mut segments = Vec::new();

    // Signature segment
    segments.push(Segment {
        kind: SegmentKind::Signature,
        byte_range: (0, 2),
        bit_range: (0, 16),
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

    // ImageHeader segment
    let _header = image.image_header(); // Will be used for detailed header annotations
    segments.push(Segment {
        kind: SegmentKind::ImageHeader,
        byte_range: (2, 0), // End will be filled by deeper analysis
        bit_range: (16, 0),
        annotations: vec![
            // Basic header annotations
            Annotation {
                bit_start: 16,
                bit_length: 1,
                path: "ImageHeader.size".to_string(),
                field_name: "div8".to_string(),
                value: jxl_bitstream::annotate::AnnotatedValue::Bool(false),
                encoding: jxl_bitstream::annotate::EncodingType::Bool,
                spec_ref: Some("ISO 18181-1:2022 A.4.2".to_string()),
                decoder_location: None,
            },
        ],
        children: Vec::new(),
    });

    // Frame segments
    if let Some(frame) = image.frame_by_keyframe(0) {
        let frame_header = frame.header();

        let encoding_str = if frame_header.encoding == jxl_frame::header::Encoding::VarDct {
            "VarDct"
        } else {
            "Modular"
        };

        segments.push(Segment {
            kind: SegmentKind::FrameHeader {
                frame_idx: 0,
                encoding: encoding_str.to_string(),
            },
            byte_range: (0, 0), // Placeholder
            bit_range: (0, 0),
            annotations: Vec::new(),
            children: Vec::new(),
        });

        // If VarDCT, add HfMetadata segments
        if frame_header.encoding == jxl_frame::header::Encoding::VarDct {
            for lf_group_idx in 0..frame_header.num_lf_groups() {
                segments.push(Segment {
                    kind: SegmentKind::HfMetadata {
                        frame_idx: 0,
                        lf_group_idx,
                    },
                    byte_range: (0, 0),
                    bit_range: (0, 0),
                    annotations: Vec::new(),
                    children: Vec::new(),
                });
            }

            // Add HfCoeff segments for each pass/group
            for pass_idx in 0..frame_header.passes.num_passes {
                for group_idx in 0..frame_header.num_groups() {
                    segments.push(Segment {
                        kind: SegmentKind::HfCoeff {
                            frame_idx: 0,
                            pass_idx,
                            group_idx,
                            ans_symbols_file: None,
                        },
                        byte_range: (0, 0),
                        bit_range: (0, 0),
                        annotations: Vec::new(),
                        children: Vec::new(),
                    });
                }
            }
        }
    }

    Ok(segments)
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
pub fn get_vardct_block_info(
    image: &JxlImage,
    frame_idx: usize,
) -> Result<Vec<jxl_bitstream::annotate::VarBlockAnnotation>, Box<dyn std::error::Error + Send + Sync + 'static>> {
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
