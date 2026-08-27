{
  description = "Textchum — a native text editor with language servers and tree-sitter highlighting";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};

        # The four libraries the GTK shell links against. WebKitGTK is
        # the 6.0 API series (the GTK 4 one), which nixpkgs calls
        # webkitgtk_6_0 to distinguish it from the GTK 3 build.
        guiLibraries = with pkgs; [
          gtk4
          libadwaita
          gtksourceview5
          webkitgtk_6_0
          glib
          openssl
        ];
      in
      {
        packages.textchum = pkgs.rustPlatform.buildRustPackage {
          pname = "textchum";
          version = "0.0.10";
          src = self;

          # linux/ is its own cargo workspace but depends on crates in
          # core/ by relative path, so the whole repository has to be
          # the source and the build is aimed at the subdirectory.
          cargoRoot = "linux";
          buildAndTestSubdir = "linux";
          cargoLock.lockFile = ./linux/Cargo.lock;

          nativeBuildInputs = with pkgs; [
            pkg-config
            # Wraps the binary so GTK finds its schemas, icon themes and
            # pixbuf loaders. Without it the app starts and then dies on
            # a missing GSettings schema.
            wrapGAppsHook4
          ];
          buildInputs = guiLibraries;

          # The suite that matters lives in core/, which this build does
          # not compile; `make check` runs it. Building linux/'s tests
          # here would only re-run what CI already does, slowly.
          doCheck = false;

          postInstall = ''
            install -Dm644 linux/data/to.perri.textchum.desktop \
              $out/share/applications/to.perri.textchum.desktop
            install -Dm644 packaging/to.perri.textchum.metainfo.xml \
              $out/share/metainfo/to.perri.textchum.metainfo.xml
            install -Dm644 linux/data/to.perri.textchum-512.png \
              $out/share/icons/hicolor/512x512/apps/to.perri.textchum.png
            install -Dm755 scripts/chum $out/bin/chum
          '';

          meta = with pkgs.lib; {
            description = "A native text editor with language servers and tree-sitter highlighting";
            longDescription = ''
              A text editor in the spirit of TextMate: native, fast, and
              focused on editing. One portable core in Rust owns every
              document; this is its GTK 4 and libadwaita shell.

              Language servers and formatters are deliberately not
              dependencies: the editor runs the ones already on your
              PATH, so add them to your own environment.
            '';
            homepage = "https://github.com/perrito666/textchum";
            license = licenses.mit;
            mainProgram = "textchum-gtk";
            platforms = platforms.linux;
          };
        };

        packages.default = self.packages.${system}.textchum;

        apps.default = flake-utils.lib.mkApp {
          drv = self.packages.${system}.textchum;
          name = "textchum-gtk";
        };

        # `nix develop` — everything needed to build both the core and
        # the shell, plus the optional tools the editor shells out to,
        # so spell check and the ctags fallback work inside the shell.
        devShells.default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
            cargo
            rustc
            rustfmt
            clippy
            pkg-config
            wrapGAppsHook4
          ];
          buildInputs = guiLibraries ++ (with pkgs; [
            hunspell
            hunspellDicts.en_US
            universal-ctags
            git
          ]);
        };
      });
}
