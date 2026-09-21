# `inputsystem`: the boundary, slot by slot

What the Rust module has to answer, and what each answer is allowed to touch.
Measured on macOS/arm64 from the translation units waf actually builds
(`OSX=1 POSIX=1 USE_SDL=1 DX_TO_GL_ABSTRACTION=1`, `_LINUX` **not** defined),
with `clang -Xclang -fdump-vtable-layouts` for the slot numbers.

`IInputSystem` is 59 vtable entries: `offset_to_top`, RTTI, then 57 callable
slots — `IAppSystem`'s 5 and `IInputSystem`'s 52.

## The three things that decide the design

**1. The module does not own the SDL event pump.** On this platform keyboard,
mouse, focus and quit never come from SDL directly. `Connect` takes
`SDLMgrInterface001` from the factory and `PollInputState` calls
`ILauncherMgr::PumpWindowsMessageLoop()` and then drains
`ILauncherMgr::GetEvents(CCocoaEvent[32], 32)`. `SleepUntilInput`,
`SetCursorPosition` and `GetRawMouseAccumulators` are forwarded to the same
object. `PollJoystick` is deliberately empty under `USE_SDL`, with a comment
saying why. SDL reaches this module only through two event *watches* it
registers — the launcher's pump invokes them synchronously — plus the
game-controller and haptic calls those watches drive, and one
`SDL_StartTextInput`. The Rust module inherits that contract unchanged: it
registers watches, it never pumps, and it calls `SDL_InitSubSystem` /
`SDL_QuitSubSystem` rather than `SDL_Init` / `SDL_Quit`, because subsystem
initialisation is ref-counted and the launcher owns the outer reference.

**2. The Steam controller path is inert in this build, by construction.**
`CInputSystem::Init` touches Steam only inside
`if ( !m_bSkipControllerInitialization && SteamAPI_InitSafe() )`, then
`m_SteamAPIContext.Init()`, then `if ( m_SteamAPIContext.SteamController() )`.
`CSteamAPIContext::Init` (`public/steam/steam_api.h:532`) opens with
`if ( !SteamClient() ) return false;`, and the `steam_api` this tree links —
now `rust/crates/source-steamapi`, as the C++ `stub_steam` was before it —
returns null from `SteamClient()` and asserts as much in its own test. So
`m_pController` is never assigned, `SteamControllerInterface()` returns null,
and every Steam slot collapses to the answer it gives with a null pointer.

This is not "no controller was plugged in when we looked". It is the linked
implementation forcing the null, which is why no runtime trace was built: the
source read answers the same question with no observation-to-generalisation
gap. It holds exactly as long as `steam_api` is a stub. If a real Steamworks
library is ever linked, slots 44–56 need writing for real, and the two origin
tables below are the only part of them that already is.

**3. `GetEventData` hands out an interior pointer, and the double buffer is
what keeps it valid.** There are two `InputState_t`s. Every write goes to
`m_InputState[ m_bIsPolling ]` — index 0 (`QUEUED`) outside a poll, index 1
(`CURRENT`) inside one. `GetEventCount`/`GetEventData` always read index 1.
So a `PostUserEvent` from a caller between frames appends to `QUEUED` and
cannot disturb the pointer `GetEventData` returned; only `PollInputState`
overwrites `CURRENT`. The pointer is valid until the next `PollInputState`,
and the Rust module reproduces the same indexing for the same reason. An empty
queue returns a dangling-but-aligned pointer, as `CUtlVector::Base()` does;
callers gate on `GetEventCount()`.

## The slots

`self` = module state only. `launcher` = a slot call on `ILauncherMgr`.
`sdl` = an SDL call. All 57 run on the thread that pumps, which is the main
thread; the SDL watches are invoked synchronously from inside that pump.

