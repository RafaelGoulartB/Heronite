# Fedora COPR

`heronite.spec` builds Heronite for the COPR
[`rafaelgoulartb/heronite`](https://copr.fedorainfracloud.org/coprs/rafaelgoulartb/heronite/):

```bash
sudo dnf copr enable rafaelgoulartb/heronite
sudo dnf install heronite
```

It is not the release's `.rpm` (made by `scripts/packages.sh` with nfpm): COPR
builds it from source on its own builders, for each Fedora release, and dnf
updates it from there. Its `share/heronite/package-format` says `copr`, so the
app tells that a new version is out and how to update, but never downloads
nor installs the release's file. A version that does not know that marker
(up to 1.1.0) takes the copy for one it does not manage: it neither checks
nor installs by itself.

## What the spec does

- **Source:** the GitHub tarball of the tag `v<Version>`. `scripts/bump-version.py`
  sets `Version` (and adds a `%changelog` entry) with the rest of the release,
  so the spec on `main` always points at the latest tag.
- **Build:** `cargo build --release --locked`, with the crates downloaded by
  cargo (the project needs "Enable internet access during builds"). Fedora's
  build flags are off: its `RUSTFLAGS` would replace `.cargo/config.toml`'s
  (clang and lld) and add full debug info to the fat-LTO build. Fedora's own
  `rust` is enough (the newest dependency needs 1.92).
- **Packages:** `heronite`, and `heronite-epic-login` for the Epic Games login
  window, so WebKitGTK stays optional (`heronite` recommends it; without it
  the app offers the browser login).

## Setting up the project

On <https://copr.fedorainfracloud.org> (a Fedora account logs in):

1. **New project** `heronite`: the supported Fedora releases (and rawhide) on
   `x86_64`, and **Enable internet access during builds** on.
2. **Packages → New package → SCM**:
   - Clone URL: `https://github.com/RafaelGoulartB/Heronite`
   - Committish: `main` (the spec there points at the latest release tag)
   - Spec file: `packaging/copr/heronite.spec`
   - SRPM build method: `make_srpm` (runs [`.copr/Makefile`](../../.copr/Makefile):
     downloads the tarball and makes the source RPM)
3. **Rebuild** the package for the first build. A failed build's logs are on
   its page (`builder-live.log` for the build itself).

## Testing it locally

In a container, as COPR does it (Docker or Podman; nothing on the host):

```bash
docker run --rm -it -v "$PWD":/src:ro fedora:44 bash -c '
  dnf -y install make && cd /src &&
  make -f .copr/Makefile srpm outdir=/tmp/out spec=/src/packaging/copr/heronite.spec &&
  dnf -y builddep /tmp/out/heronite-*.src.rpm &&
  rpmbuild --rebuild /tmp/out/heronite-*.src.rpm'
```

## Fedora's atomic desktops

On Silverblue, Kinoite or Bazzite the Flatpak is the better fit. The COPR
package works there too: put the project's `.repo` file (the link on the
project's page for the Fedora release) in `/etc/yum.repos.d/`, then
`rpm-ostree install heronite` and reboot. The app then says to update with
`rpm-ostree upgrade`.
