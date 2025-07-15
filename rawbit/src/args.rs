use std::{
    fs::read_dir,
    path::{Path, PathBuf},
    thread::available_parallelism,
};

use clap::{
    ArgAction, Args, Parser, arg,
    builder::{
        IntoResettable, Styles,
        styling::{AnsiColor, Color, Style},
    },
    command, value_parser,
};
use rayon::iter::{IntoParallelIterator as _, ParallelBridge as _, ParallelIterator as _};
use smlog::{debug, warn};

use rawler::decoders::supported_extensions;

use crate::common::{AppError, RawbitResult, map_err};

macro_rules! style {
    ($style:expr) => {
        Style::new().fg_color(Some(Color::Ansi($style)))
    };
}

const fn cli_style() -> Styles {
    Styles::styled()
        .header(style!(AnsiColor::Yellow))
        .error(style!(AnsiColor::Red))
        .literal(style!(AnsiColor::Cyan))
        .invalid(style!(AnsiColor::Red))
        .usage(style!(AnsiColor::White))
        .placeholder(style!(AnsiColor::Cyan))
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Parser)]
#[command(
    version,
    about = "A camera RAW image preprocessor and importer",
    long_about = None,
    trailing_var_arg = true,
    styles = cli_style(),
    next_line_help = true,
    color = clap::ColorChoice::Always
)]
pub struct ImportConfig {
    #[command(flatten)]
    pub source: RawSource,

    #[arg(
        short = 'o',
        long = "out-dir",
        value_name = "DIR",
        help = "directory to write converted DNGs"
    )]
    pub output_dir: PathBuf,

    #[arg(
        short = 'F',
        long = "format",
        value_name = "FORMAT",
        help = "filename format of converted DNGs; see https://docs.rs/rawbit for info on syntax"
    )]
    pub fmt_str: Option<String>,

    #[arg(
        short,
        long,
        value_name = "ARTIST",
        help = "value of the \"artist\" field in converted DNGs"
    )]
    pub artist: Option<String>,

    #[arg(
        short,
        long = "embed-raw",
        action = ArgAction::SetTrue,
        help = "embed the original raw image in the converted DNG\nNOTE: conversion may take considerably longer"
    )]
    pub embed: bool,

    #[arg(
        short,
        long,
        action = ArgAction::SetTrue,
        help = "overwrite existing files, if they exist"
    )]
    pub force: bool,

    #[arg(
        short,
        long,
        action = ArgAction::SetTrue,
        help = "ingest images from subdirectories as well, preserving directory structure in the output"
    )]
    pub recurse: bool,

    #[arg(
        long,
        action = ArgAction::SetTrue,
        help = "don't embed image preview in output DNG"
    )]
    pub no_preview: bool,

    #[arg(
        long,
        action = ArgAction::SetTrue,
        help = "don't embed image thumbnail in output DNG"
    )]
    pub no_thumbnail: bool,

    #[arg(
        long,
        action = ArgAction::SetTrue,
        help = "only print run information, don't perform conversions or write any data."
    )]
    pub dry_run: bool,

    #[arg(
        short = 'j',
        long,
        action = ArgAction::Set,
        default_missing_value = "",
        num_args = 0..=1,
        value_name = "N",
        help = "number of threads to use while processing input images, defaults to number of CPUs"
    )]
    pub n_threads: Option<usize>,

    #[command(flatten)]
    pub log_config: LogConfig,
}

impl ImportConfig {
    pub fn n_threads(&self) -> usize {
        let default_threads = available_parallelism().unwrap().get();
        self.n_threads.unwrap_or(default_threads)
    }
}

#[derive(Debug, Args)]
#[group(multiple = false)]
pub struct LogConfig {
    #[arg(
        short,
        long,
        help = "quiet output, only emit critical errors",
        trailing_var_arg = false
    )]
    pub quiet: bool,

    #[arg(
        short,
        long,
        action = ArgAction::Count,
        help = "increase log verbosity; specify multiple times to increase verbosity"
    )]
    pub verbose: u8,
}

#[derive(Debug, Args)]
#[group(required = true, multiple = false)]
pub struct RawSource {
    #[arg(
        short = 'i',
        long = "in-dir",
        value_name = "DIR",
        value_parser = value_parser!(PathBuf).into_resettable(),
        help = "directory containing raw files to convert"
    )]
    pub input_dir: Option<PathBuf>,

    #[arg(
        help = "individual files to convert",
        trailing_var_arg = true,
        value_parser = value_parser!(PathBuf).into_resettable()
    )]
    pub files: Option<Vec<PathBuf>>,
}

#[derive(Debug, Clone)]
pub struct IngestItem {
    pub input_path: PathBuf,
    pub output_prefix: PathBuf,
}

impl<I: AsRef<Path>, O: AsRef<Path>> From<(I, O)> for IngestItem {
    fn from(value: (I, O)) -> Self {
        Self {
            input_path: value.0.as_ref().to_path_buf(),
            output_prefix: value.1.as_ref().to_path_buf(),
        }
    }
}

impl RawSource {
    fn is_supported_filetype(path: &Path) -> bool {
        let ext = path
            .extension()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        supported_extensions().contains(&ext.as_ref()) || ext.to_lowercase() == "dng"
    }