| # | Slot | In → out | Touches | Notes |
| --- | --- | --- | --- | --- |
| 0 | `Connect` | factory → bool | launcher | Takes `SDLMgrInterface001`. Must not hold the state lock across the factory call: `CAppSystemGroup::FindSystem` sweeps `QueryInterface` over every system, this one included. |
| 1 | `Disconnect` | — | self | |
| 2 | `QueryInterface` | name → ptr | self | Exact match on `InputSystemVersion001`. |
| 3 | `Init` | → `InitReturnVal_t` | self, sdl | Startup tick, key tables, touch + joystick subsystems. Steam block dead (§2). |
| 4 | `Shutdown` | — | sdl | `SDL_QuitSubSystem` for what `Init` took. |
| 5 | `AttachToWindow` | `void*` → — | self | POSIX: records the handle, clears input state. No wndproc. |
| 6 | `DetachFromWindow` | — | self | `ResetInputState` then forget the handle. |
| 7 | `EnableInput` | bool | self | |
| 8 | `EnableMessagePump` | bool | self | Gates the `PumpWindowsMessageLoop` call only. |
| 9 | `PollInputState` | — | self, launcher, sdl | Copy QUEUED→CURRENT with events, `SampleDevices`, pump, drain Cocoa events, copy CURRENT→QUEUED without events. |
| 10 | `GetPollTick` | → int | self | |
| 11 | `IsButtonDown` | code → bool | self | Reads CURRENT. |
| 12–13 | `GetButtonPressedTick` / `ReleasedTick` | code → int | self | Reads CURRENT. |
| 14–15 | `GetAnalogValue` / `GetAnalogDelta` | code → int | self | Reads CURRENT. |
| 16 | `GetEventCount` | → int | self | |
| 17 | `GetEventData` | → `const InputEvent_t*` | self | **Interior pointer, §3.** |
| 18 | `PostUserEvent` | `const InputEvent_t&` | self | Appends to `m_InputState[m_bIsPolling]`. |
| 19 | `GetJoystickCount` | → int | self | |
| 20 | `EnableJoystickInput` | int, bool | self | |
| 21 | `EnableJoystickDiagonalPOV` | int, bool | self | |
| 22 | `SampleDevices` | — | self, sdl | Updates the sample tick; joystick polling is empty here, Steam polling dead. |
| 23 | `SetRumble` | f, f, int | sdl | `SDL_HapticRumblePlay`/`Stop`, gated on the `joystick` convar. |
| 24 | `StopRumble` | — | sdl | Four `SetRumble(0,0,i)`. |
| 25 | `ResetInputState` | — | self | Releases every button (posting events), zeroes analog, clears raw accumulators. |
| 26 | `SetPrimaryUserId` | int | self | |
| 27 | `ButtonCodeToString` | code → `const char*` | self | Static table, 635 entries. X-controller names when a pad is active. |
| 28 | `AnalogCodeToString` | code → `const char*` | self | Static table, 10 entries. |
| 29 | `StringToButtonCode` | `const char*` → code | self | Case-insensitive; `auxN` back-compat first. |
| 30 | `StringToAnalogCode` | `const char*` → code | self | Case-insensitive. |
| 31 | `SleepUntilInput` | int | launcher | `WaitUntilUserInput`, slot 30. |
| 32 | `VirtualKeyToButtonCode` | int → code | self | 256-entry table; POSIX fills only the ASCII and OEM entries. |
| 33 | `ButtonCodeToVirtualKey` | code → int | self | Reverse of the above. |
| 34 | `ScanCodeToButtonCode` | int → code | self | QWERTY table plus the extended-bit fixups. |
| 35 | `GetPollCount` | → int | self | |
| 36 | `SetCursorPosition` | int, int | self, launcher | Forwards, then updates MOUSE_X/Y and posts the three analog events. No-op without an attached window. |
| 37 | `GetHapticsInterfaceAddress` | → `void*` | — | Null off Windows. |
| 38 | `SetNovintPure` | bool | — | Empty off Windows. |
| 39 | `GetRawMouseAccumulators` | `int&`, `int&` → bool | launcher | `GetMouseDelta`, slot 17. True when the launcher is there. |
| 40 | `GetTouchAccumulators` | int, `float&`, `float&` → bool | self | Drains and zeroes one finger's accumulator. Always true. |
| 41 | `SetConsoleTextMode` | bool | self, sdl | After `Init`, shuts the joysticks down. |
| 42 | `SteamControllerInterface` | → `ISteamController*` | — | **Null** (§2). |
| 43 | `GetNumSteamControllersConnected` | → `uint32` | self | 0. |
| 44 | `IsSteamControllerActive` | → bool | self | false. |
| 45 | `IsSteamControllerConnected` | → bool | self | false. |
| 46 | `GetSteamControllerIndexForSlot` | int → int | self | −1; no device is ever marked active. |
| 47 | `GetRadialMenuStickValues` | int, `float&`, `float&` → bool | self | Writes 0,0 and returns **true** — the C++ returns true unconditionally. |
| 48 | `ActivateSteamControllerActionSetForSlot` | `uint64`, set | — | Nothing: the debounce flags only move when the interface is live. |
| 49 | `GetActionSetHandle(GameActionSet_t)` | set → handle | — | 0. Handles are only ever filled from Steam. |
| 50 | `GetActionSetHandle(const char*)` | name → handle | — | 0. |
| 51–52 | `GetSteamControllerActionOrigin` ×2 | → origin | — | `k_EControllerActionOrigin_None`. |
| 53 | `GetSteamControllerFontCharacterForActionOrigin` | origin → `const wchar_t*` | self | **Live table**, 39 entries, bounds-checked, `L""` outside. |
| 54 | `GetSteamControllerDescriptionForActionOrigin` | origin → `const wchar_t*` | self | **Live table**, 39 entries, same guard. |
| 55 | `SetSkipControllerInitialization` | bool | self | |
| 56 | `StartTextInput` | — | sdl | `SDL_StartTextInput`. |

