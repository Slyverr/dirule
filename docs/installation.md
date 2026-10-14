# Installation

## From a checkout

Install the CLI from the repository root with Cargo:

```sh
cargo install --path crates/dirule --locked
```

Make sure Cargo's binary directory is on `PATH` to run `dirule` from any project.

## From a release

On Linux or macOS, run the shell installer:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/Slyverr/dirule/releases/latest/download/dirule-installer.sh | sh
```

On Windows, run the PowerShell installer:

```powershell
powershell -c "irm https://github.com/Slyverr/dirule/releases/latest/download/dirule-installer.ps1 | iex"
```

The generated shell and PowerShell installers do not verify checksums. For manual installation, releases also provide archives and SHA-256 checksum files. Download an archive and its matching `.sha256` file into the same directory, then verify it on Linux or macOS with:

```sh
sha256sum -c dirule-x86_64-unknown-linux-gnu.tar.xz.sha256
```

Use the matching checksum filename for the archive you downloaded.

On Windows, compare the archive's checksum with the value in its `.sha256` file using `Get-FileHash -Algorithm SHA256`.

## Nix

Run or install Dirule from GitHub:

```sh
nix run github:Slyverr/dirule -- --help
nix profile install github:Slyverr/dirule
```

The flake is the installation path for NixOS; the shell installer does not support it. From a checkout, use `nix run . -- --help`, enter the development shell with `nix develop`, or install to your user profile with `nix profile add .`.

The flake supports x86_64 and ARM64 Linux, plus ARM64 macOS. On Intel macOS, install from source with Cargo or use a release archive.

## Shell completions

Generate a completion script with `dirule completions SHELL`. Supported shells are Bash, Elvish, Fish, PowerShell, and Zsh. Redirect the output to the completion location used by your shell.

For Bash with `bash-completion`, install it for your user with:

```sh
mkdir -p ~/.local/share/bash-completion/completions
dirule completions bash > ~/.local/share/bash-completion/completions/dirule.bash
```

For Fish, use:

```sh
mkdir -p ~/.config/fish/completions
dirule completions fish > ~/.config/fish/completions/dirule.fish
```

If you set `XDG_DATA_HOME` or `XDG_CONFIG_HOME`, use the corresponding directory under that location instead.
