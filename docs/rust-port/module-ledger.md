# Module ledger: what is left in C++, counted

The goal is the game running with no C++. Ownership moved behind the C ABI
does not show up in that count, because the C++ caller stays in the build; a
module replaced whole does. This file counts the second kind.

## Counting

The unit is a C++ translation unit compiled into a `--rust-engine` HL2 build:

```sh
python3 - <<'PY'
import json, collections
units = json.load(open('build-rust-allgames/compile_commands.json'))
by_module = collections.Counter(u['file'].split('/source-engine/')[-1].split('/')[0] for u in units)
print(len(units), dict(by_module.most_common()))
PY
```

`compile_commands.json` accumulates across reconfigurations of one build
directory, so count in a directory configured once, or after deleting the file.
On 2026-09-20 `build-rust-allgames` held 2,646 entries from several games'
configurations, the dropped `SceneFileCache.cpp` still among them, which is why
no per-module table is given here yet. The one figure that is current is Waf's
own: the HL2 graph was 2,256 tasks, compile, link and the Cargo task together,
on the install that first left `scenefilecache` out.

Waf's figure is only comparable between builds configured the same way, and
earlier notes here compared numbers that were not, so `stub_steam`'s cost was
measured as a controlled pair instead: the same fresh build directory, the same
`--build-games=hl2`, configured once with the module dropped and once with it
kept. 2,219 tasks against 2,221. The two that left are its one translation unit
and the link that made a library of it; it had no copy of `memoverride.cpp` to
take with it. Read a graph size from the first progress line of a build in a
fresh directory, where every task still has to run.

## Modules with no C++

A module is listed here only when its own translation units have left the
build and nothing in it calls back into C++ for its work. Reading files is
noted separately, because the filesystem module is still C++ and every module
that loads anything goes through it until that is ported.

| Module | Replaces | Rust | Gate | Since |
| --- | --- | --- | --- | --- |
| `scenefilecache` | `scenefilecache/SceneFileCache.cpp` | `rust/crates/source-scenefilecache`, `source_scene::cache`, `source_compress::lzma` | `rust/verify_scenefilecache.sh` | 2026-09-20 |
| `soundemittersystem` | `soundemittersystem/soundemittersystembase.cpp`, `public/SoundParametersInternal.cpp`, `game/shared/interval.cpp` | `rust/crates/source-soundemittersystem`, `rust/crates/source-soundemitter`, `source-keyvalues` | `rust/verify_soundemitter.sh` | 2026-09-20 |
| `stub_steam` | `stub_steam/steam_api.cpp` | `rust/crates/source-steamapi` | `rust/verify_steam_api.sh` | 2026-09-20 |
| `inputsystem` | `inputsystem/inputsystem.cpp`, `key_translation.cpp`, `steamcontroller.cpp`, `joystick_sdl.cpp`, `touch_sdl.cpp` | `rust/crates/source-inputsystem`, `rust/crates/source-input` | `cargo test -p source-inputsystem` | 2026-09-21 |

`scenefilecache`, `soundemittersystem` and `inputsystem` also drop the module's
copy of `public/tier0/memoverride.cpp`; `stub_steam` had only its one
translation unit. All four are dropped on macOS only; other platforms still
build the C++ module, because the gates have only run here and the table
layouts they rely on are clang's.

`inputsystem` is the first whose gate is written in Rust rather than as a C++
oracle. `rust/crates/source-inputsystem/tests/module.rs` `dlopen`s the built
library, takes its interface through `CreateInterface` and calls every slot by
index off the vtable, which is what a wrong slot number would break; the tables
behind those slots are checked in `source-input`'s own tests. That is weaker
than the differential gates above in one specific way, and it is worth being
plain about it: those compare against the C++ module's answers, and this
compares against what the port was written to produce. The enum arithmetic is
what makes that tolerable here — it is derived, not transcribed, and the
`BUTTON_CODE_LAST` and `ANALOG_CODE_LAST` assertions the C++ makes at compile
time are reproduced, so a table of the wrong length fails to build.

