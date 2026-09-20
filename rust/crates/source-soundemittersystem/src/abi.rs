//! The two structures the sound emitter interface hands across a module
//! boundary, laid out as the engine's own headers lay them out.
//!
//! These are not the module's data model; `source_soundemitter` holds that.
//! They exist because the engine reads these fields directly. The accessors
//! in `public/SoundEmitterSystem/isoundemittersystembase.h` are inline, so
//! every caller carries a compiled-in copy of this layout and the module has
//! no say in it. Offsets were taken from the compiler with
//! `clang -Xclang -fdump-record-layouts` against that header.

use source_soundemitter::float16::Float16;
use source_soundemitter::values::Interval;
use source_soundemitter::{Gender, SoundParams, WaveSlot};
use std::ffi::{c_char, c_float, c_int};

/// `CSoundParameters`, which the engine fills in and reads back per play.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SoundParameters {
    pub channel: c_int,
    pub volume: c_float,
    pub pitch: c_int,
    pub pitch_low: c_int,
    pub pitch_high: c_int,
    pub sound_level: c_int,
    pub play_to_owner_only: bool,
    pub count: c_int,
    pub soundname: [c_char; 128],
    pub delay_msec: c_int,
}

impl SoundParameters {
    /// Writes a wave path into the fixed field, truncated as `Q_strncpy`
    /// truncates it.
    pub fn set_soundname(&mut self, name: &str) {
        self.soundname = [0; 128];
        for (slot, byte) in self.soundname.iter_mut().zip(name.bytes()).take(127) {
            *slot = byte as c_char;
        }
    }

    pub fn soundname_is_empty(&self) -> bool {
        self.soundname[0] == 0
    }
}

/// `sound_interval_t<uint16>`.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
struct U16Interval {
    start: u16,
    range: u16,
}

/// `sound_interval_t<uint8>`.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
struct U8Interval {
    start: u8,
    range: u8,
}

/// `sound_interval_t<float16_with_assign>`.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
struct F16Interval {
    start: Float16,
    range: Float16,
}

const PLAY_TO_OWNER_ONLY: u8 = 1 << 0;
const HAD_MISSING_WAVE_FILES: u8 = 1 << 1;
const USES_GENDER_TOKEN: u8 = 1 << 2;
const SHOULD_PRELOAD: u8 = 1 << 3;

/// `CSoundParametersInternal`: the resident form of one sound entry.
///
/// Two of its habits are contractual rather than incidental. A single wave is
/// stored in the bytes of the array pointer itself, because the inline
/// `GetSoundNames()` returns `&m_pSoundNames` when the count is one. And the
/// four booleans are bits of one byte, in declaration order from the lowest.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct SoundParametersInternal {
    sound_names: *mut WaveSlot,
    converted_names: *mut WaveSlot,
    sound_name_count: u16,
    converted_name_count: u16,
    volume: F16Interval,
    sound_level: U16Interval,
    pitch: U8Interval,
    channel: u16,
    delay_msec: u16,
    flags: u8,
    reserved: u8,
}

impl SoundParametersInternal {
    pub fn channel(&self) -> i32 {
        i32::from(self.channel)
    }

    pub fn delay_msec(&self) -> i32 {
        i32::from(self.delay_msec)
    }

    pub fn volume(&self) -> Interval {
        let volume = self.volume;
        Interval {
            start: volume.start.to_f32(),
            range: volume.range.to_f32(),
        }
    }

    pub fn pitch(&self) -> Interval {
        let pitch = self.pitch;
        Interval {
            start: f32::from(pitch.start),
            range: f32::from(pitch.range),
        }
    }

    pub fn sound_level(&self) -> Interval {
        let level = self.sound_level;
        Interval {
            start: f32::from(level.start),
            range: f32::from(level.range),
        }
    }

    pub fn play_to_owner_only(&self) -> bool {
        self.flags & PLAY_TO_OWNER_ONLY != 0
    }

    pub fn uses_gender_token(&self) -> bool {
        self.flags & USES_GENDER_TOKEN != 0
    }

    pub fn had_missing_wave_files(&self) -> bool {
        self.flags & HAD_MISSING_WAVE_FILES != 0
    }

    pub fn set_had_missing_wave_files(&mut self, missing: bool) {
        self.set_flag(HAD_MISSING_WAVE_FILES, missing);
    }

