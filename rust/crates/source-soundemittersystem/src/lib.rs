//! The `soundemittersystem` engine module, with no C++ in it.
//!
//! The engine loads this library by name and asks its `CreateInterface` for
//! `VSoundEmitter002`; from then on it is an `ISoundEmitterSystemBase`. It
//! replaces `soundemittersystem/soundemittersystembase.cpp`, its copy of
//! `public/SoundParametersInternal.cpp` and of `game/shared/interval.cpp`.
//!
//! What a sound script means lives in [`source_soundemitter`]. What is here
//! is the boundary: the table of functions the engine calls, the resident
//! form of an entry in [`abi`], and reading the scripts through the
//! filesystem module. Nothing here holds the state lock across a call that
//! leaves the module, because the app-system group resolves interface names
//! by asking every system it holds, this one included, while `Connect` is
//! still running.

mod abi;

use abi::{gender_from_raw, ResidentParams, SoundParameters, SoundParametersInternal};
use source_cppabi::appsystem::{AppSystemMethods, InitReturnVal};
use source_cppabi::filesystem::{BaseFileSystem, BASE_FILESYSTEM_INTERFACE, FILESYSTEM_INTERFACE};
use source_cppabi::{create_interface, guard, slot, tier0, CreateInterfaceFn, Object, VTable};
use source_soundemitter::values::SNDLVL_NORM;
use source_soundemitter::{
    choose_wave, gender_expand, parse_actor_genders, parse_manifest, parse_script, Gender, Random,
    WaveNames, WaveSlot,
};
use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_uint, c_void, CStr, CString};
use std::sync::{Mutex, MutexGuard, PoisonError};

/// `SOUNDEMITTERSYSTEM_INTERFACE_VERSION`.
pub const INTERFACE_VERSION: &CStr = c"VSoundEmitter002";
const MANIFEST_FILE: &str = "scripts/game_sounds_manifest.txt";
const ACTORS_FILE: &str = "scripts/global_actors.txt";
const GAME_PATH_ID: &CStr = c"GAME";
/// `SOUNDEMITTER_INVALID_HANDLE`, which is also what a `uint16` invalid
/// handle narrows to in the `short` the interface passes it in.
const INVALID_INDEX: c_int = -1;
/// The native table refuses to grow past this, so handles stay in a `short`.
const MAX_SOUNDS: usize = 65534;
/// `CHAR_SENTENCE`: a name that stands for a sentence, not a file.
const SENTENCE: u8 = b'!';

type This = Object<Methods>;
type HandleRef = *mut i16;

/// `ISoundEmitterSystemBase`: `IAppSystem`'s five, then its own forty, in the
/// order `clang -Xclang -fdump-vtable-layouts` prints for the interface.
#[repr(C)]
struct Methods {
    app_system: AppSystemMethods<This>,
    mod_init: unsafe extern "C" fn(*mut This) -> bool,
    mod_shutdown: unsafe extern "C" fn(*mut This),
    get_sound_index: unsafe extern "C" fn(*mut This, *const c_char) -> c_int,
    is_valid_index: unsafe extern "C" fn(*mut This, c_int) -> bool,
    get_sound_count: unsafe extern "C" fn(*mut This) -> c_int,
    get_sound_name: unsafe extern "C" fn(*mut This, c_int) -> *const c_char,
    get_parameters_for_sound:
        unsafe extern "C" fn(*mut This, *const c_char, *mut SoundParameters, c_int, bool) -> bool,
    get_wave_name: unsafe extern "C" fn(*mut This, *mut u16) -> *const c_char,
    add_wave_name: AddWaveNameFn,
    lookup_sound_level: unsafe extern "C" fn(*mut This, *const c_char) -> c_int,
    get_wav_file_for_sound_by_actor:
        unsafe extern "C" fn(*mut This, *const c_char, *const c_char) -> *const c_char,
    get_wav_file_for_sound_by_gender:
        unsafe extern "C" fn(*mut This, *const c_char, c_int) -> *const c_char,
    check_for_missing_wav_files: unsafe extern "C" fn(*mut This, bool) -> c_int,
    get_source_file_for_sound: unsafe extern "C" fn(*mut This, c_int) -> *const c_char,
    first: unsafe extern "C" fn(*mut This) -> c_int,
    next: unsafe extern "C" fn(*mut This, c_int) -> c_int,
    invalid_index: unsafe extern "C" fn(*mut This) -> c_int,
    internal_get_parameters_for_sound:
        unsafe extern "C" fn(*mut This, c_int) -> *mut SoundParametersInternal,
    add_sound: unsafe extern "C" fn(
        *mut This,
        *const c_char,
        *const c_char,
        *const SoundParametersInternal,
    ) -> bool,
    remove_sound: unsafe extern "C" fn(*mut This, *const c_char),
    move_sound: unsafe extern "C" fn(*mut This, *const c_char, *const c_char),
    rename_sound: unsafe extern "C" fn(*mut This, *const c_char, *const c_char),
    update_sound_parameters:
        unsafe extern "C" fn(*mut This, *const c_char, *const SoundParametersInternal),
    get_num_sound_scripts: unsafe extern "C" fn(*mut This) -> c_int,
    get_sound_script_name: unsafe extern "C" fn(*mut This, c_int) -> *const c_char,
    is_sound_script_dirty: unsafe extern "C" fn(*mut This, c_int) -> bool,
    find_sound_script: unsafe extern "C" fn(*mut This, *const c_char) -> c_int,
    save_changes_to_sound_script: unsafe extern "C" fn(*mut This, c_int),
    expand_sound_name_macros:
        unsafe extern "C" fn(*mut This, *mut SoundParametersInternal, *const c_char),
    get_actor_gender: unsafe extern "C" fn(*mut This, *const c_char) -> c_int,
    gender_expand_string_by_actor:
        unsafe extern "C" fn(*mut This, *const c_char, *const c_char, *mut c_char, c_int),
    gender_expand_string_by_gender:
        unsafe extern "C" fn(*mut This, c_int, *const c_char, *mut c_char, c_int),
    is_using_gender_token: unsafe extern "C" fn(*mut This, *const c_char) -> bool,
    get_manifest_file_time_checksum: unsafe extern "C" fn(*mut This) -> c_uint,
    add_sound_overrides: unsafe extern "C" fn(*mut This, *const c_char, bool),
    clear_sound_overrides: unsafe extern "C" fn(*mut This),
    get_parameters_for_sound_ex: unsafe extern "C" fn(
        *mut This,
        *const c_char,
        HandleRef,
        *mut SoundParameters,
        c_int,
        bool,
    ) -> bool,
    lookup_sound_level_by_handle:
        unsafe extern "C" fn(*mut This, *const c_char, HandleRef) -> c_int,
    reload_sound_entries_in_list: unsafe extern "C" fn(*mut This, *mut c_void),
    flush: unsafe extern "C" fn(*mut This),
}