`stub_steam` is the first of the three that something else *links*. The other
two are found through `CreateInterface` and named by nobody, so dropping their
C++ costs the build nothing. `steam_api` is in the `use` list of eight
subprojects — `launcher`, `serverbrowser`, `game/client`, `game/server`,
`inputsystem`, `gameui`, `dedicated` and `engine` — and Waf resolves a `use`
name to a task generator, so with the C++ one gone there has to be another
answering to the name or the library leaves the link line without a word. That
is what `rust/wscript`'s `cargo_link_library` feature is: a task generator whose
link task is Cargo's, which is where Waf reads the `-l` name, the `-L`
directory, and the outputs it makes the linker wait for.

Earlier, and of a different kind: `tier1`'s `snappy.cpp`,
`snappy-sinksource.cpp` and `snappy-stubs-internal.cpp` leave a Rust build
because the codec behind them is Rust, but `tier1` itself is still C++.

## Partly ported, or blocked on a dependency

| Area | What is Rust | What is still C++ |
| --- | --- | --- |
| Filesystem | Pack and VPK parsing, mount tables, search plans, read handles, path selection | `filesystem_stdio` itself: the interface, the search-path registry and the file handles the rest of the engine uses |
| Presentation | Direct3D 9 translated to Metal, the renderer behind it | Everything that calls it: material system, shader API glue, ToGL |
| Host | Process entry, argument storage, app-system lifecycle state | Every subsystem the lifecycle drives |

The two modules above each read their scripts through the C++ filesystem's
`IBaseFileSystem`, which is four calls in
`rust/crates/source-cppabi/src/filesystem.rs`: open, read, close, size, plus
exists and modification time. That is the one dependency they have on C++,
it carries only bytes, and it is the piece that disappears when `filesystem`
is ported. No new C++ was written for either module.

## Candidates, smallest boundary first

Sizes are lines of C++ in the module's own directory. "Boundary" is what makes
it harder than `scenefilecache`, which had eleven slots, plain C types and one
dependency.

Sizes are lines of C++ in the module's own directory. "Boundary" is what makes
it harder than the three already done. The `inputsystem` row was measured on
2026-09-20 with the same tools the ports use, so the next round can start from
it rather than re-deriving it.

| Module | C++ | Boundary |
| --- | --- | --- |
| ~~`inputsystem`~~ | ~~4,017~~ | Done on 2026-09-21; see the row above and `docs/rust-port/inputsystem-boundary.md`. What the estimate below got right and wrong is at the end of this file. **Measured:** `IInputSystem` has 52 virtual functions of its own beside `IAppSystem`'s 5, and the only types crossing are `ButtonCode_t` and `AnalogCode_t`, which are enums, and `InputEvent_t`, which is plain data — no containers, as with `soundemittersystem`. The work is not the boundary but what sits behind it: the module owns SDL2 event pumping, the button-code translation tables and joystick handling, and it links `SDL2` and `steam_api`. SDL is a third-party library that stays after the C++ engine is gone, so binding it is a real boundary rather than migration scaffolding. `steam_api` is Rust now, so that half of its link line is already done. |
| `vpklib` | 2,089 | A static library of C++ classes used directly by `filesystem`, not an interface; goes with the filesystem module. |
| `datacache` | 5,376 | `IDataCache`/`IMDLCache` hand out `studiohdr_t` and vertex data pointers that the renderer and physics keep. |
| `filesystem` | 8 files | `IFileSystem` declares 108 virtual functions of its own, beside `IAppSystem`'s 5 and `IBaseFileSystem`'s 17 (counted with the same clang dump), and passes `CUtlBuffer`; most of the behaviour behind it is already Rust, which makes it the first large module worth taking whole. It is also what would retire the one C++ call the two finished modules still make. |

