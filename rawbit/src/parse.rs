// Copyright (c) Carter J. Canedy <cartercanedy42@gmail.com>
// rawbit is free software, distributable under the terms of the MIT license
// See https://raw.githubusercontent.com/cartercanedy/rawbit/refs/heads/master/LICENSE.txt

use std::{borrow::Cow, cell::LazyCell, error, fmt};

use chrono::NaiveDateTime;
use phf::{Map, phf_map};
use rawler::decoders::RawMetadata;
use smlog::warn;
use zips::zip;

use crate::common::{AppError, RawbitResult};

const OPEN_EXPANSION: char = '{';
const CLOSE_EXPANSION: char = '}';
const EXIF_DT_FMT: &str = "%Y:%m:%d %H:%M:%S";

const IMG_ORIG_FNAME_ITEM: FmtItem<'static> =
    FmtItem::Metadata(MetadataKind::ImageOriginalFilename);

const MD_KIND_MAP: Map<&str, MetadataKind> = const {
    use MetadataKind::*;
    phf_map! {
        "camera.make" => CameraMake,
        "camera.model" => CameraModel,
        "camera.shutter_speed" => CameraShutterSpeed,
        "camera.iso" => CameraISO,
        "camera.exposure_compensation" => CameraExposureComp,
        "camea.flash" => CameraFlash,
        "lens.make" => LensMake,
        "lens.model" => LensModel,
        "lens.focal_length" => LensFocalLength,
        "lens.focus_distance" => LensFocusDist,
        "lens.fstop" => LensFStop,
        "image.width" => ImageWidth,
        "image.height" => ImageHeight,
        "image.bit_depth" => ImageBitDepth,
        "image.color_space" => ImageColorSpace,
        "image.sequence_number" => ImageSequenceNumber,
        "image.original_filename" => ImageOriginalFilename
    }
};

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetadataKind {
    CameraMake,
    CameraModel,
    CameraShutterSpeed,
    CameraExposureComp,
    CameraISO,
    CameraFlash,
    LensFStop,
    LensMake,
    LensModel,
    LensFocalLength,
    LensFocusDist,
    ImageColorSpace,
    ImageSequenceNumber,
    ImageHeight,
    ImageWidth,
    ImageBitDepth,
    ImageOriginalFilename,
}

impl MetadataKind {
    pub fn expand_with_metadata<'a>(self, md: &'a RawMetadata, original: &str) -> Cow<'a, str> {
        use MetadataKind::*;
        type CowStr<'a> = Cow<'a, str>;

        match self {
            CameraMake => CowStr::Borrowed(&md.make),
            CameraModel => CowStr::Borrowed(&md.model),

            CameraISO => CowStr::Owned(
                md.exif
                    .iso_speed
                    .as_ref()
                    .map_or(const { String::new() }, ToString::to_string),
            ),

            CameraShutterSpeed => CowStr::Owned(
                md.exif
                    .shutter_speed_value
                    .as_ref()
                    .map_or(const { String::new() }, ToString::to_string),
            ),

            LensMake => CowStr::Borrowed(md.exif.lens_make.as_ref().map_or("", |s| s.as_ref())),

            LensModel => CowStr::Borrowed(md.exif.lens_model.as_ref().map_or("", |s| s.as_ref())),

            LensFocalLength => CowStr::Owned(
                md.exif
                    .focal_length
                    .as_ref()
                    .map_or(const { String::new() }, |focal_len| {
                        focal_len.to_string().replace('/', "_")
                    }),
            ),

            ImageOriginalFilename => CowStr::Owned(original.to_string()),

            _ => {
                warn!("using unimplemented metadata tag: {}", self.as_str());
                CowStr::Borrowed("")
            }
        }
    }

    fn as_str(self) -> &'static str {
        MD_KIND_MAP.entries().find(|(_, v)| **v == self).unwrap().0
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum FmtItem<'a> {
    Literal(Cow<'a, str>),
    DateTime(Cow<'a, str>),
    Metadata(MetadataKind),
}

#[derive(Debug)]
pub struct FilenameFormat<'a>(Box<[FmtItem<'a>]>);

