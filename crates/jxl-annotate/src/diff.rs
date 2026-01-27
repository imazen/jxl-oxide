//! Diff command implementation for comparing two JXL files.

use crate::annotator::{annotate_file, AnnotateOptions};
use jxl_bitstream::annotate::{Checkpoint, CheckpointDiff, CheckpointStats};
use serde::{Deserialize, Serialize};
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

    // Compare checkpoints
    let checkpoint_diffs = compare_checkpoints(
        &result_a.checkpoints,
        &result_b.checkpoints,
        config.tolerance,
    );

    let summary = DiffSummary {
        identical: different == 0 && missing_in_a == 0 && checkpoint_diffs.iter().all(|d| d.identical),
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
        checkpoint_diffs,
    })
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
