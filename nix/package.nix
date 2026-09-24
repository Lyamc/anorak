{ lib
, rustPlatform
, openssl
, pkg-config
}:

rustPlatform.buildRustPackage {
  pname = "anorak";
  version = "0.1.0";

  src = lib.cleanSource ../.;
  cargoLock.lockFile = ../Cargo.lock;

  nativeBuildInputs = [ pkg-config ];
  buildInputs = [ openssl ];

  # tests/test_query.rs compares StatusCode values from two different
  # versions of the http crate, so `cargo test` does not compile.
  doCheck = false;

  postInstall = ''
    mkdir -p $out/share/anorak
    cp -r assets $out/share/anorak/assets
  '';

  meta = {
    description = "Search a Torznab indexer and send results to Transmission";
    mainProgram = "anorak";
    license = lib.licenses.gpl3Plus;
  };
}
