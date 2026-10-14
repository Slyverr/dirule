{
  lib,
  rustPlatform,
  git,
}:

let
  manifest = builtins.fromTOML (builtins.readFile ../Cargo.toml);
in
rustPlatform.buildRustPackage {
  pname = "dirule";
  version = manifest.workspace.package.version;
  src = lib.cleanSource ../.;

  nativeBuildInputs = [ git ];

  cargoLock.lockFile = ../Cargo.lock;
  cargoBuildFlags = [
    "-p"
    "dirule"
  ];
  cargoTestFlags = [ "--workspace" ];

  meta = {
    description = "Local-first filesystem rules CLI";
    license = lib.licenses.mit;
    mainProgram = "dirule";
  };
}
