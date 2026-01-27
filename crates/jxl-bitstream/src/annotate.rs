//! Bitstream annotation support for debugging JXL encoders.
//!
//! This module provides types and traits for recording byte-level annotations
//! of the JXL bitstream, including where in the decoder each field was parsed.
//!
//! Enable with `--features annotate`.

use crate::{Bitstream, BitstreamResult};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

// Re-export whereat for call-site tracking
pub use whereat;

/// Location in decoder source code where a field was parsed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecoderLocation {
    /// Source file path (relative to crate root)
    pub file: String,
    /// Line number
    pub line: u32,
    /// Column number
    pub column: u32,
}

impl DecoderLocation {
    /// Capture the current location using `#[track_caller]`.
    #[track_caller]
    pub fn here() -> Self {
        let loc = std::panic::Location::caller();
        Self {
            file: loc.file().to_string(),
            line: loc.line(),
            column: loc.column(),
        }
    }
}

/// Type of encoding used to read a value from the bitstream.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum EncodingType {
    /// Fixed N bits: u(n)
    Bits { n: u8 },
    /// U32 with 4 branches
    U32 { selector: u8, extra_bits: u8 },
    /// Variable-length U64
    U64,
    /// Half-precision float
    F16,
    /// Boolean (1 bit)
    Bool,
    /// Enum with U32 encoding
    Enum { type_name: String },
    /// Nested Bundle structure
    Bundle { type_name: String },
    /// Zero padding to byte boundary
    ZeroPadToByte,
}

/// A parsed value from the bitstream.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AnnotatedValue {
    Bool(bool),
    U32(u32),
    U64(u64),
    I32(i32),
    I64(i64),
    F32(f32),
    Enum {
        name: String,
        variant: String,
        value: u32,
    },
    /// Nested structure - annotations stored separately
    Nested {
        type_name: String,
    },
    /// Array of values
    Array(Vec<AnnotatedValue>),
    /// Padding (no semantic value)
    Padding,
}

/// Single annotation for a parsed field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Annotation {
    /// Bit offset from start of codestream
    pub bit_start: u64,
    /// Number of bits consumed
    pub bit_length: u32,
    /// Hierarchical path: "ImageHeader.metadata.color.primaries"
    pub path: String,
    /// Human-readable field name
    pub field_name: String,
    /// Parsed value
    pub value: AnnotatedValue,
    /// Encoding type used
    pub encoding: EncodingType,
    /// ISO 18181-1 spec section reference (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spec_ref: Option<String>,
    /// Where in the decoder this was parsed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decoder_location: Option<DecoderLocation>,
}

/// Segment kind for semantic grouping.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum SegmentKind {
    // Container layer
    Container { box_type: String },

    // Codestream structure
    Signature,
    ImageHeader,

    // Frame structure
    FrameHeader { frame_idx: u32, encoding: String },
    Toc { frame_idx: u32 },

    // Global data
    LfGlobal { frame_idx: u32 },
    HfGlobal { frame_idx: u32 },

    // Per-group data (VarDCT)
    HfMetadata { frame_idx: u32, lf_group_idx: u32 },
    LfCoeff { frame_idx: u32, lf_group_idx: u32 },
    HfCoeff {
        frame_idx: u32,
        pass_idx: u32,
        group_idx: u32,
        /// Path to external ANS symbol file (if written separately)
        #[serde(skip_serializing_if = "Option::is_none")]
        ans_symbols_file: Option<PathBuf>,
    },

    // Modular mode
    ModularGlobal { frame_idx: u32 },
    ModularGroup { frame_idx: u32, group_idx: u32 },
}

/// A segment of the bitstream with its annotations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    pub kind: SegmentKind,
    pub byte_range: (u64, u64),
    pub bit_range: (u64, u64),
    /// Annotations within this segment
    pub annotations: Vec<Annotation>,
    /// Child segments (for hierarchical nesting)
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub children: Vec<Segment>,
}

/// Stack frame for tracking current parsing context.
#[derive(Debug, Clone)]
pub struct ContextFrame {
    pub name: String,
    pub start_bit: u64,
}

/// Stack of parsing contexts.
#[derive(Debug, Default)]
pub struct ContextStack {
    frames: Vec<ContextFrame>,
}

impl ContextStack {
    pub fn new() -> Self {
        Self { frames: Vec::new() }
    }

    pub fn push(&mut self, name: impl Into<String>, start_bit: u64) {
        self.frames.push(ContextFrame {
            name: name.into(),
            start_bit,
        });
    }

    pub fn pop(&mut self) -> Option<ContextFrame> {
        self.frames.pop()
    }

    /// Get the current hierarchical path (e.g., "ImageHeader.metadata.color")
    pub fn path(&self) -> String {
        self.frames
            .iter()
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>()
            .join(".")
    }

