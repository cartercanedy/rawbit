<div align="center">
<img width="640" height="320" src="https://raw.githubusercontent.com/cartercanedy/rawbit/refs/heads/master/res/rawbit.png"/>
<br>

# rawbit

A **camera RAW image preprocessor and importer** written in Rust.  

Rawbit processes raw image files by converting them to the DNG format in parallel,
while offering the ability to manipulate metadata and customize file name formatting.

</div>

## Features

- **RAW Image Conversion**: Converts camera RAW files to DNG format.
- **Flexible Input/Output**:
  - Process individual files or entire directories.
  - Define output directories with optional overwrite support.
- **Custom Filename Formatting**: Supports user-defined naming conventions for output files.
- **Metadata Handling**: Supports EXIF metadata manipulation
- **Multi-Threaded Processing**: Leverages multiple CPU cores for parallel image processing.

*__all written in Rust, btw...__*

## Installation

### Pre-built binaries
Pre-built binaries releases are available for download from the latest [GitHub release](https://github.com/cartercanedy/rawbit/releases/latest).

I plan on making binary releases available for all major platforms via package managers.
In the meantime, there are [AUR](https://aur.archlinux.org) & [crates.io](https://crates.io) packages available:

### Arch Linux

You can install rawbit with your preferred [AUR helper](https://wiki.archlinux.org/title/AUR_helpers). Example:

```sh
paru -S rawbit
```

### Crates.io

1. Install [Rust](https://www.rust-lang.org/tools/install) and ensure `cargo` is available.
2. Install via cargo:
```sh
cargo install rawbit
```

## Examples

### Convert a single file

```sh
rawbit --out-dir "./dng" --format "%Y-%m-%d_%H-%M-%S_{image.original_filename}" ./raw/ABC1234.ARW

# or 

rawbit -o"./dng" -F"%Y-%m-%d_%H-%M-%S_{image.original_filename}" ./raw/ABC1234.ARW
```

### Convert an entire directory

```sh
rawbit --in-dir "./raw" --out-dir "./dng" --format "%Y-%m-%d_{camera.model}_{lens.model}_{image.original_filename}"

# or

rawbit -i"./raw" -o"./dng" -F"%Y-%m-%d_{camera.model}_{lens.model}_{image.original_filename}"
```

## Usage

<pre>
<span style="color: #F5F1DE">Usage:</span> <span style="color: #00aaaa">rawbit</span> <span style="color: #00aaaa">[OPTIONS]</span> <span style="color: #00aaaa">--out-dir</span><span style="color: #00aaaa"> </span><span style="color: #00aaaa">&lt;DIR&gt;</span> <span style="color: #00aaaa">&lt;--in-dir &lt;DIR&gt;|FILES&gt;</span>

<span style="color: #aa5500">Arguments:</span>
  <span style="color: #00aaaa">[FILES]...</span>
          individual files to convert

<span style="color: #aa5500">Options:</span>
  <span style="color: #00aaaa">-i</span>, <span style="color: #00aaaa">--in-dir</span><span style="color: #00aaaa"> </span><span style="color: #00aaaa">&lt;DIR&gt;</span>
          directory containing raw files to convert
  <span style="color: #00aaaa">-o</span>, <span style="color: #00aaaa">--out-dir</span><span style="color: #00aaaa"> </span><span style="color: #00aaaa">&lt;DIR&gt;</span>
          directory to write converted DNGs
  <span style="color: #00aaaa">-F</span>, <span style="color: #00aaaa">--format</span><span style="color: #00aaaa"> </span><span style="color: #00aaaa">&lt;FORMAT&gt;</span>
          filename format of converted DNGs; see https://docs.rs/rawbit for info on syntax
  <span style="color: #00aaaa">-a</span>, <span style="color: #00aaaa">--artist</span><span style="color: #00aaaa"> </span><span style="color: #00aaaa">&lt;ARTIST&gt;</span>
          value of the "artist" field in converted DNGs
  <span style="color: #00aaaa">-e</span>, <span style="color: #00aaaa">--embed-raw</span>
          embed the original raw image in the converted DNG
          NOTE: conversion may take considerably longer
  <span style="color: #00aaaa">-f</span>, <span style="color: #00aaaa">--force</span>
          overwrite existing files, if they exist
  <span style="color: #00aaaa">-r</span>, <span style="color: #00aaaa">--recurse</span>
          ingest images from subdirectories as well, preserving directory structure in the output
      <span style="color: #00aaaa">--no-preview</span>
          don't embed image preview in output DNG
      <span style="color: #00aaaa">--no-thumbnail</span>
          don't embed image thumbnail in output DNG
      <span style="color: #00aaaa">--dry-run</span>
          only print run information, don't perform conversions or write any data.
  <span style="color: #00aaaa">-j</span>, <span style="color: #00aaaa">--n-threads</span><span style="color: #00aaaa"> [</span><span style="color: #00aaaa">&lt;N&gt;</span><span style="color: #00aaaa">]</span>
          number of threads to use while processing input images, defaults to number of CPUs
  <span style="color: #00aaaa">-q</span>, <span style="color: #00aaaa">--quiet</span>
          quiet output, only emit critical errors
  <span style="color: #00aaaa">-v</span>, <span style="color: #00aaaa">--verbose</span><span style="color: #00aaaa">...</span>
          increase log verbosity; specify multiple times to increase verbosity
  <span style="color: #00aaaa">-h</span>, <span style="color: #00aaaa">--help</span>
          Print help
  <span style="color: #00aaaa">-V</span>, <span style="color: #00aaaa">--version</span>
          Print version
</pre>

## Filename formatting

This is the distinguishing feature of `rawbit`.

### Date/time interpolation

You can insert the date-time information read from the RAW image's metadata using
syntax similar to libc's `strftime`.
More information can be found [here](https://docs.rs/chrono/latest/chrono/format/strftime/index.html)

### Metadata interpolation

Similar to the date/time interpolation, some well-known names in between squirly braces (i.e.
"{...}") expands into image-specific EXIF metadata in the filename:
| Variable      | Description | Example |
|---------------|-------------|---------|
| `camera.make` | Camera make | |
| `camera.model` | Camera model | |
| `camera.shutter_speed` | Shutter speed used to take the image | |
| `camera.iso` | Sensor sensitivity (ISO) used to take the image | |
| `lens.make` | Lens make | |
| `lens.model` | Lens model | |
| `lens.f_stop` | Lens aperture F stop value use to take the image | |
| `image.original_filename` | Image's original filename.<br>Automatically inserted if not specified in the original format string | |

*__Note:__*  
More metadata fields are a WIP, more to come soon...

## Why not use [`dnglab`](https://github.com/dnglab/dnglab)?

`dnglab convert` is extremely versatile and robust, but my main motivation for developing `rawbit` was to enable a more flexible batch DNG conversion/import workflow with entirely free (as in freedom) software enabling it.

This project utilizes the same library that powers DNGLab, so I owe a huge thanks to the DNGLab/Rawler team for their awesome work that made this project possible.

## Special thanks

[DNGLab/Rawler](https://github.com/dnglab/dnglab/blob/main/rawler): Rust-native RAW image manipulation tools from the ground-up  
[rayon](https://github.com/rayon-rs/rayon)/[tokio](https://github.com/tokio-rs/tokio): For making fearless concurrency a peice of cake  
[Adam Perkowski](https://github.com/adamperkowski): Contributing CI and package manager support  