impl<'a> FilenameFormat<'a> {
    pub fn render_filename(&self, original_filename: &str, md: &RawMetadata) -> String {
        let mut fname_str = String::new();

        let date = LazyCell::new(Box::new(move || {
            let date_str = &md.exif.date_time_original.clone().unwrap_or_default();
            NaiveDateTime::parse_from_str(date_str, EXIF_DT_FMT).ok()
        }));

        for atom in &self.0 {
            let rendered = match atom {
                FmtItem::Literal(lit) => lit.clone(),
                FmtItem::Metadata(md_kind) => md_kind.expand_with_metadata(md, original_filename),

                FmtItem::DateTime(item) => date.as_ref().map_or(Cow::Borrowed(""), |date| {
                    Cow::Owned(date.format(item.as_ref()).to_string())
                }),
            };

            fname_str.push_str((rendered).as_ref());
        }

        fname_str
    }

    pub fn parse(fmt: &'a str) -> RawbitResult<Self> {
        #[derive(Debug)]
        enum ScanState {
            Start,
            Literal,
            DateTime,
            ExpansionStart,
            ExpansionBody,
        }

        let mut items = vec![];
        let mut to_parse = fmt;

        let mut consumed = 0;
        let mut state = ScanState::Start;

        while !to_parse.is_empty() {
            let mut end = false;
            let split_at = to_parse
                .chars()
                .zip(1..)
                .take_while(|(c, _)| {
                    use ScanState::*;
                    match (&state, c) {
                        _ if end => false,

                        (Start, sym) => {
                            state = match sym {
                                '%' => DateTime,
                                &OPEN_EXPANSION => ExpansionStart,
                                _ => Literal,
                            };

                            true
                        }

                        (ExpansionStart, sym) => {
                            (state, end) = if sym == &OPEN_EXPANSION {
                                (Literal, true)
                            } else {
                                (ExpansionBody, false)
                            };

                            true
                        }

                        (DateTime, _) | (ExpansionBody, &CLOSE_EXPANSION) => {
                            end = true;
                            true
                        }

                        (Literal, '%' | &OPEN_EXPANSION) => false,

                        _ => true,
                    }
                })
                .last()
                .unwrap()
                .1;

            if let Some((s, remainder)) = to_parse.split_at_checked(split_at) {
                to_parse = remainder;

                // catch escaped double left squirly braces, only render one
                if s == "{{" {
                    items.push(FmtItem::Literal(Cow::Borrowed(&s[0..1])));
                } else {
                    items.push(match state {
                        ScanState::Literal => FmtItem::Literal(Cow::Borrowed(s)),

                        ScanState::DateTime => {
                            if s.len() != 2 {
                                return Err(AppError::FmtStrParse(Error::invalid_expansion(
                                    consumed,
                                    s.len(),
                                    to_parse,
                                )));
                            }

                            FmtItem::DateTime(Cow::Borrowed(s))
                        }

                        ScanState::ExpansionBody => {
                            assert!(
                                s.starts_with(OPEN_EXPANSION),
                                "An expansion was interpreted incorrectly: fmt: {to_parse}, seq: {s}"
                            );

                            if s.ends_with(CLOSE_EXPANSION) {
                                expand(&s[1..s.len() - 1]).ok_or(AppError::FmtStrParse(
                                    Error::invalid_expansion(consumed, s.len(), to_parse),
                                ))?
                            } else {
                                return Err(AppError::FmtStrParse(
                                    Error::unterminated_expansion(consumed, s.len(), to_parse),
                                ));
                            }
                        }

                        _ => unreachable!(),
                    });
                }

                consumed += s.len();
            } else {
                return Err(AppError::FmtStrParse(Error::new(
                    consumed,
                    to_parse.len() - consumed,
                    to_parse,
                    ErrorKind::Unknown,
                )));
            }

            state = ScanState::Start;
        }

        if !items.contains(&IMG_ORIG_FNAME_ITEM) {
            items.push(IMG_ORIG_FNAME_ITEM);
        }

        Ok(Self(items.into_boxed_slice()))
    }
}

#[inline]
fn expand(s: &str) -> Option<FmtItem> {
    Some(FmtItem::Metadata(MD_KIND_MAP.get(s)?.to_owned()))
}

