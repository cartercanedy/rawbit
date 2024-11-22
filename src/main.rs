// Copyright (c) Carter J. Canedy <cartercanedy42@gmail.com>
// rawbit is free software, distributable under the terms of the MIT license
// See https://raw.githubusercontent.com/cartercanedy/rawbit/refs/heads/master/LICENSE.txt

mod error;
mod parse;

use error::{AppError, ConvertError};
use parse::{parse_name_format, FmtItem};

use std::{
    borrow::Cow,
    fmt::Display,
    fs::{self, OpenOptions},
    io::{self, Cursor, Seek as _, SeekFrom},
    path::PathBuf,
    process::ExitCode,
};

use clap::{
    arg,
    builder::{
        styling::{AnsiColor, Color, Style},
        Styles,
    },
    command, ArgAction, Args, Parser,
};

use chrono::NaiveDateTime;
use rawler::{decoders::*, dng::convert, get_decoder, RawFile};
use rayon::{prelude::*, ThreadPoolBuilder};
use smlog::{debug, error, ignore, info, log::LevelFilter, warn, Log};

fn n_threads() -> usize {
    std::thread::available_parallelism().unwrap().get()
}

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

#[derive(Parser)]
#[command(
    version,
    about = "A camera RAW image preprocessor and importer",
    long_about = None,
    trailing_var_arg = true,
    styles = cli_style(),
    next_line_help = true,
    color = clap::ColorChoice::Always
)]
struct ImportArgs {
    #[command(flatten)]
    source: ImageSource,

    #[arg(
        short = 'o',
        long = "out-dir",
        value_name = "DIR",
        help = "directory to write converted DNGs"
    )]
    dst_path: PathBuf,

    #[arg(
        short = 'F',
        long = "format",
        value_name = "FORMAT",
        help = "filename format of converted DNGs; see https://docs.rs/rawbit for info on syntax"
    )]
    fmt_str: Option<String>,

    #[arg(
        short,
        long,
        value_name = "ARTIST",
        help = "value of the \"artist\" field in converted DNGs"
    )]
    artist: Option<String>,

    #[arg(
        long = "embed-original",
        default_value_t = false,
        help = "embed the original raw image in the converted DNG\nNOTE: conversion may take considerably longer"
    )]
    embed: bool,

    #[arg(
        short = 'j',
        long,
        value_name = "N",
        default_value_t = n_threads(),
        help = "number of threads to use while processing input images, defaults to number of CPUs"
    )]
    n_threads: usize,

    #[command(flatten)]
    log_config: LogConfig,

    #[arg(
        short,
        long,
        default_value_t = false,
        help = "overwrite existing files, if they exist"
    )]
    force: bool,
}

#[derive(Args)]
#[group(multiple = false)]
struct LogConfig {
    #[arg(
        short,
        long,
        help = "quiet output, only emit critical errors",
        trailing_var_arg = false
    )]
    quiet: bool,

    #[arg(
        short,
        action = ArgAction::Count,
        help = "increase log verbosity; specify multiple times to increase verbosity"
    )]
    verbose_logs: u8,
}

#[derive(Args)]
#[group(required = true, multiple = false)]
struct ImageSource {
    #[arg(
        short = 'i',
        long = "in-dir",
        value_name = "DIR",
        help = "directory containing raw files to convert"
    )]
    src_dir: Option<PathBuf>,

    #[arg(help = "individual files to convert", trailing_var_arg = true)]
    files: Option<Vec<PathBuf>>,
}

macro_rules! lazy_wrap {
    ($closure:expr) => {
        std::cell::LazyCell::<_, Box<dyn FnOnce() -> _>>::new(Box::new($closure))
    };
}

type Result<T> = std::result::Result<T, AppError>;