    pub fn depth(&self) -> usize {
        self.frames.len()
    }
}

/// Collector for annotations during parsing.
#[derive(Debug, Default)]
pub struct AnnotationCollector {
    /// All annotations collected
    pub annotations: Vec<Annotation>,
    /// Current segment being built
    pub current_segment: Option<Segment>,
    /// Completed segments
    pub segments: Vec<Segment>,
    /// Segment stack for nesting
    segment_stack: Vec<Segment>,
}

impl AnnotationCollector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Push an annotation.
    pub fn push(&mut self, annotation: Annotation) {
        if let Some(ref mut segment) = self.current_segment {
            segment.annotations.push(annotation);
        } else {
            self.annotations.push(annotation);
        }
    }

    /// Start a new segment.
    pub fn begin_segment(&mut self, kind: SegmentKind, start_bit: u64) {
        let segment = Segment {
            kind,
            byte_range: (start_bit / 8, 0), // End filled in later
            bit_range: (start_bit, 0),
            annotations: Vec::new(),
            children: Vec::new(),
        };

        if let Some(current) = self.current_segment.take() {
            self.segment_stack.push(current);
        }
        self.current_segment = Some(segment);
    }

    /// End the current segment.
    pub fn end_segment(&mut self, end_bit: u64) {
        if let Some(mut segment) = self.current_segment.take() {
            segment.bit_range.1 = end_bit;
            segment.byte_range.1 = end_bit.div_ceil(8);

            if let Some(mut parent) = self.segment_stack.pop() {
                parent.children.push(segment);
                self.current_segment = Some(parent);
            } else {
                self.segments.push(segment);
            }
        }
    }

    /// Get all segments.
    pub fn into_segments(mut self) -> Vec<Segment> {
        // Close any open segments
        while self.current_segment.is_some() {
            self.end_segment(0); // Best effort
        }
        self.segments
    }
}

/// Annotated bitstream wrapper that records all reads.
///
/// This wraps a `Bitstream` and records every read operation with
/// bit offsets, values, and decoder locations.
pub struct AnnotatedBitstream<'buf> {
    inner: Bitstream<'buf>,
    collector: Rc<RefCell<AnnotationCollector>>,
    context: ContextStack,
}

impl<'buf> AnnotatedBitstream<'buf> {
    /// Create a new annotated bitstream.
    pub fn new(bytes: &'buf [u8]) -> Self {
        Self {
            inner: Bitstream::new(bytes),
            collector: Rc::new(RefCell::new(AnnotationCollector::new())),
            context: ContextStack::new(),
        }
    }

    /// Create with a shared collector (for nested parsing).
    pub fn with_collector(
        bytes: &'buf [u8],
        collector: Rc<RefCell<AnnotationCollector>>,
    ) -> Self {
        Self {
            inner: Bitstream::new(bytes),
            collector,
            context: ContextStack::new(),
        }
    }

    /// Get the underlying bitstream for raw access.
    pub fn inner(&self) -> &Bitstream<'buf> {
        &self.inner
    }

    /// Get mutable access to underlying bitstream.
    pub fn inner_mut(&mut self) -> &mut Bitstream<'buf> {
        &mut self.inner
    }

    /// Get the annotation collector.
    pub fn collector(&self) -> Rc<RefCell<AnnotationCollector>> {
        Rc::clone(&self.collector)
    }

    /// Current bit position.
    pub fn num_read_bits(&self) -> usize {
        self.inner.num_read_bits()
    }

    /// Enter a named context (e.g., "ImageHeader").
    pub fn enter_context(&mut self, name: impl Into<String>) {
        let start_bit = self.inner.num_read_bits() as u64;
        self.context.push(name, start_bit);
    }

    /// Exit the current context.
    pub fn exit_context(&mut self) {
        self.context.pop();
    }

    /// Begin a segment.
    pub fn begin_segment(&mut self, kind: SegmentKind) {
        let start_bit = self.inner.num_read_bits() as u64;
        self.collector.borrow_mut().begin_segment(kind, start_bit);
    }

    /// End the current segment.
    pub fn end_segment(&mut self) {
        let end_bit = self.inner.num_read_bits() as u64;
        self.collector.borrow_mut().end_segment(end_bit);
    }

    /// Read bits and record annotation.
    #[track_caller]
    pub fn read_bits_annotated(
        &mut self,
        n: usize,
        field_name: impl Into<String>,
        spec_ref: Option<impl Into<String>>,
    ) -> BitstreamResult<u32> {
        let start = self.inner.num_read_bits() as u64;
        let value = self.inner.read_bits(n)?;

        self.collector.borrow_mut().push(Annotation {
            bit_start: start,
            bit_length: n as u32,
            path: self.context.path(),
            field_name: field_name.into(),
            value: AnnotatedValue::U32(value),
            encoding: EncodingType::Bits { n: n as u8 },
            spec_ref: spec_ref.map(Into::into),
            decoder_location: Some(DecoderLocation::here()),
        });

        Ok(value)
    }

    /// Read bool and record annotation.
    #[track_caller]
    pub fn read_bool_annotated(
        &mut self,
        field_name: impl Into<String>,
        spec_ref: Option<impl Into<String>>,
    ) -> BitstreamResult<bool> {
        let start = self.inner.num_read_bits() as u64;
        let value = self.inner.read_bool()?;

        self.collector.borrow_mut().push(Annotation {
            bit_start: start,
            bit_length: 1,
            path: self.context.path(),
            field_name: field_name.into(),
            value: AnnotatedValue::Bool(value),
            encoding: EncodingType::Bool,
            spec_ref: spec_ref.map(Into::into),
            decoder_location: Some(DecoderLocation::here()),
        });

        Ok(value)
    }

    /// Consume the annotated bitstream and return collector.
    pub fn into_collector(self) -> Rc<RefCell<AnnotationCollector>> {
        self.collector
    }
}