    fn ingest_files(files: Vec<PathBuf>) -> Vec<IngestItem> {
        files
            .into_par_iter()
            .filter_map(|ref item| {
                if Self::is_supported_filetype(item) {
                    debug!("found supported file: \"{}\"", item.display());

                    Some((item, "").into())
                } else {
                    warn!("ignoring \"{}\": unsupported filetype", item.display());

                    None
                }
            })
            .collect::<Vec<_>>()
    }

    fn ingest_dir(input_dir: &Path, prefix: &Path, recurse: bool) -> RawbitResult<Vec<IngestItem>> {
        if !input_dir.is_dir() {
            return Err(AppError::DirNotFound(
                "source directory doesn't exist".into(),
                input_dir.to_path_buf(),
            ));
        }

        let dir = map_err!(
            read_dir(input_dir),
            AppError::Io,
            format!("couldn't stat directory: {}", input_dir.display()),
        )?;

        let files = dir
            .par_bridge()
            .filter_map(|item| match item {
                Ok(ref item) if item.path().is_dir() && recurse => {
                    let intermediate_dir = prefix.join(item.path().file_name().unwrap());

                    Some(Self::ingest_dir(&item.path(), &intermediate_dir, true))
                }

                Ok(ref item) if item.path().is_file() => {
                    let path = item.path();

                    if Self::is_supported_filetype(&path) {
                        debug!("found supported file: \"{}\"", path.display());

                        Some(Ok(vec![(path, prefix.to_path_buf()).into()]))
                    } else {
                        warn!("ignoring \"{}\": unsupported filetype", path.display());

                        None
                    }
                }

                _ => None,
            })
            .collect::<RawbitResult<Vec<_>>>()?
            .into_par_iter()
            .flatten()
            .collect::<Vec<_>>();

        Ok(files)
    }

    pub fn ingest(self, recurse: bool) -> RawbitResult<Vec<IngestItem>> {
        assert!(
            self.files.is_some() || self.input_dir.is_some(),
            "expected input dir or a list of individual files, got neither"
        );

        if let Some(ref dir) = self.input_dir {
            Self::ingest_dir(dir, &PathBuf::new(), recurse)
        } else if let Some(files) = self.files {
            Ok(Self::ingest_files(files))
        } else {
            unreachable!()
        }
    }
}

#[cfg(test)]
mod path_tests {
    use std::{
        fs::File,
        io::Result,
        path::{Path, PathBuf},
    };
    use tempfile::{TempDir, tempdir, tempdir_in};

    use super::{IngestItem, RawSource};

    fn setup_nested_dir(parent: Option<&Path>) -> Result<([TempDir; 2], Vec<PathBuf>)> {
        let (input_dir, mut files) = setup_flat_dir(parent)?;
        let (nested_dir, nested_files) = setup_flat_dir(Some(input_dir.path()))?;

        files.extend(nested_files);

        Ok(([input_dir, nested_dir], files))
    }

    fn setup_flat_dir(parent: Option<&Path>) -> Result<(TempDir, Vec<PathBuf>)> {
        let input_dir = match parent {
            Some(dir) => tempdir_in(dir),
            None => tempdir(),
        }?;

        let input_path = input_dir.path();
        assert!(input_path.exists());

        let temp_paths = (0..10)
            .map(|i| {
                let path = input_path.join(format!("temp_file_{i}.ARW"));
                File::create(&path).unwrap();
                path
            })
            .collect::<Vec<_>>();

        Ok((input_dir, temp_paths))
    }

    #[test]
    fn parses_flat_dir_correctly() -> Result<()> {
        let (input_dir, temp_paths) = setup_flat_dir(None)?;
        let input_path = input_dir.path();

        let args = RawSource {
            input_dir: Some(input_path.to_path_buf()),
            files: None,
        };

        let ingest = args.ingest(false).unwrap();
        assert_eq!(ingest.len(), 10);

        for IngestItem {
            input_path,
            output_prefix,
        } in &ingest
        {
            assert!(temp_paths.contains(input_path));
            assert_eq!(output_prefix.to_string_lossy().len(), 0);
        }

        Ok(())
    }

    #[test]
    fn parses_nested_dir_recursive_correctly() -> Result<()> {
        let ([input_dir, _output_dir], temp_paths) = setup_nested_dir(None)?;
        let input_path = input_dir.path();

        let args = RawSource {
            input_dir: Some(input_path.to_path_buf()),
            files: None,
        };

        let ingest = args.ingest(true).unwrap();
        assert_eq!(ingest.len(), 20);

        for IngestItem {
            input_path,
            output_prefix,
        } in &ingest
        {
            assert!(temp_paths.contains(input_path));

            let degree = output_prefix.iter().count();
            assert!(degree <= 1);
        }

        Ok(())
    }

    #[test]
    fn parses_nested_dir_flattened_correctly() -> Result<()> {
        let ([input_dir, _output_dir], temp_paths) = setup_nested_dir(None)?;
        let input_path = input_dir.path();

        let args = RawSource {
            input_dir: Some(input_path.to_path_buf()),
            files: None,
        };

        let ingest = args.ingest(false).unwrap();
        assert_eq!(ingest.len(), 10);

        for IngestItem {
            input_path,
            output_prefix,
        } in &ingest
        {
            assert!(temp_paths.contains(input_path));

            let degree = output_prefix.iter().count();
            assert_eq!(degree, 0);
        }

        Ok(())
    }
}
