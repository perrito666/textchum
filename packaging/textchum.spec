# RPM spec for Textchum.
#
# Built from a source tarball of the repository:
#
#   packaging/build-rpm.sh [version]
#
# The library names here are the Fedora ones, which differ from
# Debian's for every single dependency — the same four libraries are
# `gtk4`, `libadwaita`, `gtksourceview5` and `webkitgtk6.0` rather than
# `libgtk-4-1`, `libadwaita-1-0`, `libgtksourceview-5-0` and
# `libwebkitgtk-6.0-4`. openSUSE names them differently again, which is
# why the auto-generated `Requires` below is left to do the real work:
# rpmbuild reads the binary's ELF dependencies and asks for the
# sonames, which every distribution resolves correctly on its own.

# Fedora defines this; rpm on Debian and older distributions does not,
# and an undefined macro is installed verbatim — a directory literally
# named %%{_metainfodir}.
%{!?_metainfodir: %global _metainfodir %{_datadir}/metainfo}

Name:           textchum
Version:        %{version}
Release:        1%{?dist}
Summary:        A native text editor with language servers and highlighting

License:        MIT
URL:            https://perrito666.github.io/textchum/
Source0:        %{name}-%{version}.tar.gz

BuildRequires:  cargo
BuildRequires:  rust
BuildRequires:  gcc
BuildRequires:  pkgconfig(gtk4)
BuildRequires:  pkgconfig(libadwaita-1)
BuildRequires:  pkgconfig(gtksourceview-5)
BuildRequires:  pkgconfig(webkitgtk-6.0)
BuildRequires:  desktop-file-utils
BuildRequires:  libappstream-glib

# Each of these switches off one feature and nothing else, so none of
# them is a hard requirement: hunspell for prose spell check, ctags for
# the Jump to Definition fallback, git for the forge-URL action.
Recommends:     hunspell
Recommends:     hunspell-en
Suggests:       ctags
Suggests:       git-core

%description
A text editor in the spirit of TextMate: native, fast, and focused on
editing. One portable core in Rust owns every document; this is its
GTK 4 and libadwaita shell.

Tree-sitter highlighting, language servers for diagnostics, completion
and navigation, save preprocessors that run your own formatters, fuzzy
file opening, project-wide regular-expression search, a live Markdown
preview, and prose spell check that reads comments and leaves
identifiers alone.

Language servers and formatters are the ones you installed and are
deliberately not dependencies of this package: the editor finds them on
your PATH.

%prep
%autosetup

%build
cargo build --release --manifest-path linux/Cargo.toml

%install
install -Dm755 linux/target/release/textchum-gtk \
    %{buildroot}%{_bindir}/textchum-gtk
install -Dm755 scripts/chum %{buildroot}%{_bindir}/chum
install -Dm644 linux/data/to.perri.textchum.desktop \
    %{buildroot}%{_datadir}/applications/to.perri.textchum.desktop
install -Dm644 packaging/to.perri.textchum.metainfo.xml \
    %{buildroot}%{_metainfodir}/to.perri.textchum.metainfo.xml
install -Dm644 linux/data/to.perri.textchum-512.png \
    %{buildroot}%{_datadir}/icons/hicolor/512x512/apps/to.perri.textchum.png

%check
# Guarded so the spec still builds where these are absent — a --nodeps
# build on a non-RPM distribution, say. BuildRequires guarantees them
# on a real one, where the checks do run.
command -v desktop-file-validate >/dev/null && \
    desktop-file-validate %{buildroot}%{_datadir}/applications/to.perri.textchum.desktop
command -v appstream-util >/dev/null && \
    appstream-util validate-relax --nonet \
        %{buildroot}%{_metainfodir}/to.perri.textchum.metainfo.xml
true

%files
%license LICENSE
%doc README.md
%{_bindir}/textchum-gtk
%{_bindir}/chum
%{_datadir}/applications/to.perri.textchum.desktop
%{_metainfodir}/to.perri.textchum.metainfo.xml
%{_datadir}/icons/hicolor/512x512/apps/to.perri.textchum.png

%changelog
* Thu Aug 27 2026 Horacio Duran <https://perri.to> - 0.0.10-1
- Linux field reports answered: Ctrl+Q quits, Ctrl+Shift+T reopens a
  closed tab, several spell-check dictionaries at once, and spelling
  corrections in the context menu.