/// Manifest file for annotated output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotationManifest {
    pub version: u32,
    pub tool: String,
    pub source_file: PathBuf,
    pub source_size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_sha256: Option<String>,

    pub image: ImageInfo,
    pub frames: Vec<FrameInfo>,
    pub segment_files: Vec<SegmentFileRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageInfo {
    pub width: u32,
    pub height: u32,
    pub bit_depth: u32,
    pub num_channels: u32,
    pub has_alpha: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_space: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameInfo {
    pub index: u32,
    pub encoding: String,
    pub width: u32,
    pub height: u32,
    pub num_lf_groups: u32,
    pub num_passes: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentFileRef {
    pub kind: String,
    pub path: PathBuf,
    pub bit_range: (u64, u64),
}

/// VarDCT-specific annotation for block info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VarBlockAnnotation {
    /// Position in 8x8 blocks
    pub block_x: u32,
    pub block_y: u32,
    /// DCT transform type name
    pub dct_select: String,
    /// Size in 8x8 blocks
    pub size_blocks: (u32, u32),
    /// Size in pixels
    pub size_pixels: (u32, u32),
    /// HF quantization multiplier
    pub hf_mul: i32,
    /// Chroma-from-luma correlation values
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x_from_y: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub b_from_y: Option<i32>,
    /// EPF sigma
    #[serde(skip_serializing_if = "Option::is_none")]
    pub epf_sigma: Option<f32>,
}

/// VarDCT HfMetadata annotation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HfMetadataAnnotation {
    pub frame_idx: u32,
    pub lf_group_idx: u32,
    pub width_blocks: u32,
    pub height_blocks: u32,
    pub num_varblocks: u32,
    pub varblocks: Vec<VarBlockAnnotation>,
}

/// ANS symbol record for external file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnsSymbolRecord {
    /// Context used
    pub context: u32,
    /// Decoded symbol value
    pub symbol: u32,
    /// Cluster (after context map)
    pub cluster: u32,
    /// Bit offset in codestream
    pub bit_offset: u64,
}

// ============================================================================
// Decoded Value Checkpoints
// ============================================================================
// These types capture intermediate decode results for pipeline analysis,
// enabling comparison of two decoders/encoders at each processing stage.

/// Pipeline stage where a checkpoint was captured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PipelineStage {
    /// Raw coefficients from entropy decoding (integers)
    RawCoefficients,
    /// After dequantization (floats)
    DequantizedCoefficients,
    /// After IDCT transform (spatial domain)
    IdctOutput,
    /// After chroma-from-luma adjustment
    ChromaFromLuma,
    /// After XYB to linear RGB conversion
    ColorTransform,
    /// After upsampling (2x/4x/8x)
    Upsampling,
    /// After edge-preserving filter
    EdgePreservingFilter,
    /// After Gaborish filter
    Gaborish,
    /// Final rendered pixels
    FinalPixels,
    /// Modular channel output
    ModularChannel,
    /// LF image (DC + LF coefficients)
    LfImage,
}

/// Statistics for a checkpoint's data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointStats {
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub stddev: f64,
    /// Number of non-zero values (useful for sparsity analysis)
    pub nonzero_count: u64,
    pub total_count: u64,
}

