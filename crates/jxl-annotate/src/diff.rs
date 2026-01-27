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
    /// Overall VarDCT distribution comparison
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vardct_summary: Option<VarDctOverallSummary>,
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
    /// Total bytes in file A
    pub total_bytes_a: u64,
    /// Total bytes in file B
    pub total_bytes_b: u64,
}

/// DCT type distribution comparison.
#[derive(Debug, Serialize, Deserialize)]
pub struct DctDistributionComparison {
    /// DCT type name
    pub dct_type: String,
    /// Count in file A
    pub count_a: u32,
    /// Count in file B
    pub count_b: u32,
    /// Percentage in file A
    pub pct_a: f64,
    /// Percentage in file B
    pub pct_b: f64,
    /// Difference (pct_a - pct_b)
    pub pct_diff: f64,
}

/// VarDCT overall summary comparing distributions.
#[derive(Debug, Serialize, Deserialize)]
pub struct VarDctOverallSummary {
    pub total_blocks_a: u32,
    pub total_blocks_b: u32,
    /// DCT type distribution comparison
    pub dct_distribution: Vec<DctDistributionComparison>,
    /// HF multiplier stats for file A
    pub hf_mul_stats_a: HfMulStats,
    /// HF multiplier stats for file B
    pub hf_mul_stats_b: HfMulStats,
}

/// HF multiplier statistics.
#[derive(Debug, Serialize, Deserialize)]
pub struct HfMulStats {
    pub min: i32,
    pub max: i32,
    pub avg: f64,
}

/// Difference in a segment.
#[derive(Debug, Serialize, Deserialize)]
pub struct SegmentDiff {
    pub segment_kind: String,
    pub status: SegmentDiffStatus,
    /// Byte size in file A (if present)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_a: Option<u64>,
    /// Byte size in file B (if present)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_b: Option<u64>,
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
    println!("  File A: {} bytes", diff.summary.total_bytes_a);
    println!("  File B: {} bytes", diff.summary.total_bytes_b);
    println!("  Matching segments: {}", diff.summary.matching_segments);
    println!("  Different segments: {}", diff.summary.different_segments);
    println!("  Missing in A: {}", diff.summary.missing_in_a);
    println!("  Missing in B: {}", diff.summary.missing_in_b);

    // VarDCT overall summary
    if let Some(ref vardct_sum) = diff.vardct_summary {
        println!();
        println!("VarDCT Distribution:");
        println!("  Blocks A: {}  |  Blocks B: {}", vardct_sum.total_blocks_a, vardct_sum.total_blocks_b);
        println!("  DCT Type           A%      B%    Diff");
        for dct in vardct_sum.dct_distribution.iter().take(8) {
            println!(
                "    {:15} {:5.1}%  {:5.1}%  {:+5.1}%",
                dct.dct_type, dct.pct_a, dct.pct_b, dct.pct_diff
            );
        }
        if vardct_sum.dct_distribution.len() > 8 {
            println!("    ... and {} more types", vardct_sum.dct_distribution.len() - 8);
        }
        println!(
            "  HF mul A: min={}, max={}, avg={:.1}",
            vardct_sum.hf_mul_stats_a.min, vardct_sum.hf_mul_stats_a.max, vardct_sum.hf_mul_stats_a.avg
        );
        println!(
            "  HF mul B: min={}, max={}, avg={:.1}",
            vardct_sum.hf_mul_stats_b.min, vardct_sum.hf_mul_stats_b.max, vardct_sum.hf_mul_stats_b.avg
        );
    }

    // VarDCT per-block differences
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

        if total_dct_diffs > 0 || total_size_diffs > 0 || total_hf_mul_diffs > 0 {
            println!();
            println!("VarDCT Block Differences:");
            println!("  DCT type differences: {}", total_dct_diffs);
            println!("  Size differences: {}", total_size_diffs);
            println!("  HF multiplier differences: {}", total_hf_mul_diffs);
        }
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

    // Compute total bytes for summary
    let total_bytes_a: u64 = segments_a.iter().map(|s| s.byte_range.1 - s.byte_range.0).sum();
    let total_bytes_b: u64 = segments_b.iter().map(|s| s.byte_range.1 - s.byte_range.0).sum();

