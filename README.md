# DVP8-Detector

`p8detector` scans a directory and lists every video file that has a **Dolby Vision Profile 8.1** stream (a P8 file with an HDR10-compatible base layer).

```console
$ p8detector ./
./Movies/Example (2023).mkv
./Shows/Example S01E01.mp4
```

## How it works

This tool doesn't parse files itself. It runs [`ffprobe`](https://ffmpeg.org/ffprobe.html) on each video file and reads the Dolby Vision configuration record (`dvcC`/`dvvC`) from the stream side data:

| Profile | `dv_profile` | `dv_bl_signal_compatibility_id` |
|---------|--------------|---------------------------------|
| 8.1     | 8            | 1 (HDR10)                       |
| 8.2     | 8            | 2 (SDR)                         |
| 8.4     | 8            | 4 (HLG)                         |

Files are probed in parallel. It scans files with these extensions: `.mkv .mp4 .m4v .mov .ts .m2ts .mts`.

## Requirements

- `ffprobe` on your `PATH` (it comes with ffmpeg, e.g. `apt install ffmpeg` or `brew install ffmpeg`)
- A Rust toolchain to build it

## Install

```sh
cargo install --path .
```

## Usage

```
p8detector [OPTIONS] [PATHS]...

Arguments:
  [PATHS]...  Directories (or files) to scan [default: .]

Options:
      --no-recursive  Only scan the top level of each directory
      --any-p8        Match any Profile 8 variant (8.1, 8.2, 8.4), not just 8.1
  -v, --verbose       Print the detected Dolby Vision profile next to each file
  -j, --jobs <JOBS>   Number of parallel ffprobe processes (default: CPU count)
```

Matching paths are printed to stdout, one per line, and errors go to stderr, so you can pipe the output into other tools. The exit code is `0` if any file matched, `1` if none did, and `2` if `ffprobe` is missing.

## Limitations

Raw elementary streams (`.hevc`/`.h265`) have no container-level Dolby Vision configuration record, so they aren't scanned. Mux them into MKV or MP4 first.
