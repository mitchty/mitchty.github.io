{
  description = "mitchty.github.io flake";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";

    crane.url = "github:ipetkov/crane";

    flake-utils.url = "github:numtide/flake-utils";

    fenix.url = "github:nix-community/fenix";
    treefmt-nix.url = "github:numtide/treefmt-nix";

    git-hooks = {
      url = "github:cachix/git-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    advisory-db = {
      url = "github:rustsec/advisory-db";
      flake = false;
    };
  };

  outputs =
    { self, ... }@inputs:
    inputs.flake-utils.lib.eachDefaultSystem (
      system:
      let
        # DRY some of the meta definitions for apps/packages for this chungus amungus
        metaCommon = desc: {
          description = if desc == "" then "mitchty" else "mitchty " + desc;
          mainProgram = "mitchty";
        };

        stableRust = (
          inputs.fenix.packages.${system}.stable.withComponents [
            "cargo"
            "clippy"
            "llvm-tools"
            "rustc"
            "rust-src"
            "rustfmt"
            "rust-analyzer"
          ]
        );

        pkgs = import inputs.nixpkgs {
          inherit system;

          overlays = [
            inputs.fenix.overlays.default
            (self: super: {
              apple-sdk-test = super.apple-sdk;
            })
          ];
        };

        pkgsWasm = import inputs.nixpkgs {
          inherit system;
          overlays = [ inputs.fenix.overlays.default ];
        };

        pkgsDarwin =
          if pkgs.stdenv.isDarwin then
            import inputs.nixpkgs {
              inherit system;
              overlays = [ inputs.fenix.overlays.default ];
              # Use the host platform to get system-only linking
              crossSystem = pkgs.stdenv.hostPlatform;
            }
          else
            null;

        pkgsWindows = import inputs.nixpkgs {
          inherit system;
          overlays = [ inputs.fenix.overlays.default ];
          crossSystem = {
            config = "x86_64-w64-mingw32";
            libc = "msvcrt";
          };
        };

        inherit (pkgs) lib;

        # Build wasm-bindgen-cli at the version used by Bevy
        wasmBindgenCli = pkgsWasm.rustPlatform.buildRustPackage rec {
          pname = "wasm-bindgen-cli";
          version = "0.2.106";

          src = pkgsWasm.fetchCrate {
            inherit pname version;
            hash = "sha256-M6WuGl7EruNopHZbqBpucu4RWz44/MSdv6f0zkYw+44=";
          };

          cargoHash = "sha256-ElDatyOwdKwHg3bNH/1pcxKI7LXkhsotlDPQjiLHBwA=";

          nativeBuildInputs = [ pkgsWasm.pkg-config ];

          buildInputs =
            with pkgsWasm;
            [ openssl ]
            ++ lib.optionals stdenv.hostPlatform.isDarwin [
              apple-sdk
            ];

          checkFlags = [
            # flaky test
            "--skip=reference::tests::works"
          ];

          meta = with lib; {
            description = "CLI tool for wasm-bindgen";
            mainProgram = "wasm-bindgen";
          };
        };

        craneLib = inputs.crane.mkLib pkgs;

        craneLibWasm = (inputs.crane.mkLib pkgsWasm).overrideToolchain (
          p:
          p.fenix.combine [
            p.fenix.stable.rustc
            p.fenix.stable.cargo
            p.fenix.targets.wasm32-unknown-unknown.stable.rust-std
          ]
        );

        # Crane lib for Darwin builds that only link system libraries
        craneLibDarwin =
          if pkgs.stdenv.isDarwin then
            (inputs.crane.mkLib pkgsDarwin).overrideToolchain (
              p:
              p.fenix.combine [
                p.fenix.stable.rustc
                p.fenix.stable.cargo
                p.fenix.stable.rust-std
              ]
            )
          else
            null;

        craneLibWindows = (inputs.crane.mkLib pkgsWindows).overrideToolchain (
          p:
          p.fenix.combine [
            p.fenix.stable.rustc
            p.fenix.stable.cargo
            p.fenix.targets.x86_64-pc-windows-gnu.stable.rust-std
          ]
        );

        # Constrained src fileset to ensure that cargo deps aren't rebuilt every
        # change to crates.
        #
        # Mostly just here to be sure that build.rs using tonic notices proto
        # files and anything that affects dependencies for cargo directly.
        # Also includes assets for bevy_embedded_assets build script.
        srcDeps = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.lock
            ./Cargo.toml
            (lib.fileset.fileFilter (file: file.hasExt "toml") ./crates)
          ];
        };

        # All the junk in the trunk not used for cache dep validation
        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            (lib.fileset.fileFilter (file: file.hasExt "rs") ./crates)
            (lib.fileset.fileFilter (file: file.hasExt "toml") ./crates)
            ./Cargo.toml
            ./Cargo.lock
          ];
        };

        treefmtEval = inputs.treefmt-nix.lib.evalModule pkgs {
          projectRootFile = "flake.nix";
          programs = {
            # Because I keep on forgetting, the rfc style formatter is the
            # default for at least year now.. ref:
            # https://github.com/numtide/treefmt-nix/blob/main/programs/nixfmt-rfc-style.nix
            nixfmt.enable = true;
            rustfmt = {
              enable = true;
              edition = "2024";
            };
            taplo.enable = true;
          };

        };

        # TOO MANY DAM LAYERS OF SHENANIGANS
        #
        # So... because the hooks are their own derivation, need to be sure crap
        # like treefmt has all the formatters it needs in its derivation PATH
        # too.
        #
        # These tools are made available in the hook environment's PATH
        #
        # These things are common between the hook derivation setup and used for the devShell
        hookTools = with pkgs; {
          inherit
            # Formatters needed by treefmt
            taplo
            nixfmt-rfc-style
            rustfmt
            # Build tools needed by nix flake check
            git
            # Nix itself for running checks
            nix
            # treefmt itself
            treefmt
            ;
        };

        # Instead of running nix flake check on each commit (e.g. in
        # pre-commit), lets just be sure we're golden at push time.
        #
        # I can rewrite the commit history to fix it at that point if things
        # fail or not.
        #
        # TODO: Need to brain on how to do this with syncing my git clone around
        # on separate machines, the pre-hook gets data that points to the local
        # git store. Thinking I'll need to have mutagen not sync .git/hooknames
        # directly for now until I get yeet working
        # git-hooks-check = inputs.git-hooks.lib.${system}.run {
        #   src = ./.;
        #   tools = hookTools;
        #   hooks = {
        #     nix-flake-check = {
        #       enable = true;
        #       name = "nix-flake-check";
        #       entry = "${pkgs.nix}/bin/nix flake check -L";
        #       language = "system";
        #       pass_filenames = false;
        #       stages = [ "pre-push" ];
        #     };
        #     # Make sure code is formatted in pre-commit
        #     # Note: We use the formatter check separately, so we disable this
        #     # in the git-hooks check to avoid sandbox timestamp issues
        #     treefmt.enable = false;
        #   };
        # };

        # Common arguments can be set here to avoid repeating them later
        commonArgs = {
          inherit src;
          strictDeps = true;

          nativeBuildInputs = with pkgs; [
            git
            pkg-config
          ];

          BEVY_ASSET_PATH = ./crates/mitchty/src/assets;

          buildInputs =
            with pkgs;
            [ ]
            ++ lib.optionals pkgs.stdenv.hostPlatform.isLinux [
              udev
              alsa-lib
              vulkan-loader
              xorg.libX11
              xorg.libXcursor
              xorg.libXi
              xorg.libXrandr
              libxkbcommon
              wayland
              pkgs.mold-wrapped
              pkgs.lld
            ]
            ++ lib.optionals pkgs.stdenv.isDarwin [
              apple-sdk
              rustPlatform.bindgenHook
            ];

          # Additional environment variables can be set directly
          LD_LIBRARY_PATH = lib.optionalString pkgs.stdenv.isLinux (
            lib.makeLibraryPath (
              with pkgs;
              [
                vulkan-loader
                xorg.libX11
                xorg.libXcursor
                xorg.libXi
                xorg.libXrandr
                libxkbcommon
                wayland
                alsa-lib
                udev
              ]
            )
          );
        };

        commonArgsWasm = {
          inherit src;
          strictDeps = true;

          nativeBuildInputs = with pkgsWasm; [
            git
            wasm-bindgen-cli
            binaryen
          ];

          buildInputs = [ ];

          BEVY_ASSET_PATH = ./crates/mitchty/src/assets;
          CARGO_BUILD_TARGET = "wasm32-unknown-unknown";
        };

        # Common arguments for Darwin builds (system libraries only)
        commonArgsDarwin =
          if pkgs.stdenv.isDarwin then
            {
              inherit src;
              strictDeps = true;

              nativeBuildInputs = [ pkgsDarwin.git ];

              buildInputs = with pkgsDarwin; [
                apple-sdk
                libiconv
              ];
            }
          else
            { };

        commonArgsWindows =
          let
            buildPlatformSuffix = lib.strings.toLower pkgs.pkgsBuildHost.stdenv.hostPlatform.rust.cargoEnvVarTarget;
          in
          {
            inherit src;
            strictDeps = true;

            nativeBuildInputs = with pkgs; [
              git
              buildPackages.nasm
              buildPackages.cmake
            ];

            buildInputs = with pkgsWindows.windows; [ pthreads ];

            CARGO_BUILD_TARGET = "x86_64-pc-windows-gnu";
            CFLAGS = "-Wno-stringop-overflow -Wno-array-bounds -Wno-restrict";
            CFLAGS_x86_64-pc-windows-gnu = "-I${pkgsWindows.windows.pthreads}/include";
            "CC_${buildPlatformSuffix}" = "cc";
            "CXX_${buildPlatformSuffix}" = "c++";
          };

        # Build *just* the cargo dependencies (of the entire workspace),
        # so we can reuse all of that work (e.g. via cachix) when running in CI
        # It is *highly* recommended to use something like cargo-hakari to avoid
        # cache misses when building individual top-level-crates
        # Note: buildDepsOnly already uses --all-targets by default
        # Important: Must use same env vars (especially RUSTFLAGS) as actual builds
        # Using dev profile by default for better debug info on panics
        cargoArtifacts = craneLib.buildDepsOnly (
          commonArgs
          // nixEnvArgs
          // devArgs
          // {
            src = srcDeps;
          }
        );

        # Cargo artifacts for release builds
        cargoArtifactsRelease = craneLib.buildDepsOnly (
          commonArgs
          // nixEnvArgs
          // releaseArgs
          // {
            src = srcDeps;
          }
        );

        # Cargo artifacts for WASM builds (release)
        cargoArtifactsWasm = craneLibWasm.buildDepsOnly (
          commonArgsWasm
          // nixEnvArgs
          // releaseArgs
          // {
            src = srcDeps;
          }
        );

        # Cargo artifacts for WASM builds (debug)
        cargoArtifactsWasmDebug = craneLibWasm.buildDepsOnly (
          commonArgsWasm
          // nixEnvArgs
          // devArgs
          // {
            src = srcDeps;
          }
        );

        # Cargo artifacts for Darwin builds (release)
        cargoArtifactsDarwin =
          if pkgs.stdenv.isDarwin then
            craneLibDarwin.buildDepsOnly (
              commonArgsDarwin
              // releaseArgs
              // {
                src = srcDeps;
              }
            )
          else
            null;

        cargoArtifactsWindows = craneLibWindows.buildDepsOnly (
          commonArgsWindows
          // releaseArgs
          // {
            src = srcDeps;
          }
        );

        version = self.rev or self.dirtyShortRev or "nix-flake-cant-get-git-commit-sha";

        individualCrateArgs = commonArgs // {
          inherit cargoArtifacts;
          #          inherit (craneLib.crateNameFromCargoToml { inherit src; }) version;
          # NB: we disable tests since we'll run them all via cargo-nextest
          doCheck = false;
        };

        fileSetForCrate =
          crate:
          lib.fileset.toSource {
            root = ./.;
            fileset = lib.fileset.unions [
              ./Cargo.toml
              ./Cargo.lock
              (craneLib.fileset.commonCargoSources crate)
              (lib.fileset.fileFilter (file: file.hasExt "rs") ./crates/mitchty/src)
              (lib.fileset.maybeMissing ./crates/${crate}/Cargo.toml)
              (lib.fileset.fileFilter (file: file.hasExt "ktx2") ./.)
            ];
          };

        webServerRuntimeInputs = [
          pkgs.python3
        ]
        ++ lib.optionals pkgs.stdenv.isDarwin [ pkgs.darwin.system_cmds ]
        ++ lib.optionals pkgs.stdenv.isLinux [ pkgs.xdg-utils ];

        # sh fragment to open a url on macos/linux, used in the web/web-release
        # apps for dev testing of wasm builds.
        openBrowserScript = ''
          URL="http://localhost:8000"
          if command -v open >/dev/null 2>&1; then
            echo "open $URL"
            open "$URL" || :
          elif command -v xdg-open >/dev/null 2>&1; then
            echo "xdg-open $URL"
            xdg-open "$URL" || :
          else
            echo "unsure how to open a browser programmatically on this os."
            echo "in your browser of choice manually open: $URL"
          fi
        '';

        # Function to cover building local testing web app scripts for
        # debug/release builds in web/web-release apps.
        mkWebServerApp =
          name: wasmPackage: includeAssets:
          let
            buildType = if includeAssets then " debug build" else " release build";
            assetCopyScript =
              if includeAssets then
                ''
                  echo "copying assets for debug build"
                  cp -rv ${./crates/mitchty/src/assets} "$TMPDIR/assets"
                  chmod -R u+w "$TMPDIR/assets"
                ''
              else
                ''
                  # nop
                '';
          in
          pkgs.writeShellApplication {
            inherit name;
            runtimeInputs = webServerRuntimeInputs;
            text = ''
              set -e
              # Create temporary directory for serving
              TMPDIR=$(mktemp -d)
              trap 'rm -rf "$TMPDIR"' EXIT TERM INT QUIT

              echo "building wasm files for${buildType}"
              mkdir -p "$TMPDIR/wasm"
              cp -rv ${wasmPackage}/wasm/* "$TMPDIR/wasm/"
              cp ${./index.html} "$TMPDIR/index.html"

              ${assetCopyScript}

              echo "starting local webserver at http://localhost:8000"
              echo "press Ctrl+C to stop"

              cd "$TMPDIR"

              # Start server in background
              python3 -m http.server 8000 &
              SERVER_PID=$!
              trap 'kill $SERVER_PID 2>/dev/null || true; rm -rf "$TMPDIR"' EXIT

              # Give the server a bit to start before trying to open a browser
              # to it.
              sleep 1

              ${openBrowserScript}

              # Block on the python webserver.
              wait $SERVER_PID
            '';
          };

        nixEnvArgs = {
          STUPIDNIXFLAKEHACK = version;
          # Clippy lints can be set in source via attributes instead
        };

        devArgs = {
          CARGO_PROFILE = "dev";
        };

        releaseArgs = {
          CARGO_PROFILE = "release";
          RUSTFLAGS = "-D warnings";
        };

        # Build the top-level crates of the workspace as individual derivations.
        # This allows consumers to only depend on (and build) only what they need.
        # Though it is possible to build the entire workspace as a single derivation,
        # so this is left up to you on how to organize things
        #
        # Note that the cargo workspace must define `workspace.members` using wildcards,
        # otherwise, omitting a crate (like we do below) will result in errors since
        # cargo won't be able to find the sources for all members.

        # Default build: dev profile with debug symbols to match cargo parlance
        mitchty = craneLib.buildPackage (
          individualCrateArgs
          // nixEnvArgs
          // devArgs
          // {
            pname = "mitchty";
            cargoExtraArgs = "-p mitchty";
            src = fileSetForCrate ./crates/mitchty;
          }
        );

        # Optimized LTO build with release profile
        mitchty-lto = craneLib.buildPackage (
          commonArgs
          // nixEnvArgs
          // releaseArgs
          // {
            pname = "mitchty";
            cargoArtifacts = cargoArtifactsRelease;
            cargoExtraArgs = "-p mitchty";
            src = fileSetForCrate ./crates/mitchty;
            doCheck = false;
          }
        );

        mitchty-wasm-lto =
          let
            wasmBuild = craneLibWasm.buildPackage (
              commonArgsWasm
              // nixEnvArgs
              // releaseArgs
              // {
                pname = "mitchty-wasm-lto";
                version = version;
                cargoArtifacts = cargoArtifactsWasm;
                cargoExtraArgs = "-p mitchty";
                src = fileSetForCrate ./crates/mitchty;

                STUPIDNIXFLAKEHACK = version;

                # Don't run checks for WASM builds
                doCheck = false;

                # Don't install binaries - we'll handle WASM files specially
                doInstallCargoArtifacts = false;
                installPhase = ''
                  runHook preInstall
                  mkdir -p $out
                  cp -r target/wasm32-unknown-unknown/release $out/
                  runHook postInstall
                '';
              }
            );
          in
          pkgsWasm.runCommand "mitchty-wasm-lto-bindgen"
            {
              nativeBuildInputs = [
                wasmBindgenCli
                pkgsWasm.binaryen
              ];
            }
            ''
              mkdir -p $out/wasm

              # Run wasm-bindgen on the built WASM file
              ${wasmBindgenCli}/bin/wasm-bindgen \
                --out-dir $out/wasm \
                --target web \
                --no-typescript \
                ${wasmBuild}/release/mitchty.wasm

              # Optimize with wasm-opt (enable all features needed by Bevy)
              ${pkgsWasm.binaryen}/bin/wasm-opt -Oz \
                --enable-bulk-memory \
                --enable-mutable-globals \
                --enable-nontrapping-float-to-int \
                --enable-sign-ext \
                --enable-simd \
                -o $out/wasm/mitchty_bg_optimized.wasm \
                $out/wasm/mitchty_bg.wasm

              mv $out/wasm/mitchty_bg_optimized.wasm $out/wasm/mitchty_bg.wasm
            '';

        mitchty-wasm =
          let
            wasmBuild = craneLibWasm.buildPackage (
              commonArgsWasm
              // nixEnvArgs
              // devArgs
              // {
                pname = "mitchty-wasm";
                version = version;
                cargoArtifacts = cargoArtifactsWasmDebug;
                cargoExtraArgs = "-p mitchty";
                src = fileSetForCrate ./crates/mitchty;

                STUPIDNIXFLAKEHACK = version;

                # Don't run checks for WASM builds
                doCheck = false;

                # Don't install binaries - we'll handle WASM files specially
                doInstallCargoArtifacts = false;
                installPhase = ''
                  runHook preInstall
                  mkdir -p $out
                  cp -r target/wasm32-unknown-unknown/debug $out/
                  runHook postInstall
                '';
              }
            );
          in
          pkgsWasm.runCommand "mitchty-wasm-bindgen"
            {
              nativeBuildInputs = [
                wasmBindgenCli
                pkgsWasm.binaryen
              ];
            }
            ''
              mkdir -p $out/wasm

              # Run wasm-bindgen on the built WASM file
              ${wasmBindgenCli}/bin/wasm-bindgen \
                --out-dir $out/wasm \
                --target web \
                --no-typescript \
                --debug \
                --keep-debug \
                ${wasmBuild}/debug/mitchty.wasm

              # Skip wasm-opt for debug builds to preserve debug info
              # The WASM file is already usable from wasm-bindgen
            '';

        # Darwin release build (system libraries only, portable)
        mitchty-release-darwin =
          if pkgs.stdenv.isDarwin then
            craneLibDarwin.buildPackage (
              commonArgsDarwin
              // releaseArgs
              // {
                pname = "mitchty-release";
                version = version;
                cargoArtifacts = cargoArtifactsDarwin;
                cargoExtraArgs = "-p mitchty";
                src = fileSetForCrate ./crates/mitchty;

                STUPIDNIXFLAKEHACK = version;

                # Don't check during cross-compilation
                doCheck = false;

                meta = metaCommon "release apple silicon build" // {
                  platforms = [
                    "x86_64-darwin"
                    "aarch64-darwin"
                  ];
                };
              }
            )
          else
            null;

        mitchty-release-windows = craneLibWindows.buildPackage (
          commonArgsWindows
          // releaseArgs
          // {
            pname = "mitchty-release";
            version = version;
            cargoArtifacts = cargoArtifactsWindows;
            cargoExtraArgs = "-p mitchty";
            src = fileSetForCrate ./crates/mitchty;

            STUPIDNIXFLAKEHACK = version;

            # Don't check during cross-compilation
            doCheck = false;

            meta = metaCommon "release windows x86_64 build";
          }
        );
      in
      {
        checks = {
          formatter = treefmtEval.config.build.check self;
          # TODO: see above comment
          # git-hooks = git-hooks-check;

          # Run clippy (and deny all warnings) on the workspace source,
          # again, reusing the dependency artifacts from above.
          #
          # Note that this is done as a separate derivation so that
          # we can block the CI if there are issues here, but not
          # prevent downstream consumers from building our crate by itself.
          mitchty-clippy = craneLib.cargoClippy (
            commonArgs
            // nixEnvArgs
            // devArgs
            // {
              inherit cargoArtifacts;
              cargoClippyExtraArgs = "--all-targets -- --deny warnings";
            }
          );

          mitchty-doc = craneLib.cargoDoc (
            commonArgs
            // nixEnvArgs
            // devArgs
            // {
              inherit cargoArtifacts;
              # This can be commented out or tweaked as necessary, e.g. set to
              # `--deny rustdoc::broken-intra-doc-links` to only enforce that lint
              env.RUSTDOCFLAGS = "--deny warnings";
            }
          );

          # Audit dependencies
          # 2025-12-16 commented out cause deps of deps are inactive and not sure how I want to handle that right now
          # mitchty-audit = craneLib.cargoAudit {
          #   inherit src;
          #   inherit (inputs) advisory-db;
          # };

          # Run tests with cargo-nextest
          # Consider setting `doCheck = false` on other crate derivations
          # if you do not want the tests to run twice
          mitchty-nextest = craneLib.cargoNextest (
            commonArgs
            // nixEnvArgs
            // devArgs
            // {
              inherit cargoArtifacts;
              partitions = 1;
              partitionType = "count";
              cargoNextestPartitionsExtraArgs = "--no-tests=pass";
            }
          );
        };

        packages = {
          inherit
            mitchty
            mitchty-lto
            mitchty-wasm
            mitchty-wasm-lto
            ;
          default = mitchty;
          # Expose checks as packages for individual running with shorter names
          clippy = self.checks.${system}.mitchty-clippy;
          doc = self.checks.${system}.mitchty-doc;
          nextest = self.checks.${system}.mitchty-nextest;
        }
        // lib.optionalAttrs pkgs.stdenv.isLinux {
          inherit mitchty-release-windows;
        }
        // lib.optionalAttrs pkgs.stdenv.isDarwin {
          mitchty-release = mitchty-release-darwin;
        };

        apps = {
          mitchty =
            (inputs.flake-utils.lib.mkApp {
              drv = mitchty;
            })
            // {
              meta = metaCommon "Dev build";
            };
          mitchty-lto =
            (inputs.flake-utils.lib.mkApp {
              drv = mitchty-lto;
            })
            // {
              meta = metaCommon "LTO optimized build";
            };
          default = self.apps.${system}.mitchty;
          build-all =
            let
              all-targets =
                pkgs.runCommand "mitchty-build-all"
                  {
                    buildInputs = [
                      mitchty
                      mitchty-lto
                      mitchty-wasm
                      mitchty-wasm-lto
                    ]
                    ++ lib.optionals pkgs.stdenv.isLinux [
                      mitchty-release-windows
                    ];
                  }
                  ''
                    mkdir -p $out/bin
                    cat > $out/bin/mitchty-build-all <<'EOF'
                    #!/bin/sh
                    echo ok
                    EOF
                    chmod +x $out/bin/mitchty-build-all
                  '';
            in
            {
              type = "app";
              program = "${all-targets}/bin/mitchty-build-all";
              meta = {
                description = "Build all mitchty targets in parallel";
                mainProgram = "mitchty-build-all";
              };
            };
          # Makes updating everything at once a bit easier.
          # nix run .#update
          update = {
            type = "app";
            program = "${
              pkgs.writeShellApplication {
                name = "update";
                # runtimeInputs = [
                #   pkgs.nix
                #   pkgs.jq
                # ];
                text = ''
                  set -e
                  nix flake update
                  cargo update --verbose
                  cargo upgrade --verbose
                '';
              }
            }/bin/update";
            meta = {
              description = "Update flake inputs and cargo dependencies";
              mainProgram = "update";
            };
          };
          # Serve WASM build locally for testing
          # nix run .#web
          web = {
            type = "app";
            program = "${mkWebServerApp "web" mitchty-wasm true}/bin/web";
            meta = {
              description = "Serve WASM build locally for testing";
              mainProgram = "web";
            };
          };
          # Serve WASM LTO build locally for testing
          # nix run .#web-lto
          web-lto = {
            type = "app";
            program = "${mkWebServerApp "web-lto" mitchty-wasm-lto false}/bin/web-lto";
            meta = {
              description = "Serve WASM LTO optimized build";
              mainProgram = "web-lto";
            };
          };
        }
        # Note its a bit jank but I'm using mitchty-release for github action build
        # targets, -release in this parlance isn't cargo build --release its
        # "build release binaries for a commit/tag/version"
        // lib.optionalAttrs pkgs.stdenv.isLinux {
          mitchty-release-windows =
            (inputs.flake-utils.lib.mkApp {
              drv = mitchty-release-windows;
            })
            // {
              # TODO: is this mitchty.exe as the main program? Maybe I can test
              # this out via wine?
              meta = metaCommon "run release cross compiled windows build";
            };
        }
        // lib.optionalAttrs pkgs.stdenv.isDarwin {
          mitchty-release =
            (inputs.flake-utils.lib.mkApp {
              drv = mitchty-release-darwin;
            })
            // {
              meta = metaCommon "run release portable macos build" // {
                platforms = [
                  "x86_64-darwin"
                  "aarch64-darwin"
                ];
              };
            };
        };

        devShells.default = craneLib.devShell {
          checks = self.checks.${system};

          packages = (
            with pkgs;
            [
              act
              adrs
              cargo-bloat
              cargo-edit
              cargo-outdated
              cargo-unused-features
              gitFull
              nil
              pandoc
              stableRust
              wasm-bindgen-cli
              binaryen
              wasm-pack
            ]
            ++ (lib.attrValues hookTools)
            ++ commonArgs.buildInputs
            ++ commonArgs.nativeBuildInputs
          );

          # TODO: once hook syncing is working re-enable
          # shellHook = ''
          #   ${git-hooks-check.shellHook}
          # '';

          # Make sure eglot+etc.. pick the right rust-src for eglot+lsp mode stuff using direnv
          RUST_SRC_PATH = "${stableRust}/lib/rustlib/src/rust/library";

          # Set library path for Bevy
          LD_LIBRARY_PATH = commonArgs.LD_LIBRARY_PATH;
        };
      }
    );
}
