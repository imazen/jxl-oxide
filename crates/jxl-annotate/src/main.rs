//! JXL Annotate - Bitstream annotation and analysis tool for JPEG XL files.
//!
//! This tool provides:
//! - Byte-level annotation of JXL bitstreams
//! - Semantic segmentation by algorithm type (VarDCT, Modular)
//! - Comparison of two JXL files
//! - Export of decoded checkpoint values for pipeline analysis

use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod annotator;
mod inspect;
mod diff;
mod output;

#[derive(Parser)]
#[command(name = "jxl-annotate")]
#[command(about = "JXL bitstream annotation and analysis tool")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Inspect a JXL file and produce annotations
    Inspect {
        /// Input JXL file
        input: PathBuf,

        /// Output directory for annotations
        #[arg(short, long)]
        output: PathBuf,

        /// Include ANS symbol data in separate files
        #[arg(long)]
        include_ans: bool,

        /// Include decoded value checkpoints
        #[arg(long)]
        include_checkpoints: bool,

        /// Maximum depth for nested annotations
        #[arg(long, default_value = "10")]
        max_depth: usize,

        /// Only annotate specific frame indices
        #[arg(long)]
        frames: Option<Vec<u32>>,
    },

    /// Compare two JXL files semantically
    Diff {
        /// First JXL file
        file_a: PathBuf,

        /// Second JXL file
        file_b: PathBuf,

        /// Output file for diff results
        #[arg(short, long)]
        output: PathBuf,

        /// Only compare VarDCT data
        #[arg(long)]
        vardct_only: bool,

        /// Ignore container-level differences
        #[arg(long)]
        ignore_container: bool,

        /// Tolerance for floating-point comparisons
        #[arg(long, default_value = "1e-6")]
        tolerance: f64,
    },

    /// Show basic info about a JXL file (like jxlinfo but with more detail)
    Info {
        /// Input JXL file(s) - accepts multiple files
        #[arg(required = true)]
        input: Vec<PathBuf>,

        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Show per-frame VarDCT statistics (useful for animation analysis)
        #[arg(long)]
        per_frame: bool,

        /// Show one-line summary (useful for scripting)
        #[arg(long, short)]
        summary: bool,
    },

    /// Extract a specific segment from annotations
    Extract {
        /// Input annotation directory (from inspect command)
        input: PathBuf,

        /// Segment path (e.g., "frame0.lf_group0.hf_metadata")
        segment: String,

        /// Output file
        #[arg(short, long)]
        output: PathBuf,
    },

    /// Show hex dump of a JXL file with basic annotations
    Hexdump {
        /// Input JXL file
        input: PathBuf,

        /// Number of bytes to show (default: all)
        #[arg(short, long)]
        bytes: Option<usize>,

        /// Start offset in bytes
        #[arg(short, long, default_value = "0")]
        offset: usize,
    },
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Inspect {
            input,
            output,
            include_ans,
            include_checkpoints,
            max_depth,
            frames,
        } => {
            inspect::run_inspect(
                &input,
                &output,
                include_ans,
                include_checkpoints,
                max_depth,
                frames.as_deref(),
            )?;
        }

        Commands::Diff {
            file_a,
            file_b,
            output,
            vardct_only,
            ignore_container,
            tolerance,
        } => {
            diff::run_diff(
                &file_a,
                &file_b,
                &output,
                vardct_only,
                ignore_container,
                tolerance,
            )?;
        }

        Commands::Info { input, json, per_frame, summary } => {
            for file in &input {
                if let Err(e) = inspect::run_info(file, json, per_frame, summary) {
                    eprintln!("Error processing {}: {}", file.display(), e);
                }
            }
        }

        Commands::Extract {
            input,
            segment,
            output,
        } => {
            output::extract_segment(&input, &segment, &output)?;
        }

        Commands::Hexdump {
            input,
            bytes,
            offset,
        } => {
            inspect::run_hexdump(&input, bytes, offset)?;
        }
    }

    Ok(())
}
