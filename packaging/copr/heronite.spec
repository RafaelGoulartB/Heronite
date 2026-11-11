# Fedora COPR rafaelgoulartb/heronite (see README.md beside this file). Built
# from the release tag's source on COPR's builders, with internet access on:
# cargo downloads the crates of Cargo.lock. Not the release's .rpm
# (scripts/packages.sh): dnf updates this one, so it writes the `copr` marker
# and the app only announces new versions. scripts/bump-version.py keeps
# Version in step with Cargo.toml.

# Cargo strips the release binaries (profile.release.strip): no debuginfo.
%global debug_package %{nil}
# Fedora's RUSTFLAGS would replace .cargo/config.toml's (clang + lld) and add
# full debuginfo to a fat-LTO build: build with the project's own settings.
# (Undefined, not 0: rpm only checks that it is defined.)
%undefine _auto_set_build_flags

Name:           heronite
Version:        1.6.1
Release:        1%{?dist}
Summary:        Fast native game launcher for Steam, Epic, GOG, emulators and more

License:        GPL-3.0-only
URL:            https://heronite.app
Source0:        https://github.com/RafaelGoulartB/Heronite/archive/refs/tags/v%{version}.tar.gz#/%{name}-%{version}.tar.gz

ExclusiveArch:  x86_64

BuildRequires:  rust >= 1.92
BuildRequires:  cargo
BuildRequires:  gcc
BuildRequires:  clang
BuildRequires:  clang-devel
BuildRequires:  lld
BuildRequires:  cmake
BuildRequires:  pkgconfig
BuildRequires:  pkgconfig(xkbcommon)
BuildRequires:  pkgconfig(xkbcommon-x11)
BuildRequires:  pkgconfig(xcb)
BuildRequires:  pkgconfig(x11-xcb)
BuildRequires:  pkgconfig(wayland-client)
BuildRequires:  pkgconfig(libudev)
BuildRequires:  pkgconfig(fontconfig)
BuildRequires:  pkgconfig(freetype2)
BuildRequires:  pkgconfig(gtk+-3.0)
BuildRequires:  pkgconfig(webkit2gtk-4.1)
BuildRequires:  pkgconfig(libsoup-3.0)
BuildRequires:  desktop-file-utils
BuildRequires:  libappstream-glib

# rpm finds the linked libraries (glibc, libxcb, libxkbcommon, libudev...) by
# itself. These are loaded at run time, so it cannot see them.
Requires:       libvulkan.so.1()(64bit)
Requires:       libwayland-client.so.0()(64bit)
Requires:       libfontconfig.so.1()(64bit)
Requires:       hicolor-icon-theme
# The Epic Games login window; without it the app offers the browser login.
Recommends:     %{name}-epic-login%{?_isa} = %{version}-%{release}
Recommends:     mesa-vulkan-drivers
Recommends:     xdg-utils
# Runs games in the sandbox; without it the sandbox says so.
Recommends:     bubblewrap

%description
Every game you own in one Collection: Steam, Epic Games, GOG, Amazon Games,
Battle.net, EA app, Ubisoft Connect, Rockstar Games, emulated ROMs, and
native Linux and Windows games, launched natively, with Wine or Proton, or
through an emulator. Records every session, with play time statistics and a
controller-first console mode.

%package epic-login
Summary:        Epic Games login window for Heronite
Requires:       %{name}%{?_isa} = %{version}-%{release}

%description epic-login
The WebKitGTK window Heronite opens to log in to Epic Games. Without it,
Heronite offers the login in the web browser instead. A package of its own
so that WebKitGTK stays optional.

%prep
%autosetup -n Heronite-%{version}

%build
export CARGO_HOME="$PWD/.cargo-home"
cargo build --release --locked -p heronite -p heronite-epic-login

%install
install -Dm0755 target/release/heronite-rs %{buildroot}%{_bindir}/heronite-rs
# Found next to heronite-rs by the Epic page.
install -Dm0755 target/release/heronite-epic-login %{buildroot}%{_bindir}/heronite-epic-login
install -Dm0644 packaging/linux/app.heronite.Heronite.desktop \
    %{buildroot}%{_datadir}/applications/app.heronite.Heronite.desktop
install -Dm0644 packaging/linux/app.heronite.Heronite.png \
    %{buildroot}%{_datadir}/icons/hicolor/256x256/apps/app.heronite.Heronite.png
install -Dm0644 packaging/linux/app.heronite.Heronite.metainfo.xml \
    %{buildroot}%{_metainfodir}/app.heronite.Heronite.metainfo.xml
# Tells the app that dnf updates it (Package::MARKER in crates/update).
install -dm0755 %{buildroot}%{_datadir}/heronite
echo copr > %{buildroot}%{_datadir}/heronite/package-format

%check
desktop-file-validate %{buildroot}%{_datadir}/applications/app.heronite.Heronite.desktop
appstream-util validate-relax --nonet %{buildroot}%{_metainfodir}/app.heronite.Heronite.metainfo.xml

%files
%license LICENSE licenses/
%doc THIRD-PARTY-NOTICES.md
%{_bindir}/heronite-rs
%{_datadir}/applications/app.heronite.Heronite.desktop
%{_datadir}/icons/hicolor/256x256/apps/app.heronite.Heronite.png
%{_metainfodir}/app.heronite.Heronite.metainfo.xml
%{_datadir}/heronite/

%files epic-login
%{_bindir}/heronite-epic-login

%changelog
* Wed Nov 11 2026 RafaGoulartB <rafagoulartb@gmail.com> - 1.6.1-1
- Update to 1.6.1

* Wed Nov 11 2026 RafaGoulartB <rafagoulartb@gmail.com> - 1.6.0-1
- Update to 1.6.0

* Mon Nov 09 2026 RafaGoulartB <rafagoulartb@gmail.com> - 1.5.0-1
- Update to 1.5.0

* Sun Nov 08 2026 RafaGoulartB <rafagoulartb@gmail.com> - 1.4.0-1
- Update to 1.4.0

* Sat Nov 07 2026 RafaGoulartB <rafagoulartb@gmail.com> - 1.3.0-1
- Update to 1.3.0

* Sat Nov 07 2026 RafaGoulartB <rafagoulartb@gmail.com> - 1.2.0-1
- Update to 1.2.0

* Thu Nov 05 2026 RafaGoulartB <rafagoulartb@gmail.com> - 1.1.1-1
- Update to 1.1.1

* Thu Nov 05 2026 RafaGoulartB <rafagoulartb@gmail.com> - 1.1.0-1
- First COPR package