static TABLE: VTable<Methods> = VTable::new(Methods {
    app_system: AppSystemMethods {
        connect,
        disconnect,
        query_interface,
        init,
        shutdown,
    },
    mod_init,
    mod_shutdown,
    get_sound_index,
    is_valid_index,
    get_sound_count,
    get_sound_name,
    get_parameters_for_sound,
    get_wave_name,
    add_wave_name: ADD_WAVE_NAME,
    lookup_sound_level,
    get_wav_file_for_sound_by_actor,
    get_wav_file_for_sound_by_gender,
    check_for_missing_wav_files,
    get_source_file_for_sound,
    first,
    next,
    invalid_index,
    internal_get_parameters_for_sound,
    add_sound,
    remove_sound,
    move_sound,
    rename_sound,
    update_sound_parameters,
    get_num_sound_scripts,
    get_sound_script_name,
    is_sound_script_dirty,
    find_sound_script,
    save_changes_to_sound_script,
    expand_sound_name_macros,
    get_actor_gender,
    gender_expand_string_by_actor,
    gender_expand_string_by_gender,
    is_using_gender_token,
    get_manifest_file_time_checksum,
    add_sound_overrides,
    clear_sound_overrides,
    get_parameters_for_sound_ex,
    lookup_sound_level_by_handle,
    reload_sound_entries_in_list,
    flush,
});
static SOUND_EMITTER: This = Object::new(&TABLE);

/* `ISoundEmitterSystemBase` is `IAppSystem`'s five virtual functions and its
own forty. A table of any other size would put every later call one slot
out, so the count is checked here rather than found in a game. */
const _: () = assert!(size_of::<Methods>() == 45 * size_of::<*const c_void>());

/// One sound the game can ask for.
struct Entry {
    name: CString,
    /// The name folded once, because every lookup is case-insensitive.
    key: String,
    script: usize,
    is_override: bool,
    params: ResidentParams,
}

struct ScriptFile {
    name: CString,
    dirty: bool,
}

