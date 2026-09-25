//! Build-time capture of the exact target triple (0058 WK05/WK08): the
//! handshake's EndpointInfo.target must be the compile-time triple the
//! binary was built for, so deployment can bind a worker to the exact
//! release/target it verified — the `{arch}-{os}` shorthand cannot
//! distinguish musl from gnu and is not a catalog key.

fn main() {
    let target = std::env::var("TARGET").expect("cargo sets TARGET for build scripts");
    println!("cargo:rustc-env=STROP_TARGET_TRIPLE={target}");
    println!("cargo:rerun-if-env-changed=TARGET");
}
