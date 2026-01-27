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
            segment.byte_range.1 = (end_bit + 7) / 8;

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