/// A checkpoint capturing decoded values at a pipeline stage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Which pipeline stage
    pub stage: PipelineStage,
    /// Frame index
    pub frame_idx: u32,
    /// Group/block coordinates (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_idx: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_x: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_y: Option<u32>,
    /// Channel (0=Y/R, 1=X/G, 2=B)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<u32>,
    /// Data dimensions
    pub width: u32,
    pub height: u32,
    /// Statistics (always included for quick comparison)
    pub stats: CheckpointStats,
    /// Path to external data file (for full array comparison)
    /// Format: .npy (NumPy), .bin (raw f32), or .png (visualization)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_file: Option<PathBuf>,
    /// Where in decoder this checkpoint was captured
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decoder_location: Option<DecoderLocation>,
}

impl CheckpointStats {
    /// Compute statistics from a slice of f32 values.
    pub fn from_f32_slice(data: &[f32]) -> Self {
        if data.is_empty() {
            return Self {
                min: 0.0,
                max: 0.0,
                mean: 0.0,
                stddev: 0.0,
                nonzero_count: 0,
                total_count: 0,
            };
        }

        let mut min = f64::MAX;
        let mut max = f64::MIN;
        let mut sum = 0.0f64;
        let mut nonzero = 0u64;

        for &v in data {
            let v = v as f64;
            min = min.min(v);
            max = max.max(v);
            sum += v;
            if v != 0.0 {
                nonzero += 1;
            }
        }

        let count = data.len() as f64;
        let mean = sum / count;

        let variance = data.iter().map(|&v| {
            let diff = v as f64 - mean;
            diff * diff
        }).sum::<f64>() / count;

        Self {
            min,
            max,
            mean,
            stddev: variance.sqrt(),
            nonzero_count: nonzero,
            total_count: data.len() as u64,
        }
    }

    /// Compute statistics from a slice of i32 values.
    pub fn from_i32_slice(data: &[i32]) -> Self {
        if data.is_empty() {
            return Self {
                min: 0.0,
                max: 0.0,
                mean: 0.0,
                stddev: 0.0,
                nonzero_count: 0,
                total_count: 0,
            };
        }

        let mut min = i32::MAX;
        let mut max = i32::MIN;
        let mut sum = 0i64;
        let mut nonzero = 0u64;

        for &v in data {
            min = min.min(v);
            max = max.max(v);
            sum += v as i64;
            if v != 0 {
                nonzero += 1;
            }
        }

        let count = data.len() as f64;
        let mean = sum as f64 / count;

        let variance = data.iter().map(|&v| {
            let diff = v as f64 - mean;
            diff * diff
        }).sum::<f64>() / count;

        Self {
            min: min as f64,
            max: max as f64,
            mean,
            stddev: variance.sqrt(),
            nonzero_count: nonzero,
            total_count: data.len() as u64,
        }
    }
}

/// Difference between two checkpoints at the same pipeline stage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointDiff {
    pub stage: PipelineStage,
    pub frame_idx: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_idx: Option<u32>,
    /// Statistics of the difference (a - b)
    pub diff_stats: CheckpointStats,
    /// Max absolute difference
    pub max_abs_diff: f64,
    /// Mean absolute difference
    pub mean_abs_diff: f64,
    /// Root mean square difference
    pub rms_diff: f64,
    /// Are values identical within tolerance?
    pub identical: bool,
    /// Tolerance used for comparison
    pub tolerance: f64,
}

/// Collector for checkpoints during decoding.
#[derive(Debug, Default)]
pub struct CheckpointCollector {
    pub checkpoints: Vec<Checkpoint>,
}

impl CheckpointCollector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a checkpoint with statistics only (no data export).
    pub fn push_stats(&mut self, checkpoint: Checkpoint) {
        self.checkpoints.push(checkpoint);
    }

    /// Get checkpoints for a specific stage.
    pub fn get_by_stage(&self, stage: PipelineStage) -> Vec<&Checkpoint> {
        self.checkpoints.iter().filter(|c| c.stage == stage).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_stack() {
        let mut stack = ContextStack::new();
        assert_eq!(stack.path(), "");

        stack.push("ImageHeader", 0);
        assert_eq!(stack.path(), "ImageHeader");

        stack.push("size", 16);
        assert_eq!(stack.path(), "ImageHeader.size");

        stack.pop();
        assert_eq!(stack.path(), "ImageHeader");
    }

    #[test]
    fn test_annotation_serialization() {
        let ann = Annotation {
            bit_start: 0,
            bit_length: 16,
            path: "signature".to_string(),
            field_name: "JPEG XL Signature".to_string(),
            value: AnnotatedValue::U32(0x0AFF),
            encoding: EncodingType::Bits { n: 16 },
            spec_ref: Some("ISO 18181-1:2022 A.4.1".to_string()),
            decoder_location: None,
        };

        let json = serde_json::to_string_pretty(&ann).unwrap();
        assert!(json.contains("signature"));
        assert!(json.contains("2815")); // 0x0AFF
    }
}
