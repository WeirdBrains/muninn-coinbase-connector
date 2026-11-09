amd64 is Debian's name for x86_64. aarch64 is the Fedora and Arch name for arm64.
The generic Linux archive is built on Ubuntu 22.04 and loads on systems with glibc 2.35 or newer.
Debian, Fedora, and Arch packages are built on those distros.

| Asset | Install the library from |
|---|---|
| `linux-x86_64` and `linux-arm64` archives | the `.so` in the archive |
| `.deb` | `/usr/lib/muninn/libmuninn_coinbase_connector.so` |
| Fedora `.rpm` | `/usr/lib64/muninn/libmuninn_coinbase_connector.so` |
| Arch package | `/usr/lib/muninn/libmuninn_coinbase_connector.so` |
| Windows `.msi` | `C:\Program Files\Muninn Coinbase Connector\muninn_coinbase_connector.dll` |
| Windows `.zip` | `muninn_coinbase_connector.dll` in the archive |
| macOS `.pkg` | `/usr/local/lib/muninn/libmuninn_coinbase_connector.dylib` |
| macOS archive | `libmuninn_coinbase_connector.dylib` in the archive |

The Windows and macOS installers are not code-signed.

`mnnn plugins install PATH --kind ingest-connector --name coinbase`
