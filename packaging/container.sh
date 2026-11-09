#!/bin/sh
# Build the plugin inside a Debian, Fedora, or Arch container and write one
# package under /work/dist. KIND is deb, rpm, or arch. ARCH is the distro's
# architecture name (amd64, arm64, x86_64, or aarch64).
set -eu

: "${VERSION:?}"
: "${KIND:?}"
: "${ARCH:?}"

cd /work
mkdir -p /work/dist

install_rust() {
  curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs -o /tmp/rustup-init.sh
  sh /tmp/rustup-init.sh -y --profile minimal --default-toolchain stable
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
}

build_plugin() {
  cargo test --workspace --locked --release
  cargo build --locked --release -p muninn-coinbase-connector
  strip --strip-unneeded target/release/libmuninn_coinbase_connector.so
}

case "$KIND" in
  deb)
    apt-get update
    apt-get install -y --no-install-recommends \
      build-essential ca-certificates curl pkg-config dpkg-dev
    install_rust
    build_plugin
    stage=$(mktemp -d)
    mkdir -p "$stage/DEBIAN" "$stage/usr/lib/muninn" \
      "$stage/usr/share/doc/muninn-coinbase-connector"
    install -m 0755 target/release/libmuninn_coinbase_connector.so \
      "$stage/usr/lib/muninn/libmuninn_coinbase_connector.so"
    install -m 0644 LICENSE \
      "$stage/usr/share/doc/muninn-coinbase-connector/copyright"
    depends=$(dpkg-shlibdeps -O "$stage/usr/lib/muninn/libmuninn_coinbase_connector.so" \
      | sed 's/^shlibs:Depends=//')
    cat > "$stage/DEBIAN/control" <<EOF
Package: muninn-coinbase-connector
Version: ${VERSION}
Section: net
Priority: optional
Architecture: ${ARCH}
Maintainer: Weird Brains LLC <weirdbrains@users.noreply.github.com>
Homepage: https://github.com/WeirdBrains/muninn-coinbase-connector
Description: Muninn ingest connector for Coinbase Exchange FIX
 Coinbase Exchange FIX client and the Muninn plugin that stores each
 application message as JSON.
Depends: ${depends}
EOF
    dpkg-deb --root-owner-group --build "$stage" \
      "/work/dist/muninn-coinbase-connector_${VERSION}_${ARCH}.deb"
    ;;
  rpm)
    dnf install -y gcc make rpm-build curl ca-certificates pkgconf-pkg-config binutils
    host=$(uname -m)
    if [ "$host" != "$ARCH" ]; then
      echo "container is ${host}, expected ${ARCH}" >&2
      exit 1
    fi
    install_rust
    build_plugin
    top=/tmp/rpmbuild
    mkdir -p "$top/BUILD" "$top/RPMS" "$top/SOURCES" "$top/SPECS" "$top/SRPMS"
    cat > "$top/SPECS/muninn-coinbase-connector.spec" <<EOF
%global debug_package %{nil}
%global _build_id_links none

Name:           muninn-coinbase-connector
Version:        ${VERSION}
Release:        1
Summary:        Muninn ingest connector for Coinbase Exchange FIX
License:        BSD-3-Clause
URL:            https://github.com/WeirdBrains/muninn-coinbase-connector

%description
Coinbase Exchange FIX client and the Muninn plugin that stores each
application message as JSON.

%install
mkdir -p %{buildroot}%{_libdir}/muninn
install -m 0755 /work/target/release/libmuninn_coinbase_connector.so %{buildroot}%{_libdir}/muninn/libmuninn_coinbase_connector.so
mkdir -p %{buildroot}%{_datadir}/licenses/%{name}
install -m 0644 /work/LICENSE %{buildroot}%{_datadir}/licenses/%{name}/LICENSE

%files
%{_libdir}/muninn/libmuninn_coinbase_connector.so
%{_datadir}/licenses/%{name}/LICENSE
EOF
    rpmbuild --define "_topdir $top" -bb "$top/SPECS/muninn-coinbase-connector.spec"
    set -- "$top/RPMS/${ARCH}/"*.rpm
    if [ "$#" -ne 1 ]; then
      echo "expected one rpm, found $#:" "$@" >&2
      exit 1
    fi
    cp "$1" "/work/dist/muninn-coinbase-connector-${VERSION}-fedora-${ARCH}.rpm"
    ;;
  arch)
    pacman-key --init
    pacman-key --populate archlinux
    pacman -Syu --noconfirm --needed base-devel curl ca-certificates pkgconf binutils
    host=$(uname -m)
    if [ "$host" != "$ARCH" ]; then
      echo "container is ${host}, expected ${ARCH}" >&2
      exit 1
    fi
    install_rust
    build_plugin
    id builder >/dev/null 2>&1 || useradd --create-home builder
    work=/tmp/pkg
    mkdir -p "$work"
    cat > "$work/PKGBUILD" <<EOF
pkgname=muninn-coinbase-connector
pkgver=${VERSION}
pkgrel=1
pkgdesc='Muninn ingest connector for Coinbase Exchange FIX'
arch=('${ARCH}')
url='https://github.com/WeirdBrains/muninn-coinbase-connector'
license=('BSD-3-Clause')
options=('!strip' '!debug')

package() {
  install -Dm755 /work/target/release/libmuninn_coinbase_connector.so \\
    "\$pkgdir/usr/lib/muninn/libmuninn_coinbase_connector.so"
  install -Dm644 /work/LICENSE \\
    "\$pkgdir/usr/share/licenses/\$pkgname/LICENSE"
}
EOF
    chown -R builder:builder "$work"
    su builder -s /bin/bash -c "cd '$work' && makepkg --noconfirm --skipinteg --nodeps --nocheck"
    set -- "$work"/muninn-coinbase-connector-"${VERSION}"-1-"${ARCH}".pkg.tar.*
    if [ "$#" -ne 1 ]; then
      echo "expected one Arch package, found $#:" "$@" >&2
      exit 1
    fi
    cp "$1" /work/dist/
    ;;
  *)
    echo "unknown KIND ${KIND}" >&2
    exit 1
    ;;
esac

chmod -R a+rX /work/dist
ls -l /work/dist
