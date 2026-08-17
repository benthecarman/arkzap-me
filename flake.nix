{
  description = "arkzap-me LNURL server";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      supportedSystems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.rustPlatform.buildRustPackage {
            pname = "arkzap-me";
            version = "0.1.0";

            src = pkgs.lib.fileset.toSource {
              root = ./.;
              fileset = pkgs.lib.fileset.unions [
                ./Cargo.toml
                ./Cargo.lock
                ./src
                ./migrations
              ];
            };

            cargoHash = "sha256-1GqHz52wjaETMkZ8eccvRmSQ0k+L/usq03pZT9e3M68=";

            nativeBuildInputs = [
              pkgs.pkg-config
              pkgs.protobuf
            ];
            buildInputs = [
              pkgs.openssl
              pkgs.postgresql.lib
            ];

            # bark-rest's build script otherwise tries to inspect a .git directory,
            # which is intentionally absent from Nix's vendored Cargo sources.
            GIT_HASH = "3ec92d287a65195d73463c7f186ed4b1387fbf57";

            meta = {
              description = "LNURL server for Bark and Arkade";
              homepage = "https://arkzap.me";
              license = pkgs.lib.licenses.mit;
              mainProgram = "arkzap-me";
            };
          };
        }
      );

      apps = forAllSystems (system: {
        default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/arkzap-me";
          meta = self.packages.${system}.default.meta;
        };
      });

      devShells = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.mkShell {
            inputsFrom = [ self.packages.${system}.default ];
            packages = [
              pkgs.cargo
              pkgs.clippy
              pkgs.rustc
              pkgs.rustfmt
            ];
          };
        }
      );

      nixosModules.default = import ./nix/module.nix self;
    };
}
