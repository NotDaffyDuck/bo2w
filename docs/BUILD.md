# Build prerequisites

You need Rust stable through rustup (`rust-toolchain.toml` fixes the version), a
C toolchain, and the system libraries Bevy links. bo2mp turns on both the `x11`
and `wayland` features (see `Cargo.toml`), so the Wayland headers are required.
Gamepad input and rumble (`bevy_gilrs`) link libudev, which means the udev
headers (`systemd-devel` or `libudev-dev`) are required as well.

```bash
# Fedora
sudo dnf install gcc-c++ pkgconf-pkg-config libX11-devel alsa-lib-devel systemd-devel wayland-devel libxkbcommon-devel

# Debian / Ubuntu
sudo apt install g++ pkg-config libx11-dev libasound2-dev libudev-dev libwayland-dev libxkbcommon-dev

# Arch / Manjaro
sudo pacman -S gcc pkgconf libx11 alsa-lib systemd wayland libxkbcommon libxcursor libxrandr libxi
```

For other distributions and GPU Vulkan drivers, see Bevy's
[`linux_dependencies.md`](https://github.com/bevyengine/bevy/blob/main/docs/linux_dependencies.md).

## What a native Linux build does not need

`clang`, `llvm`, `lld` and `glibc-static` are unnecessary; plain `gcc-c++` links
the game. Only the Windows cross build wants `llvm` (for the `llvm-lib` tool):

```bash
sudo dnf install llvm        # Fedora: provides llvm-lib
```

Then, once, `make setup-windows` adds the Windows Rust target and cargo-xwin.

## macOS

Install the Xcode command line tools (`xcode-select --install`) and rustup, and
that is all: winit, wgpu (Metal), CoreAudio and gilrs use system frameworks, and
the `x11` / `wayland` features compile to nothing. The menu and
`make map mp_raid` have been run on Apple silicon (M2 Max, Metal). Pipelined
rendering defaults to off there; `bootstrap/src/app_stack.rs` explains why
(`BO2MP_PIPELINED_RENDERING` overrides it).

## The game files

The game reads a Black Ops II install: point `BO2MP_GAMES` at the folder that
holds it (the Windows depot of a Steam copy works on any platform).

Then follow "Install" and "Run / Play" in `README.md`: copy `.env.example`, set
`BO2MP_GAMES`, and run `make map mp_raid`. The portable Windows folder is
described in [`WINDOWS.md`](WINDOWS.md).
