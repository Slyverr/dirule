{
  description = "Local-first filesystem rules CLI";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          dirule = pkgs.rustPlatform.buildRustPackage {
            pname = "dirule";
            version = cargoToml.workspace.package.version;
            src = pkgs.lib.cleanSource ./.;

            nativeBuildInputs = [ pkgs.git ];

            cargoLock.lockFile = ./Cargo.lock;
            cargoBuildFlags = [
              "-p"
              "dirule"
            ];
            cargoTestFlags = [ "--workspace" ];

            meta = {
              description = "Local-first filesystem rules CLI";
              license = pkgs.lib.licenses.mit;
              mainProgram = "dirule";
            };
          };
        in
        {
          default = dirule;
          dirule = dirule;
        }
      );

      apps = forAllSystems (
        system:
        let
          package = self.packages.${system}.default;
        in
        {
          default = {
            type = "app";
            program = "${package}/bin/dirule";
            meta = {
              description = "Local-first filesystem rules CLI";
              mainProgram = "dirule";
            };
          };
        }
      );

      devShells = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              cargo
              cargo-deny
              clippy
              git
              rust-analyzer
              rustc
              rustfmt
            ];
          };
        }
      );

      formatter = forAllSystems (system: nixpkgs.legacyPackages.${system}.nixfmt);
    };
}
