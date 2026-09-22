# `filesystem`: what the port is actually against

Measurement slice, 2026-09-22. No code written. This is the file the next
session should start from, in the same way
`docs/rust-port/inputsystem-boundary.md` was for that port.

## The module

`filesystem_stdio` builds eight translation units on macOS, plus its copy of
`memoverride.cpp`:

| TU | Lines |
| --- | --- |
| `basefilesystem.cpp` | 6,712 |
| `QueuedLoader.cpp` | 1,978 |
| `filesystem_stdio.cpp` | 1,612 |
| `filesystem_async.cpp` | 1,537 |
| `packfile.cpp` | 1,300 |
| `filetracker.cpp` | 596 |
| `../public/zip_utils.cpp`, `../public/kevvaluescompiler.cpp` | also compiled in |

`filesystem_steam.cpp` (1,536) is **not** in this module's source list; it
belongs to a different one. The total for the directory is 15,537 lines, which
is three times what the ledger's earlier "8 files" note implied.

It is **loadable**: named in no other subproject's `use` list, so it needs no
`cargo_link_library` generator the way `steam_api` did. But it *links*
`vpklib`, and so does `dedicated`, so `vpklib` is a link-time dependency of two
subprojects and cannot be left behind on its own.

## The surface

`IFileSystem` is 134 vtable entries — 132 callable, and that already includes
the 17 it inherits from `IBaseFileSystem`. Categorised from the clang dump:

| Count | Group |
| --- | --- |
| 21 | async I/O — `AsyncRead*`, `AsyncWrite*`, `AsyncAppend*`, `AsyncFinish*`, `AsyncSuspend`/`Resume`, fetchers |
| 16 | search paths and mounts — `AddSearchPath`, `AddPackFile`, `AddVPKFile`, `RelativePathToFullPath`, … |
| 12 | read / write / stat variants — `OpenEx`, `ReadEx`, `ReadLine`, `GetLocalCopy`, … |
| 10 | pure-server / CRC — whitelists, cached hashes, VPK hashes |
| 6 | find / iterate — `FindFirst`, `FindNext`, `FindClose`, `FindFirstEx`, filename handles |
| 5 | `IAppSystem` lifecycle |
| 4 | passes `CUtlBuffer` |
| 2 | module loading — `LoadModule`, `UnloadModule` |
| 56 | the core file operations and the rest |

A first pass at this counted 50 slots as pure-server, which was wrong: clang
marks every slot `[pure]` because they are pure virtual, and the regex matched
that rather than the feature. **It is 10.** The hope that half this module is
inert the way `inputsystem`'s Steam half was is therefore dead — there is no
large inert region here, and the port has to answer essentially all 132.

## What is already Rust

This is what makes the module worth taking despite its size. It is not a port
from nothing:

| Crate | Lines | Holds |
| --- | --- | --- |
| `source-filesystem` | 3,915 | `gameinfo`, `mount_table`, `pack_archive`, `pack_mounts`, `search_plan`, `selection` |
| `source-pak` | 1,078 | pack archives |
| `source-vpk` | 688 | VPK parsing — which is what `vpklib` is for |
| `source-binary` | 513 | binary reading |

And the behaviour is not merely written, it is *running*. The engine log from a
live session shows the C++ module already delegating:

```
Rust filesystem read active: materials/debug/debugmrmwireframe.vmt (145 bytes)
Rust filesystem read handle active: materials/debug/debugmrmwireframe.vmt
Rust filesystem write path active: …/hl2/videoconfig_mac.cfg
Rust filesystem path resolution active: resource/game-icon.bmp -> …
Rust filesystem wildcard search active: media/valve.*
Rust filesystem metadata active: materials/console/background03.vtf (174968 bytes)
```

That is `source-abi`, a **transitional** ABI: the C++ module owns the interface
and calls *into* Rust for pieces of behaviour. The port inverts it — Rust owns
the 132-slot interface, and the behaviour comes from `source-filesystem`
directly rather than through `source-abi`'s bridge.

So the question for the next session is not "write a filesystem". It is:

1. Which of the 132 slots does the engine actually call, and with what?
2. How much of each is already answerable from `source-filesystem`?
3. What is the ownership contract on the handle types — `FileHandle_t`,
   `FileFindHandle_t`, `FileNameHandle_t` — and on the four `CUtlBuffer` slots?

## The three hazards, named in advance

**Async I/O is 21 slots and is the largest single group.** `filesystem_async.cpp`
is 1,537 lines and `QueuedLoader.cpp` another 1,978. Whether the engine's hot
paths use them, or whether they can start out answering synchronously the way
the C++ does when its thread pool is absent, is the first thing to measure —
it is potentially a third of the work that does not have to exist on day one.

**`CUtlBuffer` finally has to cross.** Four slots pass it. Every port so far
has got away without reproducing a Valve container, and `soundemittersystem`'s
lesson was that the interface passes far less than the implementation uses. That
lesson runs out here: these four slots are the interface.

**`vpklib` comes too.** It is linked by `filesystem` *and* `dedicated`, so
dropping the C++ `filesystem` without handling `vpklib` breaks the dedicated
server's link line. `source-vpk` already exists, which is the good news; the
waf shape is the `steam_api` problem again — a `cargo_link_library` generator
answering to the name `vpklib`, because two subprojects name it.

## What this slice did not do

No code. No design review. No decision about the crate split. The next session
should build the per-slot ledger — caller, inputs, ownership, lifetime, thread
— before writing anything, because that is what turned `inputsystem` from a
guess into a port that worked first time.