#[derive(Default)]
struct State {
    filesystem: Option<BaseFileSystem>,
    init_count: i32,
    entries: Vec<Entry>,
    by_key: HashMap<String, usize>,
    waves: WaveNames,
    wave_strings: Vec<CString>,
    scripts: Vec<ScriptFile>,
    override_files: Vec<String>,
    /// Entries an override displaced, kept so clearing brings them back.
    displaced: Vec<Entry>,
    actors: Vec<(String, Gender)>,
    checksum: u32,
    random: Random,
    /// The buffer `GetWavFileForSound` answers from, which the native module
    /// also reuses between calls.
    scratch: CString,
    empty: CString,
    /// Wave arrays handed to structures this module does not own, kept alive
    /// because the caller frees the structure but never these.
    lent: Vec<ResidentParams>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn state() -> MutexGuard<'static, Option<State>> {
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Runs `body` with the module's state, creating it on first use.
fn with_state<R>(fallback: R, body: impl FnOnce(&mut State) -> R) -> R {
    guard(fallback, || {
        let mut held = state();
        body(held.get_or_insert_with(State::default))
    })
}

/// The module's one export, which the engine resolves by this name.
///
/// # Safety
///
/// `name` must be null or NUL-terminated and `return_code` null or writable.
#[no_mangle]
pub unsafe extern "C" fn CreateInterface(
    name: *const c_char,
    return_code: *mut c_int,
) -> *mut c_void {
    let interfaces = [(INTERFACE_VERSION, SOUND_EMITTER.as_interface())];
    // SAFETY: forwarded under the caller's contract.
    unsafe { create_interface(&interfaces, name, return_code) }
}

unsafe extern "C" fn connect(_: *mut This, factory: Option<CreateInterfaceFn>) -> bool {
    guard(false, || {
        let Some(factory) = factory else {
            return false;
        };
        // SAFETY: the factory is the engine's, and each name yields the
        // interface type its binding assumes.
        let filesystem = unsafe {
            BaseFileSystem::from_filesystem(factory(
                FILESYSTEM_INTERFACE.as_ptr(),
                std::ptr::null_mut(),
            ))
            .or_else(|| {
                BaseFileSystem::from_raw(factory(
                    BASE_FILESYSTEM_INTERFACE.as_ptr(),
                    std::ptr::null_mut(),
                ))
            })
        };
        if filesystem.is_none() {
            tier0::error("The soundemittersystem system requires the filesystem to run!\n");
            return false;
        }
        state().get_or_insert_with(State::default).filesystem = filesystem;
        true
    })
}

unsafe extern "C" fn disconnect(_: *mut This) {
    with_state((), |state| state.filesystem = None);
}

/// Called while the group resolves any interface name, possibly from inside
/// `connect`, so it takes no lock.
unsafe extern "C" fn query_interface(_: *mut This, name: *const c_char) -> *mut c_void {
    // SAFETY: forwarded under this module's own factory contract.
    unsafe { CreateInterface(name, std::ptr::null_mut()) }
}

unsafe extern "C" fn init(_: *mut This) -> InitReturnVal {
    InitReturnVal::Ok
}

unsafe extern "C" fn shutdown(_: *mut This) {}

unsafe extern "C" fn mod_init(_: *mut This) -> bool {
    guard(false, || {
        let (already, filesystem) = with_state((true, None), |state| {
            state.init_count += 1;
            (state.init_count > 1, state.filesystem)
        });
        if already {
            return true;
        }
        let Some(filesystem) = filesystem else {
            return false;
        };
        load_everything(filesystem);
        true
    })
}

unsafe extern "C" fn mod_shutdown(_: *mut This) {
    with_state((), |state| {
        state.init_count -= 1;
        if state.init_count > 0 {
            return;
        }
        state.entries.clear();
        state.by_key.clear();
        state.scripts.clear();
        state.displaced.clear();
        state.override_files.clear();
        state.waves.clear();
        state.wave_strings.clear();
        state.actors.clear();
    });
}

unsafe extern "C" fn flush(this: *mut This) {
    guard((), || {
        let filesystem = with_state(None, |state| {
            state.entries.clear();
            state.by_key.clear();
            state.scripts.clear();
            state.displaced.clear();
            state.override_files.clear();
            state.waves.clear();
            state.wave_strings.clear();
            state.actors.clear();
            state.filesystem
        });
        let _ = this;
        if let Some(filesystem) = filesystem {
            load_everything(filesystem);
        }
    });
}

/// `InternalModInit`: the actors, then every script the manifest names, and a
/// checksum over the manifest's own timestamps.
fn load_everything(filesystem: BaseFileSystem) {
    if let Some(text) = read_text(filesystem, ACTORS_FILE) {
        let actors = parse_actor_genders(&text);
        with_state((), |state| state.actors = actors);
    }

    let Some(manifest) = read_text(filesystem, MANIFEST_FILE) else {
        tier0::error(&format!("Unable to load manifest file '{MANIFEST_FILE}'\n"));
        return;
    };
    let mut checksum_input = Vec::new();
    accumulate(&mut checksum_input, filesystem, MANIFEST_FILE);
    for (path, preload) in parse_manifest(&manifest) {
        accumulate(&mut checksum_input, filesystem, &path);
        add_sounds_from_file(filesystem, &path, preload, false, false);
    }
    let checksum = source_binary::crc32(&checksum_input);
    with_state((), |state| state.checksum = checksum);
}

/// `AccumulateFileNameAndTimestampIntoChecksum`: the file's time, then its
/// name, appended to one running checksum.
fn accumulate(input: &mut Vec<u8>, filesystem: BaseFileSystem, path: &str) {
    let Ok(name) = CString::new(path) else {
        return;
    };
    let time = filesystem.file_time(&name, GAME_PATH_ID);
    input.extend_from_slice(&time.to_le_bytes());
    input.extend_from_slice(path.as_bytes());
}

/// `AddSoundsFromFile`. An entry that is already known is skipped, unless
/// this file is a map's overrides, which displace it, or a reload, which
/// updates it in place.
fn add_sounds_from_file(
    filesystem: BaseFileSystem,
    path: &str,
    preload: bool,
    is_override: bool,
    refresh: bool,
) {
    let Some(text) = read_text(filesystem, path) else {
        if !is_override {
            tier0::warning(&format!(
                "CSoundEmitterSystem::AddSoundsFromFile:  No such file {path}\n"
            ));
        }
        return;
    };

    let (mut waves, script) = with_state((WaveNames::default(), 0), |state| {
        let script = state.scripts.len();
        state.scripts.push(ScriptFile {
            name: to_cstring(path),
            dirty: false,
        });
        (std::mem::take(&mut state.waves), script)
    });
    let sounds = parse_script(&text, &mut waves);
    with_state((), |state| {
        state.waves = waves;
        state.sync_wave_strings();
        for sound in sounds {
            state.insert(sound, script, preload, is_override, refresh);
        }
    });
}

impl State {
    fn sync_wave_strings(&mut self) {
        while self.wave_strings.len() < self.waves.len() {
            let symbol = self.wave_strings.len() as u16;
            let name = self.waves.get(symbol).unwrap_or_default();
            self.wave_strings.push(to_cstring(name));
        }
    }

    fn intern_wave(&mut self, name: &str) -> u16 {
        let symbol = self.waves.intern(name);
        self.sync_wave_strings();
        symbol
    }

    fn find(&self, name: &str) -> Option<usize> {
        self.by_key.get(&fold(name)).copied()
    }

