# jxl-inspect

[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org/)

**A command-line tool for inspecting and debugging JPEG XL files.**

Built for encoder developers who need to understand what's inside a JXL bitstream—DCT block decisions, quantization parameters, section sizes, and byte-level differences between files.

## Why This Tool?

When developing a JXL encoder, you need to answer questions like:
- "Why is my file 20% larger than the reference encoder's output?"
- "What DCT block sizes is the reference encoder choosing?"
- "Where exactly do my encoded bytes diverge from the reference?"
- "How is quantization distributed across the image?"

`jxl-inspect` answers these questions with detailed analysis and visual tools.

## Installation

```bash
cargo install --git https://github.com/imazen/jxl-inspect
```

Or build from source:
```bash
git clone https://github.com/imazen/jxl-inspect
cd jxl-inspect
cargo build --release
# Binary: target/release/jxl-inspect
```

## Quick Examples

### See what's in a file
```bash
$ jxl-inspect info photo.jxl

File: photo.jxl
Size: 388268 bytes

Image:
  Dimensions: 2048x2560
  Bit depth: 8 bits
  Color: Color
  XYB encoded: Yes

Frame 1:
  Encoding: VarDCT
  Passes: 1
  LF groups: 2
  Groups: 80
  EPF iterations: 3
  Global scale: 1316
  Quant LF: 16

VarDCT Statistics:
  Total varblocks: 30136
  DCT transform types:
    Dct8              9068 ( 30.1%)
    Dct8x16           5642 ( 18.7%)
    Dct16x8           5514 ( 18.3%)
    Dct16             2485 (  8.2%)
    Dct32             1386 (  4.6%)
    ...
  HF multiplier: min=2, max=9, avg=6.7
```

### See where bytes are spent
```bash
$ jxl-inspect info photo.jxl --breakdown

Section Size Breakdown:
  Section              Bytes     Pct
  -------              -----     ---
  HfCoeff             281035   72.4%   <- AC coefficients dominate
  HfMetadata          101040   26.0%   <- Block decisions & quant
  HfGlobal              3066    0.8%
  LfGlobal              2936    0.8%
  FrameHeaders           181    0.0%
  TOC                    181    0.0%
  TOTAL               388268  100.0%
```

### Visualize DCT block decisions
```bash
$ jxl-inspect block-map photo.jxl --width 64

Block Strategy Map for frame 1 (256x320 blocks, scale 1:4)
Legend: 8=Dct8 G=Dct16 T=Dct32 R=8x16/16x8 W=16x32 A=AFV s=small

8888RRR8888RRRGGGG8888RRRR8888TTTTGGGGRRRRGGGG8888RRRR8888GGGG
RRR88888RRR8888GGGGRRRRTTTT8888GGGGRRRRGGGG8888RRRR8888GGGGRRR
8888GGGGRRRRTTTTGGGG8888RRRR8888GGGGTTTTRRRRGGGG8888RRRR8888GG
GGGGTTTTRRRRGGGG8888RRRR8888GGGGRRRRTTTTGGGG8888RRRR8888GGGGRR
...
```

### Visualize quantization distribution
```bash
$ jxl-inspect quant-map photo.jxl --width 64

HF Multiplier Heatmap for frame 1 (256x320 blocks, scale 1:4)
HF range: 2 - 9 (0=high quality, z=low quality)

5566778899887766554433445566778899aabbccddee
4455667788776655443322334455667788899aabbccd
3344556677665544332211223344556677788899aabb
...
```

### Compare two encoder outputs
```bash
$ jxl-inspect diff reference.jxl my_encoder.jxl -o diff.json

Diff Summary:
  Identical: false
  File A: 388268 bytes
  File B: 391045 bytes
  Matching segments: 8
  Different segments: 3

VarDCT Distribution:
  Blocks A: 30136  |  Blocks B: 30892
  DCT Type           A%      B%    Diff
    Dct8           30.1%   28.4%   +1.7%
    Dct8x16        18.7%   19.2%   -0.5%
    Dct16          8.2%    9.1%    -0.9%
    ...
```

