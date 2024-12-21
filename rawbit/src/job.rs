use std::{
    error,
    fs::{create_dir_all, remove_file, OpenOptions},
    io::{self, BufReader, BufWriter},
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use rawler::{
    decoders::{RawDecodeParams, RawMetadata},
    dng::{self, convert::ConvertParams},
    get_decoder, RawFile, RawlerError,
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
    fn run_blocking(self) -> Result<(), Error> {
        let config = self.0;

        let input = map_err!(
            OpenOptions::new()
                .read(true)
                .write(false)
                .open(&config.input_path),
            Error::Io,
            "Couldn't open input RAW file",
        )?;

        let mut raw_file = RawFile::new(config.input_path.as_path(), BufReader::new(input));

        let decoder = map_err!(
            get_decoder(&mut raw_file),
            Error::ImgOp,
            "no compatible RAW image decoder available",
        )?;

        let md = map_err!(
            decoder.raw_metadata(&mut raw_file, RawDecodeParams::default()),
            Error::ImgOp,
            "couldn't extract image metadata",
        )?;

        map_err!(raw_file.file.rewind(), Error::Io, "input file io error",)?;

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

        let output_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output_path);

        let mut output_file = BufWriter::new(map_err!(
            output_file,
            Error::Io,
            format!("couldn't create output file: {}", output_path.display()),
        )?);

        info!("Writing DNG: \"{}\"", output_path.display());

        let cvt_result = dng::convert::convert_raw_stream(
            raw_file.file,
            &mut output_file,
            config.input_path.to_string_lossy(),
            &config.convert_opts,
        );

        map_err!(cvt_result, Error::ImgOp, "couldn't convert image to DNG",)
    }
}

#[async_trait]
impl Job for RawConvertJob {
    fn new(config: JobConfig) -> Self {
        assert!(config.input_path.is_file());

        Self(config)
    }

    async fn run(self) -> Result<(), Error> {
        tokio::task::spawn_blocking(|| self.run_blocking())
            .await
            .unwrap()
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
            .open(&config.input_path);

        let input_file = BufReader::new(map_err!(
            input_file,
            Error::Io,
            format!("couldn't read file: {}", config.input_path.display())
        )?);

        let mut raw_file = RawFile::new(&config.input_path, input_file);

        let decoder = map_err!(
            get_decoder(&mut raw_file),
            Error::ImgOp,
            "no available decoder"
        )?;

        let md = map_err!(
            decoder.raw_metadata(&mut raw_file, RawDecodeParams { image_index: 0 }),
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