    fn insert(
        &mut self,
        sound: source_soundemitter::ParsedSound,
        script: usize,
        preload: bool,
        is_override: bool,
        refresh: bool,
    ) {
        let key = fold(&sound.name);
        match self.by_key.get(&key).copied() {
            Some(existing) if is_override => {
                let mut entry = self.new_entry(sound, key, script, preload, true);
                std::mem::swap(&mut self.entries[existing], &mut entry);
                if !entry.is_override {
                    self.displaced.push(entry);
                }
            }
            Some(existing) if refresh => {
                self.entries[existing].params = ResidentParams::new(&sound.params);
            }
            Some(_) => {}
            None => {
                if self.entries.len() >= MAX_SOUNDS {
                    return;
                }
                let entry = self.new_entry(sound, key.clone(), script, preload, is_override);
                self.by_key.insert(key, self.entries.len());
                self.entries.push(entry);
            }
        }
    }

    fn new_entry(
        &self,
        sound: source_soundemitter::ParsedSound,
        key: String,
        script: usize,
        preload: bool,
        is_override: bool,
    ) -> Entry {
        let mut params = ResidentParams::new(&sound.params);
        params.params_mut().set_should_preload(preload);
        Entry {
            name: to_cstring(&sound.name),
            key,
            script,
            is_override,
            params,
        }
    }

    /// `GetParametersForSoundEx` once the entry is known.
    fn fill(&mut self, index: usize, out: &mut SoundParameters, gender: Gender, emitting: bool) {
        let mut slots = self.entries[index].params.waves();
        let chosen = choose_wave(&mut slots, gender, &mut self.random);
        let entry = &mut self.entries[index];
        for (index, slot) in slots.iter().enumerate() {
            entry.params.set_available(index, slot.available != 0);
        }
        let params = entry.params.params();
        out.channel = params.channel();
        out.volume = self.random.from_interval(params.volume());
        out.pitch = self.random.from_interval(params.pitch()) as c_int;
        out.pitch_low = params.pitch().start as c_int;
        out.pitch_high = out.pitch_low + params.pitch().range as c_int;
        out.delay_msec = params.delay_msec();
        out.count = params.wave_count() as c_int;
        out.set_soundname("");
        if let Some(chosen) = chosen {
            let symbol = slots[chosen].symbol;
            let name = self.waves.get(symbol).unwrap_or_default().to_owned();
            out.set_soundname(&name);
            if emitting {
                self.entries[index].params.set_available(chosen, false);
            }
        }
        let params = self.entries[index].params.params();
        out.sound_level = self.random.from_interval(params.sound_level()) as c_int;
        out.play_to_owner_only = params.play_to_owner_only();
    }
}

/// A sound index as the native table reads one: its handles are `uint16`, so
/// an `int` that does not fit is truncated rather than rejected, both when
/// checking it and when following it.
fn handle(index: c_int) -> u16 {
    index as u16
}

/// The lookup key: sound names are matched without regard to case.
fn fold(name: &str) -> String {
    name.to_ascii_lowercase()
}

/// Reads a file through the engine's search paths.
///
/// This is the module's one remaining dependency on a part of the engine that
/// is still C++. It reads bytes; nothing of the filesystem's structure comes
/// across, so when `filesystem` is itself ported this is the only place that
/// changes. Bytes become text one byte per character, which round-trips any
/// encoding a script is written in, since only ASCII punctuation is parsed.
fn read_text(filesystem: BaseFileSystem, path: &str) -> Option<String> {
    let name = CString::new(path).ok()?;
    let bytes = filesystem.read_file(&name, GAME_PATH_ID)?;
    Some(bytes.iter().map(|byte| char::from(*byte)).collect())
}

/// The inverse of the decoding above.
fn to_cstring(text: &str) -> CString {
    let bytes: Vec<u8> = text
        .chars()
        .filter(|c| *c != '\0')
        .map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?'))
        .collect();
    CString::new(bytes).unwrap_or_default()
}

/// # Safety
///
/// `text` must be null or NUL-terminated.
unsafe fn borrow<'a>(text: *const c_char) -> &'a str {
    if text.is_null() {
        return "";
    }
    // SAFETY: non-null and terminated per the contract; the bytes are read
    // one per character, as `read_text` writes them.
    let bytes = unsafe { CStr::from_ptr(text) }.to_bytes();
    std::str::from_utf8(bytes).unwrap_or("")
}

/// Copies a string into a caller's fixed buffer, as `Q_strncpy` does.
///
/// # Safety
///
/// `out` must point at `max_len` writable bytes.
unsafe fn write_buffer(out: *mut c_char, max_len: c_int, text: &str) {
    if out.is_null() || max_len <= 0 {
        return;
    }
    let capacity = (max_len - 1) as usize;
    let bytes = text.as_bytes();
    let count = bytes.len().min(capacity);
    // SAFETY: `count` is within the caller's buffer, and the terminator goes
    // at `count`, which is at most `max_len - 1`.
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr().cast::<c_char>(), out, count);
        *out.add(count) = 0;
    }
}

unsafe extern "C" fn get_sound_index(_: *mut This, name: *const c_char) -> c_int {
    // SAFETY: the engine passes a terminated name.
    let name = unsafe { borrow(name) };
    with_state(INVALID_INDEX, |state| {
        state
            .find(name)
            .and_then(|index| c_int::try_from(index).ok())
            .unwrap_or(INVALID_INDEX)
    })
}

unsafe extern "C" fn is_valid_index(_: *mut This, index: c_int) -> bool {
    with_state(false, |state| {
        usize::from(handle(index)) < state.entries.len()
    })
}