#[cfg(test)]
mod test_parse {
    use crate::parse::FilenameFormat;

    use super::{FmtItem, MetadataKind, OPEN_EXPANSION};
    #[test]
    fn parses_expansions_and_strftime_ok() {
        assert!(FilenameFormat::parse("%Y-%m-%d_{camera.make}").is_ok());
    }

    #[test]
    fn fails_to_parse_incomplete_expansion() {
        const BAD_EXPANSION: &str = "{camera.make";
        assert!(FilenameFormat::parse(BAD_EXPANSION).is_err());
    }

    #[test]
    fn escaped_double_squirly_brace_only_prints_one() {
        let escaped = format!(
            "{}{}%Y{{image.original_filename}}",
            &OPEN_EXPANSION, &OPEN_EXPANSION
        );
        let parsed = FilenameFormat::parse(&escaped);

        assert!(parsed.is_ok());

        let parsed = parsed.unwrap();

        assert!(parsed.0.len() == 3);

        assert!(matches!(
            parsed.0[0], FmtItem::Literal(ref s) if s.chars().next().unwrap() == OPEN_EXPANSION && s.len() == 1
        ));

        assert!(matches!(parsed.0[1], FmtItem::DateTime(..)));
    }

    #[test]
    fn inserts_fname_automatically() {
        const FMT_STR_NO_FNAME: &str = "%Y";

        let parsed = FilenameFormat::parse(FMT_STR_NO_FNAME).unwrap();

        assert_eq!(
            parsed.0.as_ref(),
            &[
                FmtItem::DateTime("%Y".into()),
                FmtItem::Metadata(MetadataKind::ImageOriginalFilename)
            ]
        );
    }
}

#[derive(Clone, Copy, Debug)]
pub enum ErrorKind {
    UnterminatedExpansion,
    InvalidExpansion,
    Unknown,
}

#[derive(Clone, Debug)]
pub struct Error {
    pub kind: ErrorKind,
    pub original: String,
    pub start: u16,
    pub width: u16,
}

impl Error {
    pub fn new(
        start: impl TryInto<u16>,
        width: impl TryInto<u16>,
        original: impl AsRef<str>,
        kind: ErrorKind,
    ) -> Self {
        // there's no way that someone is using a fmt str > 65535 chars
        // just in case, tho
        assert!(
            original.as_ref().len() < u16::MAX as usize,
            "what are you doing with that long of a format string"
        );

        let (start, width) = zip!(start.try_into().ok(), width.try_into().ok()).unwrap();

        Self {
            original: original.as_ref().into(),
            start,
            width,
            kind,
        }
    }

    pub fn unterminated_expansion<S: TryInto<u16>, W: TryInto<u16>>(
        start: S,
        width: W,
        original: &str,
    ) -> Self {
        Self::new(start, width, original, ErrorKind::UnterminatedExpansion)
    }

    pub fn invalid_expansion<S: TryInto<u16>, W: TryInto<u16>>(
        start: S,
        width: W,
        original: &str,
    ) -> Self {
        Self::new(start, width, original, ErrorKind::InvalidExpansion)
    }

    fn print_error_details(&self, f: &mut fmt::Formatter<'_>, msg: &str) -> fmt::Result {
        let (start, width) = (self.start as usize, self.width as usize);

        let padding = format!("{:>1$}", "", start);
        let underline = format!("{:~<1$}", "^", width);

        writeln!(f, "{msg}")?;
        writeln!(f, "> {}", self.original)?;
        writeln!(f, "> {padding}{underline}")
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use ErrorKind::{InvalidExpansion, Unknown, UnterminatedExpansion};

        let (start, width, orig) = (self.start as usize, self.width as usize, &self.original);

        let err_seq = &orig[start..start + width];

        let err_msg = format!(
            "{}: {}",
            match self.kind {
                UnterminatedExpansion => "unterminated variable expansion",
                InvalidExpansion => "invalid variable expansion",
                Unknown => "unknown error",
            },
            err_seq
        );

        self.print_error_details(f, err_msg.as_str())
    }
}

impl error::Error for Error {}
