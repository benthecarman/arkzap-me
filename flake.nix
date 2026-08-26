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

            cargoHash = "sha256-j99I7bnzyHQv5iH9uGUV57bpnot+Uu/EmMQCEEvg3iQ=";

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
            GIT_HASH = "09e5b81d220d16f1e876df4aa772185640301528";

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
