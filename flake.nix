{
  description = "Flake providing Vox World, a multiplayer voxel RPG written in Rust.";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    nci = {
      url = "github:90-008/nix-cargo-integration";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.parts.follows = "parts";
      inputs.dream2nix.follows = "d2n";
      inputs.crane.follows = "crane";
    };
    parts = {
      url = "github:hercules-ci/flake-parts";
      inputs.nixpkgs-lib.follows = "nixpkgs";
    };
    d2n = {
      url = "github:NeuralModder/dream2nix/git-fetcher-no-shallow";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    crane = {
      url = "github:ipetkov/crane/v0.21.0";
      flake = false;
    };
  };

  outputs = inp: let
    lib = inp.nci.inputs.nixpkgs.lib;

    git = let
      sourceInfo = inp.self.sourceInfo;
      shortRev = lib.strings.concatStrings (lib.lists.take 8 (lib.strings.stringToCharacters (sourceInfo.rev or sourceInfo.dirtyRev)));
    in {
      version = "/" + shortRev + "/" + toString sourceInfo.lastModified;
    };

    filteredSource = let
      pathsToIgnore = [
        "flake.nix"
        "flake.lock"
        "nix"
        "assets"
        "README.md"
        "CONTRIBUTING.md"
        "CHANGELOG.md"
        "CODE_OF_CONDUCT.md"
        ".github"
        ".gitlab"
      ];
      ignorePaths = path: type: let
        split = lib.splitString "/" path;
        actual = lib.drop 4 split;
        _path = lib.concatStringsSep "/" actual;
      in
        lib.all (n: ! (lib.hasPrefix n _path)) pathsToIgnore;
    in
      builtins.path {
        name = "voxworld-source";
        path = toString ./.;
        # filter out unnecessary paths
        filter = ignorePaths;
      };
  in
    inp.parts.lib.mkFlake {inputs = inp;} {
      imports = [inp.nci.flakeModule];
      systems = ["x86_64-linux"];
      perSystem = {
        config,
        pkgs,
        lib,
        ...
      }: let
        checkIfLfsIsSetup = checkFile: ''
          checkFile="${checkFile}"
          result="$(${pkgs.file}/bin/file --mime-type $checkFile)"
          if [ "$result" = "$checkFile: image/jpeg" ]; then
            echo "Git LFS seems to be setup properly."
            true
          else
            echo "
              Git Large File Storage (git-lfs) has not been set up correctly.
              Most common reasons:
                - git-lfs was not installed before cloning this repository.
                - This repository was not cloned from the primary GitLab mirror.
                - The GitHub mirror does not support LFS.
              Check the Vox World repository documentation for details.
              Run 'nix-shell -p git git-lfs --run \"git lfs install --local && git lfs fetch && git lfs checkout\"'
              or 'nix shell nixpkgs#git-lfs nixpkgs#git -c sh -c \"git lfs install --local && git lfs fetch && git lfs checkout\"'.
            "
            false
          fi
        '';
        assets = pkgs.runCommand "voxworld-assets" {} ''
          mkdir $out
          ln -sf ${./assets} $out/assets
          ${checkIfLfsIsSetup "$out/assets/voxygen/background/bg_main.jpg"}
        '';
        wrapWithAssets = old:
          pkgs.runCommand
          old.name
          {
            meta = old.meta or {};
            passthru =
              (old.passthru or {})
              // {
                unwrapped = old;
              };
            nativeBuildInputs = [pkgs.makeWrapper];
          }
          ''
            cp -rs --no-preserve=mode,ownership ${old} $out
            wrapProgram $out/bin/* \
              --set VELOREN_ASSETS ${assets} \
              --set VELOREN_GIT_VERSION "${git.version}" \
          '';
        voxworld-common-env = {
          # We don't add in any information here because otherwise anything
          # that depends on common will be recompiled. We will set these in
          # our wrapper instead.
          VELOREN_GIT_VERSION = "/0/0";
          VOXWORLD_USERDATA_STRATEGY = "system";
        };
        voxygenOut = config.nci.outputs."voxworld-client";
        serverCliOut = config.nci.outputs."voxworld-server-cli";
      in {
        packages.voxworld-client = wrapWithAssets voxygenOut.packages.release;
        packages.voxworld-client-dev = wrapWithAssets voxygenOut.packages.dev;
        packages.voxworld-client-tlto = wrapWithAssets voxygenOut.packages.release-thinlto;
        packages.voxworld-server-cli = wrapWithAssets serverCliOut.packages.release;
        packages.voxworld-server-cli-dev = wrapWithAssets serverCliOut.packages.dev;
        packages.voxworld-server-cli-tlto = wrapWithAssets serverCliOut.packages.release-thinlto;
        packages.default = config.packages."voxworld-client";

        devShells.default = config.nci.outputs."voxworld".devShell.overrideAttrs (old: {
          VELOREN_ASSETS = "";
          shellHook = ''
            ${checkIfLfsIsSetup "$PWD/assets/voxygen/background/bg_main.jpg"}
            if [ $? -ne 0 ]; then
              exit 1
            fi
            export VELOREN_ASSETS="$PWD/assets"
            export VELOREN_GIT_VERSION="${git.version}"
          '';
        });

        nci.projects."voxworld" = {
          export = false;
          path = filteredSource;
        };
        nci.crates."voxworld-server-cli" = rec {
          profiles = {
            release.features = ["default-publish"];
            release.runTests = false;
            dev.features = ["default-publish"];
            dev.runTests = false;
            release-thinlto.features = ["default-publish"];
            release-thinlto.runTests = false;
          };
          depsDrvConfig.mkDerivation.nativeBuildInputs = [pkgs.mold];
          drvConfig = {
            mkDerivation = depsDrvConfig.mkDerivation;
            env = voxworld-common-env;
          };
        };
        nci.crates."voxworld-client" = rec {
          profiles = {
            release.features = ["default-publish"];
            release.runTests = false;
            dev.features = ["default-publish"];
            dev.runTests = false;
            release-thinlto.features = ["default-publish"];
            release-thinlto.runTests = false;
          };
          runtimeLibs = with pkgs; [
            wayland
            wayland-protocols
            libX11
            libXi
            libxcb
            libXcursor
            libXrandr
            libxkbcommon
            shaderc.lib
            udev
            alsa-lib
            vulkan-loader
            stdenv.cc.cc.lib
          ];
          depsDrvConfig = {
            env =
              voxworld-common-env
              // {
                SHADERC_LIB_DIR = "${pkgs.shaderc.lib}/lib";
              };
            mkDerivation = {
              buildInputs = with pkgs; [
                alsa-lib
                libxkbcommon
                udev
                libxcb

                fontconfig
              ];
              nativeBuildInputs = with pkgs; [
                python3
                pkg-config
                cmake
                gnumake
                mold
              ];
            };
          };
          drvConfig = {
            env =
              depsDrvConfig.env
              // {
                dontUseCmakeConfigure = true;
                VOXYGEN_NULL_SOUND_PATH = ./assets/voxygen/audio/null.ogg;
              };
            mkDerivation =
              depsDrvConfig.mkDerivation
              // {
                prePatch = ''
                                sed -i 's:"../../../assets/voxygen/audio/null.ogg":env!("VOXYGEN_NULL_SOUND_PATH"):' \
                  voxygen/src/audio/soundcache.rs
                '';
              };
            rust-crane.buildFlags = ["--bin=voxworld-client"];
          };
        };
      };
    };
}
