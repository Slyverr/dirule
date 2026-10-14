{ ... }:

{
  perSystem =
    { pkgs, ... }:
    {
      devShells.default = pkgs.mkShell {
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
    };
}
