//! The sound emitter system's behaviour, without the engine around it.
//!
//! This is what `soundemittersystem/soundemittersystembase.cpp` decides:
//! which keys a sound script entry may carry, what `$gender` in a wave path
//! expands to, how a wave is chosen for a speaker, and which entry wins when
//! two scripts declare the same sound. It reads and answers in Rust terms;
//! the C++ interface the engine calls lives in `source-soundemittersystem`,
//! which is the only place the engine's structure layouts appear.

pub mod float16;
pub mod values;

use source_keyvalues::{Node, Value};
use std::collections::HashMap;
use values::{
    atoi, attenuation_from_string, pitch_from_string, sound_level_from_string, text_to_channel,
    volume_from_string, Interval, CHAN_AUTO, PITCH_NORM, SNDLVL_NORM, VOL_NORM,
};

/// `$gender`, the token a wave path uses to stand for either recording.
pub const GENDER_MACRO: &str = "$gender";

/// The native code normalizes names through 256-byte buffers.
const NAME_BUFFER: usize = 255;

/// `gender_t`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum Gender {
    #[default]
    None = 0,
    Male = 1,
    Female = 2,
}

impl Gender {
    pub fn from_raw(value: u8) -> Self {
        match value {
            1 => Self::Male,
            2 => Self::Female,
            _ => Self::None,
        }
    }

    /// The word a `$gender` token becomes, or nothing for an unvoiced actor.
    fn word(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Male => Some("male"),
            Self::Female => Some("female"),
        }
    }
}

/// One wave a sound entry may play: which name, for which speaker, and
/// whether it is still unplayed in the current round.
///
/// The engine reads these four bytes through the interface, so the layout is
/// fixed: `SoundFile` in `public/SoundEmitterSystem/isoundemittersystembase.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C, packed)]
pub struct WaveSlot {
    pub symbol: u16,
    pub gender: u8,
    pub available: u8,
}

impl WaveSlot {
    pub fn new(symbol: u16, gender: Gender) -> Self {
        Self {
            symbol,
            gender: gender as u8,
            available: 1,
        }
    }
}

/// The values one sound entry carries, before they are narrowed to the
/// storage the engine reads.
#[derive(Debug, Clone, PartialEq)]
pub struct SoundParams {
    pub channel: i32,
    pub volume: Interval,
    pub pitch: Interval,
    pub sound_level: Interval,
    pub delay_msec: i32,
    pub play_to_owner_only: bool,
    pub uses_gender_token: bool,
    /// The waves to choose between, with `$gender` already expanded.
    pub waves: Vec<WaveSlot>,
    /// The unexpanded paths, kept because the editor writes them back out.
    pub converted: Vec<WaveSlot>,
}

impl Default for SoundParams {
    fn default() -> Self {
        Self {
            channel: CHAN_AUTO,
            volume: Interval::point(VOL_NORM),
            pitch: Interval::point(PITCH_NORM),
            sound_level: Interval::point(SNDLVL_NORM as f32),
            delay_msec: 0,
            play_to_owner_only: false,
            uses_gender_token: false,
            waves: Vec::new(),
            converted: Vec::new(),
        }
    }
}

/// A parsed entry: the name the game asks for and what it gets.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedSound {
    pub name: String,
    pub params: SoundParams,
}

/// The wave paths, held once and referred to by number.
///
/// The numbers are what the engine sees, and it hands them back to ask for
/// the string, so they only have to be consistent with themselves.
#[derive(Debug, Default)]
pub struct WaveNames {
    names: Vec<String>,
    by_name: HashMap<String, u16>,
}

impl WaveNames {
    pub fn intern(&mut self, name: &str) -> u16 {
        if let Some(symbol) = self.by_name.get(name) {
            return *symbol;
        }
        let symbol = u16::try_from(self.names.len()).unwrap_or(u16::MAX);
        if symbol == u16::MAX {
            return symbol;
        }
        self.names.push(name.to_owned());
        self.by_name.insert(name.to_owned(), symbol);
        symbol
    }