fn render_filename(orig_fname: &str, md: &RawMetadata, items: &[FmtItem]) -> String {
    let mut fname_str = String::new();

    let date = lazy_wrap!(|| {
        let date_str = &md.exif.date_time_original.clone().unwrap_or_default();
        NaiveDateTime::parse_from_str(date_str, EXIF_DT_FMT).ok()
    });

    for atom in items {
        let rendered = match atom {
            FmtItem::Literal(lit) => lit.clone(),

            FmtItem::DateTime(item) => {
                if let Some(date) = date.as_ref() {
                    Cow::Owned(date.format(item.as_ref()).to_string())
                } else {
                    Cow::Borrowed("")
                }
            }

            FmtItem::Metadata(md_kind) => md_kind.expand_with_metadata(md, orig_fname),
        };

        fname_str.push_str((rendered).as_ref());
    }

    fname_str
}

const EXIF_DT_FMT: &str = "%Y:%m:%d %H:%M:%S";

macro_rules! exit {
    ($c:expr) => {
        std::process::ExitCode::from($c)
    };
}

fn main() -> ExitCode {
    let args = ImportArgs::parse();
    let LogConfig {
        quiet,
        verbose_logs,
    } = args.log_config;

    let filter: LevelFilter = if quiet {
        ignore("rawler");
        LevelFilter::Error
    } else {
        if verbose_logs < 2 {
            ignore("rawler");
        }

        match verbose_logs {
            0 => LevelFilter::Info,
            1 => LevelFilter::Debug,
            2.. => LevelFilter::Trace,
        }
    };

    Log::init(filter);

    match run(args) {
        Err(err) => {
            use AppError::*;

            let (err_str, cause, exit_code): (String, Option<&dyn Display>, u8) = match err {
                FmtStrParse(e) => (e.to_string(), None, 1),
                Io(s, ref e) => (s, Some(e), 2),
                DirNotFound(s, ref e) => (format!("{s}: {}", e.display()), None, 3),
                AlreadyExists(s, ref e) => (format!("{s}: {}", e.display()), None, 4),
                Other(s, ref e) => (s, Some(e), 5),
            };

            error!("{err_str}");
            if let Some(cause) = cause {
                debug!("{cause}");
            }

            exit!(exit_code)
        }

        Ok(_) => exit!(0),
    }
}

macro_rules! map_app_err {
    ($r:expr, $s:expr, $err_t:path) => {
        $r.map_err(|e| ($err_t)($s.into(), e))
    };
}

macro_rules! map_convert_err {
    ($r:expr, $s:expr, $dst_path:expr, $err_t:path) => {
        $r.map_err(|e| ($dst_path, ($err_t)($s.into(), e)))
    };
}

impl ImageSource {
    pub fn get_files(self) -> Result<Vec<PathBuf>> {
        if let Some(ref dir) = self.src_dir {
            if !dir.exists() || !dir.is_dir() {
                Err(AppError::DirNotFound(
                    "source directory doesn't exist".into(),
                    dir.clone(),
                ))
            } else {
                let dir_stat = map_app_err!(
                    fs::read_dir(dir),
                    format!("couldn't stat directory: {}", dir.display()),
                    AppError::Io
                )?;

                let paths = dir_stat
                    .filter_map(|entry| entry.ok().map(|e| e.path()))
                    .collect();

                Ok(paths)
            }
        } else {
            let files = self
                .files
                .expect("expected directory or path(s), got neither")
                .into_iter()
                .filter(|f| f.is_file())
                .collect();

            Ok(files)
        }
    }
}