Slots 49–52 answer from state that only Steam fills, so they are constants
here rather than stubs: the C++ returns the same values through the same code.
Slots 53 and 54 are pure tables and are ported in full, because the UI calls
them whether or not a controller is attached.

## What crosses, beyond what the ledger first claimed

`ButtonCode_t` and `AnalogCode_t` are enums and `InputEvent_t` is five ints,
as recorded. The rest, found by dumping the vtable rather than reading the
ledger: `ISteamController*` (slot 42), `EControllerActionOrigin`,
`ControllerActionSetHandle_t` (a `uint64`), `GameActionSet_t`, `const wchar_t*`
(slots 53–54, and `wchar_t` is 4 bytes here), `void*` out of slot 37, and
`int&` / `float&` out-parameters in slots 39, 40 and 47.

## Splitting it

- `source-input` — the deterministic part, no FFI: the code enums and their
  classification arithmetic, the three name tables and the conversions both
  ways, the virtual-key and scan-code tables, the two-slot input state with its
  event queue, and the Cocoa-event translation written as a function from plain
  data to state changes. Testable on its own.
- `source-inputsystem` — the cdylib: the 57-slot table, `CreateInterface`,
  the `ILauncherMgr` slot calls, the SDL declarations and the two event
  watches. The only unsafe code.

## How it is checked

No C++ was written for this port, including for its tests. The checks are:

- `source-input`'s own tests, 36 of them, over the tables and the state
  machine: the enum arithmetic against the numbers the headers produce, the
  name tables against `BUTTON_CODE_LAST` and `ANALOG_CODE_LAST` the way the
  C++ asserts them at compile time, every name round-tripping back to a code
  with that name, the `auxN` back-compatibility branch including `atoi`'s
  behaviour on text that is not a number, the keypad/navigation split on the
  extended bit, and the double buffer — a press reported once, a queued event
  delivered once, a held key surviving a quiet frame, a reset releasing what
  was down.
- `source-inputsystem`'s unit tests, 13, over the vtable's shape and the
  slots that answer without a device.
- `rust/crates/source-inputsystem/tests/module.rs`, 7, which `dlopen`s the
  built library, takes the interface through `CreateInterface` and calls every
  slot **by index off the vtable**. That is the part nothing else covers: an
  entry in the wrong place reads a neighbouring function pointer and calls it
  with the wrong arguments. It also asserts the module exports `CreateInterface`
  and nothing else, imports no C++ runtime, and imports no `SDL_` symbol —
  the last being what keeps it inside the launcher's SDL rather than beside it.

What none of that covers, said plainly so it is not mistaken for covered: the
polling loop against a real `ILauncherMgr`, the SDL watches against a real
device, and anything a player would notice. Those are play tests.

An earlier draft of this port carried a C++ differential harness that built the
C++ module as an oracle and compared every answer, in the shape the three
earlier modules use. It ran once — 8,214 comparisons, no disagreements, over
the whole code range in both directions, all 256 virtual keys, all 635 reverse
lookups, all 128 scan codes against both extended-bit values, and every Steam
slot — and was then deleted, because the goal of this project is Rust and the
harness was C++. The result is recorded here because it is evidence, and the
fact that it is no longer re-runnable is recorded with it.