    pub fn get(&self, symbol: u16) -> Option<&str> {
        self.names.get(usize::from(symbol)).map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    pub fn clear(&mut self) {
        self.names.clear();
        self.by_name.clear();
    }
}

/// Reads one sound script entry's keys. Unknown keys are ignored, as they are
/// natively, so a script written for a later engine still loads.
pub fn parse_entry(node: &Node, waves: &mut WaveNames) -> SoundParams {
    let mut params = SoundParams::default();
    let Some(children) = node.children() else {
        return params;
    };
    for key in children {
        let text = key.string().unwrap_or_default();
        let name = key.name.as_str();
        if name.eq_ignore_ascii_case("channel") {
            params.channel = text_to_channel(text);
        } else if name.eq_ignore_ascii_case("volume") {
            params.volume = volume_from_string(text);
        } else if name.eq_ignore_ascii_case("pitch") {
            params.pitch = pitch_from_string(text);
        } else if name.eq_ignore_ascii_case("wave") {
            expand_into(&mut params, text, waves);
        } else if name.eq_ignore_ascii_case("rndwave") {
            for wave in key.children().unwrap_or_default() {
                expand_into(&mut params, wave.string().unwrap_or_default(), waves);
            }
        } else if name.eq_ignore_ascii_case("attenuation")
            || name.eq_ignore_ascii_case("CompatibilityAttenuation")
        {
            let compatibility = name.eq_ignore_ascii_case("CompatibilityAttenuation");
            params.sound_level = attenuation_from_string(text, compatibility);
        } else if name.eq_ignore_ascii_case("soundlevel") {
            params.sound_level = sound_level_from_string(text);
        } else if name.eq_ignore_ascii_case("play_to_owner_only") {
            params.play_to_owner_only = atoi(text) != 0;
        } else if name.eq_ignore_ascii_case("delay_msec") {
            params.delay_msec = atoi(text).max(0);
        }
    }
    params
}

/// Every entry a sound script file declares, in file order.
pub fn parse_script(text: &str, waves: &mut WaveNames) -> Vec<ParsedSound> {
    let Ok(document) = source_keyvalues::parse(text) else {
        return Vec::new();
    };
    document
        .roots()
        .filter(|node| matches!(node.value, Value::Object(ref children) if !children.is_empty()))
        .map(|node| ParsedSound {
            name: node.name.clone(),
            params: parse_entry(node, waves),
        })
        .collect()
}

/// The files a manifest asks for, and whether each is preloaded.
pub fn parse_manifest(text: &str) -> Vec<(String, bool)> {
    let Ok(document) = source_keyvalues::parse(text) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    for root in document.roots() {
        for key in root.children().unwrap_or_default() {
            let preload = key.name.eq_ignore_ascii_case("preload_file");
            if preload || key.name.eq_ignore_ascii_case("precache_file") {
                if let Some(path) = key.string() {
                    files.push((path.to_owned(), preload));
                }
            }
        }
    }
    files
}

/// `scripts/global_actors.txt`: which model speaks with which voice. The
/// native table holds 255 actors and ignores the rest.
pub fn parse_actor_genders(text: &str) -> Vec<(String, Gender)> {
    let Ok(document) = source_keyvalues::parse(text) else {
        return Vec::new();
    };
    let mut actors: Vec<(String, Gender)> = Vec::new();
    for root in document.roots() {
        for actor in root.children().unwrap_or_default() {
            if actors.len() > 254 {
                return actors;
            }
            if actors
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case(&actor.name))
            {
                continue;
            }
            let gender = match actor.string().unwrap_or_default() {
                value if value.eq_ignore_ascii_case("male") => Gender::Male,
                value if value.eq_ignore_ascii_case("female") => Gender::Female,
                _ => Gender::None,
            };
            actors.push((actor.name.clone(), gender));
        }
    }
    actors
}

/// `ExpandSoundNameMacros`: a path with `$gender` in it becomes one wave per
/// voice, and the path as written is kept aside for the editor.
pub fn expand_into(params: &mut SoundParams, wave: &str, waves: &mut WaveNames) {
    let Some(offset) = find_ignore_case(wave, GENDER_MACRO) else {
        params
            .waves
            .push(WaveSlot::new(waves.intern(wave), Gender::None));
        return;
    };
    let (before, after) = split_name(wave, offset, GENDER_MACRO.len());
    for gender in [Gender::Male, Gender::Female] {
        let expanded = values::truncate(
            &format!("{before}{}{after}", gender.word().unwrap_or_default()),
            NAME_BUFFER,
        )
        .to_owned();
        params
            .waves
            .push(WaveSlot::new(waves.intern(&expanded), gender));
        params.uses_gender_token = true;
    }
    params
        .converted
        .push(WaveSlot::new(waves.intern(wave), Gender::None));
}