unsafe extern "C" fn get_sound_count(_: *mut This) -> c_int {
    with_state(0, |state| state.entries.len() as c_int)
}

unsafe extern "C" fn get_sound_name(_: *mut This, index: c_int) -> *const c_char {
    with_state(std::ptr::null(), |state| {
        state
            .entries
            .get(usize::from(handle(index)))
            .map_or(&state.empty, |entry| &entry.name)
            .as_ptr()
    })
}

unsafe extern "C" fn first(_: *mut This) -> c_int {
    with_state(INVALID_INDEX, |state| {
        if state.entries.is_empty() {
            INVALID_INDEX
        } else {
            0
        }
    })
}

unsafe extern "C" fn next(_: *mut This, index: c_int) -> c_int {
    with_state(INVALID_INDEX, |state| {
        let following = index.saturating_add(1);
        if usize::try_from(following).is_ok_and(|following| following < state.entries.len()) {
            following
        } else {
            INVALID_INDEX
        }
    })
}

unsafe extern "C" fn invalid_index(_: *mut This) -> c_int {
    INVALID_INDEX
}

unsafe extern "C" fn internal_get_parameters_for_sound(
    _: *mut This,
    index: c_int,
) -> *mut SoundParametersInternal {
    with_state(std::ptr::null_mut(), |state| {
        state
            .entries
            .get(usize::from(handle(index)))
            .map_or(std::ptr::null_mut(), |entry| entry.params.as_ptr())
    })
}

unsafe extern "C" fn get_parameters_for_sound(
    this: *mut This,
    name: *const c_char,
    out: *mut SoundParameters,
    gender: c_int,
    emitting: bool,
) -> bool {
    // SAFETY: forwarded under the caller's contract; the handle is local.
    unsafe {
        let mut handle: i16 = INVALID_INDEX as i16;
        get_parameters_for_sound_ex(this, name, &mut handle, out, gender, emitting)
    }
}

unsafe extern "C" fn get_parameters_for_sound_ex(
    _: *mut This,
    name: *const c_char,
    handle: HandleRef,
    out: *mut SoundParameters,
    gender: c_int,
    emitting: bool,
) -> bool {
    guard(false, || {
        if out.is_null() {
            return false;
        }
        // SAFETY: the engine passes a terminated name and a writable handle.
        let requested = unsafe { borrow(name) };
        let index = unsafe {
            match handle.as_ref().copied() {
                Some(stored) if stored != INVALID_INDEX as i16 => c_int::from(stored),
                _ => {
                    let found = get_sound_index(std::ptr::null_mut(), name);
                    if let Some(handle) = handle.as_mut() {
                        *handle = found as i16;
                    }
                    found
                }
            }
        };
        let Ok(index) = usize::try_from(index) else {
            return false;
        };

        let (filled, had_missing) = with_state((false, false), |state| {
            if index >= state.entries.len() {
                return (false, false);
            }
            // SAFETY: the engine owns the structure it passed.
            let out = unsafe { &mut *out };
            state.fill(index, out, gender_from_raw(gender), emitting);
            (
                !out.soundname_is_empty(),
                state.entries[index]
                    .params
                    .params()
                    .had_missing_wave_files(),
            )
        });
        if !filled {
            tier0::warning(&format!(
                "CSoundEmitterSystemBase::GetParametersForSound:  sound {requested} has no wave or rndwave key!\n"
            ));
            return false;
        }
        if had_missing {
            // SAFETY: filled above by this call.
            let wave = unsafe { borrow((*out).soundname.as_ptr()) }.to_owned();
            if !wave.starts_with(char::from(SENTENCE)) && !wave_exists(&wave) {
                return false;
            }
        }
        true
    })
}

/// Whether the wave is on disk, under the name the engine would look for.
fn wave_exists(wave: &str) -> bool {
    let filesystem = with_state(None, |state| state.filesystem);
    let Some(filesystem) = filesystem else {
        return true;
    };
    let Ok(path) = CString::new(format!("sound/{}", skip_sound_chars(wave))) else {
        return true;
    };
    filesystem.file_exists(&path, GAME_PATH_ID)
}

/// `PSkipSoundChars`: the prefix characters that say how a wave is played
/// rather than where it is.
fn skip_sound_chars(name: &str) -> &str {
    name.trim_start_matches(|c| "*?!#@><^)}".contains(c))
}

unsafe extern "C" fn lookup_sound_level(this: *mut This, name: *const c_char) -> c_int {
    // SAFETY: forwarded under the caller's contract.
    unsafe {
        let mut handle: i16 = INVALID_INDEX as i16;
        lookup_sound_level_by_handle(this, name, &mut handle)
    }
}

unsafe extern "C" fn lookup_sound_level_by_handle(
    this: *mut This,
    name: *const c_char,
    handle: HandleRef,
) -> c_int {
    guard(SNDLVL_NORM, || {
        let mut params = empty_parameters();
        // SAFETY: forwarded under the caller's contract.
        let found = unsafe {
            get_parameters_for_sound_ex(
                this,
                name,
                handle,
                &mut params,
                Gender::None as c_int,
                false,
            )
        };
        if found {
            params.sound_level
        } else {
            SNDLVL_NORM
        }
    })
}

fn empty_parameters() -> SoundParameters {
    SoundParameters {
        channel: 0,
        volume: 1.0,
        pitch: 100,
        pitch_low: 100,
        pitch_high: 100,
        sound_level: SNDLVL_NORM,
        play_to_owner_only: false,
        count: 0,
        soundname: [0; 128],
        delay_msec: 0,
    }
}

