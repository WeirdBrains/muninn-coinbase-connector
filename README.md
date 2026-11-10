# muninn-coinbase-connector

A [Muninn](https://github.com/WeirdBrains/muninn) ingest connector for Coinbase Exchange
FIX, and the FIX client it is built on.

`crates/coinbase-exchange-fix` is a standalone FIX 5.0 SP2 client. It logs on to the
Exchange order-entry, market-data, and drop-copy gateways, signs the logon, and keeps
the session (heartbeats, test requests, sequence reset, and paged resend). It has no
knowledge of Muninn.

`muninn-coinbase-connector` is the ingest connector. On each run it opens one FIX
session, reads for a bounded interval, and stores each application message as JSON.
When the schedule has a pipeline it calls `pipe`. When the schedule has a rule set it
calls `run_ruleset`. Drop copy and order entry keep the highest inbound sequence and
resume from the next one. Market data starts a fresh session so each pull gets a new
snapshot. Order entry listens for execution reports on that session.

Licensed under the BSD 3-Clause License. Copyright (c) 2026 Weird Brains LLC.
See [LICENSE](LICENSE).

## Build

Install a recent Rust toolchain, then from this repository:

```sh
cargo build --release
```

The plugin is `target/release/libmuninn_coinbase_connector.so`
(`.dylib` on macOS, `.dll` on Windows).

## Install

Muninn loads that library as a prebuilt plugin. Create a source for the raw items,
install the library, and approve it:

```sh
mnnn sources add coinbase
mnnn plugins install target/release/libmuninn_coinbase_connector.so --kind ingest-connector --name coinbase
mnnn spaces approve coinbase
```

## Try a run

Credentials come from the environment, or from schedule options
`api_key`, `api_secret`, and `api_passphrase`.

```sh
export COINBASE_API_KEY=...
export COINBASE_API_SECRET=...
export COINBASE_API_PASSPHRASE=...

mnnn spaces run coinbase --connector-dry-run \
  --option gateway=drop-copy \
  --option environment=sandbox
```

A dry run records `store_raw`, `pipe`, and `run_ruleset` instead of storing, piping,
or running rules. The FIX session itself still connects.

Leave `environment` unset and the connector uses the sandbox. Set
`environment=production` when the schedule should use the production gateways.

## Schedule

```sh
mnnn schedule add coinbase 15m \
  --plugin coinbase \
  --source coinbase \
  --pipeline coinbase \
  --connector-ruleset coinbase \
  --option gateway=drop-copy \
  --option environment=production
```

`--pipeline` and `--connector-ruleset` are optional. If you set them, the connector
calls `pipe` and `run_ruleset` on every successful pull, including a pull that stored
nothing. Raw items stay in the source. `mnnn sources data coinbase` lists them, and
`mnnn sources cat coinbase ITEM` writes one back out.

Each stored item is JSON: `msg_type`, `seq`, `poss_dup`, and `fields` (tag and value).
Heartbeats and the other admin messages are omitted. Signature and passphrase fields
are stored as `***`.

### Options

| Option | Meaning |
|---|---|
| `gateway` | `drop-copy` (default), `market-data`, `market-data-nosnap`, or `order-entry` |
| `environment` | `sandbox` (default) or `production` |
| `venue` | `exchange` (default) or `mica` |
| `symbols` | Comma-separated product ids. Required for market data, for example `BTC-USD,ETH-USD` |
| `depth` | `l2` (default), `l1`, or `l3` |
| `listen_ms` | How long to read after logon. Default `10000`, at most `60000` |
| `max_messages` | Application messages stored this run. Default `2000`, at most `20000` |
| `heartbeat` | Seconds. Default `30` |
| `reset` | `y` starts a new session. `n` resumes. By default, market data resets every run, and drop copy and order entry reset until a sequence cursor exists |
| `api_key`, `api_secret`, `api_passphrase` | Or `COINBASE_API_KEY`, `COINBASE_API_SECRET`, and `COINBASE_API_PASSPHRASE` |

## Release

`release.py` bumps `muninn-coinbase-connector` in `Cargo.toml` (minor by default),
writes the entry in `CHANGELOG.md`, commits the tree with your message, tags
`vMAJOR.MINOR.PATCH`, and pushes the branch and the tag.

```sh
python release.py "Resume drop copy from the stored sequence"
python release.py --patch "Correct the sandbox market-data port"
python release.py --major "First stable connector"
```

`--major` and `--patch` choose the other components. The current branch must already
track a remote, or the script pushes it to `origin` and sets the upstream.

Pushing a `vMAJOR.MINOR.PATCH` tag that matches `Cargo.toml` runs the GitHub Actions
release. It publishes one plugin build for each of these:

| System | x86_64 (Debian calls this amd64) | arm64 (Fedora and Arch call this aarch64) |
|---|---|---|
| Windows | `.msi` and `.zip` for `x86_64` | `.msi` and `.zip` for `arm64` |
| macOS | `.pkg` and archive for `x86_64` | `.pkg` and archive for `arm64` |
| Generic Linux (Ubuntu 22.04, glibc 2.35 or newer) | `linux-x86_64` archive | `linux-arm64` archive |
| Debian and deb-based distros with a new enough libc | `.deb` for `amd64` | `.deb` for `arm64` |
| Fedora | `.rpm` for `x86_64` | `.rpm` for `aarch64` |
| Arch Linux | `.pkg.tar.zst` for `x86_64` | `.pkg.tar.zst` for `aarch64` |

The Windows installer and the macOS package put the library in Muninn's plugin
directory. When `MUNINN_HOME` is set they use `$MUNINN_HOME/code/plugins`.
Otherwise they use the default home, `~/.muninn/code/plugins` (on Windows,
`%USERPROFILE%\.muninn\code\plugins`). The Windows package reads the variable
from the account that runs the installer. The macOS package reads it from the
logged-in user's session, then from that user's shell startup files. Then:

```sh
mnnn plugins install ~/.muninn/code/plugins/libmuninn_coinbase_connector.dylib --kind ingest-connector --name coinbase
```

On Windows the file is `muninn_coinbase_connector.dll` in that folder. The `.zip` and
macOS archive are the same library without an installer. A Debian package installs
`/usr/lib/muninn/libmuninn_coinbase_connector.so`. Fedora uses `/usr/lib64/muninn/`.
Arch uses `/usr/lib/muninn/`. Point `mnnn plugins install` at that file. The Windows
and macOS installers are not code-signed.
