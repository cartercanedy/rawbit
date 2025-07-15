use std::{
    error,
    fs::{create_dir_all, remove_file},
    path::{Path, PathBuf},
};

use tokio::{
    fs::OpenOptions,
    io::{self, AsyncReadExt as _},
};

use async_trait::async_trait;
use rawler::{
    decoders::{RawDecodeParams, RawMetadata},
    dng::{self, convert::ConvertParams},
    get_decoder,
    rawsource::RawSource,
    RawlerError,
};

use smlog::info;

use crate::{common::map_err, parse::FilenameFormat};

#[derive(Debug)]
pub enum Error {
    ImgOp(String, RawlerError),
    Io(String, io::Error),
    AlreadyExists(String),
    #[allow(unused)]
    Other(String, Box<dyn error::Error + Send + Sync>),
}

#[async_trait]
pub trait Job {
    fn new(config: JobConfig) -> Self;
    async fn run(self) -> Result<(), Error>;
}

#[derive(Debug)]
pub struct JobConfig {
    pub input_path: PathBuf,
    pub output_dir: PathBuf,
    pub filename_format: &'static FilenameFormat<'static>,
    pub force: bool,
    pub convert_opts: ConvertParams,
}

#[derive(Debug)]
pub struct RawConvertJob(JobConfig);

fn build_output_filename(input_path: &Path, fmt: &FilenameFormat, md: &RawMetadata) -> PathBuf {
    let input_fname_no_ext = input_path
        .file_stem()
        .unwrap_or_else(|| panic!("couldn't deduce filename from {}", input_path.display()))
        .to_string_lossy();

    let output_fname = fmt.render_filename(input_fname_no_ext.as_ref(), md) + ".dng";

    output_fname.into()
}

impl RawConvertJob {
    async fn run_async(self) -> Result<(), Error> {
        let config = self.0;

        let mut input = map_err!(
            OpenOptions::new()
                .read(true)
                .write(false)
                .open(&config.input_path)
                .await,
            Error::Io,
            "Couldn't open input RAW file",
        )?;

        let mut buf = vec![];

        map_err!(
            input.read_to_end(&mut buf).await,
            Error::Io,
            format!("couldn't read from file: '{}'", config.input_path.display())
        )?;

        let raw_file = RawSource::new_from_slice(&buf[..]);

        let decoder = map_err!(
            get_decoder(&raw_file),
            Error::ImgOp,
            "no compatible RAW image decoder available",
        )?;

        let md = map_err!(
            decoder.raw_metadata(&raw_file, &RawDecodeParams::default()),
            Error::ImgOp,
            "couldn't extract image metadata",
        )?;

        let transformed_fname =
            build_output_filename(&config.input_path, config.filename_format, &md);

        map_err!(
            create_dir_all(&config.output_dir),
            Error::Io,
            format!("couldn't make output dir: {}", config.output_dir.display())
        )?;

        let output_path = config.output_dir.join(transformed_fname);

        if output_path.exists() {
            if !config.force {
                Err(Error::AlreadyExists(format!(
                    "won't overwrite existing file: {}",
                    output_path.display()
                )))
            } else if output_path.is_dir() {
                Err(Error::AlreadyExists(format!(
                    "computed filepath already exists as a directory: {}",
                    output_path.display()
                )))
            } else {
                map_err!(
                    remove_file(&output_path),
                    Error::Io,
                    format!("couldn't remove existing file: {}", output_path.display()),
                )
            }?;
        }

        let output_file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output_path);

        map_err!(
            tokio::task::spawn_blocking(move || {
                let mut output_file = std::io::BufWriter::new(map_err!(
                    output_file,
                    Error::Io,
                    format!("couldn't create output file: {}", output_path.display()),
                )?);

                info!("Writing DNG: \"{}\"", output_path.display());

                let cvt_result = dng::convert::convert_raw_source(
                    &raw_file,
                    &mut output_file,
                    config.input_path.to_string_lossy(),
                    &config.convert_opts,
                );

                map_err!(cvt_result, Error::ImgOp, "couldn't convert image to DNG",)
            })
            .await
            .map_err(Box::new),
            Error::Other,
            format!("async error")
        )?
    }
}

#[async_trait]
impl Job for RawConvertJob {
    fn new(config: JobConfig) -> Self {
        assert!(config.input_path.is_file());

        Self(config)
    }

    async fn run(self) -> Result<(), Error> {
        self.run_async().await
    }
}

pub struct DryRunJob(JobConfig);

#[async_trait]
impl Job for DryRunJob {
    fn new(config: JobConfig) -> Self {
        assert!(config.input_path.is_file());

        Self(config)
    }

    async fn run(self) -> Result<(), Error> {
        let config = self.0;

        let input_file = OpenOptions::new()
            .read(true)
            .write(false)
            .open(&config.input_path)
            .await;

        let mut input_file = map_err!(
            input_file,
            Error::Io,
            format!("couldn't read file: {}", config.input_path.display())
        )?;

        let mut buf = vec![];
        map_err!(
            input_file.read_to_end(&mut buf).await,
            Error::Io,
            format!("couldn't read from file: '{}'", config.input_path.display())
        )?;

        let src = RawSource::new_from_slice(&buf[..]).with_path(&config.input_path);

        let decoder = map_err!(get_decoder(&src), Error::ImgOp, "no available decoder")?;

        const DECODE_PARAMS: RawDecodeParams = RawDecodeParams { image_index: 0 };
        let md = map_err!(
            decoder.raw_metadata(&src, &DECODE_PARAMS),
            Error::ImgOp,
            format!(
                "error while retreiving metadata from RAW: {}",
                config.input_path.display()
            )
        )?;

        let output_fname = build_output_filename(&config.input_path, config.filename_format, &md);

        let output_path = config.output_dir.join(output_fname);

        info!("dry run: would've written DNG: {}", output_path.display());

        Ok(())
    }
}