    pub fn set_should_preload(&mut self, preload: bool) {
        self.set_flag(SHOULD_PRELOAD, preload);
    }

    fn set_flag(&mut self, flag: u8, value: bool) {
        if value {
            self.flags |= flag;
        } else {
            self.flags &= !flag;
        }
    }

    pub fn wave_count(&self) -> usize {
        usize::from(self.sound_name_count)
    }

    pub fn converted_count(&self) -> usize {
        usize::from(self.converted_name_count)
    }

    /// Where the waves live, which is inside this struct when there is one.
    ///
    /// # Safety
    ///
    /// The returned pointer is valid while the entry owning this structure
    /// is, and the engine may write the availability byte through it.
    pub fn waves(&self) -> *mut WaveSlot {
        Self::slots(&raw const self.sound_names, self.sound_name_count)
    }

    /// As [`waves`](Self::waves), for the unexpanded `$gender` paths.
    pub fn converted(&self) -> *mut WaveSlot {
        Self::slots(&raw const self.converted_names, self.converted_name_count)
    }

    /// A single wave lives in the bytes of the pointer field itself; more
    /// than one lives where the field points.
    fn slots(field: *const *mut WaveSlot, count: u16) -> *mut WaveSlot {
        if count == 1 {
            field.cast::<WaveSlot>().cast_mut()
        } else {
            // SAFETY: the field is part of a live structure and holds either
            // null or the array this module allocated for it.
            unsafe { field.read_unaligned() }
        }
    }
}

/// One entry's resident state: the structure the engine reads, and the wave
/// arrays it points at.
///
/// The arrays are owned here and freed here. The engine only reads them, and
/// writes one byte of them (availability), so nothing outside this module
/// ever allocates or releases them.
#[derive(Debug)]
pub struct ResidentParams {
    /// Boxed so its address survives the table growing around it: the engine
    /// keeps the pointer `InternalGetParametersForSound` gave it.
    params: Box<SoundParametersInternal>,
    waves: Vec<WaveSlot>,
    converted: Vec<WaveSlot>,
}

/* The engine-visible structure holds raw pointers into arrays this type owns,
so it is no less shareable than the entry table around it; the state lock
serializes this module's own access, and the engine writes only the
availability byte, as it does natively with no lock at all. */
// SAFETY: the pointers are owned by this value and outlive every reader.
unsafe impl Send for ResidentParams {}

impl ResidentParams {
    pub fn new(parsed: &SoundParams) -> Self {
        let mut resident = Self {
            params: Box::new(SoundParametersInternal {
                sound_names: std::ptr::null_mut(),
                converted_names: std::ptr::null_mut(),
                sound_name_count: 0,
                converted_name_count: 0,
                volume: F16Interval {
                    start: Float16::from_f32(parsed.volume.start),
                    range: Float16::from_f32(parsed.volume.range),
                },
                sound_level: U16Interval {
                    start: narrow_u16(parsed.sound_level.start),
                    range: narrow_u16(parsed.sound_level.range),
                },
                pitch: U8Interval {
                    start: narrow_u8(parsed.pitch.start),
                    range: narrow_u8(parsed.pitch.range),
                },
                channel: narrow_u16(parsed.channel as f32),
                delay_msec: narrow_u16(parsed.delay_msec as f32),
                flags: 0,
                reserved: 0,
            }),
            waves: parsed.waves.clone(),
            converted: parsed.converted.clone(),
        };
        resident
            .params
            .set_flag(PLAY_TO_OWNER_ONLY, parsed.play_to_owner_only);
        resident
            .params
            .set_flag(USES_GENDER_TOKEN, parsed.uses_gender_token);
        resident.publish();
        resident
    }

