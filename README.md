# iroh ping

[![Documentation](https://img.shields.io/badge/docs-latest-blue.svg?style=flat-square)](https://docs.rs/iroh-ping/)
[![Crates.io](https://img.shields.io/crates/v/iroh-ping.svg?style=flat-square)](https://crates.io/crates/iroh-ping)
[![downloads](https://img.shields.io/crates/d/iroh-ping.svg?style=flat-square)](https://crates.io/crates/iroh-ping)
[![Chat](https://img.shields.io/discord/1161119546170687619?logo=discord&style=flat-square)](https://discord.com/invite/DpmJgtU7cW)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg?style=flat-square)](LICENSE-MIT)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg?style=flat-square)](LICENSE-APACHE)
[![CI](https://img.shields.io/github/actions/workflow/status/n0-computer/iroh-ping/ci.yaml?branch=main&style=flat-square&label=CI)](https://github.com/n0-computer/iroh-ping/actions/workflows/ci.yaml)

A very simple iroh protocol for pinging a remote node. It's a high level example & easy starting point for new projects.

Walk through the quickstart example in the [Documentation Website](https://docs.iroh.computer/quickstart).

## Running the Examples

### Quickstart Example

This example demonstrates basic ping functionality between two endpoints.

First, start the receiver in one terminal:
```sh
cargo run --example quickstart receiver
```

The receiver will print a ticket. Copy this ticket, then in another terminal run:
```sh
cargo run --example quickstart sender <TICKET>
```

Replace `<TICKET>` with the ticket printed by the receiver.

### Measuring the data rate

Pass `--flood` to the sender to follow the ping with a stream of bytes, printing the data rate every second until you stop it with Ctrl+C:

```sh
cargo run --example quickstart sender --flood <TICKET>
```

### Optional: push metrics to iroh-services

If the `IROH_SERVICES_API_SECRET` environment variable is set, the receiver will register with [iroh-services](https://services.iroh.computer) and push endpoint metrics in the background. If it isn't set, the example prints a notice and continues without it.

```sh
export IROH_SERVICES_API_SECRET=<your-api-key>
cargo run --example quickstart receiver
```

Get a free API key at [services.iroh.computer](https://services.iroh.computer).

## Dev and prod swarm demo

Launch a whole demo in one process, with a separate endpoint, identity, services
client, and metrics for every simulated device. This example uses the local
`../iroh-services/iroh-services` crate; keep that sibling checkout available.

```sh
cp swarm.toml swarm.local.toml
export IROH_SERVICES_API_SECRET='<your-project-api-key>'
cargo run --release --example swarm -- swarm.local.toml
```

Edit `swarm.local.toml` to set the relay URLs and device counts. The default starts
three devices in group `dev` with `app_version = "2.0.0-dev"`. All five production
devices use group `prod`: two have `app_version = "1.4.0"`, and three have
`app_version = "1.5.0"`. Production devices also have `environment = "prod"`. The version is a dashboard attribute; all devices run
the same demo binary. Add more `[[swarms]]` entries for other groups.

Every device automatically runs network diagnostics and uploads a report on each
startup. It also grants the project's services endpoint permission to request
fresh diagnostics later and accepts those requests through `ClientHost`.
Startup reports run concurrently across devices; swarm traffic starts once all
reports have uploaded. A failed grant or upload stops startup with the device's
name and error. The services project must allow network diagnostics.

Each swarm joins its own stable iroh-gossip topic. The two production swarms
remain separate even though they share the dashboard group `prod`. The
launcher gives each node the first node's address to bootstrap; gossip discovers
and maintains the remaining neighbors. There is no fixed ring or traffic between
swarms. The number of live neighbors is managed by gossip, rather than a forced
all-to-all connection count.

Every device repeatedly broadcasts about 1 KiB through gossip, then pings and
floods one of its live gossip neighbors, rotating through them between rounds.
Gossip events keep processing during transfers. Large flood payloads still use
the ping/flood protocol; gossip carries the broadcasts and manages membership.

Shared settings apply to every client:

- `api_key_env`: environment variable containing the project's API key. An
  optional inline `api_key` takes precedence. `swarm.local.toml` is gitignored.
- `identity_dir`: directory holding saved device keys, default `.swarm-identities`,
  relative to the config file. The default directory is gitignored.
- `relays`: custom relay URLs, authenticated with the project key. An empty list
  uses the public n0 relays. Iroh can still upgrade connections to direct paths,
  especially with all devices on the same machine; this is not relay-only load.
- `metrics_interval_secs`: reporting interval for endpoint, gossip, and ping/flood metrics.
- `duration_secs`: runtime after all devices are ready; `0` runs until Ctrl+C.

Per swarm, configure `name`, `group`, `nodes` (at least two), `attributes`,
`flood_secs`, and `pause_secs`. Floods send as fast as the connection permits;
shorter floods and longer pauses reduce the duty cycle. Device names include the
swarm, an index, and an identity suffix. Identities are reused across restarts,
including when groups or version attributes change. Keep each internal swarm
`name` stable (`prod-a` and `prod-b` label the two independent gossip swarms).
Increasing `nodes` creates additional identities; reducing it preserves unused
keys for later. Use a different `identity_dir` for a separate demo.

The console prints device IDs, connections, and cumulative traffic totals. In
your services project, filter by group `dev` or `prod` and compare the `app_version`
attribute. Startup verifies services connectivity and metadata registration;
failures stop the demo with an error. Ctrl+C also works during startup and
flushes metrics before closing the endpoints. Failed transfers retry after the
configured pause (at least 100 ms).

Validate the config without credentials or network connections:

```sh
cargo run --example swarm -- swarm.local.toml --check
```

If startup reports `Unauthorized: Oneshot recv error`, the services authentication
RPC ended without a reply. This happens before metadata or swarm traffic starts;
the label alone does not identify the reason. Check the backend authentication
log for the device ID and verify that the API key belongs to the current project.
The API key contains the services endpoint address; custom relay URLs only set
the relays used by devices. For client diagnostics, run with
`RUST_LOG=info,iroh_services=debug`.

## This is not the "real" ping

Iroh has all sorts of internal ping-type messages, this is a high level demo of a protocol, and in no way necessary for iroh's normal operation.

## License

Copyright 2026 N0, INC.

This project is licensed under either of

 * Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or
   http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or
   http://opensource.org/licenses/MIT)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this project by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