fn run(args: ImportArgs) -> Result<()> {
    let ImportArgs {
        source,
        dst_path,
        fmt_str: fmt,
        n_threads,
        artist,
        force,
        embed,
        ..
    } = args;

    ThreadPoolBuilder::new()
        .num_threads(n_threads)
        .thread_name(|n| format!("rawbit-worker-{n}"))
        .build_global()
        .expect("failed to initialize worker threads");

    let ingest = source.get_files()?;

    if dst_path.exists() {
        if !dst_path.is_dir() {
            Err(AppError::AlreadyExists(
                "destination path exists and isn't a directory".into(),
                (&dst_path).into(),
            ))
        } else {
            Ok(())
        }
    } else {
        map_app_err!(
            fs::create_dir_all(&dst_path),
            "couldn't create destination directory",
            AppError::Io
        )
    }?;

    let fmt_items = if let Some(ref fmt) = fmt {
        Some(parse_name_format(fmt)?)
    } else {
        None
    };

    type ConvertResult = std::result::Result<(), (PathBuf, ConvertError)>;
    ingest
        .par_iter()
        .map(|path| -> ConvertResult {
            assert!(path.exists());
            assert!(path.is_file());

            let path_str = path.to_string_lossy();

            let in_file = OpenOptions::new().read(true).open(path);

            let f = map_convert_err!(in_file, "can't open file", path.clone(), ConvertError::Io)?;

            let mut raw_file = RawFile::new(path, f);

            let decoder = map_convert_err!(
                get_decoder(&mut raw_file),
                "no compatible RAW image decoder available",
                path.clone(),
                ConvertError::ImgOp
            )?;

            let md = map_convert_err!(
                decoder.raw_metadata(&mut raw_file, Default::default()),
                "couldn't extract image metadata",
                path.clone(),
                ConvertError::ImgOp
            )?;

            let orig_fname = path
                .file_stem()
                .unwrap_or_else(|| panic!("couldn't deduce the filename from {}", &path_str))
                .to_string_lossy();

            let out_path = dst_path.join(
                match fmt_items {
                    Some(ref items) => render_filename(orig_fname.as_ref(), &md, items),
                    None => orig_fname.into(),
                } + ".dng",
            );

            if out_path.exists() {
                if !force {
                    return Err((
                        path.clone(),
                        ConvertError::AlreadyExists(format!(
                            "won't overwrite existing file: {}",
                            out_path.display()
                        )),
                    ));
                } else if out_path.is_dir() {
                    return Err((
                        path.clone(),
                        ConvertError::AlreadyExists(format!(
                            "computed filepath already exists as a directory: {}",
                            out_path.display()
                        )),
                    ));
                } else {
                    map_app_err!(
                        fs::remove_file(&out_path),
                        format!("couldn't remove existing file: {}", out_path.display()),
                        ConvertError::Io
                    )
                    .map_err(|e| (path.clone(), e))?
                }
            }

            let mut raw_output_stream = Cursor::new(vec![]);

            let cvt_params = convert::ConvertParams {
                preview: true,
                thumbnail: true,
                embedded: embed,
                software: "rawbit".to_string(),
                artist: artist.clone().or_else(|| md.exif.artist.clone()),
                ..Default::default()
            };

            raw_file
                .file
                .seek(SeekFrom::Start(0))
                .unwrap_or_else(|_| panic!("file IO seeking error: {}", path.display()));

            let cvt_result = convert::convert_raw_stream(
                raw_file.file,
                &mut raw_output_stream,
                &path_str,
                &cvt_params,
            );

            map_convert_err!(
                cvt_result,
                "couldn't convert image to DNG",
                path.clone(),
                ConvertError::ImgOp
            )?;

            raw_output_stream
                .seek(SeekFrom::Start(0))
                // i don't know if this will ever fail unless ENOMEM
                .expect("in-memory IO seeking error");

            let out_file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&out_path);

            let mut out_file = map_convert_err!(
                out_file,
                format!("couldn't create output file: {}", out_path.display()),
                path.clone(),
                ConvertError::Io
            )?;

            info!("Writing DNG: \"{}\"", path.display());

            map_app_err!(
                io::copy(&mut raw_output_stream, &mut out_file),
                format!(
                    "couldn't write converted DNG to disk: {}",
                    out_path.display()
                ),
                ConvertError::Io
            )
            .map_err(|e| (path.clone(), e))?;

            Ok(())
        })
        .for_each(|result| {
            if let Err((path, cvt_err)) = result {
                let (err_str, cause): (&str, Option<&dyn Display>) = match cvt_err {
                    ConvertError::AlreadyExists(ref err_str) => (err_str, None),
                    ConvertError::Io(ref err_str, ref cause) => (err_str, Some(cause)),
                    ConvertError::ImgOp(ref err_str, ref cause) => (err_str, Some(cause)),
                    ConvertError::Other(ref err_str, ref cause) => (err_str, Some(cause)),
                };

                warn!("while processing \"{}\": {err_str}", path.display());
                if let Some(dbg) = cause {
                    debug!("Cause of last error:\n{dbg}");
                }
            }
        });

    Ok(())
}
