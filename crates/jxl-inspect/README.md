# jxl-inspect

A command-line tool for inspecting and analyzing JPEG XL (JXL) bitstreams. Built on top of [jxl-oxide](https://github.com/tirr-c/jxl-oxide), this tool provides detailed insights into JXL file structure, VarDCT encoding parameters, and enables comparison between files.

**Primary use case:** Debugging JXL encoders by analyzing the bitstream structure and comparing encoder outputs.

## Installation

```bash
# From source (requires Rust 1.80+)
cargo install --git https://github.com/imazen/jxl-inspect

# Or clone and build
git clone https://github.com/imazen/jxl-inspect
cd jxl-inspect
cargo build --release
```

## Quick Start

```bash
# Get detailed info about a JXL file
jxl-inspect info image.jxl

# One-line summary for scripting
jxl-inspect info image.jxl --summary

# Compare two JXL files
jxl-inspect diff encoder_a.jxl encoder_b.jxl -o diff.json

# Visualize DCT block types
jxl-inspect block-map image.jxl

# Visualize quantization distribution
jxl-inspect quant-map image.jxl
```

## Commands

### `info` - File Information

Show detailed information about a JXL file including image metadata, frame info, and VarDCT statistics.

```bash
# Basic info
jxl-inspect info image.jxl

# JSON output (for programmatic use)
jxl-inspect info image.jxl --json

# Per-frame statistics (useful for animations)
jxl-inspect info animation.jxl --per-frame

# Section size breakdown (% of file per segment type)
jxl-inspect info image.jxl --breakdown

# One-line summary (for scripting/batch processing)
jxl-inspect info image.jxl --summary

# Process multiple files
jxl-inspect info *.jxl --summary
```

**Example output:**
```
File: image.jxl
Size: 388268 bytes

Image:
  Dimensions: 2048x2560
  Bit depth: 8 bits
  Color: Color
  XYB encoded: Yes
  Orientation: 1
  Extra channels: 0

Frame 1:
  Encoding: VarDCT
  Size: 2048x2560
  Passes: 1
  LF groups: 2
  Groups: 80
  X QM scale: 3
  B QM scale: 2
  EPF iterations: 3
  Global scale: 1316
  Quant LF: 16
  DC quant step: 0.194529

VarDCT Statistics:
  Total varblocks: 30136
  DCT transform types:
    Dct8              9068 ( 30.1%)
    Dct8x16           5642 ( 18.7%)
    Dct16x8           5514 ( 18.3%)
    Dct16             2485 (  8.2%)
    ...
  HF multiplier: min=2, max=9, avg=6.7
```

**Section breakdown output:**
```
Section Size Breakdown:
  Section              Bytes     Pct
  -------              -----     ---
  HfCoeff             281035   72.4%
  HfMetadata          101040   26.0%
  HfGlobal              3066    0.8%
  LfGlobal              2936    0.8%
  FrameHeaders           181    0.0%
  TOC                    181    0.0%
  TOTAL               388268  100.0%
```

### `diff` - Semantic Comparison

Compare two JXL files and output detailed differences in VarDCT block parameters.

```bash
# Basic diff
jxl-inspect diff file_a.jxl file_b.jxl -o diff.json

# Only compare VarDCT data
jxl-inspect diff file_a.jxl file_b.jxl -o diff.json --vardct-only

# Ignore container differences
jxl-inspect diff file_a.jxl file_b.jxl -o diff.json --ignore-container

# Set tolerance for floating-point comparisons
jxl-inspect diff file_a.jxl file_b.jxl -o diff.json --tolerance 1e-4
```

**Console output includes:**
- File size comparison
- Segment matching summary
- VarDCT distribution comparison (DCT type percentages in both files)
- HF multiplier statistics comparison
- Block-level difference counts

### `block-map` - DCT Block Type Visualization

Display an ASCII art visualization of DCT transform types used across the image.

```bash
# Default width (80 chars)
jxl-inspect block-map image.jxl

# Wider output
jxl-inspect block-map image.jxl --width 120

# Specific frame (for animations)
jxl-inspect block-map animation.jxl --frame 5
```

**Legend:**
- `8` = Dct8 (8x8)
- `G` = Dct16 (16x16)
- `T` = Dct32 (32x32)
- `S` = Dct64 (64x64)
- `R` = Dct8x16 or Dct16x8 (rectangular)
- `W` = Dct16x32 or Dct32x16 (wide)
- `A` = AFV (Adaptive Foveal Vision) variants
- `s` = Small transforms (Dct4x8, Dct8x4)
- `4` = Dct4
- `2` = Dct2x2

### `quant-map` - Quantization Heatmap

Display an ASCII heatmap of HF multiplier (quantization) values across the image.

```bash
jxl-inspect quant-map image.jxl
jxl-inspect quant-map image.jxl --width 100
```

Characters `0-9` and `a-z` represent normalized HF multiplier values:
- Lower values (0, 1, 2...) = lower quantization = higher quality
- Higher values (...x, y, z) = higher quantization = lower quality

Useful for visualizing spatially-adaptive quantization patterns.

### `hex-diff` - Byte-Level Comparison

Compare two files at the byte level and show the first N differences with context.

```bash
# Show first 20 differences
jxl-inspect hex-diff file_a.jxl file_b.jxl

# Show first 50 differences
jxl-inspect hex-diff file_a.jxl file_b.jxl -n 50

# More context bytes around differences
jxl-inspect hex-diff file_a.jxl file_b.jxl --context 8

# Show JXL segment context for each difference
jxl-inspect hex-diff file_a.jxl file_b.jxl --with-segments
```

**Example output:**
```
File A: encoder_a.jxl (388268 bytes)
File B: encoder_b.jxl (388290 bytes)

Found 1523 byte differences (showing first 20)

Offset 0x00000007 (7): 84 -> 08
  Segment B: FrameHeader { frame_idx: 0, encoding: "Modular" }
  A:  4F  E8  FF  80 [84] 12  04  0C  C5
  B:  13  E8  7F  0C [08] 81  00  00  8E
  A: | O  .  .  . [.] .  .  .  . |
  B: | .  .  .  . [.] .  .  .  . |
```

### `export-csv` - CSV Export

Export VarDCT block statistics to CSV for analysis in spreadsheets or data tools.

```bash
# Summary statistics per frame
jxl-inspect export-csv image.jxl -o stats.csv

# Per-block data (warning: can be very large)
jxl-inspect export-csv image.jxl -o blocks.csv --per-block
```

**Summary CSV columns:**
- `frame_idx` - Frame index
- `encoding` - Encoding type (VarDCT/Modular)
- `total_blocks` - Total varblock count
- `dct8_pct`, `dct16_pct`, `dct32_pct`, etc. - DCT type percentages
- `hf_mul_min`, `hf_mul_max`, `hf_mul_avg` - HF multiplier statistics

**Per-block CSV columns:**
- `frame_idx`, `lf_group_idx` - Location
- `block_x`, `block_y` - Block position
- `dct_type` - DCT transform type
- `size_w`, `size_h` - Block size in 8x8 units
- `hf_mul` - HF quantization multiplier

### `hexdump` - Hex Dump

Show a hex dump of the raw JXL file bytes.

```bash
# Dump entire file
jxl-inspect hexdump image.jxl

# Dump first 256 bytes
jxl-inspect hexdump image.jxl --bytes 256

# Dump 128 bytes starting at offset 1024
jxl-inspect hexdump image.jxl --offset 1024 --bytes 128
```

### `inspect` - Full Annotation Export

Export detailed bitstream annotations to a directory (for advanced analysis).

```bash
jxl-inspect inspect image.jxl -o ./annotations/

# Include ANS symbol data
jxl-inspect inspect image.jxl -o ./annotations/ --include-ans

# Include decoded checkpoints
jxl-inspect inspect image.jxl -o ./annotations/ --include-checkpoints
```

### `extract` - Extract Segment

Extract a specific segment from previously generated annotations.

```bash
jxl-inspect extract ./annotations/ "frame0.lf_group0.hf_metadata" -o segment.json
```

## Use Cases for Encoder Development

### Comparing Encoder Outputs

When developing a JXL encoder, compare your output against a reference:

```bash
# Quick comparison
jxl-inspect diff reference.jxl my_encoder.jxl -o diff.json

# Check if DCT selection is similar
jxl-inspect block-map reference.jxl > ref_blocks.txt
jxl-inspect block-map my_encoder.jxl > my_blocks.txt
diff ref_blocks.txt my_blocks.txt

# Compare quantization patterns
jxl-inspect quant-map reference.jxl > ref_quant.txt
jxl-inspect quant-map my_encoder.jxl > my_quant.txt
```

### Analyzing Encoding Efficiency

```bash
# See where bytes are spent
jxl-inspect info image.jxl --breakdown

# Export for detailed analysis
jxl-inspect export-csv image.jxl -o analysis.csv --per-block
```

### Debugging Bitstream Issues

```bash
# Find exact byte differences
jxl-inspect hex-diff expected.jxl actual.jxl --with-segments -n 100

# Examine raw bytes at specific location
jxl-inspect hexdump actual.jxl --offset 0x1234 --bytes 64
```

### Batch Processing

```bash
# Get one-line summary for all files
for f in *.jxl; do jxl-inspect info "$f" --summary; done

# JSON output for processing
jxl-inspect info *.jxl --json | jq '.vardct_stats.total_blocks'
```

## Output Formats

- **Console:** Human-readable text output (default)
- **JSON:** Structured data with `--json` flag (info command)
- **CSV:** Tabular data export (export-csv command)
- **ASCII Art:** Visual representations (block-map, quant-map commands)

## Environment Variables

- `RUST_LOG` - Control log verbosity (e.g., `RUST_LOG=debug jxl-inspect info image.jxl`)

## Building from Source

```bash
git clone https://github.com/imazen/jxl-inspect
cd jxl-inspect
cargo build --release
# Binary at target/release/jxl-inspect
```

## Dependencies

This tool is built on [jxl-oxide](https://github.com/tirr-c/jxl-oxide), a pure Rust JPEG XL decoder.

## License

MIT OR Apache-2.0

## Contributing

Contributions welcome! This tool is designed to help JXL encoder developers, so features that aid debugging and analysis are especially appreciated.
