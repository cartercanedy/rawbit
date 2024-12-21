// Copyright (c) Carter J. Canedy <cartercanedy42@gmail.com>
// rawbit is free software, distributable under the terms of the MIT license
// See https://raw.githubusercontent.com/cartercanedy/rawbit/refs/heads/master/LICENSE.txt

#![deny(
    clippy::all,
    clippy::pedantic,
    clippy::nursery,
    clippy::cargo,
    clippy::cast_possible_wrap
)]
#![allow(
    clippy::enum_glob_use,
    clippy::multiple_crate_versions,
    clippy::module_name_repetitions
)]

use std::fmt::Display;

use clap::Parser as _;
use futures::future::join_all;
use parse::FilenameFormat;
use rawler::dng::{convert::ConvertParams, CropMode, DngCompression};
use rayon::{
    iter::{IntoParallelRefIterator as _, ParallelIterator as _},
    ThreadPoolBuilder,
};
use smlog::{debug, error, ignore, log::LevelFilter, warn, Log};
use tokio::{fs, runtime::Builder};

mod args;
mod common;
mod job;
mod parse;

use args::{ImportConfig, IngestItem, LogConfig};
use common::{map_err, AppError, RawbitResult};
use job::{DryRunJob, Job, JobConfig, RawConvertJob};

fn main() -> Result<(), u32> {
    let args = ImportConfig::parse();
    let LogConfig {
        quiet,
        verbose: verbose_logs,
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

    ThreadPoolBuilder::new()
        .num_threads(args.n_threads())
        .thread_name(|n| format!("rawbit-rayon-worker-{n}"))
        .build_global()
        .unwrap();

    let rt = Builder::new_multi_thread()
        .enable_all()
        .thread_name("rawbit-tokio-worker")
        .worker_threads(args.n_threads())
        .thread_stack_size(3 * 1024 * 1024)
        .build()
        .unwrap();

    let _rt_guard = rt.enter();

    match rt.block_on(run(args)) {
        Err(err) => {
            use AppError::*;

            let (err_str, cause, exit_code): (String, Option<&dyn Display>, _) = match err {
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

            Err(exit_code)
        }

        _ => Ok(()),
    }
}

async fn run(args: ImportConfig) -> RawbitResult<()> {
    let n_threads = args.n_threads();

    let ImportConfig {
        source,
        output_dir,
        fmt_str,
        artist,
        force,
        embed,
        recurse,
        no_preview,
        no_thumbnail,
        dry_run,
        ..
    } = args;

    let ingest = source.ingest(recurse)?.leak();

    if output_dir.exists() {
        if output_dir.is_dir() {
            Ok(())
        } else {
            Err(AppError::AlreadyExists(
                "destination path exists and isn't a directory".into(),
                (&output_dir).into(),
            ))
        }
    } else {
        map_err!(
            fs::create_dir_all(&output_dir).await,
            AppError::Io,
            "couldn't create destination directory"
        )
    }?;

    let fmt_str = fmt_str.map_or("", |s| s.leak() as &'static str);
    let filename_format = Box::leak(Box::new(FilenameFormat::parse(fmt_str)?));

    let opts = ConvertParams {
        artist,
        apply_scaling: false,
        crop: CropMode::Best,
        compression: DngCompression::Lossless,
        embedded: embed,
        index: 0,
        preview: !no_preview,
        thumbnail: !no_thumbnail,
        software: "rawbit".into(),
        ..Default::default()
    };

    for chunk in ingest.chunks(n_threads) {
        let jobs = chunk
            .par_iter()
            .cloned()
            .map(
                |IngestItem {
                     input_path,
                     ref output_prefix,
                 }| {
                    let config = JobConfig {
                        input_path,
                        output_dir: output_dir.join(output_prefix),
                        filename_format,
                        force,
                        convert_opts: opts.clone(),
                    };

                    if dry_run {
                        DryRunJob::new(config).run()
                    } else {
                        RawConvertJob::new(config).run()
                    }
                },
            )
            .collect::<Vec<_>>();

        join_all(jobs)
            .await
            .into_iter()
            .zip(chunk.iter().map(|item| item.input_path.clone()))
            .for_each(|(result, input_path)| {
                if let Err(cvt_err) = result {
                    use job::Error::*;

                    let (err_str, cause): (&str, Option<&dyn Display>) = match cvt_err {
                        AlreadyExists(ref err_str) => (err_str, None),
                        Io(ref err_str, ref cause) => (err_str, Some(cause)),
                        ImgOp(ref err_str, ref cause) => (err_str, Some(cause)),
                        Other(ref err_str, ref cause) => (err_str, Some(cause)),
                    };

                    warn!("while processing \"{}\": {err_str}", input_path.display());
                    if let Some(dbg) = cause {
                        debug!("Cause of last error:\n{dbg}");
                    }
                }
            });
    }

    Ok(())
}
