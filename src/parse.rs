// Copyright (c) Carter J. Canedy <cartercanedy42@gmail.com>
// rawbit is free software, distributable under the terms of the MIT license
// See https://raw.githubusercontent.com/cartercanedy/rawbit/refs/heads/master/LICENSE.txt

use std::borrow::Cow;

use phf::{phf_map, Map};
use rawler::decoders::RawMetadata;

use crate::{
    error::{AppError, ParseError, ParseErrorType},
    Result,
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
    pub fn expand_with_metadata<'a>(&self, md: &'a RawMetadata, orig_fname: &str) -> Cow<'a, str> {
        use MetadataKind::*;
        type CowStr<'a> = Cow<'a, str>;

        match self {
            CameraMake => CowStr::Borrowed(&md.make),
            CameraModel => CowStr::Borrowed(&md.model),

            CameraISO => CowStr::Owned(if let Some(iso) = &md.exif.iso_speed {
                iso.to_string()
            } else {
                String::new()
            }),

            CameraShutterSpeed => {
                CowStr::Owned(if let Some(speed) = &md.exif.shutter_speed_value {
                    speed.to_string().replace("/", "_")
                } else {
                    String::new()
                })
            }

            LensMake => CowStr::Borrowed(if let Some(ref make) = &md.exif.lens_make {
                make
            } else {
                ""
            }),

            LensModel => CowStr::Borrowed(if let Some(ref model) = &md.exif.lens_model {
                model
            } else {
                ""
            }),

            LensFocalLength => CowStr::Owned(if let Some(focal_len) = &md.exif.focal_length {
                focal_len.to_string().replace("/", "_")
            } else {
                String::new()
            }),

            ImageOriginalFilename => CowStr::Owned(orig_fname.to_string()),

            _ => CowStr::Borrowed(""),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum FmtItem<'a> {
    Literal(Cow<'a, str>),
    DateTime(Cow<'a, str>),
    Metadata(MetadataKind),
}

// I have to do this bc nvim is dumb dumb and can't tell that a quoted open squirly brace isn't a
// new code block...
//
// :)))))
const EXPANSION_BRACKETS: (char, char) = ('{', '}');
const OPEN_EXPANSION: char = EXPANSION_BRACKETS.0;
const CLOSE_EXPANSION: char = EXPANSION_BRACKETS.1;

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

#[inline]
fn expand(s: &str) -> Option<FmtItem> {
    Some(FmtItem::Metadata(MD_KIND_MAP.get(s)?.to_owned()))
}

#[allow(unused_parens)]
pub fn parse_name_format(fmt: &str) -> Result<Box<[FmtItem]>> {
    let mut items = vec![];
    let mut to_parse = fmt;

    #[derive(Debug)]
    enum ScanState {
        Start,
        Literal,
        DateTime,
        ExpansionStart,
        ExpansionBody,
    }

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

            const DOUBLE_OPEN_BRACE: &str = ["{{", "}}"][0];
            // catch escaped double left squirly braces, only render one
            if s == DOUBLE_OPEN_BRACE {
                items.push(FmtItem::Literal(Cow::Borrowed(&s[0..1])));
            } else {
                items.push(match state {
                    ScanState::Literal => FmtItem::Literal(Cow::Borrowed(s)),

                    ScanState::DateTime => {
                        if s.len() != 2 {
                            return Err(AppError::FmtStrParse(ParseError::invalid_expansion(
                                consumed,
                                s.len(),
                                fmt,
                            )));
                        }

                        FmtItem::DateTime(Cow::Borrowed(s))
                    }

                    ScanState::ExpansionBody => {
                        assert!(
                            s.starts_with(OPEN_EXPANSION),
                            "An expansion was interpreted incorrectly: fmt: {}, seq: {}",
                            fmt,
                            s
                        );

                        if s.ends_with(CLOSE_EXPANSION) {
                            expand(&s[1..s.len() - 1]).ok_or(AppError::FmtStrParse(
                                ParseError::invalid_expansion(consumed, s.len(), fmt),
                            ))?
                        } else {
                            return Err(AppError::FmtStrParse(ParseError::unterminated_expansion(
                                consumed,
                                s.len(),
                                fmt,
                            )));
                        }
                    }

                    _ => unreachable!(),
                });
            }

            consumed += s.len();
        } else {
            dbg!(items, &state);

            return Err(AppError::FmtStrParse(ParseError::new(
                consumed,
                fmt.len() - consumed,
                fmt,
                ParseErrorType::Unknown,
            )));
        }

        state = ScanState::Start;
    }

    const IMG_ORIG_FNAME_ITEM: FmtItem<'static> =
        FmtItem::Metadata(MetadataKind::ImageOriginalFilename);

    if !items.contains(&IMG_ORIG_FNAME_ITEM) {
        items.push(IMG_ORIG_FNAME_ITEM)
    }

    Ok(items.into_boxed_slice())
}

#[cfg(test)]
mod test_parse {
    use super::{FmtItem, MetadataKind, OPEN_EXPANSION};

    use super::parse_name_format;
    #[test]
    fn parses_expansions_and_strftime_ok() {
        assert!(parse_name_format("%Y-%m-%d_{camera.make}").is_ok())
    }

    #[test]
    fn fails_to_parse_incomplete_expansion() {
        // again with the bad bracket parsing
        const BAD_EXPANSION: &str = ["{camera.make", "}"][0];
        assert!(parse_name_format(BAD_EXPANSION).is_err())
    }

    #[test]
    fn escaped_double_squirly_brace_only_prints_one() {
        let escaped = format!(
            "{}{}%Y{{image.original_filename}}",
            &OPEN_EXPANSION, &OPEN_EXPANSION
        );
        let parsed = parse_name_format(&escaped);

        assert!(parsed.is_ok());

        let parsed = parsed.unwrap();

        assert!(parsed.len() == 3);

        assert!(matches!(
            parsed[0], FmtItem::Literal(ref s) if s.chars().next().unwrap() == OPEN_EXPANSION && s.len() == 1
        ));

        assert!(matches!(parsed[1], FmtItem::DateTime(..)));
    }

    #[test]
    fn inserts_fname_automatically() {
        const FMT_STR_NO_FNAME: &str = "%Y";

        let parsed = parse_name_format(FMT_STR_NO_FNAME).unwrap();

        assert_eq!(
            parsed.as_ref(),
            &[
                FmtItem::DateTime("%Y".into()),
                FmtItem::Metadata(MetadataKind::ImageOriginalFilename)
            ]
        )
    }
}
