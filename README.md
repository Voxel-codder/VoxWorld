# Vox World

Vox World is a multiplayer voxel RPG set in a vast fantasy world.

The project focuses on open-world exploration, cooperative survival, real-time
combat, crafting, settlements, and persistent multiplayer server play. The code
base is written primarily in Rust and is designed to run dedicated game servers
as well as native clients.

## Status

Vox World is in active development. Expect rough edges, rapid iteration, and
occasional compatibility changes between builds.

## Repository Layout

- `server-cli/` - dedicated server command-line entry point
- `server/` - game server systems and persistence
- `client/` - client-side game logic
- `voxygen/` - native graphical client
- `common/` - shared game data, networking, ECS, and utilities
- `assets/` - game data, localization, audio, models, and metadata
- `world/` - world generation and simulation

## Building

Install Rust with the toolchain specified in `rust-toolchain`, then build the
server:

```powershell
cargo build --release -p voxworld-server-cli --locked
```

The first build can take a long time because the workspace is large.

## Running A Server

Run the dedicated server locally:

```powershell
.\target\release\voxworld-server-cli.exe --non-interactive --no-auth
```

On Linux:

```sh
./target/release/voxworld-server-cli --non-interactive --no-auth
```

The default game server port is `14004/tcp`. Server configuration and save data
are stored under the Vox World userdata directory.

## Railway Notes

For Railway deployment, keep the root directory set to `/` so Cargo can see the
full workspace and assets.

Recommended build command:

```sh
cargo build --release -p voxworld-server-cli --locked
```

Recommended start command:

```sh
./target/release/voxworld-server-cli --non-interactive --no-auth
```

Recommended variables:

```text
RUST_LOG=info,common::net=info
RAILPACK_RUST_VERSION=nightly-2025-09-08
RAILPACK_BUILD_APT_PACKAGES=mold
VOXWORLD_USERDATA=/data/userdata
```

Use a persistent Railway volume mounted at `/data` if you want server state,
configuration, and player data to survive redeploys.

## License

Vox World is distributed under the GNU General Public License v3.0 or later.
See `LICENSE` for the full license text and retained copyright notices.
