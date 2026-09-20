//! The values a sound script line can hold, and the spellings it may use.
//!
//! These follow `public/SoundParametersInternal.cpp` and `public/soundflags.h`:
//! the tables are the ones the shipped scripts are written against, and an
//! unknown spelling falls back the way the native parser does rather than
//! failing the whole entry.

use std::fmt::Write as _;

pub const VOL_NORM: f32 = 1.0;
pub const PITCH_NORM: f32 = 100.0;
pub const PITCH_LOW: f32 = 95.0;
pub const PITCH_HIGH: f32 = 120.0;

pub const CHAN_AUTO: i32 = 0;
pub const SNDLVL_NORM: i32 = 75;

/// `interval_t`: a start and a width, not a start and an end.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Interval {
    pub start: f32,
    pub range: f32,
}

impl Interval {
    pub const fn point(start: f32) -> Self {
        Self { start, range: 0.0 }
    }

    pub const fn end(&self) -> f32 {
        self.start + self.range
    }
}

/// `ReadInterval`: the first two comma-separated numbers, as `atof` reads
/// them, and the second is an end rather than a width. Input past the native
/// 127-character buffer is dropped.
pub fn read_interval(text: &str) -> Interval {
    let text = truncate(text, 127);
    let mut parts = text.split(',').filter(|part| !part.is_empty());
    let start = parts.next().map_or(0.0, atof);
    match parts.next() {
        Some(end) => Interval {
            start,
            range: atof(end) - start,
        },
        None => Interval::point(start),
    }
}

/// `atof`: as much of a leading number as parses, and zero when none does.
pub fn atof(text: &str) -> f32 {
    let text = text.trim_start();
    let bytes = text.as_bytes();
    let mut end = 0;
    let mut seen_digit = false;
    let mut seen_dot = false;
    let mut seen_exponent = false;
    let mut last_number = 0;
    while end < bytes.len() {
        match bytes[end] {
            b'0'..=b'9' => {
                seen_digit = true;
                last_number = end + 1;
            }
            b'+' | b'-' if end == 0 || matches!(bytes[end - 1], b'e' | b'E') => {}
            b'.' if !seen_dot && !seen_exponent => seen_dot = true,
            b'e' | b'E' if seen_digit && !seen_exponent => seen_exponent = true,
            _ => break,
        }
        end += 1;
    }
    if !seen_digit {
        return 0.0;
    }
    /* A trailing exponent marker with no digits after it is not part of the
    number, so the parse stops at the last digit seen. */
    text[..last_number].parse().unwrap_or(0.0)
}

/// `atoi`.
pub fn atoi(text: &str) -> i32 {
    let text = text.trim_start();
    let digits = text
        .find(|c: char| !(c.is_ascii_digit() || c == '-' || c == '+'))
        .map_or(text, |end| &text[..end]);
    digits.parse().unwrap_or(0)
}

const CHANNELS: [(&str, i32); 8] = [
    ("CHAN_AUTO", 0),
    ("CHAN_WEAPON", 1),
    ("CHAN_VOICE", 2),
    ("CHAN_ITEM", 3),
    ("CHAN_BODY", 4),
    ("CHAN_STREAM", 5),
    ("CHAN_STATIC", 6),
    ("CHAN_VOICE2", 7),
];

/// The sound levels with names, in the order the native table lists them: the
/// first name for a value is the one printed back out.
const SOUND_LEVELS: [(&str, i32); 30] = [
    ("SNDLVL_NONE", 0),
    ("SNDLVL_20dB", 20),
    ("SNDLVL_25dB", 25),
    ("SNDLVL_30dB", 30),
    ("SNDLVL_35dB", 35),
    ("SNDLVL_40dB", 40),
    ("SNDLVL_45dB", 45),
    ("SNDLVL_50dB", 50),
    ("SNDLVL_55dB", 55),
    ("SNDLVL_IDLE", 60),
    ("SNDLVL_TALKING", 80),
    ("SNDLVL_60dB", 60),
    ("SNDLVL_65dB", 65),
    ("SNDLVL_STATIC", 66),
    ("SNDLVL_70dB", 70),
    ("SNDLVL_NORM", 75),
    ("SNDLVL_75dB", 75),
    ("SNDLVL_80dB", 80),
    ("SNDLVL_85dB", 85),
    ("SNDLVL_90dB", 90),
    ("SNDLVL_95dB", 95),
    ("SNDLVL_100dB", 100),
    ("SNDLVL_105dB", 105),
    ("SNDLVL_110dB", 110),
    ("SNDLVL_120dB", 120),
    ("SNDLVL_130dB", 130),
    ("SNDLVL_GUNFIRE", 140),
    ("SNDLVL_140dB", 140),
    ("SNDLVL_150dB", 150),
    ("SNDLVL_180dB", 180),
];

