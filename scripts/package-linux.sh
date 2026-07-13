#!/usr/bin/env bash
# Build Debian and RPM packages from the released x86-64 GNU archive.
set -euo pipefail

if [ "$#" -ne 3 ]; then
  echo "Usage: $0 VERSION GNU_ARCHIVE OUTPUT_DIRECTORY" >&2
  exit 2
fi

version="${1#v}"
archive="$2"
output="$3"
release_epoch="${SOURCE_DATE_EPOCH:-946684800}"
repo_root="$(cd "$(dirname "$0")/.." && pwd)"
archive="$(cd "$(dirname "$archive")" && pwd)/$(basename "$archive")"
mkdir -p "$output"
output="$(cd "$output" && pwd)"

for command in tar dpkg-deb dpkg-shlibdeps rpmbuild rpm; do
  command -v "$command" >/dev/null || {
    echo "ERROR: required command '$command' is unavailable" >&2
    exit 1
  }
done

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
tar -xzf "$archive" -C "$work"
release_root="$work/ctx-v${version}-x86_64-unknown-linux-gnu"
binary="$release_root/ctx"
if [ ! -x "$binary" ]; then
  echo "ERROR: archive does not contain executable $release_root/ctx" >&2
  exit 1
fi

# dpkg-shlibdeps reads the built ELF and maps its actual NEEDED entries to
# Debian packages. This avoids stale, manually maintained runtime dependencies.
mkdir -p "$work/debian"
maintainer="${DEB_MAINTAINER:-$(sed -n 's/^authors = \["\(.*\)"\]/\1/p' "$repo_root/Cargo.toml")}"
if [ -z "$maintainer" ]; then
  echo "ERROR: set DEB_MAINTAINER or configure a Cargo package author" >&2
  exit 1
fi
sed -e "s/@VERSION@/$version/g" \
    -e "s|@MAINTAINER@|$maintainer|g" \
    -e "s/@DEPENDS@//g" \
    "$repo_root/packaging/deb/control.template" > "$work/debian/control"
depends="$(cd "$work" && dpkg-shlibdeps -O "$binary" | sed -n 's/^shlibs:Depends=//p')"
if [ -z "$depends" ]; then
  echo "ERROR: dpkg-shlibdeps did not determine runtime dependencies" >&2
  exit 1
fi

debroot="$work/debroot"
install -Dpm 0755 "$binary" "$debroot/usr/bin/ctx"
install -Dpm 0644 "$release_root/LICENSE-MIT" "$debroot/usr/share/doc/ctx/LICENSE-MIT"
install -Dpm 0644 "$release_root/LICENSE-APACHE" "$debroot/usr/share/doc/ctx/LICENSE-APACHE"
install -d "$debroot/DEBIAN"
sed -e "s/@VERSION@/$version/g" \
    -e "s|@MAINTAINER@|$maintainer|g" \
    -e "s|@DEPENDS@|$depends|g" \
    "$repo_root/packaging/deb/control.template" > "$debroot/DEBIAN/control"
find "$debroot" -exec touch -h -d "@$release_epoch" {} +
SOURCE_DATE_EPOCH="$release_epoch" \
  dpkg-deb --root-owner-group --build "$debroot" "$output/ctx_${version}_amd64.deb"

# rpmbuild's dependency generator likewise inspects the ELF, producing exact
# shared-library requirements for the RPM ecosystem.
topdir="$work/rpmbuild"
mkdir -p "$topdir"/{BUILD,BUILDROOT,RPMS,SOURCES,SPECS,SRPMS}
install -m 0755 "$binary" "$topdir/SOURCES/ctx"
install -m 0644 "$release_root/LICENSE-MIT" "$release_root/LICENSE-APACHE" "$topdir/SOURCES/"
cp "$repo_root/packaging/rpm/ctx.spec" "$topdir/SPECS/"
SOURCE_DATE_EPOCH="$release_epoch" rpmbuild -bb \
  --define "_topdir $topdir" \
  --define "ctx_version $version" \
  --define "_build_id_links none" \
  --define "_buildhost github-actions" \
  --define "use_source_date_epoch_as_buildtime 1" \
  --define "clamp_mtime_to_source_date_epoch 1" \
  "$topdir/SPECS/ctx.spec"
rpm_path="$(find "$topdir/RPMS" -type f -name '*.rpm' -print -quit)"
if [ -z "$rpm_path" ]; then
  echo "ERROR: rpmbuild did not create an RPM" >&2
  exit 1
fi
cp "$rpm_path" "$output/ctx-${version}-1.x86_64.rpm"

dpkg-deb --info "$output/ctx_${version}_amd64.deb"
dpkg-deb --contents "$output/ctx_${version}_amd64.deb"
rpm -qip "$output/ctx-${version}-1.x86_64.rpm"
rpm -qlp "$output/ctx-${version}-1.x86_64.rpm"