unsafe extern "C" fn get_wav_file_for_sound_by_gender(
    this: *mut This,
    name: *const c_char,
    gender: c_int,
) -> *const c_char {
    guard(name, || {
        let mut params = empty_parameters();
        // SAFETY: forwarded under the caller's contract.
        let found = unsafe {
            let mut handle: i16 = INVALID_INDEX as i16;
            get_parameters_for_sound_ex(this, name, &mut handle, &mut params, gender, false)
        };
        if !found || params.soundname_is_empty() {
            return name;
        }
        // SAFETY: filled by the call above.
        let wave = unsafe { borrow(params.soundname.as_ptr()) }.to_owned();
        with_state(name, |state| {
            state.scratch = to_cstring(&wave);
            state.scratch.as_ptr()
        })
    })
}

unsafe extern "C" fn get_wav_file_for_sound_by_actor(
    this: *mut This,
    name: *const c_char,
    actor: *const c_char,
) -> *const c_char {
    // SAFETY: forwarded under the caller's contract.
    unsafe {
        let gender = get_actor_gender(this, actor);
        get_wav_file_for_sound_by_gender(this, name, gender)
    }
}

unsafe extern "C" fn get_wave_name(_: *mut This, symbol: *mut u16) -> *const c_char {
    with_state(std::ptr::null(), |state| {
        // SAFETY: the engine passes a reference to a symbol it holds.
        let symbol = match unsafe { symbol.as_ref() } {
            Some(symbol) => *symbol,
            None => return state.empty.as_ptr(),
        };
        state
            .wave_strings
            .get(usize::from(symbol))
            .unwrap_or(&state.empty)
            .as_ptr()
    })
}

/// `AddWaveName`, whose `CUtlSymbol` return travels in memory because that
/// class has a copy constructor, which the Itanium ABI treats as non-trivial.
/// The destination pointer arrives in x8 on AArch64, so the thunk below moves
/// it into an ordinary argument; every other target this crate compiles for
/// passes it as the first argument already.
type AddWaveNameFn = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void);

#[cfg(target_arch = "aarch64")]
std::arch::global_asm!(
    ".globl _source_add_wave_name_thunk",
    ".p2align 2",
    "_source_add_wave_name_thunk:",
    "mov x2, x8",
    "b _source_add_wave_name",
);

#[cfg(target_arch = "aarch64")]
extern "C" {
    /// Takes `this` and the name, and the indirect result in x8.
    fn source_add_wave_name_thunk(this: *mut c_void, name: *mut c_void, unused: *mut c_void);
}

#[cfg(target_arch = "aarch64")]
const ADD_WAVE_NAME: AddWaveNameFn = source_add_wave_name_thunk;

#[cfg(not(target_arch = "aarch64"))]
const ADD_WAVE_NAME: AddWaveNameFn = add_wave_name_indirect_first;

/// Everywhere else the indirect result is simply the first argument.
#[cfg(not(target_arch = "aarch64"))]
unsafe extern "C" fn add_wave_name_indirect_first(
    out: *mut c_void,
    this: *mut c_void,
    name: *mut c_void,
) {
    // SAFETY: the two leading pointers are swapped relative to AArch64.
    unsafe { source_add_wave_name(this, name.cast(), out.cast()) }
}

#[no_mangle]
unsafe extern "C" fn source_add_wave_name(_: *mut c_void, name: *const c_char, out: *mut u16) {
    // SAFETY: the engine passes a terminated name and a place for the symbol.
    let name = unsafe { borrow(name) }.to_owned();
    let symbol = with_state(u16::MAX, |state| state.intern_wave(&name));
    if !out.is_null() {
        // SAFETY: the caller's indirect-result pointer.
        unsafe { out.write(symbol) };
    }
}

unsafe extern "C" fn get_actor_gender(_: *mut This, model: *const c_char) -> c_int {
    // SAFETY: the engine passes a terminated model name or null.
    let model = unsafe { borrow(model) };
    let base = file_base(model);
    with_state(Gender::None as c_int, |state| {
        state
            .actors
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(base))
            .map_or(Gender::None, |(_, gender)| *gender) as c_int
    })
}

/// `Q_FileBase`: the name with its directories and extension removed.
fn file_base(path: &str) -> &str {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    match name.rfind('.') {
        Some(dot) if dot > 0 => &name[..dot],
        _ => name,
    }
}

unsafe extern "C" fn gender_expand_string_by_gender(
    _: *mut This,
    gender: c_int,
    input: *const c_char,
    out: *mut c_char,
    max_len: c_int,
) {
    guard((), || {
        // SAFETY: the engine passes a terminated string and a buffer of at
        // least `max_len` bytes.
        unsafe {
            let expanded = gender_expand(gender_from_raw(gender), borrow(input));
            write_buffer(out, max_len, &expanded);
        }
    })
}

unsafe extern "C" fn gender_expand_string_by_actor(
    this: *mut This,
    actor: *const c_char,
    input: *const c_char,
    out: *mut c_char,
    max_len: c_int,
) {
    // SAFETY: forwarded under the caller's contract.
    unsafe {
        let gender = get_actor_gender(this, actor);
        gender_expand_string_by_gender(this, gender, input, out, max_len);
    }
}

unsafe extern "C" fn is_using_gender_token(_: *mut This, name: *const c_char) -> bool {
    // SAFETY: the engine passes a terminated name.
    let name = unsafe { borrow(name) };
    with_state(false, |state| {
        state
            .find(name)
            .is_some_and(|index| state.entries[index].params.params().uses_gender_token())
    })
}

