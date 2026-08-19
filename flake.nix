{
  description = ''
    solcore-rs dev shell.

    Goal: build/run the compiler + the playground without depending on
    whatever Rust/Node/npm/wasm-pack happen to be (or not be) installed on
    the host. Everything below comes from the Nix store, pinned by
    flake.lock, so `nix develop` gives the same toolchain on any machine and
    never touches a system-wide npm.
  '';

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, rust-overlay }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
      forEachSystem = nixpkgs.lib.genAttrs systems;
      pkgsFor = system: import nixpkgs {
        inherit system;
        overlays = [ rust-overlay.overlays.default ];
      };
    in
    {
      devShells = forEachSystem (system:
        let
          pkgs = pkgsFor system;

          # Reads ./rust-toolchain.toml so the Nix pin and the rustup pin
          # (used by CI and by anyone without this flake) can never drift
          # apart. rust-toolchain.toml itself doesn't list the wasm target
          # (CI adds it via a separate `rustup target add`), so it's added
          # here explicitly.
          rustToolchain = (pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml).override {
            targets = [ "wasm32-unknown-unknown" ];
          };
        in
        {
          default = pkgs.mkShell {
            packages = [
              rustToolchain
              pkgs.nodejs_22 # bundles npm; matches CI's `node-version: "22"`
              pkgs.wasm-pack # NOTE: nixpkgs tracks upstream wasm-pack releases,
                              # not necessarily CI's pinned 0.15.0 exactly -
                              # `wasm-pack --version` in the shell to check. For
                              # byte-for-byte CI parity: `cargo install
                              # wasm-pack --version 0.15.0 --locked` once inside
                              # this shell (uses the pinned rustc/cargo above,
                              # installs to ~/.cargo/bin ahead of the nixpkgs one
                              # on PATH).
            ];

            # Keep npm's cache inside the repo instead of ~/.npm, so nothing
            # this shell does reaches outside the project or a prior/global
            # npm setup. `binaryen` (wasm-opt) is still installed as an npm
            # devDependency per package.json/package-lock.json - that part is
            # unchanged, just now running under a hermetic, pinned node/npm
            # rather than whatever was already on $PATH.
            shellHook = ''
              export npm_config_cache="$PWD/.npm-cache"
              echo "solcore-rs dev shell:"
              echo "  $(rustc --version)"
              echo "  $(node --version) / npm $(npm --version)"
              echo "  $(wasm-pack --version)"
            '';
          };
        });
    };
}
