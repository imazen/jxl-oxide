//! Diff command implementation for comparing two JXL files.

use crate::annotator::{annotate_file, AnnotateOptions};
use jxl_bitstream::annotate::{
    Checkpoint, CheckpointDiff, CheckpointStats, HfMetadataAnnotation, VarBlockAnnotation,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Configuration for diff comparison.
#[derive(Debug, Clone)]
pub struct DiffConfig {
    pub vardct_only: bool,
    pub ignore_container: bool,
    pub tolerance: f64,
}

/// Result of comparing two JXL files.
#[derive(Debug, Serialize, Deserialize)]
pub struct DiffResult {
    pub file_a: String,
    pub file_b: String,
    pub summary: DiffSummary,
    pub segment_diffs: Vec<SegmentDiff>,
    pub vardct_diffs: Vec<VarDctLfGroupDiff>,
    pub checkpoint_diffs: Vec<CheckpointDiff>,
}

/// Summary of differences.
#[derive(Debug, Serialize, Deserialize)]
pub struct DiffSummary {
    pub identical: bool,
    pub total_segments_a: usize,
    pub total_segments_b: usize,
    pub matching_segments: usize,
    pub different_segments: usize,
    pub missing_in_a: usize,
    pub missing_in_b: usize,
}

/// Difference in a segment.
#[derive(Debug, Serialize, Deserialize)]
pub struct SegmentDiff {
    pub segment_kind: String,
    pub status: SegmentDiffStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum SegmentDiffStatus {
    Identical,
    Different,
    OnlyInA,
    OnlyInB,
}

/// VarDCT diff for an LF group.
#[derive(Debug, Serialize, Deserialize)]
pub struct VarDctLfGroupDiff {
    pub frame_idx: u32,
    pub lf_group_idx: u32,
    pub summary: VarDctDiffSummary,
    /// Blocks that differ between the two files
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub differing_blocks: Vec<VarBlockDiff>,
}

/// Summary of VarDCT differences in an LF group.
#[derive(Debug, Serialize, Deserialize)]
pub struct VarDctDiffSummary {
    pub total_blocks_a: u32,
    pub total_blocks_b: u32,
    pub matching_blocks: u32,
    pub dct_type_differences: u32,
    pub size_differences: u32,
    pub hf_mul_differences: u32,
    pub only_in_a: u32,
    pub only_in_b: u32,
}

/// Difference for a single varblock.
#[derive(Debug, Serialize, Deserialize)]
pub struct VarBlockDiff {
    pub block_x: u32,
    pub block_y: u32,
    pub diff_type: VarBlockDiffType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub a: Option<VarBlockAnnotation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub b: Option<VarBlockAnnotation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VarBlockDiffType {
    /// Block exists only in file A
    OnlyInA,
    /// Block exists only in file B
    OnlyInB,
    /// DCT transform type differs
    DctTypeDiff,
    /// Block size differs
    SizeDiff,
    /// HF quantization multiplier differs
    HfMulDiff,
    /// Multiple differences
    Multiple,
}

/// Run the diff command.
pub fn run_diff(
    file_a: &Path,
    file_b: &Path,
    output: &Path,
    vardct_only: bool,
    ignore_container: bool,
    tolerance: f64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    tracing::info!(
        "Comparing {} vs {}",
        file_a.display(),
        file_b.display()
    );

    let config = DiffConfig {
        vardct_only,
        ignore_container,
        tolerance,
    };

    let options = AnnotateOptions {
        include_checkpoints: true,
        ..Default::default()
    };

    let result_a = annotate_file(file_a, &options)?;
    let result_b = annotate_file(file_b, &options)?;

    let diff = compare_annotations(&result_a, &result_b, &config)?;

    // Write diff result
    let output_json = serde_json::to_string_pretty(&diff)?;
    std::fs::write(output, output_json)?;

    // Print summary
    println!("Diff Summary:");
    println!("  Identical: {}", diff.summary.identical);
    println!("  Matching segments: {}", diff.summary.matching_segments);
    println!("  Different segments: {}", diff.summary.different_segments);
    println!("  Missing in A: {}", diff.summary.missing_in_a);
    println!("  Missing in B: {}", diff.summary.missing_in_b);

    // VarDCT summary
    if !diff.vardct_diffs.is_empty() {
        let total_dct_diffs: u32 = diff
            .vardct_diffs
            .iter()
            .map(|d| d.summary.dct_type_differences)
            .sum();
        let total_size_diffs: u32 = diff
            .vardct_diffs
            .iter()
            .map(|d| d.summary.size_differences)
            .sum();
        let total_hf_mul_diffs: u32 = diff
            .vardct_diffs
            .iter()
            .map(|d| d.summary.hf_mul_differences)
            .sum();

        println!();
        println!("VarDCT Block Differences:");
        println!("  DCT type differences: {}", total_dct_diffs);
        println!("  Size differences: {}", total_size_diffs);
        println!("  HF multiplier differences: {}", total_hf_mul_diffs);
    }

    println!();
    println!("Full diff written to: {}", output.display());

    Ok(())
}

/// Compare two annotation results.
fn compare_annotations(
    result_a: &crate::annotator::AnnotationResult,
    result_b: &crate::annotator::AnnotationResult,
    config: &DiffConfig,
) -> Result<DiffResult, Box<dyn std::error::Error + Send + Sync + 'static>> {
    let mut segment_diffs = Vec::new();
    let mut matching = 0usize;
    let different = 0usize;

    // Compare segments by kind
    // This is a simplified comparison - a full implementation would
    // match segments more carefully and compare their contents

    let segments_a: Vec<_> = result_a
        .segments
        .iter()
        .filter(|s| !config.ignore_container || !matches!(s.kind, jxl_bitstream::annotate::SegmentKind::Container { .. }))
        .filter(|s| !config.vardct_only || matches!(
            s.kind,
            jxl_bitstream::annotate::SegmentKind::HfMetadata { .. }
                | jxl_bitstream::annotate::SegmentKind::HfCoeff { .. }
                | jxl_bitstream::annotate::SegmentKind::HfGlobal { .. }
        ))
        .collect();

    let segments_b: Vec<_> = result_b
        .segments
        .iter()
        .filter(|s| !config.ignore_container || !matches!(s.kind, jxl_bitstream::annotate::SegmentKind::Container { .. }))
        .filter(|s| !config.vardct_only || matches!(
            s.kind,
            jxl_bitstream::annotate::SegmentKind::HfMetadata { .. }
                | jxl_bitstream::annotate::SegmentKind::HfCoeff { .. }
                | jxl_bitstream::annotate::SegmentKind::HfGlobal { .. }
        ))
        .collect();

    // Simple comparison: check if segment kinds match
    for seg_a in &segments_a {
        let kind_str = format!("{:?}", seg_a.kind);
        let matching_b = segments_b.iter().find(|s| format!("{:?}", s.kind) == kind_str);

        if let Some(_seg_b) = matching_b {
            // TODO: Compare segment contents
            segment_diffs.push(SegmentDiff {
                segment_kind: kind_str,
                status: SegmentDiffStatus::Identical, // Simplified
                details: None,
            });
            matching += 1;
        } else {
            segment_diffs.push(SegmentDiff {
                segment_kind: kind_str,
                status: SegmentDiffStatus::OnlyInA,
                details: None,
            });
        }
    }

    // Check for segments only in B
    let mut missing_in_a = 0usize;
    for seg_b in &segments_b {
        let kind_str = format!("{:?}", seg_b.kind);
        let matching_a = segments_a.iter().find(|s| format!("{:?}", s.kind) == kind_str);

        if matching_a.is_none() {
            segment_diffs.push(SegmentDiff {
                segment_kind: kind_str,
                status: SegmentDiffStatus::OnlyInB,
                details: None,
            });
            missing_in_a += 1;
        }
    }

    // Compare VarDCT block annotations
    let vardct_diffs = compare_vardct_annotations(
        &result_a.vardct_annotations,
        &result_b.vardct_annotations,
    );

    // Compare checkpoints
    let checkpoint_diffs = compare_checkpoints(
        &result_a.checkpoints,
        &result_b.checkpoints,
        config.tolerance,
    );

    // Count VarDCT differences for summary
    let vardct_identical = vardct_diffs.iter().all(|d| {
        d.summary.dct_type_differences == 0
            && d.summary.size_differences == 0
            && d.summary.hf_mul_differences == 0
            && d.summary.only_in_a == 0
            && d.summary.only_in_b == 0
    });

    let summary = DiffSummary {
        identical: different == 0
            && missing_in_a == 0
            && vardct_identical
            && checkpoint_diffs.iter().all(|d| d.identical),
        total_segments_a: segments_a.len(),
        total_segments_b: segments_b.len(),
        matching_segments: matching,
        different_segments: different,
        missing_in_a,
        missing_in_b: segments_a.len().saturating_sub(matching),
    };

    Ok(DiffResult {
        file_a: result_a.manifest.source_file.to_string_lossy().to_string(),
        file_b: result_b.manifest.source_file.to_string_lossy().to_string(),
        summary,
        segment_diffs,
        vardct_diffs,
        checkpoint_diffs,
    })
}

/// Compare VarDCT block annotations between two files.
fn compare_vardct_annotations(
    annotations_a: &[HfMetadataAnnotation],
    annotations_b: &[HfMetadataAnnotation],
) -> Vec<VarDctLfGroupDiff> {
    let mut diffs = Vec::new();

    // Build lookup maps by (frame_idx, lf_group_idx)
    let map_a: HashMap<(u32, u32), &HfMetadataAnnotation> = annotations_a
        .iter()
        .map(|a| ((a.frame_idx, a.lf_group_idx), a))
        .collect();

    let map_b: HashMap<(u32, u32), &HfMetadataAnnotation> = annotations_b
        .iter()
        .map(|a| ((a.frame_idx, a.lf_group_idx), a))
        .collect();

    // Get all unique keys
    let mut all_keys: Vec<_> = map_a.keys().chain(map_b.keys()).copied().collect();
    all_keys.sort();
    all_keys.dedup();

    for (frame_idx, lf_group_idx) in all_keys {
        let ann_a = map_a.get(&(frame_idx, lf_group_idx));
        let ann_b = map_b.get(&(frame_idx, lf_group_idx));

        match (ann_a, ann_b) {
            (Some(a), Some(b)) => {
                // Compare the varblocks
                let lf_diff = compare_lf_group_varblocks(frame_idx, lf_group_idx, a, b);
                diffs.push(lf_diff);
            }
            (Some(a), None) => {
                // Only in A
                diffs.push(VarDctLfGroupDiff {
                    frame_idx,
                    lf_group_idx,
                    summary: VarDctDiffSummary {
                        total_blocks_a: a.num_varblocks,
                        total_blocks_b: 0,
                        matching_blocks: 0,
                        dct_type_differences: 0,
                        size_differences: 0,
                        hf_mul_differences: 0,
                        only_in_a: a.num_varblocks,
                        only_in_b: 0,
                    },
                    differing_blocks: Vec::new(),
                });
            }
            (None, Some(b)) => {
                // Only in B
                diffs.push(VarDctLfGroupDiff {
                    frame_idx,
                    lf_group_idx,
                    summary: VarDctDiffSummary {
                        total_blocks_a: 0,
                        total_blocks_b: b.num_varblocks,
                        matching_blocks: 0,
                        dct_type_differences: 0,
                        size_differences: 0,
                        hf_mul_differences: 0,
                        only_in_a: 0,
                        only_in_b: b.num_varblocks,
                    },
                    differing_blocks: Vec::new(),
                });
            }
            (None, None) => unreachable!(),
        }
    }

    diffs
}

/// Compare varblocks within an LF group.
fn compare_lf_group_varblocks(
    frame_idx: u32,
    lf_group_idx: u32,
    ann_a: &HfMetadataAnnotation,
    ann_b: &HfMetadataAnnotation,
) -> VarDctLfGroupDiff {
    // Build lookup maps by (block_x, block_y)
    let blocks_a: HashMap<(u32, u32), &VarBlockAnnotation> = ann_a
        .varblocks
        .iter()
        .map(|b| ((b.block_x, b.block_y), b))
        .collect();

    let blocks_b: HashMap<(u32, u32), &VarBlockAnnotation> = ann_b
        .varblocks
        .iter()
        .map(|b| ((b.block_x, b.block_y), b))
        .collect();

    let mut matching_blocks = 0u32;
    let mut dct_type_differences = 0u32;
    let mut size_differences = 0u32;
    let mut hf_mul_differences = 0u32;
    let mut only_in_a = 0u32;
    let mut only_in_b = 0u32;
    let mut differing_blocks = Vec::new();

    // Check blocks in A
    for (&(bx, by), block_a) in &blocks_a {
        if let Some(block_b) = blocks_b.get(&(bx, by)) {
            // Both files have a block at this position
            let dct_diff = block_a.dct_select != block_b.dct_select;
            let size_diff = block_a.size_blocks != block_b.size_blocks;
            let hf_mul_diff = block_a.hf_mul != block_b.hf_mul;

            if dct_diff || size_diff || hf_mul_diff {
                let diff_type = if dct_diff && size_diff && hf_mul_diff {
                    VarBlockDiffType::Multiple
                } else if dct_diff && size_diff {
                    VarBlockDiffType::Multiple
                } else if dct_diff {
                    VarBlockDiffType::DctTypeDiff
                } else if size_diff {
                    VarBlockDiffType::SizeDiff
                } else {
                    VarBlockDiffType::HfMulDiff
                };

                if dct_diff {
                    dct_type_differences += 1;
                }
                if size_diff {
                    size_differences += 1;
                }
                if hf_mul_diff {
                    hf_mul_differences += 1;
                }

                differing_blocks.push(VarBlockDiff {
                    block_x: bx,
                    block_y: by,
                    diff_type,
                    a: Some((*block_a).clone()),
                    b: Some((*block_b).clone()),
                });
            } else {
                matching_blocks += 1;
            }
        } else {
            // Only in A
            only_in_a += 1;
            differing_blocks.push(VarBlockDiff {
                block_x: bx,
                block_y: by,
                diff_type: VarBlockDiffType::OnlyInA,
                a: Some((*block_a).clone()),
                b: None,
            });
        }
    }

    // Check blocks only in B
    for (&(bx, by), block_b) in &blocks_b {
        if !blocks_a.contains_key(&(bx, by)) {
            only_in_b += 1;
            differing_blocks.push(VarBlockDiff {
                block_x: bx,
                block_y: by,
                diff_type: VarBlockDiffType::OnlyInB,
                a: None,
                b: Some((*block_b).clone()),
            });
        }
    }

    // Sort differing blocks by position for consistent output
    differing_blocks.sort_by_key(|d| (d.block_y, d.block_x));

    VarDctLfGroupDiff {
        frame_idx,
        lf_group_idx,
        summary: VarDctDiffSummary {
            total_blocks_a: ann_a.num_varblocks,
            total_blocks_b: ann_b.num_varblocks,
            matching_blocks,
            dct_type_differences,
            size_differences,
            hf_mul_differences,
            only_in_a,
            only_in_b,
        },
        differing_blocks,
    }
}

/// Compare checkpoints between two files.
fn compare_checkpoints(
    checkpoints_a: &[Checkpoint],
    checkpoints_b: &[Checkpoint],
    tolerance: f64,
) -> Vec<CheckpointDiff> {
    let mut diffs = Vec::new();

    // Match checkpoints by stage, frame, and group
    for cp_a in checkpoints_a {
        let matching = checkpoints_b.iter().find(|cp_b| {
            cp_a.stage == cp_b.stage
                && cp_a.frame_idx == cp_b.frame_idx
                && cp_a.group_idx == cp_b.group_idx
                && cp_a.channel == cp_b.channel
        });

        if let Some(cp_b) = matching {
            // Compare statistics
            let diff_mean = (cp_a.stats.mean - cp_b.stats.mean).abs();
            let diff_max = (cp_a.stats.max - cp_b.stats.max).abs();
            let identical = diff_mean < tolerance && diff_max < tolerance;

            diffs.push(CheckpointDiff {
                stage: cp_a.stage,
                frame_idx: cp_a.frame_idx,
                group_idx: cp_a.group_idx,
                diff_stats: CheckpointStats {
                    min: cp_a.stats.min - cp_b.stats.min,
                    max: cp_a.stats.max - cp_b.stats.max,
                    mean: cp_a.stats.mean - cp_b.stats.mean,
                    stddev: (cp_a.stats.stddev - cp_b.stats.stddev).abs(),
                    nonzero_count: cp_a.stats.nonzero_count.abs_diff(cp_b.stats.nonzero_count),
                    total_count: cp_a.stats.total_count,
                },
                max_abs_diff: diff_max,
                mean_abs_diff: diff_mean,
                rms_diff: 0.0, // Would need full data to compute
                identical,
                tolerance,
            });
        }
    }

    diffs
}
