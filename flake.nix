{
  description = "Local-first filesystem rules CLI";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
  };

  outputs =
    inputs@{ flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = import ./nix/systems.nix;

      imports = [
        ./nix/parts/development.nix
        ./nix/parts/formatter.nix
        ./nix/parts/package.nix
      ];
    };
}