    /// Points the engine-visible structure at the arrays, using the inline
    /// slot when there is exactly one of them.
    fn publish(&mut self) {
        self.params.sound_name_count = self.waves.len().min(u16::MAX as usize) as u16;
        self.params.converted_name_count = self.converted.len().min(u16::MAX as usize) as u16;
        self.params.sound_names = match self.waves.len() {
            0 => std::ptr::null_mut(),
            1 => {
                let slot = self.waves[0];
                // SAFETY: the field is four bytes larger than a WaveSlot and
                // is only read back through the same inline accessor.
                unsafe {
                    (&raw mut self.params.sound_names)
                        .cast::<WaveSlot>()
                        .write_unaligned(slot);
                    (&raw const self.params.sound_names).read_unaligned()
                }
            }
            _ => self.waves.as_mut_ptr(),
        };
        self.params.converted_names = match self.converted.len() {
            0 => std::ptr::null_mut(),
            1 => {
                let slot = self.converted[0];
                // SAFETY: as above, for the converted-name field.
                unsafe {
                    (&raw mut self.params.converted_names)
                        .cast::<WaveSlot>()
                        .write_unaligned(slot);
                    (&raw const self.params.converted_names).read_unaligned()
                }
            }
            _ => self.converted.as_mut_ptr(),
        };
    }

    pub fn as_ptr(&self) -> *mut SoundParametersInternal {
        std::ptr::from_ref(self.params.as_ref()).cast_mut()
    }

    pub fn params(&self) -> &SoundParametersInternal {
        &self.params
    }

    pub fn params_mut(&mut self) -> &mut SoundParametersInternal {
        &mut self.params
    }

    /// The waves as a slice, reading the inline slot when there is one.
    pub fn waves(&self) -> Vec<WaveSlot> {
        let count = self.params.wave_count();
        let base = self.params.waves();
        (0..count)
            // SAFETY: `count` slots live at `base`, either inline or in the
            // vector this structure owns.
            .map(|index| unsafe { base.add(index).read_unaligned() })
            .collect()
    }

    /// Marks one wave as played, wherever it is stored.
    pub fn set_available(&mut self, index: usize, available: bool) {
        if index >= self.params.wave_count() {
            return;
        }
        let base = self.params.waves();
        // SAFETY: the index was just bounds-checked against the count.
        unsafe {
            let mut slot = base.add(index).read_unaligned();
            slot.available = u8::from(available);
            base.add(index).write_unaligned(slot);
        }
    }

    /// Replaces the stored values, as the editor does when it saves a sound.
    pub fn replace(&mut self, source: &SoundParametersInternal) {
        let waves = collect(source.waves(), source.wave_count());
        let converted = collect(source.converted(), source.converted_count());
        *self.params = *source;
        self.waves = waves;
        self.converted = converted;
        self.publish();
    }
}

/// Reads `count` slots from a structure this module does not own.
///
/// # Safety
///
/// `base` must point at `count` readable slots, which is what the interface
/// promises for a `CSoundParametersInternal` a caller passes in.
fn collect(base: *mut WaveSlot, count: usize) -> Vec<WaveSlot> {
    if base.is_null() || count == 0 {
        return Vec::new();
    }
    (0..count)
        // SAFETY: per the contract above.
        .map(|index| unsafe { base.add(index).read_unaligned() })
        .collect()
}

/// Reads a caller's structure into the module's own terms.
///
/// # Safety
///
/// `source` must point at a live `CSoundParametersInternal`.
pub unsafe fn to_parsed(source: *const SoundParametersInternal) -> SoundParams {
    // SAFETY: per the contract above.
    let source = unsafe { &*source };
    SoundParams {
        channel: source.channel(),
        volume: source.volume(),
        pitch: source.pitch(),
        sound_level: source.sound_level(),
        delay_msec: source.delay_msec(),
        play_to_owner_only: source.play_to_owner_only(),
        uses_gender_token: source.uses_gender_token(),
        waves: collect(source.waves(), source.wave_count()),
        converted: collect(source.converted(), source.converted_count()),
    }
}

/// `gender_t` as the interface passes it.
pub fn gender_from_raw(value: c_int) -> Gender {
    Gender::from_raw(u8::try_from(value).unwrap_or(0))
}

/* The engine's narrowing is a C cast, which truncates toward zero; Rust's
saturates instead, which only differs for values no script produces. */
fn narrow_u16(value: f32) -> u16 {
    value as u16
}