Next: `inputsystem`, the first port that removes a four-figure number of lines
and the first to bind a library that is meant to stay.

What `stub_steam` actually cost, against the estimate above that called it an
afternoon and a link-and-call check: the forty-five stubs were half an hour and
the estimate was right about them. The part the estimate missed is that the
module is linked rather than loaded, which is a different kind of work — Waf's
`use` resolution, the identity the linker records in eight consumers, and
whether `@rpath` still resolves in a tree where nothing carries an `LC_RPATH`.
The lesson for the rows above is to ask, before estimating, not only what a
module's interface passes but who names it at link time.

An earlier draft of this file named `soundemittersystem` as the module that
would force layout-compatible `CUtlVector`, `CUtlString`, `CUtlBuffer` and
`KeyValues` into Rust. That was wrong, and worth recording. Those types are
all over the module's C++ *implementation*, but its interface passes none of
them: only plain integers, `char *`, two flat structures and a symbol that is
an index. The port needed none of them. It needed a KeyValues *parser*, which
`source-keyvalues` already was, and Rust's own `Vec`, `String` and `HashMap`
for the rest.

That is the general lesson for the modules above: what a module's interface
passes is a much smaller thing than what its implementation uses, and only the
interface has to be reproduced. `datacache` and `filesystem` are ranked last
here because they are the ones whose interfaces really do hand out C++
containers and raw structure pointers the engine then keeps.

## What `inputsystem` actually cost

The estimate above said the work was not the boundary but what sits behind it:
SDL event pumping, the translation tables, joystick handling. Two of those
three were wrong, and both in the same direction.

**It does not pump SDL.** On this platform keyboard, mouse, focus and quit
never come from SDL at all — they arrive as `CCocoaEvent`s from the launcher's
`SDLMgrInterface001`, and `PollJoystick` is deliberately empty with a comment
saying why. SDL carries game controllers and touch, through two *watches* the
launcher's pump invokes. So the SDL surface is about twenty entry points, not
the 124 distinct `SDL_*` symbols the module references, and the ownership
hazard that looked worst — pumping in the wrong place — was never the module's
to get wrong. What remained was to not introduce it: the state lock is dropped
around the pump, because the watches re-enter the module from inside it.

**Most of the Steam Controller file is dead here.** `steamcontroller.cpp` is
695 lines, and all but two tables are behind `SteamControllerInterface()`,
which returns null because `CSteamAPIContext::Init` fails at its first line
against the `steam_api` stub this tree links. That is forced by the linked
implementation rather than observed once, which is why no runtime trace was
built for it. The two origin tables are ported whole because the UI reads them
whether or not a controller is attached.

**The tables were the work.** 635 button-code names, 10 analog names, a
48-entry gamepad renaming, a 256-entry virtual-key table and its reverse, a
128-entry scan-code table with its extended-bit fixups, a 256-entry SDL
scancode keymap, and 43 Steam Controller keys. The general lesson from
`soundemittersystem` held again — read the interface, not the implementation —
but with a twist: here the interface *is* mostly tables, so the implementation
was small and the transcription was the risk. Deriving the enum bounds from
`MAX_JOYSTICKS`, `SK_MAX_KEYS` and friends rather than writing the numbers
down is what makes a mistranscribed table a build failure instead of a wrong
key binding.

Known divergence, recorded rather than hidden: the C++ reads
`joy_axisbutton_threshold`, `joy_axis_deadzone` and `joy_gamecontroller_config`
as convars. Reading those needs the cvar interface, which is `vstdlib`, which
is C++. The Rust module compiles the first two defaults in and does not pass
the third to SDL at all, so a player who has changed either threshold, or who
relies on a custom controller mapping set through that convar rather than
through Steam, gets the default instead. Wiring `ICvar` in through the factory
by vtable slot is the fix, and it is the next thing this module wants.