    // Compare segments by kind with size information
    for seg_a in &segments_a {
        let kind_str = format!("{:?}", seg_a.kind);
        let size_a = seg_a.byte_range.1 - seg_a.byte_range.0;
        let matching_b = segments_b.iter().find(|s| format!("{:?}", s.kind) == kind_str);

        if let Some(seg_b) = matching_b {
            let size_b = seg_b.byte_range.1 - seg_b.byte_range.0;
            let size_matches = size_a == size_b;
            segment_diffs.push(SegmentDiff {
                segment_kind: kind_str,
                status: if size_matches { SegmentDiffStatus::Identical } else { SegmentDiffStatus::Different },
                size_a: Some(size_a),
                size_b: Some(size_b),
                details: if !size_matches {
                    Some(format!("Size differs: {} vs {} bytes", size_a, size_b))
                } else {
                    None
                },
            });
            matching += 1;
        } else {
            segment_diffs.push(SegmentDiff {
                segment_kind: kind_str,
                status: SegmentDiffStatus::OnlyInA,
                size_a: Some(size_a),
                size_b: None,
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
            let size_b = seg_b.byte_range.1 - seg_b.byte_range.0;
            segment_diffs.push(SegmentDiff {
                segment_kind: kind_str,
                status: SegmentDiffStatus::OnlyInB,
                size_a: None,
                size_b: Some(size_b),
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

    // Compute overall VarDCT summary
    let vardct_summary = compute_vardct_overall_summary(
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
        total_bytes_a,
        total_bytes_b,
    };

    Ok(DiffResult {
        file_a: result_a.manifest.source_file.to_string_lossy().to_string(),
        file_b: result_b.manifest.source_file.to_string_lossy().to_string(),
        summary,
        segment_diffs,
        vardct_summary,
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

/// Compute overall VarDCT distribution comparison.
fn compute_vardct_overall_summary(
    annotations_a: &[HfMetadataAnnotation],
    annotations_b: &[HfMetadataAnnotation],
) -> Option<VarDctOverallSummary> {
    if annotations_a.is_empty() && annotations_b.is_empty() {
        return None;
    }

    // Aggregate DCT type counts and HF mul stats for A
    let mut dct_counts_a: HashMap<String, u32> = HashMap::new();
    let mut total_blocks_a = 0u32;
    let mut hf_mul_sum_a = 0i64;
    let mut hf_mul_min_a = i32::MAX;
    let mut hf_mul_max_a = i32::MIN;

    for ann in annotations_a {
        total_blocks_a += ann.num_varblocks;
        for block in &ann.varblocks {
            *dct_counts_a.entry(block.dct_select.clone()).or_default() += 1;
            hf_mul_sum_a += block.hf_mul as i64;
            hf_mul_min_a = hf_mul_min_a.min(block.hf_mul);
            hf_mul_max_a = hf_mul_max_a.max(block.hf_mul);
        }
    }

    // Aggregate DCT type counts and HF mul stats for B
    let mut dct_counts_b: HashMap<String, u32> = HashMap::new();
    let mut total_blocks_b = 0u32;
    let mut hf_mul_sum_b = 0i64;
    let mut hf_mul_min_b = i32::MAX;
    let mut hf_mul_max_b = i32::MIN;

    for ann in annotations_b {
        total_blocks_b += ann.num_varblocks;
        for block in &ann.varblocks {
            *dct_counts_b.entry(block.dct_select.clone()).or_default() += 1;
            hf_mul_sum_b += block.hf_mul as i64;
            hf_mul_min_b = hf_mul_min_b.min(block.hf_mul);
            hf_mul_max_b = hf_mul_max_b.max(block.hf_mul);
        }
    }

    // Build combined list of DCT types
    let mut all_types: Vec<String> = dct_counts_a.keys().cloned().collect();
    for key in dct_counts_b.keys() {
        if !all_types.contains(key) {
            all_types.push(key.clone());
        }
    }

    // Sort by total count (A + B) descending
    all_types.sort_by(|a, b| {
        let count_a = dct_counts_a.get(a).unwrap_or(&0) + dct_counts_b.get(a).unwrap_or(&0);
        let count_b = dct_counts_a.get(b).unwrap_or(&0) + dct_counts_b.get(b).unwrap_or(&0);
        count_b.cmp(&count_a)
    });

    // Build distribution comparison
    let dct_distribution: Vec<DctDistributionComparison> = all_types
        .iter()
        .map(|dct_type| {
            let count_a = *dct_counts_a.get(dct_type).unwrap_or(&0);
            let count_b = *dct_counts_b.get(dct_type).unwrap_or(&0);
            let pct_a = if total_blocks_a > 0 {
                (count_a as f64 / total_blocks_a as f64) * 100.0
            } else {
                0.0
            };
            let pct_b = if total_blocks_b > 0 {
                (count_b as f64 / total_blocks_b as f64) * 100.0
            } else {
                0.0
            };
            DctDistributionComparison {
                dct_type: dct_type.clone(),
                count_a,
                count_b,
                pct_a,
                pct_b,
                pct_diff: pct_a - pct_b,
            }
        })
        .collect();

    // Compute HF mul stats
    let hf_mul_stats_a = if total_blocks_a > 0 {
        HfMulStats {
            min: hf_mul_min_a,
            max: hf_mul_max_a,
            avg: hf_mul_sum_a as f64 / total_blocks_a as f64,
        }
    } else {
        HfMulStats { min: 0, max: 0, avg: 0.0 }
    };

    let hf_mul_stats_b = if total_blocks_b > 0 {
        HfMulStats {
            min: hf_mul_min_b,
            max: hf_mul_max_b,
            avg: hf_mul_sum_b as f64 / total_blocks_b as f64,
        }
    } else {
        HfMulStats { min: 0, max: 0, avg: 0.0 }
    };

    Some(VarDctOverallSummary {
        total_blocks_a,
        total_blocks_b,
        dct_distribution,
        hf_mul_stats_a,
        hf_mul_stats_b,
    })
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