fn narrow_u8(value: f32) -> u8 {
    value as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_layout_the_engine_compiled_against() {
        assert_eq!(size_of::<SoundParametersInternal>(), 36);
        assert_eq!(align_of::<SoundParametersInternal>(), 1);
        assert_eq!(size_of::<WaveSlot>(), 4);
        assert_eq!(size_of::<SoundParameters>(), 164);

        let base = SoundParametersInternal {
            sound_names: std::ptr::null_mut(),
            converted_names: std::ptr::null_mut(),
            sound_name_count: 0,
            converted_name_count: 0,
            volume: F16Interval {
                start: Float16::default(),
                range: Float16::default(),
            },
            sound_level: U16Interval { start: 0, range: 0 },
            pitch: U8Interval { start: 0, range: 0 },
            channel: 0,
            delay_msec: 0,
            flags: 0,
            reserved: 0,
        };
        let address = std::ptr::from_ref(&base) as usize;
        let offset = |field: usize| field - address;
        assert_eq!(offset(&raw const base.sound_name_count as usize), 16);
        assert_eq!(offset(&raw const base.volume as usize), 20);
        assert_eq!(offset(&raw const base.sound_level as usize), 24);
        assert_eq!(offset(&raw const base.pitch as usize), 28);
        assert_eq!(offset(&raw const base.channel as usize), 30);
        assert_eq!(offset(&raw const base.delay_msec as usize), 32);
        assert_eq!(offset(&raw const base.flags as usize), 34);
    }

    #[test]
    fn stores_one_wave_inside_the_pointer_field() {
        let mut parsed = SoundParams::default();
        parsed.waves.push(WaveSlot::new(7, Gender::Male));
        let resident = ResidentParams::new(&parsed);

        assert_eq!(resident.params().wave_count(), 1);
        let inline = resident.params().waves();
        assert_eq!(
            inline as usize,
            resident.as_ptr() as usize,
            "stored in place"
        );
        assert_eq!(resident.waves(), parsed.waves);

        parsed.waves.push(WaveSlot::new(9, Gender::Female));
        let resident = ResidentParams::new(&parsed);
        assert_eq!(resident.params().wave_count(), 2);
        assert_ne!(
            resident.params().waves() as usize,
            resident.as_ptr() as usize
        );
        assert_eq!(resident.waves(), parsed.waves);
    }

    #[test]
    fn narrows_values_the_way_the_resident_form_does() {
        let parsed = SoundParams {
            channel: 6,
            volume: Interval {
                start: 0.7,
                range: 0.1,
            },
            pitch: Interval {
                start: 95.0,
                range: 10.0,
            },
            sound_level: Interval {
                start: 140.0,
                range: 0.0,
            },
            delay_msec: 250,
            play_to_owner_only: true,
            uses_gender_token: true,
            ..SoundParams::default()
        };
        let resident = ResidentParams::new(&parsed);
        let params = resident.params();
        assert_eq!(params.channel(), 6);
        assert_eq!(params.delay_msec(), 250);
        assert_eq!(params.sound_level().start, 140.0);
        assert_eq!(params.pitch().start, 95.0);
        assert_eq!(params.pitch().range, 10.0);
        assert!((params.volume().start - 0.7).abs() < 0.001);
        assert!(params.play_to_owner_only());
        assert!(params.uses_gender_token());
        assert!(!params.had_missing_wave_files());

        // The four flags share one byte, so setting one must not clear another.
        let mut resident = resident;
        resident.params_mut().set_had_missing_wave_files(true);
        resident.params_mut().set_should_preload(true);
        assert!(resident.params().had_missing_wave_files());
        assert!(resident.params().play_to_owner_only());
        assert!(resident.params().uses_gender_token());
        resident.params_mut().set_had_missing_wave_files(false);
        assert!(!resident.params().had_missing_wave_files());
        assert!(resident.params().uses_gender_token());
    }

    #[test]
    fn round_trips_a_structure_a_caller_owns() {
        let mut parsed = SoundParams::default();
        parsed.waves.push(WaveSlot::new(1, Gender::Male));
        parsed.waves.push(WaveSlot::new(2, Gender::Female));
        parsed.converted.push(WaveSlot::new(3, Gender::None));
        let resident = ResidentParams::new(&parsed);

        // SAFETY: the pointer names the structure just built.
        let read_back = unsafe { to_parsed(resident.as_ptr()) };
        assert_eq!(read_back.waves, parsed.waves);
        assert_eq!(read_back.converted, parsed.converted);
        assert_eq!(read_back.channel, parsed.channel);
    }
}