/// `GenderExpandString`: the path a given speaker actually plays. A path
/// without the token, or a speaker with no voice, is left as it is.
pub fn gender_expand(gender: Gender, text: &str) -> String {
    let Some(offset) = find_ignore_case(text, GENDER_MACRO) else {
        return values::truncate(text, NAME_BUFFER).to_owned();
    };
    let Some(word) = gender.word() else {
        return values::truncate(text, NAME_BUFFER).to_owned();
    };
    let (before, after) = split_name(text, offset, GENDER_MACRO.len());
    values::truncate(&format!("{before}{word}{after}"), NAME_BUFFER).to_owned()
}

pub fn uses_gender_token(text: &str) -> bool {
    find_ignore_case(text, GENDER_MACRO).is_some()
}

/// `SplitName`, which cuts the token out and truncates both halves to the
/// native buffer size.
fn split_name(text: &str, offset: usize, token_len: usize) -> (&str, &str) {
    let before = values::truncate(&text[..offset], NAME_BUFFER);
    let after = text
        .get(offset + token_len..)
        .map_or("", |rest| values::truncate(rest, NAME_BUFFER));
    (before, after)
}

/// `Q_stristr`.
fn find_ignore_case(haystack: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).find(|start| {
        haystack
            .get(*start..start + needle.len())
            .is_some_and(|window| window.eq_ignore_ascii_case(needle))
    })
}

/// `FindBestSoundForGender`: a wave this speaker has not used yet, chosen at
/// random; when they have used all of them the round starts over, and when
/// the entry has nothing for this speaker any wave will do.
pub fn choose_wave(slots: &mut [WaveSlot], gender: Gender, random: &mut Random) -> Option<usize> {
    reset_exhausted_round(slots, gender);
    if slots.is_empty() {
        return None;
    }
    let mut available: Vec<usize> = Vec::new();
    for (index, slot) in slots.iter().enumerate() {
        if slot.gender == gender as u8 && slot.available != 0 {
            available.push(index);
        }
    }
    if available.is_empty() {
        return Some(random.int(0, slots.len() - 1));
    }
    Some(available[random.int(0, available.len() - 1)])
}

/// `EnsureAvailableSlotsForGender`: once every wave for a speaker has played,
/// they all become playable again. An entry with no wave for that speaker is
/// left alone, which is what makes the fallback above possible.
fn reset_exhausted_round(slots: &mut [WaveSlot], gender: Gender) {
    let mut has_any = false;
    let mut has_available = false;
    for slot in slots.iter() {
        if slot.gender == gender as u8 {
            has_any = true;
            has_available |= slot.available != 0;
        }
    }
    if has_any && !has_available {
        for slot in slots.iter_mut() {
            if slot.gender == gender as u8 {
                slot.available = 1;
            }
        }
    }
}

/// The variation in sound selection.
///
/// The native code draws from the engine's shared random stream. This one is
/// the module's own, so sound selection no longer advances a sequence the
/// rest of the engine also draws from; what plays stays varied, but it is not
/// the same sequence a native run would produce.
#[derive(Debug)]
pub struct Random {
    state: u64,
}

impl Default for Random {
    fn default() -> Self {
        Self::with_seed(0x2545_f491_4f6c_dd1d)
    }
}

impl Random {
    pub const fn with_seed(seed: u64) -> Self {
        Self { state: seed | 1 }
    }