const ATTENUATIONS: [(&str, f32); 6] = [
    ("ATTN_NONE", 0.0),
    ("ATTN_NORM", 0.8),
    ("ATTN_IDLE", 2.0),
    ("ATTN_STATIC", 1.25),
    ("ATTN_RICOCHET", 1.5),
    ("ATTN_GUNFIRE", 0.27),
];

/// `TextToChannel`. Anything that is not a `CHAN_` name is a number.
pub fn text_to_channel(text: &str) -> i32 {
    if !starts_with_ignore_case(text, "chan_") {
        return atoi(text);
    }
    lookup(&CHANNELS, text).unwrap_or(CHAN_AUTO)
}

/// `TextToSoundLevel`, including its acceptance of `SNDLVL_<number>` for the
/// levels the table has no name for.
pub fn text_to_sound_level(text: &str) -> i32 {
    if let Some(level) = lookup(&SOUND_LEVELS, text) {
        return level;
    }
    if starts_with_ignore_case(text, "SNDLVL_") {
        let level = atoi(&text["SNDLVL_".len()..]);
        if level > 0 && level <= 180 {
            return level;
        }
    }
    SNDLVL_NORM
}

/// `TranslateAttenuation`.
pub fn text_to_attenuation(text: &str) -> f32 {
    lookup(&ATTENUATIONS, text).unwrap_or(0.8)
}

/// `ATTN_TO_SNDLVL`.
pub fn attenuation_to_sound_level(attenuation: f32) -> i32 {
    if attenuation == 0.0 {
        0
    } else {
        (50.0 + 20.0 / attenuation) as i32
    }
}

/// `SNDLEVEL_TO_COMPATIBILITY_MODE`, the range above 255 that means a
/// GoldSrc-style attenuation.
pub fn sound_level_to_compatibility_mode(level: i32) -> i32 {
    level + 256
}

pub fn sound_level_to_string(level: i32) -> String {
    reverse_lookup(&SOUND_LEVELS, level).map_or_else(|| level.to_string(), str::to_owned)
}

pub fn channel_to_string(channel: i32) -> String {
    reverse_lookup(&CHANNELS, channel).map_or_else(|| channel.to_string(), str::to_owned)
}

pub fn volume_to_string(volume: f32) -> String {
    if volume == VOL_NORM {
        return "VOL_NORM".to_owned();
    }
    format_fixed(volume)
}

pub fn pitch_to_string(pitch: f32) -> String {
    for (name, value) in [
        ("PITCH_NORM", PITCH_NORM),
        ("PITCH_LOW", PITCH_LOW),
        ("PITCH_HIGH", PITCH_HIGH),
    ] {
        if pitch == value {
            return name.to_owned();
        }
    }
    format_fixed(pitch)
}

/// `%.3f`, which is what the native code writes numbers back with.
fn format_fixed(value: f32) -> String {
    let mut text = String::new();
    let _ = write!(text, "{value:.3}");
    text
}

/// `VolumeFromString`.
pub fn volume_from_string(text: &str) -> Interval {
    if text.eq_ignore_ascii_case("VOL_NORM") {
        Interval::point(VOL_NORM)
    } else {
        read_interval(text)
    }
}

/// `PitchFromString`.
pub fn pitch_from_string(text: &str) -> Interval {
    for (name, value) in [
        ("PITCH_NORM", PITCH_NORM),
        ("PITCH_LOW", PITCH_LOW),
        ("PITCH_HIGH", PITCH_HIGH),
    ] {
        if text.eq_ignore_ascii_case(name) {
            return Interval::point(value);
        }
    }
    read_interval(text)
}

/// `SoundLevelFromString`.
pub fn sound_level_from_string(text: &str) -> Interval {
    if starts_with_ignore_case(text, "SNDLVL_") {
        Interval::point(text_to_sound_level(text) as f32)
    } else {
        read_interval(text)
    }
}

