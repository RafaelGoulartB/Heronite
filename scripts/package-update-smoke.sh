#!/usr/bin/env bash
# Installs a package update the way the app does, as root on clean containers
# with the real pkexec and polkit: the update crate's `install_package`
# example calls `Package::install_verified`, which copies the file into a
# root-owned folder, checks it against the verified SHA-256 and runs the
# package manager on the copy. Small test packages (one depends on `jq`, one
# on a package that does not exist) stand in for Heronite's, so no release
# build is needed. Checks on every system:
#   - the package installs, with its dependency from the repositories;
#   - a file changed after its hash was taken is refused and not installed;
#   - a package manager failure is reported;
#   - the private copy in /var/tmp is gone afterwards, every time;
#   - a path with quotes, spaces and `$(...)` runs nothing.
# Needs docker.
#   usage: scripts/package-update-smoke.sh [deb|rpm|arch ...]   (default: all)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORK=$ROOT/target/package-update-smoke
formats=("$@")
[[ ${#formats[@]} -gt 0 ]] || formats=(deb rpm arch)
mkdir -p "$WORK/pkgs" "$WORK/t"

# The example, built on the oldest glibc of the systems below (Ubuntu 22.04
# has 2.35) so the one binary runs on all of them. The image has no clang or
# lld, which .cargo/config.toml links with: its own cc links instead.
echo "== building the install_package example"
docker run --rm -u "$(id -u):$(id -g)" -v "$ROOT:/src" -w /src \
  -e CARGO_HOME=/src/target/package-update-smoke/cargo -e CARGO_TARGET_DIR=/src/target/package-update-smoke/target \
  -e CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=cc -e RUSTFLAGS=-Cdebuginfo=0 \
  rust:1-bullseye cargo build -q --locked --profile quick -p heronite-update --example install_package
cp "$WORK/target/quick/examples/install_package" "$WORK/t/install_package"

# Inside the container, after the format's part below defined KIND, EXT,
# build_pkg <dependency> <file>, is_installed and uninstall.
cat > "$WORK/t/cases.sh" <<'EOF'
set -u
mkdir -p /run/dbus && dbus-daemon --system --fork
polkitd=$(ls /usr/lib/polkit-1/polkitd /usr/libexec/polkit-1/polkitd /usr/lib/polkit/polkitd 2>/dev/null | head -1)
"$polkitd" --no-debug >/dev/null 2>&1 &
for _ in $(seq 50); do [ "$(pkexec /usr/bin/id -un 2>/dev/null)" = root ] && break; sleep 0.2; done
[ "$(pkexec /usr/bin/id -un 2>/dev/null)" = root ] || { echo "  FAIL  pkexec does not work here"; exit 1; }

dir="/tmp/My 'odd' dir; \$(touch /tmp/pwned)"
mkdir -p "$dir"
fail=0
check() { if eval "$2"; then echo "  ok    $1"; else echo "  FAIL  $1"; fail=1; fi; }
copies() { ls -d /var/tmp/heronite-update.* 2>/dev/null | wc -l; }
good="$dir/heronite-test.$EXT" tampered="$dir/heronite-test-tampered.$EXT" broken="$dir/heronite-test-broken.$EXT"
build_pkg jq "$good" && cp "$good" "$tampered" && build_pkg no-such-package-xyz "$broken" || { echo "  FAIL  building the test packages"; exit 1; }

out=$(/t/install_package "$KIND" "$good")
check "installs with the verified hash"                '[ "$out" = "RESULT: Ok" ] && is_installed'
check "the package's file is on disk"                  '[ "$(cat /usr/share/heronite-test/hello 2>/dev/null)" = hi ]'
check "its dependency came from the repositories"      'command -v jq >/dev/null'
check "the private copy is gone"                       '[ "$(copies)" = 0 ]'
check "the file's path ran nothing"                    '[ ! -e /tmp/pwned ]'
uninstall >/dev/null 2>&1

out=$(/t/install_package "$KIND" "$tampered" --tamper)
check "a file changed after its hash is refused"       'echo "$out" | grep -q "changed after it was verified" && ! is_installed'
check "the private copy is gone"                       '[ "$(copies)" = 0 ]'

out=$(/t/install_package "$KIND" "$broken")
check "a package manager failure is reported"          'echo "$out" | grep -q "^RESULT: Err(The package manager failed"'
check "the private copy is gone"                       '[ "$(copies)" = 0 ]'
exit $fail
EOF

cat > "$WORK/t/deb.sh" <<'EOF'
KIND=deb EXT=deb
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq >/dev/null
apt-get install -y -qq pkexec polkitd dbus >/dev/null 2>&1 || apt-get install -y -qq policykit-1 dbus >/dev/null
build_pkg() {
  rm -rf /b && mkdir -p /b/DEBIAN /b/usr/share/heronite-test && echo hi > /b/usr/share/heronite-test/hello
  printf 'Package: heronite-test\nVersion: 1.0\nArchitecture: all\nMaintainer: test <test@example.com>\nDepends: %s\nDescription: test\n' "$1" > /b/DEBIAN/control
  dpkg-deb --build /b "$2" >/dev/null
}
is_installed() { dpkg -s heronite-test 2>/dev/null | grep -q "Status: install ok installed"; }
uninstall() { apt-get remove -y heronite-test; }
. /t/cases.sh
EOF

cat > "$WORK/t/rpm.sh" <<'EOF'
KIND=rpm EXT=rpm
if command -v dnf >/dev/null; then dnf install -y -q polkit dbus-daemon >/dev/null 2>&1
else zypper -q --non-interactive install -y pkexec polkit dbus-1-daemon >/dev/null; fi
build_pkg() { cp "/pkgs/$1.rpm" "$2"; }
is_installed() { rpm -q heronite-test >/dev/null 2>&1; }
uninstall() { rpm -e heronite-test; }
. /t/cases.sh
EOF

cat > "$WORK/t/arch.sh" <<'EOF'
KIND=arch EXT=pkg.tar.zst
pacman -Sy --noconfirm --needed polkit dbus >/dev/null 2>&1
build_pkg() {
  rm -rf /b && mkdir -p /b/usr/share/heronite-test && echo hi > /b/usr/share/heronite-test/hello
  printf 'pkgname = heronite-test\npkgbase = heronite-test\npkgver = 1.0-1\npkgdesc = test\narch = any\nsize = 3\ndepend = %s\n' "$1" > /b/.PKGINFO
  (cd /b && bsdtar --zstd -cf "$2" .PKGINFO usr)
}
is_installed() { pacman -Q heronite-test >/dev/null 2>&1; }
uninstall() { pacman -R --noconfirm heronite-test; }
. /t/cases.sh
EOF

# The two test RPMs (noarch), built once for Fedora and openSUSE.
cat > "$WORK/t/rpms.sh" <<'EOF'
set -e
dnf install -y -q rpm-build >/dev/null 2>&1
mkdir -p ~/rpmbuild/SPECS
for dep in jq no-such-package-xyz; do
  printf '%s\n' 'Name: heronite-test' 'Version: 1.0' 'Release: 1' 'Summary: test' 'License: MIT' 'BuildArch: noarch' \
    "Requires: $dep" '%description' 'test' '%install' \
    'mkdir -p %{buildroot}/usr/share/heronite-test && echo hi > %{buildroot}/usr/share/heronite-test/hello' \
    '%files' '/usr/share/heronite-test/hello' > ~/rpmbuild/SPECS/test.spec
  rpmbuild -bb ~/rpmbuild/SPECS/test.spec >/dev/null 2>&1
  cp ~/rpmbuild/RPMS/noarch/heronite-test-1.0-1.noarch.rpm "/pkgs/$dep.rpm"
done
chown -R "$OWNER" /pkgs
EOF

failed=()
run() {
  local image=$1 format=$2
  echo "== $image ($format)"
  docker run --rm -v "$WORK/t:/t:ro" -v "$WORK/pkgs:/pkgs:ro" "$image" bash "/t/$format.sh" || failed+=("$image")
}

for format in "${formats[@]}"; do
  case $format in
    deb)
      for image in ubuntu:22.04 debian:12 ubuntu:24.04; do run "$image" deb; done
      ;;
    rpm)
      docker run --rm -e OWNER="$(id -u):$(id -g)" -v "$WORK/t:/t:ro" -v "$WORK/pkgs:/pkgs" fedora:41 bash /t/rpms.sh
      for image in fedora:41 fedora:latest opensuse/tumbleweed; do run "$image" rpm; done
      ;;
    arch)
      run archlinux:latest arch
      ;;
    *) echo "unknown format: $format" >&2; exit 2 ;;
  esac
done

if [[ ${#failed[@]} -gt 0 ]]; then
  echo "failed on: ${failed[*]}" >&2
  exit 1
fi
echo "all passed"