### Find exact byte differences
```bash
$ jxl-inspect hex-diff reference.jxl my_encoder.jxl --with-segments -n 5

File A: reference.jxl (388268 bytes)
File B: my_encoder.jxl (391045 bytes)

Found 2847 byte differences (showing first 5)

Offset 0x00000A3F (2623): 7C -> 8A
  Segment A: HfMetadata { frame_idx: 1, lf_group_idx: 0 }
  Segment B: HfMetadata { frame_idx: 1, lf_group_idx: 0 }
  A:  3F  2A  1B [7C] 9D  4E  2F
  B:  3F  2A  1B [8A] 9D  4E  2F
```

### One-liner for scripting
```bash
$ jxl-inspect info *.jxl --summary

photo1.jxl: 2048x2560 8bit color VarDCT 30136blk Dct8:30% 388268B
photo2.jxl: 1920x1080 8bit color VarDCT 12960blk Dct16:35% 156432B
anim.jxl: 800x600 8bit color VarDCT 24f 45000blk Dct8:42% 892156B
```

---

## Commands Reference

### `info` — File Information

```bash
jxl-inspect info <file.jxl> [OPTIONS]

Options:
  --json        Output as JSON (for programmatic use)
  --summary     One-line output (for scripting)
  --breakdown   Show section size breakdown
  --per-frame   Show per-frame stats (for animations)
```

### `diff` — Semantic Comparison

```bash
jxl-inspect diff <file_a.jxl> <file_b.jxl> -o <output.json> [OPTIONS]

Options:
  --vardct-only       Only compare VarDCT data
  --ignore-container  Ignore container-level differences
  --tolerance <f64>   Tolerance for float comparisons (default: 1e-6)
```

Output JSON includes:
- Segment-by-segment comparison
- VarDCT distribution comparison
- Per-block differences (DCT type, size, HF multiplier)

### `block-map` — DCT Block Visualization

```bash
jxl-inspect block-map <file.jxl> [OPTIONS]

Options:
  --frame <n>   Frame index (default: first VarDCT frame)
  --width <n>   Max output width in chars (default: 80)
```

**Character legend:**
| Char | DCT Type | Block Size |
|------|----------|------------|
| `8` | Dct8 | 8×8 |
| `G` | Dct16 | 16×16 |
| `T` | Dct32 | 32×32 |
| `S` | Dct64 | 64×64 |
| `R` | Dct8x16/16x8 | Rectangular |
| `W` | Dct16x32/32x16 | Wide |
| `A` | AFV variants | Adaptive |
| `s` | Dct4x8/8x4 | Small |
| `4` | Dct4 | 4×4 |
| `2` | Dct2x2 | 2×2 |

### `quant-map` — Quantization Heatmap

```bash
jxl-inspect quant-map <file.jxl> [OPTIONS]

Options:
  --frame <n>   Frame index (default: first VarDCT frame)
  --width <n>   Max output width in chars (default: 80)
```

Characters `0-9` then `a-z` show normalized HF multiplier:
- `0-3`: Low quantization (high quality regions)
- `4-6`: Medium quantization
- `7-z`: High quantization (low quality regions)

### `hex-diff` — Byte-Level Comparison

```bash
jxl-inspect hex-diff <file_a> <file_b> [OPTIONS]

Options:
  -n, --max-diffs <n>   Max differences to show (default: 20)
  --context <n>         Context bytes around diff (default: 4)
  --with-segments       Show JXL segment for each diff location
```

### `export-csv` — Data Export

```bash
jxl-inspect export-csv <file.jxl> -o <output.csv> [OPTIONS]

Options:
  --per-block   Export every block (warning: large files)
```

**Summary mode columns:** `frame_idx`, `encoding`, `total_blocks`, `dct8_pct`, `dct16_pct`, `dct32_pct`, `hf_mul_min`, `hf_mul_max`, `hf_mul_avg`

**Per-block mode columns:** `frame_idx`, `lf_group_idx`, `block_x`, `block_y`, `dct_type`, `size_w`, `size_h`, `hf_mul`