/// The `attenuation` and `CompatibilityAttenuation` keys, which are written as
/// attenuations and stored as sound levels.
pub fn attenuation_from_string(text: &str, compatibility: bool) -> Interval {
    let mut level = if starts_with_ignore_case(text, "ATTN_") {
        Interval::point(attenuation_to_sound_level(text_to_attenuation(text)) as f32)
    } else {
        let interval = read_interval(text);
        let start = attenuation_to_sound_level(interval.start) as f32;
        let end = attenuation_to_sound_level(interval.end()) as f32;
        Interval {
            start,
            range: end - start,
        }
    };
    if compatibility {
        level = Interval::point(sound_level_to_compatibility_mode(level.start as i32) as f32);
    }
    level
}

fn lookup<T: Copy>(table: &[(&str, T)], text: &str) -> Option<T> {
    table
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(text))
        .map(|(_, value)| *value)
}

fn reverse_lookup<'a, T: PartialEq>(table: &[(&'a str, T)], wanted: T) -> Option<&'a str> {
    table
        .iter()
        .find(|(_, value)| *value == wanted)
        .map(|(name, _)| *name)
}

pub(crate) fn starts_with_ignore_case(text: &str, prefix: &str) -> bool {
    text.len() >= prefix.len() && text[..prefix.len()].eq_ignore_ascii_case(prefix)
}

pub(crate) fn truncate(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_intervals_as_a_start_and_a_width() {
        assert_eq!(read_interval("0.5"), Interval::point(0.5));
        assert_eq!(
            read_interval("0.5, 0.8"),
            Interval {
                start: 0.5,
                range: 0.3
            }
        );
        assert_eq!(read_interval(""), Interval::default());
        assert_eq!(read_interval("junk"), Interval::default());
        assert_eq!(read_interval("95,"), Interval::point(95.0));
        assert_eq!(read_interval("1.5e1"), Interval::point(15.0));
    }

    #[test]
    fn maps_the_spellings_the_scripts_use() {
        assert_eq!(text_to_channel("CHAN_STATIC"), 6);
        assert_eq!(text_to_channel("chan_weapon"), 1);
        assert_eq!(text_to_channel("5"), 5);
        assert_eq!(text_to_channel("CHAN_NOT_A_THING"), CHAN_AUTO);
        assert_eq!(text_to_sound_level("SNDLVL_140dB"), 140);
        assert_eq!(text_to_sound_level("SNDLVL_GUNFIRE"), 140);
        assert_eq!(text_to_sound_level("SNDLVL_77"), 77);
        assert_eq!(text_to_sound_level("SNDLVL_999"), SNDLVL_NORM);
        assert_eq!(text_to_sound_level("nonsense"), SNDLVL_NORM);
        assert_eq!(volume_from_string("VOL_NORM"), Interval::point(1.0));
        assert_eq!(pitch_from_string("PITCH_LOW"), Interval::point(95.0));
        assert_eq!(
            pitch_from_string("95,105"),
            Interval {
                start: 95.0,
                range: 10.0
            }
        );
    }

    #[test]
    fn turns_attenuations_into_sound_levels() {
        assert_eq!(attenuation_to_sound_level(0.0), 0);
        assert_eq!(attenuation_to_sound_level(0.8), 75);
        assert_eq!(attenuation_from_string("ATTN_NORM", false).start, 75.0);
        assert_eq!(attenuation_from_string("ATTN_NONE", false).start, 0.0);
        assert_eq!(
            attenuation_from_string("ATTN_NORM", true).start,
            (75 + 256) as f32
        );
        let ramp = attenuation_from_string("0.8, 2.0", false);
        assert_eq!((ramp.start, ramp.end()), (75.0, 60.0));
    }

    #[test]
    fn writes_values_back_the_way_the_editor_wrote_them() {
        assert_eq!(sound_level_to_string(75), "SNDLVL_NORM");
        assert_eq!(sound_level_to_string(77), "77");
        assert_eq!(channel_to_string(6), "CHAN_STATIC");
        assert_eq!(volume_to_string(1.0), "VOL_NORM");
        assert_eq!(volume_to_string(0.5), "0.500");
        assert_eq!(pitch_to_string(100.0), "PITCH_NORM");
    }
}
