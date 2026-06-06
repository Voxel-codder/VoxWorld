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
- `web-client/` - browser/WASM client porting surface
- `web-gateway/` - HTTP static server, browser play session host, and raw WebSocket-to-TCP gateway
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

## Railway Web Deployment

For Railway deployment, keep the root directory set to `/` so Cargo can see the
full workspace and assets. This branch includes `railway.toml`, which builds the
native server, the web gateway, and the WASM browser shell.

Railway will use:

```sh
bash scripts/railway-build.sh
```

and then:

```sh
bash scripts/railway-start.sh
```

Recommended variables:

```text
RUST_LOG=info,common::net=info
RAILPACK_RUST_VERSION=nightly-2025-09-08
RAILPACK_BUILD_APT_PACKAGES=mold
VOXWORLD_USERDATA=/data/userdata
VOXWORLD_MAX_PLAYERS=100
VOXWORLD_WEB_MAX_SESSIONS=100
VELOREN_GIT_VERSION=/0/0
```

Use a persistent Railway volume mounted at `/data` if you want server state,
configuration, and player data to survive redeploys.

The public Railway HTTP domain serves the browser client and upgrades `/play`
connections to the web gateway. The gateway starts a headless native client
session for each browser connection, then relays browser JSON input into the
native game client. `/ws` remains available as a raw WebSocket-to-TCP proxy for
lower-level transport experiments.

The gateway also exposes `/api/status` so the web client can show whether the
native server is reachable and how many players are online. Railway uses
`/api/health` for deploy health checks so a deployment only becomes healthy once
the gateway can reach the native game server.

The server default player cap is 100, and Railway also sets
`VOXWORLD_MAX_PLAYERS=100` unless you override it. The web gateway defaults its
active `/play` session limit to the same value, or to
`VOXWORLD_WEB_MAX_SESSIONS` when that variable is set.

## License

Vox World is distributed under the GNU General Public License v3.0 or later.
See `LICENSE` for the full license text and retained copyright notices.