unsafe extern "C" fn get_manifest_file_time_checksum(_: *mut This) -> c_uint {
    with_state(0, |state| state.checksum)
}

unsafe extern "C" fn check_for_missing_wav_files(_: *mut This, verbose: bool) -> c_int {
    guard(0, || {
        let waves: Vec<(usize, Vec<WaveSlot>, String)> = with_state(Vec::new(), |state| {
            (0..state.entries.len())
                .map(|index| {
                    let entry = &state.entries[index];
                    (
                        index,
                        entry.params.waves(),
                        entry.name.to_string_lossy().into_owned(),
                    )
                })
                .collect()
        });
        let mut missing = 0;
        for (index, slots, sound) in waves {
            let mut entry_missing = false;
            for slot in slots {
                let name = with_state(String::new(), |state| {
                    state.waves.get(slot.symbol).unwrap_or_default().to_owned()
                });
                if name.is_empty() || name.starts_with(char::from(SENTENCE)) {
                    continue;
                }
                if wave_exists(&name) {
                    continue;
                }
                entry_missing = true;
                missing += 1;
                if verbose {
                    tier0::warning(&format!("Sound {sound} references missing file {name}\n"));
                }
            }
            if entry_missing {
                with_state((), |state| {
                    if let Some(entry) = state.entries.get_mut(index) {
                        entry.params.params_mut().set_had_missing_wave_files(true);
                    }
                });
            }
        }
        missing
    })
}

unsafe extern "C" fn get_source_file_for_sound(_: *mut This, index: c_int) -> *const c_char {
    with_state(std::ptr::null(), |state| {
        usize::try_from(index)
            .ok()
            .filter(|index| *index < state.entries.len())
            .and_then(|index| state.entries.get(index))
            .and_then(|entry| state.scripts.get(entry.script))
            .map_or(state.empty.as_ptr(), |script| script.name.as_ptr())
    })
}

unsafe extern "C" fn get_num_sound_scripts(_: *mut This) -> c_int {
    with_state(0, |state| state.scripts.len() as c_int)
}

unsafe extern "C" fn get_sound_script_name(_: *mut This, index: c_int) -> *const c_char {
    with_state(std::ptr::null(), |state| {
        usize::try_from(index)
            .ok()
            .and_then(|index| state.scripts.get(index))
            .map_or(state.empty.as_ptr(), |script| script.name.as_ptr())
    })
}

unsafe extern "C" fn is_sound_script_dirty(_: *mut This, index: c_int) -> bool {
    with_state(false, |state| {
        usize::try_from(index)
            .ok()
            .and_then(|index| state.scripts.get(index))
            .is_some_and(|script| script.dirty)
    })
}

unsafe extern "C" fn find_sound_script(_: *mut This, name: *const c_char) -> c_int {
    // SAFETY: the engine passes a terminated name.
    let name = unsafe { borrow(name) };
    with_state(INVALID_INDEX, |state| {
        state
            .scripts
            .iter()
            .position(|script| script.name.to_bytes().eq_ignore_ascii_case(name.as_bytes()))
            .map_or(INVALID_INDEX, |index| index as c_int)
    })
}

unsafe extern "C" fn add_sound_overrides(_: *mut This, path: *const c_char, preload: bool) {
    guard((), || {
        // SAFETY: the engine passes a terminated path.
        let path = unsafe { borrow(path) }.to_owned();
        let filesystem = with_state(None, |state| {
            if state
                .override_files
                .iter()
                .any(|known| known.eq_ignore_ascii_case(&path))
            {
                return None;
            }
            state.override_files.push(path.clone());
            state.filesystem
        });
        if let Some(filesystem) = filesystem {
            add_sounds_from_file(filesystem, &path, preload, true, false);
        }
    })
}

unsafe extern "C" fn clear_sound_overrides(_: *mut This) {
    with_state((), |state| {
        state.entries.retain(|entry| !entry.is_override);
        for displaced in std::mem::take(&mut state.displaced) {
            state.entries.push(displaced);
        }
        state.override_files.clear();
        state.reindex();
    });
}

impl State {
    /// Rebuilds the name index after entries move, which is the one time the
    /// handles the engine holds stop meaning what they meant.
    fn reindex(&mut self) {
        self.by_key.clear();
        for (index, entry) in self.entries.iter().enumerate() {
            self.by_key.insert(entry.key.clone(), index);
        }
    }
}

unsafe extern "C" fn reload_sound_entries_in_list(_: *mut This, list: *mut c_void) {
    guard((), || {
        if list.is_null() {
            return;
        }
        let (filesystem, scripts) = with_state((None, Vec::new()), |state| {
            (
                state.filesystem,
                state
                    .scripts
                    .iter()
                    .map(|script| script.name.to_string_lossy().into_owned())
                    .collect::<Vec<_>>(),
            )
        });
        let Some(filesystem) = filesystem else {
            return;
        };
        let mut processed: Vec<String> = Vec::new();
        for path in scripts {
            if path.is_empty() || processed.contains(&path) {
                continue;
            }
            let Ok(name) = CString::new(path.clone()) else {
                continue;
            };
            // SAFETY: `IFileList`'s first virtual function is `IsFileInList`.
            let wanted = unsafe {
                let is_in_list: unsafe extern "C" fn(*mut c_void, *const c_char) -> bool =
                    slot(list, 0);
                is_in_list(list, name.as_ptr())
            };
            if !wanted {
                continue;
            }
            tier0::warning(&format!(
                "Reloading sound file '{path}' due to pure settings.\n"
            ));
            add_sounds_from_file(filesystem, &path, false, false, true);
            processed.push(path);
        }
    })
}