    fn next(&mut self) -> u64 {
        // xorshift64*, which is enough for choosing between a handful of waves.
        let mut state = self.state;
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        self.state = state;
        state.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// A number in `low..=high`.
    pub fn int(&mut self, low: usize, high: usize) -> usize {
        if high <= low {
            return low;
        }
        low + (self.next() >> 33) as usize % (high - low + 1)
    }

    pub fn float(&mut self, low: f32, high: f32) -> f32 {
        if high <= low {
            return low;
        }
        let unit = (self.next() >> 40) as f32 / (1u32 << 24) as f32;
        low + unit * (high - low)
    }

    /// A value from an interval, as `sound_interval_t::Random` gives one.
    pub fn from_interval(&mut self, interval: Interval) -> f32 {
        self.float(interval.start, interval.end())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCRIPT: &str = r#"
        // a comment
        "Weapon.Fire"
        {
            "channel"    "CHAN_WEAPON"
            "volume"     "0.7"
            "soundlevel" "SNDLVL_140dB"
            "pitch"      "95,105"
            "rndwave"
            {
                "wave" "weapons/fire1.wav"
                "wave" "weapons/fire2.wav"
            }
        }
        "NPC.Talk"
        {
            "channel" "CHAN_VOICE"
            "wave"    "vo/npc/$gender01/hello.wav"
            "delay_msec" "-5"
            "play_to_owner_only" "1"
        }
    "#;

    #[test]
    fn reads_the_keys_a_script_entry_may_carry() {
        let mut waves = WaveNames::default();
        let sounds = parse_script(SCRIPT, &mut waves);
        assert_eq!(sounds.len(), 2);

        let fire = &sounds[0];
        assert_eq!(fire.name, "Weapon.Fire");
        assert_eq!(fire.params.channel, 1);
        assert_eq!(fire.params.volume, Interval::point(0.7));
        assert_eq!(fire.params.sound_level, Interval::point(140.0));
        assert_eq!(fire.params.pitch.start, 95.0);
        assert_eq!(fire.params.pitch.end(), 105.0);
        assert_eq!(fire.params.waves.len(), 2);
        assert!(!fire.params.uses_gender_token);
        assert_eq!(
            waves.get(fire.params.waves[0].symbol),
            Some("weapons/fire1.wav")
        );

        let talk = &sounds[1];
        assert_eq!(talk.params.channel, 2);
        assert!(talk.params.play_to_owner_only);
        assert_eq!(talk.params.delay_msec, 0);
        assert!(talk.params.uses_gender_token);
        assert_eq!(talk.params.waves.len(), 2);
        assert_eq!(
            waves.get(talk.params.waves[0].symbol),
            Some("vo/npc/male01/hello.wav")
        );
        assert_eq!(
            waves.get(talk.params.waves[1].symbol),
            Some("vo/npc/female01/hello.wav")
        );
        assert_eq!(talk.params.converted.len(), 1);
        assert_eq!(
            waves.get(talk.params.converted[0].symbol),
            Some("vo/npc/$gender01/hello.wav")
        );
    }

    #[test]
    fn expands_the_gender_token_where_it_appears() {
        assert_eq!(
            gender_expand(Gender::Male, "vo/npc/$gender01/hi.wav"),
            "vo/npc/male01/hi.wav"
        );
        assert_eq!(
            gender_expand(Gender::Female, "vo/NPC/$GENDER01/hi.wav"),
            "vo/NPC/female01/hi.wav"
        );
        // No voice, or no token: the path is played as written.
        assert_eq!(
            gender_expand(Gender::None, "vo/npc/$gender01/hi.wav"),
            "vo/npc/$gender01/hi.wav"
        );
        assert_eq!(
            gender_expand(Gender::Male, "ambient/wind.wav"),
            "ambient/wind.wav"
        );
        assert!(uses_gender_token("a/$Gender/b"));
        assert!(!uses_gender_token("a/gender/b"));
    }

    #[test]
    fn plays_every_wave_before_repeating_one() {
        let mut waves = WaveNames::default();
        let sounds = parse_script(SCRIPT, &mut waves);
        let mut slots = sounds[0].params.waves.clone();
        let mut random = Random::default();

        let first = choose_wave(&mut slots, Gender::None, &mut random).unwrap();
        slots[first].available = 0;
        let second = choose_wave(&mut slots, Gender::None, &mut random).unwrap();
        assert_ne!(first, second, "an unplayed wave is preferred");
        slots[second].available = 0;
        // Both are spent, so the round restarts rather than going silent.
        assert!(choose_wave(&mut slots, Gender::None, &mut random).is_some());
        assert!(slots.iter().all(|slot| slot.available != 0));

        // An entry with no wave for this speaker still answers with one.
        let mut talk = sounds[1].params.waves.clone();
        let chosen = choose_wave(&mut talk, Gender::None, &mut random).unwrap();
        assert!(chosen < talk.len());
        assert!(choose_wave(&mut [], Gender::Male, &mut random).is_none());
    }

    #[test]
    fn reads_the_manifest_and_the_actor_list() {
        let manifest = r#"
            game_sounds_manifest
            {
                "precache_file" "scripts/game_sounds.txt"
                "preload_file"  "scripts/game_sounds_weapons.txt"
                "faceposer_file" "scripts/faceposer.txt"
            }
        "#;
        assert_eq!(
            parse_manifest(manifest),
            vec![
                ("scripts/game_sounds.txt".to_owned(), false),
                ("scripts/game_sounds_weapons.txt".to_owned(), true)
            ]
        );

        let actors = r#"
            "allactors"
            {
                "alyx.mdl"  "female"
                "barney.mdl" "MALE"
                "strider.mdl" "none"
                "alyx.mdl"  "male"
            }
        "#;
        assert_eq!(
            parse_actor_genders(actors),
            vec![
                ("alyx.mdl".to_owned(), Gender::Female),
                ("barney.mdl".to_owned(), Gender::Male),
                ("strider.mdl".to_owned(), Gender::None),
            ]
        );
    }
}