### `hexdump` — Raw Bytes

```bash
jxl-inspect hexdump <file.jxl> [OPTIONS]

Options:
  --bytes <n>    Number of bytes (default: all)
  --offset <n>   Start offset (default: 0)
```

### `inspect` — Full Annotation Export

```bash
jxl-inspect inspect <file.jxl> -o <output_dir/> [OPTIONS]

Options:
  --include-ans          Include ANS symbol data
  --include-checkpoints  Include decoded value checkpoints
  --max-depth <n>        Max nesting depth (default: 10)
  --frames <list>        Only specific frames
```

### `extract` — Extract Segment

```bash
jxl-inspect extract <annotations_dir/> <segment_path> -o <output.json>

# Example segment paths:
#   frame0.lf_group0.hf_metadata
#   frame1.hf_global
```

---

## Understanding the Output

### What is VarDCT?

JPEG XL's VarDCT mode (Variable-size DCT) is similar to JPEG but with:
- **Variable block sizes**: 2×2 up to 64×64, chosen per-region
- **Adaptive quantization**: HF multiplier varies spatially
- **XYB color space**: Perceptually optimized

### Key Metrics

| Metric | What It Tells You |
|--------|-------------------|
| **DCT distribution** | Block size choices—larger blocks = smoother regions |
| **HF multiplier** | Per-block quantization strength (higher = more compression) |
| **Section breakdown** | Where bytes are spent (coefficients vs. metadata) |
| **Global scale** | Overall quantization level |
| **EPF iterations** | Edge-preserving filter strength (0-3) |

---

## Encoder Development Workflows

### "Why is my file larger?"

```bash
# Compare section breakdown
jxl-inspect info reference.jxl --breakdown
jxl-inspect info my_output.jxl --breakdown

# If HfMetadata is larger: block decisions differ
jxl-inspect block-map reference.jxl > ref.txt
jxl-inspect block-map my_output.jxl > mine.txt
diff ref.txt mine.txt

# If HfCoeff is larger: quantization or coefficient coding differs
jxl-inspect diff reference.jxl my_output.jxl -o diff.json
```

### "Where does encoding diverge?"

```bash
# Find first byte difference and its segment
jxl-inspect hex-diff reference.jxl my_output.jxl --with-segments -n 1

# Examine that region in detail
jxl-inspect hexdump my_output.jxl --offset 0x1234 --bytes 128
```

### "Is my DCT selection reasonable?"

```bash
# Visual comparison
jxl-inspect block-map reference.jxl --width 120 > ref_blocks.txt
jxl-inspect block-map my_output.jxl --width 120 > my_blocks.txt

# Statistical comparison
jxl-inspect info reference.jxl --json | jq '.vardct_stats'
jxl-inspect info my_output.jxl --json | jq '.vardct_stats'
```

### Batch analysis

```bash
# Process test corpus
for f in corpus/*.jxl; do
  echo "=== $f ==="
  jxl-inspect info "$f" --summary
done > corpus_stats.txt

# Export for spreadsheet analysis
for f in corpus/*.jxl; do
  jxl-inspect export-csv "$f" -o "${f%.jxl}.csv"
done
```

---

## Environment Variables

- `RUST_LOG=debug` — Verbose logging
- `RUST_LOG=jxl_inspect=trace` — Very verbose

---

## Requirements

- Rust 1.85+ (uses edition 2024)
- Dependencies are fetched automatically via Cargo

This tool uses [jxl-oxide](https://github.com/tirr-c/jxl-oxide) with annotation extensions from [imazen/jxl-oxide](https://github.com/imazen/jxl-oxide).

---

## License

MIT OR Apache-2.0

---

## Contributing

This tool exists to help JXL encoder developers. Contributions that add useful analysis capabilities are welcome:

- Additional visualization modes
- More detailed coefficient analysis
- Modular mode support improvements
- Performance optimizations

File issues at [github.com/imazen/jxl-inspect/issues](https://github.com/imazen/jxl-inspect/issues).