unsafe extern "C" fn expand_sound_name_macros(
    _: *mut This,
    params: *mut SoundParametersInternal,
    wave: *const c_char,
) {
    guard((), || {
        if params.is_null() {
            return;
        }
        // SAFETY: the engine passes a structure it owns and a terminated name.
        let wave = unsafe { borrow(wave) }.to_owned();
        // The caller's structure is filled through the same code path a
        // script entry takes, then copied back into their memory.
        let mut parsed = unsafe { abi::to_parsed(params) };
        with_state((), |state| {
            let mut waves = std::mem::take(&mut state.waves);
            source_soundemitter::expand_into(&mut parsed, &wave, &mut waves);
            state.waves = waves;
            state.sync_wave_strings();
            let resident = ResidentParams::new(&parsed);
            // SAFETY: the caller owns a whole structure at `params`, and the
            // arrays it now points at stay alive here for as long as the
            // module does, because the caller frees neither.
            unsafe { params.write_unaligned(*resident.params()) };
            state.lent.push(resident);
        });
    })
}

unsafe extern "C" fn add_sound(
    _: *mut This,
    name: *const c_char,
    script: *const c_char,
    params: *const SoundParametersInternal,
) -> bool {
    guard(false, || {
        if params.is_null() {
            return false;
        }
        // SAFETY: the engine passes terminated names and a live structure.
        let (name, script) = unsafe { (borrow(name).to_owned(), borrow(script).to_owned()) };
        let parsed = unsafe { abi::to_parsed(params) };
        with_state(false, |state| {
            if state.find(&name).is_some() || state.entries.len() >= MAX_SOUNDS {
                return false;
            }
            let index = state
                .scripts
                .iter()
                .position(|file| file.name.to_bytes() == script.as_bytes())
                .unwrap_or_else(|| {
                    state.scripts.push(ScriptFile {
                        name: to_cstring(&script),
                        dirty: true,
                    });
                    state.scripts.len() - 1
                });
            let key = fold(&name);
            let entry = Entry {
                name: to_cstring(&name),
                key: key.clone(),
                script: index,
                is_override: false,
                params: ResidentParams::new(&parsed),
            };
            state.by_key.insert(key, state.entries.len());
            state.entries.push(entry);
            state.mark_dirty(index);
            true
        })
    })
}

impl State {
    fn mark_dirty(&mut self, script: usize) {
        if let Some(file) = self.scripts.get_mut(script) {
            file.dirty = true;
        }
    }
}

unsafe extern "C" fn remove_sound(_: *mut This, name: *const c_char) {
    // SAFETY: the engine passes a terminated name.
    let name = unsafe { borrow(name) }.to_owned();
    with_state((), |state| {
        let Some(index) = state.find(&name) else {
            return;
        };
        let script = state.entries[index].script;
        state.entries.remove(index);
        state.reindex();
        state.mark_dirty(script);
    });
}

unsafe extern "C" fn move_sound(_: *mut This, name: *const c_char, script: *const c_char) {
    // SAFETY: the engine passes terminated names.
    let (name, script) = unsafe { (borrow(name).to_owned(), borrow(script).to_owned()) };
    with_state((), |state| {
        let Some(index) = state.find(&name) else {
            return;
        };
        let Some(target) = state
            .scripts
            .iter()
            .position(|file| file.name.to_bytes() == script.as_bytes())
        else {
            return;
        };
        let previous = state.entries[index].script;
        if previous == target {
            return;
        }
        state.entries[index].script = target;
        state.mark_dirty(previous);
        state.mark_dirty(target);
    });
}

unsafe extern "C" fn rename_sound(_: *mut This, name: *const c_char, new_name: *const c_char) {
    // SAFETY: the engine passes terminated names.
    let (name, new_name) = unsafe { (borrow(name).to_owned(), borrow(new_name).to_owned()) };
    with_state((), |state| {
        let Some(index) = state.find(&name) else {
            return;
        };
        if state.find(&new_name).is_some() {
            return;
        }
        let script = state.entries[index].script;
        state.entries[index].name = to_cstring(&new_name);
        state.entries[index].key = fold(&new_name);
        state.reindex();
        state.mark_dirty(script);
    });
}

unsafe extern "C" fn update_sound_parameters(
    _: *mut This,
    name: *const c_char,
    params: *const SoundParametersInternal,
) {
    guard((), || {
        if params.is_null() {
            return;
        }
        // SAFETY: the engine passes a terminated name and a live structure.
        let name = unsafe { borrow(name) }.to_owned();
        let source = unsafe { &*params };
        with_state((), |state| {
            let Some(index) = state.find(&name) else {
                return;
            };
            state.entries[index].params.replace(source);
            let script = state.entries[index].script;
            state.mark_dirty(script);
        });
    })
}

/// Writing a script back out belongs to the authoring tools, which are not
/// built on this platform and have no Rust port yet. The entry stays marked
/// dirty so nothing believes the file on disk was updated.
unsafe extern "C" fn save_changes_to_sound_script(_: *mut This, index: c_int) {
    with_state((), |state| {
        let name = usize::try_from(index)
            .ok()
            .and_then(|index| state.scripts.get(index))
            .map(|script| script.name.to_string_lossy().into_owned())
            .unwrap_or_default();
        tier0::warning(&format!(
            "CSoundEmitterSystem:  saving sound scripts is not implemented in this build ({name})\n"
        ));
    });
}
