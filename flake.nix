{
  description = "mitchty.github.io flake";

  outputs =
    { self, ... }@inputs:
    inputs.flake-utils.lib.eachDefaultSystem (
      system:
      let
        githubRepo = "mitchty/mitchty.github.io";

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
            inputs.mitchty.overlays.cargo-unused-features
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

        # For wine/steam and testing windows on linux.
        pkgsUnfree = import inputs.nixpkgs {
          inherit system;
          config.allowUnfree = true;
        };

        # CUDA-enabled nixpkgs varietal, mostly for the ma binary for training.
        pkgsCuda =
          if pkgs.stdenv.isLinux then
            import inputs.nixpkgs {
              inherit system;
              overlays = [ inputs.fenix.overlays.default ];
              config = {
                allowUnfree = true;
                cudaSupport = true;
              };
            }
          else
            null;

        inherit (pkgs) lib;

        # Build wasm-bindgen-cli at the version used by Bevy for wasm builds
        wasmBindgenCli = pkgsWasm.rustPlatform.buildRustPackage rec {
          pname = "wasm-bindgen-cli";
          version = "0.2.122";

          src = pkgsWasm.fetchCrate {
            inherit pname version;
            hash = "sha256-vO4RSxi/sMWxmsEs3GuljdMfIRSu75A+Q+c5wgYToRU=";
          };

          cargoHash = "sha256-Inup6vvJSG5ghNyeDPyZbfZo4d0LsMG2OJfStoaeDBs=";

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

        craneLib = (inputs.crane.mkLib pkgs).overrideToolchain (_: stableRust);

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

        # Crane lib for CUDA builds Linux only obvs
        craneLibCuda =
          if pkgs.stdenv.isLinux then
            (inputs.crane.mkLib pkgsCuda).overrideToolchain (
              p:
              p.fenix.combine [
                p.fenix.stable.rustc
                p.fenix.stable.cargo
                p.fenix.stable.rust-std
              ]
            )
          else
            null;

        # Constrained src fileset to ensure that cargo deps aren't rebuilt every
        # change to files that don't contribute to dependency chains. Only
        # Cargo.lock, Cargo.toml files, and build.rs are needed here for cargo
        # dependency rebuild detection.
        srcDeps = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.lock
            ./Cargo.toml
            (lib.fileset.fileFilter (file: file.name == "Cargo.toml") ./crates)
            (lib.fileset.fileFilter (file: file.name == "build.rs") ./crates)
          ];
        };

        # All the junk in the trunk not used for cache dep validation
        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            (lib.fileset.fileFilter (file: file.hasExt "rs") ./crates)
            (lib.fileset.fileFilter (file: file.hasExt "wesl") ./crates)
            (lib.fileset.fileFilter (file: file.hasExt "toml") ./crates)
            # This is stuff thats embedded beyond the bevy asset server
            (lib.fileset.fileFilter (file: file.hasExt "md") ./crates)
            (lib.fileset.fileFilter (file: file.hasExt "ttf") ./crates)
            (lib.fileset.fileFilter (file: file.hasExt "glb") ./crates)
            (lib.fileset.fileFilter (file: file.hasExt "mpk") ./crates)
            (lib.fileset.fileFilter (file: file.hasExt "json") ./crates)
            ./deny.toml
            ./Cargo.toml
            ./Cargo.lock
            ./.config/nextest.toml
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
            nixfmt
            rustfmt
            git
            nix
            treefmt
            convco
            ;
        };

        # Instead of running nix flake check on each commit (e.g. in
        # pre-commit), lets just be sure we're golden at push time.
        #
        # I can rewrite the commit history to fix it at that point if things
        # fail or not.
        git-hooks-check = inputs.git-hooks.lib.${system}.run {
          src = ./.;
          tools = hookTools;
          hooks = {
            nix-flake-check = {
              enable = true;
              name = "nix-flake-check";
              entry = "${pkgs.nix}/bin/nix flake check -L";
              language = "system";
              pass_filenames = false;
              stages = [ "pre-push" ];
              verbose = true;
            };
            commit-msg = {
              enable = true;
              name = "convco";
              entry = "${pkgs.lib.getExe pkgs.bash} -c '${pkgs.convco}/bin/convco check --from-stdin < \"$1\"' --";
              language = "system";
              stages = [ "commit-msg" ];
            };
            # Make sure code is formatted in pre-commit
            # Note: We use the formatter check separately, so we disable this
            # in the git-hooks check to avoid sandbox timestamp issues
            treefmt.enable = false;
          };
        };

        commonXinputs = with pkgs; [
          vulkan-loader
          libx11
          libxcursor
          libxi
          libxrandr
          libxkbcommon
          wayland
          alsa-lib
          udev
        ];

        # Common arguments can be set here to avoid repeating them later
        commonArgs = {
          inherit src;
          strictDeps = true;

          nativeBuildInputs = with pkgs; [
            git
            pkg-config
          ];

          buildInputs =
            with pkgs;
            [ ]
            ++ lib.optionals pkgs.stdenv.hostPlatform.isLinux [
              vulkan-loader
              wayland
              pkgs.mold
              pkgs.lld
            ]
            ++ lib.optionals pkgs.stdenv.hostPlatform.isLinux commonXinputs
            ++ lib.optionals pkgs.stdenv.isDarwin [
              apple-sdk
              rustPlatform.bindgenHook
              llvmPackages.libclang
            ];

          # Additional environment variables can be set directly
          LD_LIBRARY_PATH = lib.optionalString pkgs.stdenv.isLinux (lib.makeLibraryPath (commonXinputs));
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

          CARGO_BUILD_TARGET = "wasm32-unknown-unknown";
        };

        # Macos env vars shared between the derivation environment and the devshell env
        commonEnvDarwin = {
          LIBCLANG_PATH = lib.optionalString pkgs.stdenv.isDarwin "${pkgs.llvmPackages.libclang.lib}/lib";
        };

        darwinLldFlags = lib.optionalString pkgs.stdenv.isDarwin "-C link-arg=-fuse-ld=${pkgs.llvmPackages.lld}/bin/ld64.lld";

        # Common arguments for Darwin builds (system libraries only)
        commonArgsDarwin =
          if pkgs.stdenv.isDarwin then
            {
              inherit src;
              strictDeps = true;

              nativeBuildInputs = [
                pkgsDarwin.git
                pkgs.llvmPackages.lld
              ];

              buildInputs = with pkgsDarwin; [
                apple-sdk
                libiconv
              ];
            }
            // commonEnvDarwin
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

        # Use nixpkgs cudatoolkit for headers and compilation setup
        cudaMerged = if pkgs.stdenv.isLinux then pkgsCuda.cudaPackages.cudatoolkit else null;

        # Common args for the ma-cuda build, has all the cuda runtime junk in
        # its trunk so that cubec/burn-cuda can dlopen() and build at runtime.
        commonArgsCuda =
          if pkgs.stdenv.isLinux then
            {
              inherit src;
              strictDeps = true;

              nativeBuildInputs = with pkgsCuda; [
                git
                pkg-config
                # autoAddDriverRunpath patches ELF RPATH entries so the CUDA
                # libs are found at runtime even outside of NixOS, veeery needed.
                autoAddDriverRunpath
              ];

              buildInputs =
                with pkgsCuda;
                [
                  cudaPackages.cuda_cudart
                  cudaPackages.cuda_nvcc
                  cudaPackages.cuda_nvrtc
                  cudaPackages.cuda_cccl
                  cudaPackages.libcublas
                  vulkan-loader
                  wayland
                  mold
                  lld
                ]
                ++ commonXinputs;

              # Need to set this up for the cuda compilation to work, this is to
              # enable the compilation to work at runtime within the devshell.
              CUDA_PATH = "${cudaMerged}";
              LD_LIBRARY_PATH = lib.makeLibraryPath (
                commonXinputs
                ++ (with pkgsCuda; [
                  cudaPackages.cuda_cudart
                  cudaPackages.cuda_nvrtc
                  cudaPackages.libcublas
                ])
              );
            }
          else
            { };

        # Build *just* the cargo dependencies (of the entire workspace),
        # so we can reuse all of that work (e.g. via cachix) when running in CI
        # It is *highly* recommended to use something like cargo-hakari to avoid
        # cache misses when building individual top-level-crates
        # Note: buildDepsOnly already uses --all-targets by default
        # Important: Must use same env vars (especially RUSTFLAGS) as actual builds
        # Using dev profile by default for better debug info on panics
        cargoArtifacts = craneLib.buildDepsOnly (
          commonArgs
          // devArgs
          // {
            src = srcDeps;
          }
        );

        # Cargo artifacts for release builds
        cargoArtifactsRelease = craneLib.buildDepsOnly (
          commonArgs
          // releaseArgs
          // {
            src = srcDeps;
          }
        );

        # Cargo artifacts for WASM release builds. Note the `ma` crate is used
        # for burn inference and has a butt ton of stuff that doesn't build in a
        # wasm environment. I could probably get it to work without the tui
        # feature but this is a future sucker mitch task.
        cargoArtifactsWasm = craneLibWasm.buildDepsOnly (
          commonArgsWasm
          // releaseArgs
          // {
            src = srcDeps;
            cargoExtraArgs = "-p mitchty --features mitchty/webgl";
          }
        );

        # Cargo artifacts for WASM release-fast builds no lto or codegen unit restrictions
        cargoArtifactsWasmFast = craneLibWasm.buildDepsOnly (
          commonArgsWasm
          // releaseFastArgs
          // {
            src = srcDeps;
            cargoExtraArgs = "-p mitchty --features mitchty/webgl";
          }
        );

        # Cargo artifacts for WASM builds (debug)
        # Apparently yes, there is a limit and I've now hit it
        # Error loading app: CompileError: WebAssembly.instantiateStreaming(): size > maximum module size (1073741824): 1084424530 @+0
        # cargoArtifactsWasmDebug = craneLibWasm.buildDepsOnly (
        #   commonArgsWasm
        #   // nixEnvArgs
        #   // devArgs
        #   // {
        #     src = srcDeps;
        #   }
        # );

        # Cargo artifacts for Darwin builds (release)
        cargoArtifactsDarwin =
          if pkgs.stdenv.isDarwin then
            craneLibDarwin.buildDepsOnly (
              commonArgsDarwin
              // releaseArgs
              // {
                src = srcDeps;
                RUSTFLAGS = "${releaseArgs.RUSTFLAGS} ${darwinLldFlags}";
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

        # Cargo dep cache for the ma-cuda derivation specifically.
        cargoArtifactsMaCuda =
          if pkgs.stdenv.isLinux then
            craneLibCuda.buildDepsOnly (
              commonArgsCuda
              // nixEnvArgs
              // releaseArgs
              // {
                src = srcDeps;
                cargoExtraArgs = "-p ma --features ma/cuda";
              }
            )
          else
            null;

        # Release build of the ma-cuda binary using the burn CUDA backend for
        # slightly faster training.
        ma-cuda =
          if pkgs.stdenv.isLinux then
            craneLibCuda.buildPackage (
              commonArgsCuda
              // nixEnvArgs
              // releaseArgs
              // {
                pname = "ma-cuda";
                version = version;
                cargoArtifacts = cargoArtifactsMaCuda;
                cargoExtraArgs = "-p ma --bin ma --features ma/cuda";
                src = fileSetForCrate ./crates/ma;
                doCheck = false;
                meta = {
                  description = "ma training CLI CUDA flavor";
                  mainProgram = "ma";
                  platforms = [
                    "x86_64-linux"
                    "aarch64-linux"
                  ];
                };
              }
            )
          else
            null;

        version = self.rev or self.dirtyShortRev or "nix-flake-cant-get-git-commit-sha";

        individualCrateArgs = commonArgs // {
          inherit cargoArtifacts;
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
              (lib.fileset.fileFilter (file: file.hasExt "rs") ./crates)
              (lib.fileset.fileFilter (file: file.hasExt "toml") ./crates)
              (lib.fileset.fileFilter (file: file.hasExt "md") ./crates)
              (lib.fileset.fileFilter (file: file.hasExt "ktx2") ./crates)
              (lib.fileset.fileFilter (file: file.hasExt "ttf") ./crates)
              (lib.fileset.fileFilter (file: file.hasExt "glb") ./crates)
              (lib.fileset.fileFilter (file: file.hasExt "wgsl") ./crates)
              (lib.fileset.fileFilter (file: file.hasExt "wesl") ./crates)
              (lib.fileset.fileFilter (file: file.hasExt "mpk") ./crates)
              (lib.fileset.fileFilter (file: file.hasExt "json") ./crates)
            ];
          };

        webServerRuntimeInputs = [
          pkgs.python3
        ]
        ++ lib.optionals pkgs.stdenv.isDarwin [ pkgs.darwin.system_cmds ]
        ++ lib.optionals pkgs.stdenv.isLinux [ pkgs.xdg-utils ];

        # sh fragment to open a url on macos/linux, used in the web/web-release
        # apps for dev testing of wasm builds.
        #
        # Expects $URL to already be set by the caller including any query
        # string if given for the wasm test version.
        openBrowserScript = ''
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

        # Function to build and run the Windows cross-compiled binary through
        # steam-run + wine on Linux. Both steam-run and wineWow64Packages.full
        # are unfree; allowUnfree is set in pkgsUnfree so they can be pulled in
        # directly as runtimeInputs without any nix-shell --impure dance.
        # WINEPREFIX can be overridden by the caller's environment.
        mkWineExecApp =
          name: windowsPackage:
          pkgs.writeShellApplication {
            inherit name;
            runtimeInputs = [
              pkgsUnfree.steam-run
              pkgsUnfree.wineWow64Packages.full
            ];
            text = ''
              EXE="${windowsPackage}/bin/mitchty.exe"

              WINEPREFIX="''${WINEPREFIX:-$HOME/.wine64}"
              export WINEPREFIX

              echo "running $EXE via steam-run + wine"
              echo "  WINEPREFIX = $WINEPREFIX"

              steam-run wine "$EXE" "$@"
            '';
          };

        # Function to cover building local testing web app scripts for
        # debug/release builds in web/web-release apps.
        #
        # Any arguments passed after -- are treated as URL query parameters and
        # appended to the browser URL verbatim joined with &. Here to make this
        # a bit of a 1:1 mapping to the clap arg parsing. Not entirely sure I
        # like this approach but eh been working on this for over a week and
        # dgaf atm.
        #
        # Examples:
        #   nix run .#web-lto
        #   nix run .#web-lto -- show=recognizer
        #   nix run .#web-lto -- show=recognizer show=data-viewer
        #   nix run .#web-lto -- show=recognizer,data-viewer
        mkWebServerApp =
          name: wasmPackage: includeAssets:
          let
            buildType = if includeAssets then " debug build" else " release build";
            assetCopyScript =
              if includeAssets then
                ''
                  echo "copying assets for debug build"
                  mkdir -p "$TMPDIR/assets/crates/mitchty/src/assets"
                  cp -rv ${./crates/mitchty/src/assets}/. "$TMPDIR/assets/crates/mitchty/src/assets/"
                  chmod -R u+w "$TMPDIR/assets/crates/mitchty/src/assets"
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
              # Create temporary directory for serving
              WORK=$(mktemp -d)
              trap 'rm -rf "$WORK"' EXIT TERM INT QUIT

              echo "building wasm files for${buildType}"
              mkdir -p "$WORK/wasm"
              cp -rv ${wasmPackage}/wasm/* "$WORK/wasm/"
              cp ${./index.html} "$WORK/index.html"

              ${assetCopyScript}

              # Build the query string from any extra args passed after --.
              # ex: show=recognizer show=data-viewer  ->  ?show=recognizer&show=data-viewer
              QUERY=""
              for arg in "$@"; do
                if [ -z "$QUERY" ]; then
                  QUERY="?''${arg}"
                else
                  QUERY="''${QUERY}&''${arg}"
                fi
              done

              URL="http://localhost:8000''${QUERY}"

              echo "starting local webserver at ''${URL}"
              echo "press Ctrl+C to stop"

              cd "$WORK"

              # Start server in background
              python3 -m http.server 8000 &
              SERVER_PID=$!
              trap 'kill "''${SERVER_PID}" 2>/dev/null || true; rm -rf "$WORK"' EXIT

              # Give the server a bit to start before trying to open a browser
              # to it for it to just fail.
              sleep 1

              ${openBrowserScript}

              # Block on the python webserver so we can be ctrl-c'd on a whim.
              wait "''${SERVER_PID}"
            '';
          };

        # TODO: While this is now fixed for the dep caches, I need to probably also make the builds be deterministic so that say:
        # mitchty --version
        # mitchty 0.0.16 3717e04 debug rustc 1.94.0 built 2026-04-17 01:09:06 UTC
        # has a constant SOURCE_DATE_EPOCH, or maybe even drop it entirely. This
        # is, as is tradition a future mitch problem.
        nixEnvArgs = {
          NIX_GIT_REV = version;
          # Clippy lints can be set in source via attributes instead
        };

        devArgs = {
          CARGO_PROFILE = "dev";
        };

        releaseArgs = {
          CARGO_PROFILE = "release";
          RUSTFLAGS = "-D warnings";
        };

        # Like releaseArgs but uses the release-fast profile for quicker iteration
        releaseFastArgs = {
          CARGO_PROFILE = "release-fast";
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
        mitchty-unwrapped = craneLib.buildPackage (
          individualCrateArgs
          // nixEnvArgs
          // devArgs
          // {
            pname = "mitchty";
            cargoExtraArgs = "-p mitchty";
            src = fileSetForCrate ./crates/mitchty;
          }
        );

        # "fake" asset root based off of the current source for the debug
        # version of mitchty. I figure that I can just use cargo run outside of
        # nix to use dynamic loading of things. The nix version doesn't need to
        # deal with this. Maybe I just ditch the debug derivation.
        mitchty-dev-asset-root = pkgs.runCommand "mitchty-dev-asset-root" { } ''
          mkdir -p $out/crates/mitchty/src/assets
          cp -r ${./crates/mitchty/src/assets}/. $out/crates/mitchty/src/assets/
        '';

        # Wrap the debug binary so BEVY_ASSET_PATH points at the fake workspace
        # root saved above.
        mitchty = pkgs.symlinkJoin {
          name = "mitchty";
          paths = [ mitchty-unwrapped ];
          nativeBuildInputs = [ pkgs.makeWrapper ];
          postBuild = ''
            wrapProgram $out/bin/mitchty \
              --set BEVY_ASSET_PATH ${mitchty-dev-asset-root}
          '';
        };

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
            BEVY_ASSET_PATH = ./crates/mitchty/src/assets;
          }
        );

        # Release build of the plain ma binary with wgpu backend
        ma = craneLib.buildPackage (
          commonArgs
          // nixEnvArgs
          // releaseArgs
          // {
            pname = "ma";
            cargoArtifacts = cargoArtifactsRelease;
            cargoExtraArgs = "-p ma --bin ma";
            src = fileSetForCrate ./crates/ma;
            doCheck = false;
            meta = {
              description = "ma nn utility cli";
              mainProgram = "ma";
            };
          }
        );

        # This builds the mitchty derivation in release mode (I tried passing in
        # the binary etc.. but no bueno pugio needs to build crap on its own
        # laaaame). But this is here so I can get a deps svg out of the whole
        # build for later nefarious shenanigans.
        # TODO: Commented out for now, with nix-fast-build and .#packages.$SYS
        # always building this just makes things slower and I rarely need the
        # full dep graph anyway. If I can brain up a way to share the crane
        # cargo deps this would be fine but its effectively an independent build
        # vi cargo by the pugio binary at runtime.
        # ci-pugio-graph = craneLib.mkCargoDerivation (
        #   commonArgs
        #   // nixEnvArgs
        #   // releaseArgs
        #   // {
        #     pname = "ci-pugio-graph";
        #     cargoArtifacts = cargoArtifactsRelease;
        #     src = fileSetForCrate ./crates/mitchty;
        #     doCheck = false;
        #     doInstallCargoArtifacts = false;

        #     nativeBuildInputs = commonArgs.nativeBuildInputs ++ [
        #       pugio
        #       pkgs.cargo-bloat
        #       pkgs.fontconfig
        #       pkgs.graphviz
        #     ];

        #     # Pugio's SVG renderer uses fontconfig for text layout; without a
        #     # valid config it crashes before writing the output file. An empty
        #     # fonts.conf with no font dirs is enough to satisfy it.
        #     FONTCONFIG_FILE = pkgs.makeFontsConf { fontDirectories = [ ]; };

        #     buildPhaseCargoCommand = ''
        #       install -dm755 $out
        #       install -m644 /dev/null $out/deps.svg
        #       export CARGO_NET_OFFLINE=true
        #       pugio --package mitchty \
        #         --bin mitchty \
        #         --release \
        #         --scheme cum-sum \
        #         --gradient blues \
        #         --dark-mode \
        #         --no-open \
        #         --output "$out/deps.svg"
        #     '';
        #   }
        # );

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
                cargoExtraArgs = "-p mitchty --features mitchty/webgl";
                src = fileSetForCrate ./crates/mitchty;
                BEVY_ASSET_PATH = ./crates/mitchty/src/assets;

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

        # release-fast wasm build: same pipeline as mitchty-wasm-lto but uses
        # the release-fast profile, wasm-opt is skipped here as well under the
        # same rationale.
        mitchty-wasm =
          let
            wasmBuild = craneLibWasm.buildPackage (
              commonArgsWasm
              // nixEnvArgs
              // releaseFastArgs
              // {
                pname = "mitchty-wasm";
                version = version;
                cargoArtifacts = cargoArtifactsWasmFast;
                cargoExtraArgs = "-p mitchty --features mitchty/webgl";
                src = fileSetForCrate ./crates/mitchty;
                BEVY_ASSET_PATH = ./crates/mitchty/src/assets;

                doCheck = false;
                doInstallCargoArtifacts = false;
                installPhase = ''
                  runHook preInstall
                  mkdir -p $out
                  cp -r target/wasm32-unknown-unknown/release-fast $out/
                  runHook postInstall
                '';
              }
            );
          in
          pkgsWasm.runCommand "mitchty-wasm-bindgen"
            {
              nativeBuildInputs = [
                wasmBindgenCli
              ];
            }
            ''
              mkdir -p $out/wasm

              ${wasmBindgenCli}/bin/wasm-bindgen \
                --out-dir $out/wasm \
                --target web \
                --no-typescript \
                ${wasmBuild}/release-fast/mitchty.wasm
            '';

        # Darwin release build (system libraries only, portable)
        mitchty-release-darwin =
          if pkgs.stdenv.isDarwin then
            craneLibDarwin.buildPackage (
              commonArgsDarwin
              // nixEnvArgs
              // releaseArgs
              // {
                pname = "mitchty-release";
                version = version;
                cargoArtifacts = cargoArtifactsDarwin;
                cargoExtraArgs = "-p mitchty";
                src = fileSetForCrate ./crates/mitchty;
                RUSTFLAGS = "${releaseArgs.RUSTFLAGS} ${darwinLldFlags}";

                # Don't check during cross-compilation
                doCheck = false;

                # abuse install_name_tool to rewrite the dynamic link to
                # /nix/store to /usr/lib for iconv. Can't find an easy way to
                # convince the rust toolchain to not do this in nix so whatever
                # its FINE I think...
                postInstall = ''
                  for binary in $out/bin/*; do
                    libiconv_path=$(otool -L "$binary" | awk '/\/nix\/store.*libiconv/ {print $1}' || true)
                    if [ -n "$libiconv_path" ]; then
                      install_name_tool -change "$libiconv_path" /usr/lib/libiconv.2.dylib "$binary"
                    fi
                  done
                '';

                meta = metaCommon "release macos build" // {
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
          // nixEnvArgs
          // releaseArgs
          // {
            pname = "mitchty-release";
            version = version;
            cargoArtifacts = cargoArtifactsWindows;
            cargoExtraArgs = "-p mitchty";
            src = fileSetForCrate ./crates/mitchty;

            # Don't check during cross-compilation
            doCheck = false;

            meta = metaCommon "release windows x86_64 build";
          }
        );

        deny = craneLib.cargoDeny {
          inherit src;
          inherit (inputs) advisory-db;
          # Here so if there are missing versions in the deny setup treat them
          # as errors as well as other possible "mitch didn't run the full checks stuff"
          cargoDenyChecks = "-D unmatched-skip -D unnecessary-skip -D unmatched-skip-root bans licenses sources";
        };

        # Hacky way to abuse crane's deny setup to generate graphviz dot files
        # and then build pngs from it for any duplicate dependencies.
        dotdeps = craneLib.mkCargoDerivation {
          inherit src;
          pname = "dotdeps";

          nativeBuildInputs = [
            pkgs.graphviz
            cargo-deny-0_19_0
          ];

          # crane requires artifacts but like cargoDeny set this to null
          cargoArtifacts = null;
          doInstallCargoArtifacts = false;

          # Note: if there are any bans/dups etc... we let things go this is
          # intended to generate graphviz dot files.
          buildPhaseCargoCommand = ''
            mkdir -p "$out"
            cargo --offline deny check -g "$out" bans || true
          '';

          installPhaseCommand = ''
            for f in "$out"/graph_output/*.dot; do
              [ -e "$f" ] || continue
              dot -Tpng "$f" -o "$out/graph_output/$(basename "''${f%.dot}").png"
            done
          '';
        };

        cargo-deny-0_19_0 = pkgs.rustPlatform.buildRustPackage rec {
          pname = "cargo-deny";
          version = "0.19.0";

          src = pkgs.fetchFromGitHub {
            owner = "EmbarkStudios";
            repo = "cargo-deny";
            rev = version;
            hash = "sha256-kDjRP+UXYzsXTrcsPbomtATzDVTSZqXoRXf6qqCGOZw=";
          };

          cargoHash = "sha256-Lu1KhQmsQGvzgozFTcv9/hY3ZXOuaxkv0I+QPmAdZBU=";

          nativeBuildInputs = with pkgs; [ pkg-config ];
          buildInputs = with pkgs; [ zstd ] ++ lib.optionals stdenv.hostPlatform.isDarwin [ apple-sdk ];

          env = {
            ZSTD_SYS_USE_PKG_CONFIG = true;
          };

          # Tests require network access
          doCheck = false;

          meta = with lib; {
            description = "Cargo plugin to help you manage large dependency graphs";
            mainProgram = "cargo-deny";
          };
        };

        pugio = pkgs.rustPlatform.buildRustPackage rec {
          pname = "pugio";
          version = "0.2.0";

          src = pkgs.fetchCrate {
            inherit pname version;
            hash = "sha256-Eqc6Ferh5AUstigkLPRhf+xAZXFH3AEfaVjvlaPAJ/8=";
          };

          cargoHash = "sha256-RC5dPLuA32VTLk2GVFnjJ+ijl64+HYHWY6pYrIUk0Rw=";

          nativeBuildInputs = with pkgs; [ pkg-config ];
          buildInputs = with pkgs; [ ] ++ lib.optionals stdenv.hostPlatform.isDarwin [ apple-sdk ];

          doCheck = false;

          meta = with lib; {
            description = "Binary size profiler for ELF, Mach-O, PE, and WASM binaries";
            mainProgram = "pugio";
            homepage = "https://github.com/Gnarus-G/pugio";
            license = with licenses; [ mit ];
          };
        };
      in
      {
        checks = {
          inherit deny;
          formatter = treefmtEval.config.build.check self;
          git-hooks = git-hooks-check;

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
              cargoNextestPartitionsExtraArgs = "--no-tests=pass --profile ci";
              # -v so I can try debugging the cargo nextest build hang in github runners
              cargoExtraArgs = "-v";
              # Dump more stuff from wgpu and naga to try debugging gh runner stuff.
              RUST_LOG = "wgpu=warn,naga=warn";
            }
            # On Linux force wgpu to use vulkan software renderer in case this
            # is the problem.
            // lib.optionalAttrs pkgs.stdenv.isLinux {
              WGPU_BACKEND = "vulkan";
              WGPU_POWER_PREFERENCE = "none";
            }
          );
        };

        packages = {
          inherit
            mitchty
            mitchty-lto
            ma
            mitchty-wasm
            mitchty-wasm-lto
            pugio
            dotdeps
            ;
          wasm-bindgen-cli = wasmBindgenCli;
          default = mitchty;
          # Expose checks as packages for individual running with shorter names
          clippy = self.checks.${system}.mitchty-clippy;
          doc = self.checks.${system}.mitchty-doc;
          nextest = self.checks.${system}.mitchty-nextest;
          deny = self.checks.${system}.deny;
        }
        // lib.optionalAttrs pkgs.stdenv.isLinux {
          inherit mitchty-release-windows;
          inherit ma-cuda;
          wine = pkgsUnfree.wineWow64Packages.full;
          steam-run = pkgsUnfree.steam-run;
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
          ma = {
            type = "app";
            program = "${ma}/bin/ma";
            meta = {
              description = "ma nn utility cli";
              mainProgram = "ma";
            };
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

          # Hacky flake app to open the graph_output dir if its not empty
          dotdeps =
            let
              graphDir = "${dotdeps}/graph_output";
              opener = if pkgs.stdenv.isDarwin then "open" else "xdg-open";
            in
            {
              type = "app";
              program = "${
                pkgs.writeShellApplication {
                  name = "dotdeps";
                  text = ''
                    dir="${graphDir}"
                    if [ -z "$(ls -A "$dir" 2>/dev/null)" ]; then
                      echo "no duplicate cargo deps found in $dir, nothing to do."
                    else
                      ${opener} "$dir"
                    fi
                  '';
                }
              }/bin/dotdeps";
              meta = {
                description = "Open cargo-deny duplicate-dep graphs in a browser if any";
                mainProgram = "dotdeps";
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
                  ${pkgs.nix}/bin/nix flake update
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
          # Serve WASM release-fast build locally for quick iteration testing
          # nix run .#web
          web = {
            type = "app";
            program = "${mkWebServerApp "web" mitchty-wasm false}/bin/web";
            meta = {
              description = "Serve WASM release-fast build (no LTO) locally for testing";
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
              meta = metaCommon "run release cross compiled windows build";
            };

          # Build the Windows binary and launch it via wine on Linux.
          # Usage: nix run .#mitchty-wine-exec
          mitchty-wine-exec = {
            type = "app";
            program = "${mkWineExecApp "mitchty-wine-exec" mitchty-release-windows}/bin/mitchty-wine-exec";
            meta = metaCommon "run windows build via wine" // {
              platforms = [ "x86_64-linux" ];
            };
          };

          ma-cuda = {
            type = "app";
            program = "${ma-cuda}/bin/ma";
            meta = {
              description = "ma training CLI - burn CUDA backend";
              mainProgram = "ma";
              platforms = [
                "x86_64-linux"
                "aarch64-linux"
              ];
            };
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

        devShells.default =
          craneLib.devShell {
            checks = lib.filterAttrs (
              n: _:
              !lib.elem n [
                # Filtered out cause they cause the build settings to bleed
                # through to the devshell. We only want them for things like
                # checks not the devshell. If people want to build either, nix
                # build .#whatever, its setup right and works. The devshells for
                # local only development/testing with cargo build.
                "mitchty-release-windows"
                "mitchty-wasm-lto"
              ]
            ) self.checks.${system};

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
                pugio
                graphviz
                stableRust
                wasm-bindgen-cli
                binaryen
                wasm-pack
              ]
              ++ [
                cargo-deny-0_19_0
                pugio
              ]
              ++ (lib.attrValues hookTools)
              ++ commonArgs.buildInputs
              ++ commonArgs.nativeBuildInputs
              # CUDA 12 dev tools for linux devshell abusage
              ++ lib.optionals pkgs.stdenv.isLinux (
                with pkgsCuda;
                [
                  cudaPackages.cuda_cudart
                  cudaPackages.cuda_nvcc
                  cudaPackages.cuda_nvrtc
                  cudaPackages.cuda_cccl
                  cudaPackages.libcublas
                  autoAddDriverRunpath
                ]
              )
            );

            shellHook = ''
              ${git-hooks-check.shellHook}
              # For local devshell related via direnv builds with say cargo, be
              # sure that the bevy asset loader can find the local assets at
              # runtime.
              export BEVY_ASSET_PATH="$PWD"
            '';

            # Make sure eglot+etc.. pick the right rust-src for eglot+lsp mode stuff using direnv
            RUST_SRC_PATH = "${stableRust}/lib/rustlib/src/rust/library";

            # Set library path for Bevy and on linux cuda crap
            LD_LIBRARY_PATH =
              commonArgs.LD_LIBRARY_PATH
              + lib.optionalString pkgs.stdenv.isLinux (
                ":"
                + lib.makeLibraryPath (
                  with pkgsCuda;
                  [
                    cudaPackages.cuda_cudart
                    cudaPackages.cuda_nvrtc
                    cudaPackages.libcublas
                  ]
                )
              );

            # CUDA toolkit path used to find headers at runtime for jit compilation
            CUDA_PATH = lib.optionalString pkgs.stdenv.isLinux "${cudaMerged}";
          }
          // lib.optionalAttrs pkgs.stdenv.isDarwin commonEnvDarwin;
      }
    );

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

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

    # For overlays.cargo-unused-features overlay which gives me 2024 edition
    # support.
    mitchty.url = "github:mitchty/nix";
  };
}
